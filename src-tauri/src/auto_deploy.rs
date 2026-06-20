use crate::{
    deploy,
    models::{AutoDeployRecord, DeployStatus, Id, Machine, Project},
    state::AppState,
    storage,
};
use chrono::Utc;
use serde::Serialize;
use std::{collections::HashMap, path::Path, time::Duration};
use tauri::{AppHandle, Emitter};
use tokio::{
    process::Command,
    task::JoinSet,
    time::{interval, timeout, MissedTickBehavior},
};

const POLL_INTERVAL_SECS: u64 = 60;
const PER_PROJECT_TIMEOUT_SECS: u64 = 15;
/// After this many consecutive failed auto-deploys of the same SHA, stop
/// retrying until the remote SHA moves. See MEDIA_GUARD_TECHDEBT_PLAN P2.
const MAX_AUTO_DEPLOY_ATTEMPTS: u32 = 8;
const AUTO_DEPLOY_BACKOFF_MAX_EXP: u32 = 5;
const AUTO_DEPLOY_BACKOFF_CAP_SECS: u64 = 1800; // 30 min

/// Exponential backoff between retries of a failing same-SHA deploy: ~60s, 120s,
/// 240s, … capped at 30 min. `failures` is the count of prior failed attempts.
fn auto_deploy_backoff(failures: u32) -> chrono::Duration {
    let exp = failures.saturating_sub(1).min(AUTO_DEPLOY_BACKOFF_MAX_EXP);
    let secs = POLL_INTERVAL_SECS
        .saturating_mul(1u64 << exp)
        .min(AUTO_DEPLOY_BACKOFF_CAP_SECS);
    chrono::Duration::seconds(secs as i64)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoDeployTriggered {
    project_id: Id,
    project_name: String,
    branch: String,
    commit_sha: String,
    commit_sha_short: String,
}

pub fn start_auto_deploy_poller(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = interval(Duration::from_secs(POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            poll_all_projects(&app, &state).await;
        }
    });
}

async fn poll_all_projects(app: &AppHandle, state: &AppState) {
    let (projects, machines): (Vec<Project>, HashMap<Id, Machine>) = {
        let config = state.config.read().await;
        let projects: Vec<Project> = config
            .projects
            .iter()
            .filter(|p| p.auto_deploy)
            .cloned()
            .collect();
        let machines: HashMap<Id, Machine> = config
            .machines
            .iter()
            .map(|m| (m.id.clone(), m.clone()))
            .collect();
        (projects, machines)
    };
    if projects.is_empty() {
        return;
    }
    let mut set = JoinSet::new();
    for project in projects {
        let app = app.clone();
        let state = state.clone();
        let machine = project
            .machine_id
            .as_ref()
            .and_then(|id| machines.get(id))
            .cloned();
        set.spawn(async move {
            poll_project(app, state, project, machine).await;
        });
    }
    while set.join_next().await.is_some() {}
}

async fn poll_project(
    app: AppHandle,
    state: AppState,
    project: Project,
    machine: Option<Machine>,
) {
    {
        let deploy_states = state.runtime.deploy_states.read().await;
        if let Some(run) = deploy_states.get(&project.id) {
            if matches!(run.status, DeployStatus::Running) {
                return;
            }
        }
    }

    let is_local = machine.as_ref().map(|m| m.is_default_local).unwrap_or(true);
    if is_local && !Path::new(&project.root_path).join(".git").exists() {
        return;
    }

    let (branch, remote_sha) =
        match resolve_remote_head(&project.root_path, machine.as_ref()).await {
            Some(pair) => pair,
            None => {
                eprintln!(
                    "[auto_deploy] {} ({}): no main/master remote ref",
                    project.name, project.root_path
                );
                return;
            }
        };

    let previous = {
        let config = state.config.read().await;
        config.auto_deploy_state.get(&project.id).cloned()
    };

    // Decide whether this SHA has already been successfully deployed.
    // - For new records (`last_succeeded_commit` populated): trust it directly.
    // - For legacy records from before that field existed: fall back to
    //   `last_attempted_commit` to preserve existing behaviour.
    let already_synced = previous.as_ref().map(|record| match &record.last_succeeded_commit {
        Some(succeeded) => succeeded == &remote_sha,
        None => record.last_attempted_commit == remote_sha,
    });

    match (previous, already_synced) {
        (None, _) => {
            seed_record(&app, &state, &project.id, &branch, &remote_sha).await;
        }
        (Some(_), Some(true)) => {}
        (Some(record), _) => {
            // A deploy that keeps failing on the same SHA must NOT be re-triggered
            // every 60s forever (that retry storm is what fed the self-deploy loop
            // incident). Apply exponential backoff keyed on `failed_attempts` and
            // give up after a hard cap until the remote SHA moves. A new SHA resets
            // the counter (same_sha = false). See MEDIA_GUARD_TECHDEBT_PLAN P2.
            let same_sha = record.last_attempted_commit == remote_sha;
            let prior_failures = if same_sha { record.failed_attempts } else { 0 };
            if same_sha && prior_failures >= MAX_AUTO_DEPLOY_ATTEMPTS {
                return; // give up until the remote SHA changes
            }
            if same_sha && prior_failures > 0 {
                let backoff = auto_deploy_backoff(prior_failures);
                if Utc::now().signed_duration_since(record.last_attempted_at) < backoff {
                    return; // still inside the backoff window for this failing SHA
                }
            }
            // Record the attempt before triggering — the SHA, branch and timestamp
            // are useful for the UI even if the deploy never finishes. The success
            // tracking is updated separately by `execute_pipeline` so a failed
            // deploy will be retried (with backoff) on a later poll.
            record_attempt(
                &app,
                &state,
                &project.id,
                &branch,
                &remote_sha,
                prior_failures + 1,
            )
            .await;
            let commit_short: String = remote_sha.chars().take(7).collect();
            let _ = app.emit(
                "auto_deploy_triggered",
                AutoDeployTriggered {
                    project_id: project.id.clone(),
                    project_name: project.name.clone(),
                    branch: branch.clone(),
                    commit_sha: remote_sha.clone(),
                    commit_sha_short: commit_short,
                },
            );
            if let Err(err) =
                deploy::start_deployment(app.clone(), state.clone(), project.id.clone()).await
            {
                eprintln!(
                    "[auto_deploy] {}: failed to start deploy: {}",
                    project.name, err.message
                );
            }
        }
    }
}

/// First-time observation of a project: presume the working tree is already at
/// the remote SHA (we don't want to trigger a deploy on first sight just because
/// we have no history). Record both "attempted" and "succeeded" as the current
/// remote SHA so subsequent polls treat this as a no-op until the remote moves.
async fn seed_record(
    app: &AppHandle,
    state: &AppState,
    project_id: &Id,
    branch: &str,
    sha: &str,
) {
    let mut config = state.config.write().await;
    config.auto_deploy_state.insert(
        project_id.clone(),
        AutoDeployRecord {
            last_attempted_commit: sha.to_string(),
            branch: branch.to_string(),
            last_attempted_at: Utc::now(),
            last_succeeded_commit: Some(sha.to_string()),
            last_failure_notified_commit: None,
            failed_attempts: 0,
        },
    );
    if let Err(err) = storage::save_config(app, &config) {
        eprintln!("[auto_deploy] save_config (seed) failed: {}", err.message);
    }
}

/// Update only the "attempted" half of the record. `last_succeeded_commit` is
/// intentionally left alone — it is updated separately by `execute_pipeline`
/// after a successful deploy. This is what enables retry-on-failure: if a
/// deploy fails (or the orchestrator restarts mid-deploy), the next poll will
/// still see `last_succeeded_commit != remote_sha` and trigger again.
async fn record_attempt(
    app: &AppHandle,
    state: &AppState,
    project_id: &Id,
    branch: &str,
    sha: &str,
    failed_attempts: u32,
) {
    let mut config = state.config.write().await;
    let existing_succeeded = config
        .auto_deploy_state
        .get(project_id)
        .and_then(|record| record.last_succeeded_commit.clone());
    let existing_notified = config
        .auto_deploy_state
        .get(project_id)
        .and_then(|record| record.last_failure_notified_commit.clone());
    config.auto_deploy_state.insert(
        project_id.clone(),
        AutoDeployRecord {
            last_attempted_commit: sha.to_string(),
            branch: branch.to_string(),
            last_attempted_at: Utc::now(),
            last_succeeded_commit: existing_succeeded,
            last_failure_notified_commit: existing_notified,
            failed_attempts,
        },
    );
    if let Err(err) = storage::save_config(app, &config) {
        eprintln!("[auto_deploy] save_config (trigger) failed: {}", err.message);
    }
}

async fn resolve_remote_head(
    root_path: &str,
    machine: Option<&Machine>,
) -> Option<(String, String)> {
    for branch in ["main", "master"] {
        if let Some(sha) = ls_remote_sha(root_path, branch, machine).await {
            return Some((branch.to_string(), sha));
        }
    }
    None
}

async fn ls_remote_sha(
    root_path: &str,
    branch: &str,
    machine: Option<&Machine>,
) -> Option<String> {
    let ref_path = format!("refs/heads/{branch}");
    let is_remote = machine.map(|m| !m.is_default_local).unwrap_or(false);
    let result = if is_remote {
        let m = machine.expect("checked is_remote");
        let target = format!("{}@{}", m.ssh_user, m.hostname);
        let remote_cmd = format!(
            "git -C {} ls-remote origin {}",
            shell_quote(root_path),
            shell_quote(&ref_path)
        );
        let mut cmd = Command::new("ssh");
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "StrictHostKeyChecking=accept-new",
            &target,
            &remote_cmd,
        ]);
        cmd.kill_on_drop(true);
        timeout(Duration::from_secs(PER_PROJECT_TIMEOUT_SECS), cmd.output()).await
    } else {
        let mut cmd = Command::new("git");
        cmd.args(["-C", root_path, "ls-remote", "origin", &ref_path]);
        cmd.kill_on_drop(true);
        timeout(Duration::from_secs(PER_PROJECT_TIMEOUT_SECS), cmd.output()).await
    };
    let output = match result {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            eprintln!("[auto_deploy] ls-remote spawn failed at {root_path}: {err}");
            return None;
        }
        Err(_) => {
            eprintln!("[auto_deploy] ls-remote timeout at {root_path}");
            return None;
        }
    };
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
