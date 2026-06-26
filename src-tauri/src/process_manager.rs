use crate::{
    health,
    models::{
        ApiError, ApiResponse, DashboardSummary, ExternalProcess, HealthStatus, Id, LaunchdSupervision,
        LogEntry, LogHistoryFilters, LogLevel, Machine, MetricSample, PortBinding, ProcessDefinition,
        ProcessRuntimeState, ProcessStatus, Project, ProjectDetail, ProjectStatus, RestartPolicy,
        RestartPolicyKind, RuntimeProcessRecord, StreamType,
    },
    ssh_executor,
    state::AppState,
    storage,
};
use crate::platform::{self, ExternalProcessRow};
use chrono::{Duration as ChronoDuration, Utc};
use std::{
    collections::{HashMap, HashSet},
    env,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::Mutex,
    time::{sleep, Instant},
};

const LOG_HISTORY_WINDOW_MINUTES: i64 = 5;
const LOG_BATCH_FLUSH_LIMIT: usize = 32;
const LOG_BATCH_FLUSH_INTERVAL_MS: u64 = 50;
const RESTART_BACKOFF_CAP_MS: u64 = 192_000;
const RESTART_BACKOFF_MAX_EXPONENT: u32 = 6;
// A process that ran longer than the worst-case backoff before crashing is
// considered "stable": its restart_count is reset so a single late crash does
// not inherit the escalated backoff. See MEDIA_GUARD_TECHDEBT_PLAN P2.
const RESTART_STABLE_RESET_MS: u64 = RESTART_BACKOFF_CAP_MS;

pub fn log_history_since() -> chrono::DateTime<Utc> {
    Utc::now() - ChronoDuration::minutes(LOG_HISTORY_WINDOW_MINUTES)
}

// ===== launchd-delegated supervision =====
//
// A process may declare `launchd { label, domain }`. Then launchd — local on
// this host, or on the project's remote machine over SSH — is the real
// supervisor (boot-survival + KeepAlive). Karvon never spawns a child for it:
// start/stop/restart shell out to `launchctl`, and status is read back from
// `launchctl list`. This makes Karvon a truthful control panel without two
// supervisors fighting over the same process. See reconcile_launchd_process and
// start_launchd_monitor.

const LAUNCHD_MONITOR_INTERVAL_SECS: u64 = 15;
const LAUNCHD_DEFAULT_DOMAIN: &str = "gui/501";

fn launchd_domain(sup: &LaunchdSupervision) -> String {
    sup.domain
        .as_deref()
        .map(str::trim)
        .filter(|domain| !domain.is_empty())
        .unwrap_or(LAUNCHD_DEFAULT_DOMAIN)
        .to_string()
}

fn launchd_target(sup: &LaunchdSupervision) -> String {
    format!("{}/{}", launchd_domain(sup), sup.label)
}

/// launchd label/domain are interpolated into a remote shell command, so restrict
/// them to a safe charset (reverse-DNS labels, `gui/<uid>` domains) to prevent
/// shell injection. Returns false for anything outside that set.
fn is_safe_launchd_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'))
}

fn launchd_tokens_safe(sup: &LaunchdSupervision) -> bool {
    is_safe_launchd_token(&sup.label) && is_safe_launchd_token(&launchd_domain(sup))
}

#[derive(Debug)]
enum LaunchdState {
    Running(u32),
    Stopped,
    NotLoaded,
    /// Transient lookup failure (e.g. SSH blip) — do not flip the process to
    /// crashed/stopped on this; keep the last known status.
    Unknown(String),
}

/// Run `launchctl <args>` locally (machine = None) or on a remote machine over
/// SSH. Returns (success, stdout, stderr).
async fn run_launchctl(
    machine: Option<&Machine>,
    args: &[&str],
) -> Result<(bool, String, String), String> {
    match machine {
        None => {
            let output = Command::new("launchctl")
                .args(args)
                .stdin(Stdio::null())
                .output()
                .await
                .map_err(|err| err.to_string())?;
            Ok((
                output.status.success(),
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
        Some(machine) => {
            let mut shell = String::from("launchctl");
            for arg in args {
                shell.push(' ');
                shell.push_str(arg);
            }
            let output = ssh_executor::run_remote_command(machine, &shell)
                .await
                .map_err(|err| err.to_string())?;
            Ok((
                output.status.success(),
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }
}

/// Parse the `"PID" = N;` line from `launchctl list <label>` output.
fn parse_launchctl_pid(stdout: &str) -> Option<u32> {
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("\"PID\"") {
            return line
                .split('=')
                .nth(1)
                .map(|value| value.trim().trim_end_matches(';').trim())
                .and_then(|value| value.parse::<u32>().ok());
        }
    }
    None
}

async fn launchd_query(machine: Option<&Machine>, sup: &LaunchdSupervision) -> LaunchdState {
    match run_launchctl(machine, &["list", sup.label.as_str()]).await {
        Ok((true, stdout, _)) => match parse_launchctl_pid(&stdout) {
            Some(pid) => LaunchdState::Running(pid),
            None => LaunchdState::Stopped,
        },
        // A loaded-but-absent label exits non-zero ("Could not find service").
        Ok((false, _, _)) => LaunchdState::NotLoaded,
        Err(err) => LaunchdState::Unknown(err),
    }
}

/// Read true status from launchd and update the runtime state. Used by the
/// periodic monitor and after every start/stop/restart so the dashboard mirrors
/// reality instead of guessing.
async fn reconcile_launchd_process(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    sup: &LaunchdSupervision,
) -> ProcessRuntimeState {
    let machine = resolve_remote_machine(state, process).await;
    let observed = launchd_query(machine.as_ref(), sup).await;
    let mut runtime = state
        .runtime
        .states
        .read()
        .await
        .get(&process.id)
        .cloned()
        .unwrap_or_else(|| ProcessRuntimeState::stopped(process.id.clone()));

    let event = match observed {
        LaunchdState::Running(pid) => {
            let already_running = matches!(runtime.current_status, ProcessStatus::Running)
                && runtime.pid == Some(pid);
            runtime.pid = Some(pid);
            runtime.current_status = ProcessStatus::Running;
            runtime.stopped_at = None;
            runtime.exit_code = None;
            runtime.last_error = None;
            if runtime.started_at.is_none() {
                runtime.started_at = Some(Utc::now());
            }
            if already_running {
                return runtime;
            }
            "process_started"
        }
        LaunchdState::Stopped | LaunchdState::NotLoaded => {
            if matches!(runtime.current_status, ProcessStatus::Stopped) && runtime.pid.is_none() {
                return runtime;
            }
            runtime.pid = None;
            runtime.current_status = ProcessStatus::Stopped;
            runtime.memory_usage = None;
            runtime.health_status = Some(HealthStatus::Unknown);
            "process_stopped"
        }
        LaunchdState::Unknown(err) => {
            // Transient lookup failure (e.g. SSH blip): leave status untouched.
            append_log(
                app,
                state,
                process,
                StreamType::System,
                LogLevel::Debug,
                format!("launchd status lookup failed (keeping last status): {err}"),
            )
            .await;
            return runtime;
        }
    };
    set_runtime(app, state, runtime.clone(), event).await;
    runtime
}

async fn start_launchd_process(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    sup: &LaunchdSupervision,
) -> Result<ProcessRuntimeState, ApiError> {
    if !launchd_tokens_safe(sup) {
        return Err(ApiError::new(
            "INVALID_LAUNCHD",
            "Unsafe launchd label or domain",
            false,
        ));
    }
    let machine = resolve_remote_machine(state, process).await;
    let target = launchd_target(sup);
    // `enable` clears a prior `disable`; ignore its result (already-enabled is fine).
    let _ = run_launchctl(machine.as_ref(), &["enable", target.as_str()]).await;
    let (ok, _stdout, stderr) = run_launchctl(machine.as_ref(), &["kickstart", target.as_str()])
        .await
        .map_err(|err| {
            ApiError::with_details("LAUNCHCTL_FAILED", "launchctl kickstart failed", err, true)
        })?;
    append_log(
        app,
        state,
        process,
        StreamType::System,
        if ok { LogLevel::Info } else { LogLevel::Warn },
        format!("launchctl kickstart {target}"),
    )
    .await;
    if !ok && !stderr.trim().is_empty() {
        append_log(
            app,
            state,
            process,
            StreamType::System,
            LogLevel::Warn,
            format!("launchctl stderr: {}", stderr.trim()),
        )
        .await;
    }
    sleep(Duration::from_millis(600)).await;
    Ok(reconcile_launchd_process(app, state, process, sup).await)
}

async fn stop_launchd_process(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    sup: &LaunchdSupervision,
) -> Result<ProcessRuntimeState, ApiError> {
    if !launchd_tokens_safe(sup) {
        return Err(ApiError::new(
            "INVALID_LAUNCHD",
            "Unsafe launchd label or domain",
            false,
        ));
    }
    let machine = resolve_remote_machine(state, process).await;
    let target = launchd_target(sup);
    let _ = run_launchctl(machine.as_ref(), &["kill", "SIGTERM", target.as_str()]).await;
    append_log(
        app,
        state,
        process,
        StreamType::System,
        LogLevel::Info,
        format!("launchctl kill SIGTERM {target} (launchd KeepAlive may relaunch it)"),
    )
    .await;
    sleep(Duration::from_millis(600)).await;
    Ok(reconcile_launchd_process(app, state, process, sup).await)
}

async fn restart_launchd_process(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    sup: &LaunchdSupervision,
) -> Result<ProcessRuntimeState, ApiError> {
    if !launchd_tokens_safe(sup) {
        return Err(ApiError::new(
            "INVALID_LAUNCHD",
            "Unsafe launchd label or domain",
            false,
        ));
    }
    let machine = resolve_remote_machine(state, process).await;
    let target = launchd_target(sup);
    let (ok, _stdout, stderr) =
        run_launchctl(machine.as_ref(), &["kickstart", "-k", target.as_str()])
            .await
            .map_err(|err| {
                ApiError::with_details(
                    "LAUNCHCTL_FAILED",
                    "launchctl kickstart -k failed",
                    err,
                    true,
                )
            })?;
    append_log(
        app,
        state,
        process,
        StreamType::System,
        if ok { LogLevel::Info } else { LogLevel::Warn },
        format!("launchctl kickstart -k {target}"),
    )
    .await;
    if !ok && !stderr.trim().is_empty() {
        append_log(
            app,
            state,
            process,
            StreamType::System,
            LogLevel::Warn,
            format!("launchctl stderr: {}", stderr.trim()),
        )
        .await;
    }
    sleep(Duration::from_millis(800)).await;
    Ok(reconcile_launchd_process(app, state, process, sup).await)
}

/// Periodically mirror launchd state into the runtime so the dashboard stays
/// truthful for launchd-supervised processes (which Karvon does not spawn and so
/// has no exit watcher for). Runs health checks for ones launchd reports running.
pub fn start_launchd_monitor(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        sleep(Duration::from_secs(5)).await;
        loop {
            let processes: Vec<(ProcessDefinition, LaunchdSupervision)> = {
                let config = state.config.read().await;
                config
                    .processes
                    .iter()
                    .filter_map(|process| {
                        process
                            .launchd
                            .clone()
                            .map(|sup| (process.clone(), sup))
                    })
                    .collect()
            };
            for (process, sup) in processes {
                let runtime = reconcile_launchd_process(&app, &state, &process, &sup).await;
                if matches!(runtime.current_status, ProcessStatus::Running) {
                    run_process_health_check(app.clone(), state.clone(), process.id.clone()).await;
                }
            }
            sleep(Duration::from_secs(LAUNCHD_MONITOR_INTERVAL_SECS)).await;
        }
    });
}

pub async fn get_project_detail(
    state: &AppState,
    project_id: &str,
) -> Result<ProjectDetail, ApiError> {
    let config = state.config.read().await;
    let project = config
        .projects
        .iter()
        .find(|project| project.id == project_id)
        .cloned()
        .ok_or_else(|| ApiError::new("PROJECT_NOT_FOUND", "Project not found", false))?;
    let processes: Vec<_> = config
        .processes
        .iter()
        .filter(|process| process.project_id == project_id)
        .cloned()
        .collect();
    drop(config);
    let states_guard = state.runtime.states.read().await;
    let runtime_states: Vec<_> = processes
        .iter()
        .map(|process| {
            states_guard
                .get(&process.id)
                .cloned()
                .unwrap_or_else(|| ProcessRuntimeState::stopped(process.id.clone()))
        })
        .collect();
    drop(states_guard);
    let logs = state.runtime.logs.read().await;
    let recent_logs = logs
        .iter()
        .filter(|log| log.project_id == project_id)
        .rev()
        .take(250)
        .cloned()
        .collect::<Vec<_>>();
    Ok(ProjectDetail {
        project,
        processes,
        status: derive_project_status(&runtime_states),
        runtime_states,
        recent_logs,
    })
}

pub async fn start_process(
    app: AppHandle,
    state: AppState,
    process_id: Id,
) -> ApiResponse<ProcessRuntimeState> {
    sync_external_processes(app.clone(), state.clone()).await;
    match start_process_inner(app, state, process_id).await {
        Ok(runtime) => ApiResponse::ok(runtime),
        Err(error) => ApiResponse::err(error),
    }
}

async fn start_process_inner(
    app: AppHandle,
    state: AppState,
    process_id: Id,
) -> Result<ProcessRuntimeState, ApiError> {
    let (project, process, settings) = {
        let config = state.config.read().await;
        let process = config
            .processes
            .iter()
            .find(|process| process.id == process_id)
            .cloned()
            .ok_or_else(|| ApiError::new("PROCESS_NOT_FOUND", "Process not found", false))?;
        let project = config
            .projects
            .iter()
            .find(|project| project.id == process.project_id)
            .cloned()
            .ok_or_else(|| ApiError::new("PROJECT_NOT_FOUND", "Project not found", false))?;
        (project, process, config.settings.clone())
    };

    // launchd-supervised: delegate to launchctl instead of spawning a child.
    if let Some(sup) = process.launchd.clone() {
        return start_launchd_process(&app, &state, &process, &sup).await;
    }

    let existing = state.runtime.states.read().await.get(&process_id).cloned();
    if matches!(
        existing.map(|state| state.current_status),
        Some(
            ProcessStatus::Running
                | ProcessStatus::Starting
                | ProcessStatus::Queued
                | ProcessStatus::Stopping
        )
    ) {
        return Err(ApiError::new(
            "PROCESS_ALREADY_RUNNING",
            "Process is already running or stopping",
            false,
        ));
    }
    clear_stop_requests_for_process(&state, &process_id).await;

    if let Some(missing) = missing_dependency(&state, &process).await {
        let mut runtime = ProcessRuntimeState::stopped(process.id.clone());
        runtime.current_status = ProcessStatus::WaitingDependency;
        runtime.last_error = Some(format!("Dependency is not running: {missing}"));
        set_runtime(&app, &state, runtime.clone(), "process_failed").await;
        append_log(
            &app,
            &state,
            &process,
            StreamType::System,
            LogLevel::Warn,
            format!("Blocked by dependency {missing}"),
        )
        .await;
        return Ok(runtime);
    }

    let remote_machine = resolve_remote_machine(&state, &process).await;
    let is_remote = remote_machine.is_some();
    let cwd = resolve_working_directory_with_locality(&project, &process, is_remote)?;
    let mut runtime = state
        .runtime
        .states
        .read()
        .await
        .get(&process_id)
        .cloned()
        .unwrap_or_else(|| ProcessRuntimeState::stopped(process.id.clone()));
    runtime.current_status = ProcessStatus::Starting;
    runtime.started_at = Some(Utc::now());
    runtime.stopped_at = None;
    runtime.exit_code = None;
    runtime.last_error = None;
    runtime.memory_usage = None;
    runtime.health_status = Some(HealthStatus::Starting);
    runtime.port_bindings = detect_process_ports(&process);
    set_runtime(&app, &state, runtime.clone(), "process_started").await;
    append_log(
        &app,
        &state,
        &process,
        StreamType::System,
        LogLevel::Info,
        "Starting process",
    )
    .await;

    let command_tokens = process_command_tokens(&process)?;
    let command_label = display_command(&command_tokens);

    let mut child = if let Some(machine) = remote_machine.as_ref() {
        let mut command =
            ssh_executor::build_ssh_command(machine, &command_tokens, Some(&cwd), &process.env);
        match command.spawn() {
            Ok(child) => {
                append_log(
                    &app,
                    &state,
                    &process,
                    StreamType::System,
                    LogLevel::Info,
                    format!(
                        "Connecting to {}@{}:{} via SSH",
                        machine.ssh_user, machine.hostname, machine.ssh_port
                    ),
                )
                .await;
                child
            }
            Err(error) => {
                let details = format!("{command_label} (ssh {}): {error}", machine.hostname);
                mark_spawn_failure(&app, &state, &process, &mut runtime, details.clone()).await;
                schedule_auto_restart_if_eligible(&app, &state, &process).await;
                return Err(ApiError::with_details(
                    "COMMAND_EXECUTION_FAILED",
                    "Unable to execute remote process command",
                    details,
                    true,
                ));
            }
        }
    } else {
        let mut command = direct_process_command(&command_tokens);
        configure_process_command(&mut command, &cwd, &process.env, process.memory_limit_mb);

        match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let mut shell_command = shell_process_command(&command_tokens);
                configure_process_command(
                    &mut shell_command,
                    &cwd,
                    &process.env,
                    process.memory_limit_mb,
                );
                match shell_command.spawn() {
                    Ok(child) => {
                        append_log(
                            &app,
                            &state,
                            &process,
                            StreamType::System,
                            LogLevel::Debug,
                            "Resolved command through login shell",
                        )
                        .await;
                        child
                    }
                    Err(shell_error) => {
                        let details = format!(
                            "{command_label}: {shell_error}. Direct launch also failed: {error}"
                        );
                        mark_spawn_failure(&app, &state, &process, &mut runtime, details.clone())
                            .await;
                        schedule_auto_restart_if_eligible(&app, &state, &process).await;
                        return Err(ApiError::with_details(
                            "COMMAND_EXECUTION_FAILED",
                            "Unable to execute process command",
                            details,
                            true,
                        ));
                    }
                }
            }
            Err(error) => {
                let details = format!("{command_label}: {error}");
                mark_spawn_failure(&app, &state, &process, &mut runtime, details.clone()).await;
                schedule_auto_restart_if_eligible(&app, &state, &process).await;
                return Err(ApiError::with_details(
                    "COMMAND_EXECUTION_FAILED",
                    "Unable to execute process command",
                    details,
                    true,
                ));
            }
        }
    };

    let pid = child.id();
    if let Some(pid) = pid {
        let record = RuntimeProcessRecord {
            process_id: process.id.clone(),
            project_id: process.project_id.clone(),
            pid,
            process_group_id: pid,
            started_at: runtime.started_at.clone().unwrap_or_else(Utc::now),
            command: command_label.clone(),
        };
        track_runtime_process(&state, record).await;
        let _ = persist_runtime_processes(&app, &state).await;
    }

    runtime.pid = pid;
    runtime.current_status = ProcessStatus::Running;
    runtime.health_status = Some(HealthStatus::Unknown);
    set_runtime(&app, &state, runtime.clone(), "process_started").await;
    append_log(
        &app,
        &state,
        &process,
        StreamType::System,
        LogLevel::Info,
        format!(
            "Process running{}{}{}",
            pid.map(|pid| format!(" with pid {pid}"))
                .unwrap_or_default(),
            if is_remote { " (remote)" } else { "" },
            process
                .memory_limit_mb
                .filter(|_| !is_remote)
                .map(|limit| format!(" (RAM limit {limit} MB)"))
                .unwrap_or_default()
        ),
    )
    .await;

    if !is_remote {
        if let Some(pid) = pid {
            spawn_memory_monitor(
                app.clone(),
                state.clone(),
                process.project_id.clone(),
                process.id.clone(),
                pid,
            );
        }
    }

    if let Some(stdout) = child.stdout.take() {
        spawn_log_reader(
            app.clone(),
            state.clone(),
            process.clone(),
            StreamType::Stdout,
            stdout,
            is_remote,
        );
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_reader(
            app.clone(),
            state.clone(),
            process.clone(),
            StreamType::Stderr,
            stderr,
            is_remote,
        );
    }

    let wait_app = app.clone();
    let wait_state = state.clone();
    let wait_process = process.clone();
    let wait_process_group_id = pid;
    let wait_is_remote = is_remote;
    let wait_remote_machine = remote_machine.clone();
    tauri::async_runtime::spawn(async move {
        let status = child.wait().await;
        if wait_is_remote {
            cleanup_remote_process_after_exit(
                &wait_state,
                wait_remote_machine.as_ref(),
                &wait_process.id,
            )
            .await;
        }
        if let Some(process_group_id) = wait_process_group_id {
            let stop_timeout_ms = wait_state.config.read().await.settings.stop_timeout_ms;
            terminate_process_group_gracefully(process_group_id, stop_timeout_ms).await;
        }
        let recovered_pid = match (wait_is_remote, wait_process_group_id) {
            (false, Some(process_group_id)) => {
                platform::live_process_in_group(process_group_id).await
            }
            _ => None,
        };
        let stop_requested = match wait_process_group_id {
            Some(process_group_id) => {
                stop_was_requested(&wait_state, &wait_process.id, process_group_id).await
            }
            None => false,
        };
        let mut runtime = wait_state
            .runtime
            .states
            .read()
            .await
            .get(&wait_process.id)
            .cloned()
            .unwrap_or_else(|| ProcessRuntimeState::stopped(wait_process.id.clone()));
        runtime.stopped_at = Some(Utc::now());
        runtime.exit_code = status.as_ref().ok().and_then(|status| status.code());
        runtime.health_status = Some(HealthStatus::Unknown);

        if let (Some(process_group_id), Some(live_pid)) = (wait_process_group_id, recovered_pid) {
            if update_tracked_process_pid(&wait_state, &wait_process.id, live_pid, process_group_id)
                .await
            {
                runtime.pid = Some(live_pid);
                runtime.stopped_at = None;
                runtime.exit_code = None;
                runtime.current_status = ProcessStatus::Running;
                runtime.last_error = Some(
                    "Parent process exited but child process group is still running".to_string(),
                );
                let _ = persist_runtime_processes(&wait_app, &wait_state).await;
                append_log(
                    &wait_app,
                    &wait_state,
                    &wait_process,
                    StreamType::System,
                    LogLevel::Warn,
                    format!("Parent exited; recovered running process group {process_group_id}"),
                )
                .await;
                set_runtime(&wait_app, &wait_state, runtime, "process_started").await;
            }
            clear_stop_requested(&wait_state, &wait_process.id, process_group_id).await;
            return;
        }

        runtime.pid = None;
        if let Some(process_group_id) = wait_process_group_id {
            let current_group = current_tracked_process_group(&wait_state, &wait_process.id).await;
            if let Some(current_group) = current_group {
                if current_group != process_group_id {
                    clear_stop_requested(&wait_state, &wait_process.id, process_group_id).await;
                    return;
                }
                if untrack_runtime_process_if_group(&wait_state, &wait_process.id, process_group_id)
                    .await
                {
                    let _ = persist_runtime_processes(&wait_app, &wait_state).await;
                }
            } else if stop_requested
                || matches!(
                    runtime.current_status,
                    ProcessStatus::Stopping | ProcessStatus::Stopped
                )
            {
                clear_stop_requested(&wait_state, &wait_process.id, process_group_id).await;
                return;
            }
            clear_stop_requested(&wait_state, &wait_process.id, process_group_id).await;
        }

        match status {
            Ok(_exit_status) if matches!(runtime.current_status, ProcessStatus::Failed) => {
                append_log(
                    &wait_app,
                    &wait_state,
                    &wait_process,
                    StreamType::System,
                    LogLevel::Warn,
                    "Process stopped after failure",
                )
                .await;
                set_runtime(&wait_app, &wait_state, runtime, "process_failed").await;
            }
            Ok(exit_status)
                if exit_status.success()
                    || stop_requested
                    || matches!(runtime.current_status, ProcessStatus::Stopping) =>
            {
                runtime.current_status = ProcessStatus::Stopped;
                runtime.exit_code = None;
                runtime.last_error = None;
                runtime.memory_usage = None;
                append_log(
                    &wait_app,
                    &wait_state,
                    &wait_process,
                    StreamType::System,
                    LogLevel::Info,
                    "Process stopped",
                )
                .await;
                set_runtime(&wait_app, &wait_state, runtime, "process_stopped").await;
            }
            Ok(exit_status) => {
                runtime.current_status = ProcessStatus::Crashed;
                if runtime.last_error.is_none() {
                    runtime.last_error = Some(format!("Exited with status {exit_status}"));
                }
                append_log(
                    &wait_app,
                    &wait_state,
                    &wait_process,
                    StreamType::System,
                    LogLevel::Error,
                    format!("Process crashed: {exit_status}"),
                )
                .await;
                set_runtime(&wait_app, &wait_state, runtime, "process_failed").await;
                schedule_auto_restart_if_eligible(&wait_app, &wait_state, &wait_process).await;
            }
            Err(error) => {
                runtime.current_status = ProcessStatus::Failed;
                runtime.last_error = Some(error.to_string());
                append_log(
                    &wait_app,
                    &wait_state,
                    &wait_process,
                    StreamType::System,
                    LogLevel::Error,
                    format!("Process wait failed: {error}"),
                )
                .await;
                set_runtime(&wait_app, &wait_state, runtime, "process_failed").await;
                schedule_auto_restart_if_eligible(&wait_app, &wait_state, &wait_process).await;
            }
        }
    });

    if settings.auto_start_marked_projects {
        append_log(
            &app,
            &state,
            &process,
            StreamType::System,
            LogLevel::Debug,
            "Autostart setting is enabled",
        )
        .await;
    }

    Ok(runtime)
}

pub async fn stop_process(
    app: AppHandle,
    state: AppState,
    process_id: Id,
) -> ApiResponse<ProcessRuntimeState> {
    match stop_process_inner(app, state, process_id).await {
        Ok(runtime) => ApiResponse::ok(runtime),
        Err(error) => ApiResponse::err(error),
    }
}

async fn stop_process_inner(
    app: AppHandle,
    state: AppState,
    process_id: Id,
) -> Result<ProcessRuntimeState, ApiError> {
    let process = {
        let config = state.config.read().await;
        config
            .processes
            .iter()
            .find(|process| process.id == process_id)
            .cloned()
            .ok_or_else(|| ApiError::new("PROCESS_NOT_FOUND", "Process not found", false))?
    };

    // launchd-supervised: delegate to launchctl instead of signalling a child pgid.
    if let Some(sup) = process.launchd.clone() {
        return stop_launchd_process(&app, &state, &process, &sup).await;
    }

    let stop_timeout_ms = state.config.read().await.settings.stop_timeout_ms;
    let mut runtime = state
        .runtime
        .states
        .read()
        .await
        .get(&process_id)
        .cloned()
        .unwrap_or_else(|| ProcessRuntimeState::stopped(process.id.clone()));

    let Some(pid) = state
        .runtime
        .pids
        .read()
        .await
        .get(&process_id)
        .copied()
        .or(runtime.pid)
    else {
        runtime.current_status = ProcessStatus::Stopped;
        runtime.stopped_at = Some(Utc::now());
        runtime.memory_usage = None;
        set_runtime(&app, &state, runtime.clone(), "process_stopped").await;
        return Ok(runtime);
    };

    mark_stop_requested(&state, &process_id, pid).await;
    runtime.current_status = ProcessStatus::Stopping;
    set_runtime(&app, &state, runtime.clone(), "process_stopped").await;
    let remote_machine = resolve_remote_machine(&state, &process).await;
    let remote_pid = if remote_machine.is_some() {
        state
            .runtime
            .remote_pids
            .read()
            .await
            .get(&process_id)
            .copied()
    } else {
        None
    };
    append_log(
        &app,
        &state,
        &process,
        StreamType::System,
        LogLevel::Info,
        if remote_machine.is_some() {
            format!(
                "Sending SIGTERM to ssh client (pid {pid}){}",
                remote_pid
                    .map(|rp| format!("; remote pid {rp}"))
                    .unwrap_or_default()
            )
        } else {
            format!("Sending SIGTERM to process group {pid}")
        },
    )
    .await;

    match term_process_group(pid) {
        Ok(()) => {}
        Err(platform::GroupError::NotFound) => {
            runtime.current_status = ProcessStatus::Stopped;
            runtime.stopped_at = Some(Utc::now());
            runtime.pid = None;
            runtime.memory_usage = None;
            if untrack_runtime_process_if_group(&state, &process_id, pid).await {
                let _ = persist_runtime_processes(&app, &state).await;
            }
            clear_stop_requested(&state, &process_id, pid).await;
            set_runtime(&app, &state, runtime.clone(), "process_stopped").await;
            return Ok(runtime);
        }
        Err(error) => {
            return Err(ApiError::with_details(
                "COMMAND_EXECUTION_FAILED",
                "Unable to terminate process group",
                error,
                true,
            ));
        }
    }

    let force_app = app.clone();
    let force_state = state.clone();
    let force_process = process.clone();
    let force_process_group_id = pid;
    let force_remote_machine = remote_machine.clone();
    let force_remote_pid = remote_pid;
    tauri::async_runtime::spawn(async move {
        let poll_interval = Duration::from_millis(100);
        let deadline = Instant::now() + Duration::from_millis(stop_timeout_ms);
        let mut graceful_exit = false;
        while Instant::now() < deadline {
            if !process_group_exists(force_process_group_id) {
                graceful_exit = true;
                break;
            }
            sleep(poll_interval).await;
        }
        if !graceful_exit && process_group_exists(force_process_group_id) {
            append_log(
                &force_app,
                &force_state,
                &force_process,
                StreamType::System,
                LogLevel::Warn,
                format!("Force killing process group {force_process_group_id}"),
            )
            .await;
            let _ = force_kill_process_group(force_process_group_id);
            sleep(Duration::from_millis(200)).await;
        }
        if let (Some(machine), Some(remote_pid)) = (force_remote_machine.as_ref(), force_remote_pid)
        {
            match ssh_executor::kill_remote_process(machine, remote_pid, "KILL").await {
                Ok(()) => {
                    append_log(
                        &force_app,
                        &force_state,
                        &force_process,
                        StreamType::System,
                        LogLevel::Debug,
                        format!("Sent SIGKILL to remote pid {remote_pid}"),
                    )
                    .await;
                }
                Err(err) => {
                    append_log(
                        &force_app,
                        &force_state,
                        &force_process,
                        StreamType::System,
                        LogLevel::Warn,
                        format!("Remote SIGKILL failed for pid {remote_pid}: {err}"),
                    )
                    .await;
                }
            }
            force_state
                .runtime
                .remote_pids
                .write()
                .await
                .remove(&force_process.id);
        }
        if let Some(live_pid) = platform::live_process_in_group(force_process_group_id).await {
            if update_tracked_process_pid(
                &force_state,
                &force_process.id,
                live_pid,
                force_process_group_id,
            )
            .await
            {
                let _ = persist_runtime_processes(&force_app, &force_state).await;
                let mut runtime = force_state
                    .runtime
                    .states
                    .read()
                    .await
                    .get(&force_process.id)
                    .cloned()
                    .unwrap_or_else(|| ProcessRuntimeState::stopped(force_process.id.clone()));
                runtime.pid = Some(live_pid);
                runtime.current_status = ProcessStatus::Running;
                runtime.stopped_at = None;
                runtime.last_error = Some("Process group survived forced stop".to_string());
                set_runtime(&force_app, &force_state, runtime, "process_failed").await;
            }
        } else if untrack_runtime_process_if_group(
            &force_state,
            &force_process.id,
            force_process_group_id,
        )
        .await
        {
            let _ = persist_runtime_processes(&force_app, &force_state).await;
            let mut runtime = force_state
                .runtime
                .states
                .read()
                .await
                .get(&force_process.id)
                .cloned()
                .unwrap_or_else(|| ProcessRuntimeState::stopped(force_process.id.clone()));
            runtime.pid = None;
            runtime.current_status = ProcessStatus::Stopped;
            runtime.stopped_at = Some(Utc::now());
            runtime.memory_usage = None;
            runtime.last_error = None;
            set_runtime(&force_app, &force_state, runtime, "process_stopped").await;
        }
        clear_stop_requested(&force_state, &force_process.id, force_process_group_id).await;
    });

    Ok(runtime)
}

pub async fn recover_tracked_processes(app: AppHandle, state: AppState) {
    let processes_by_id: HashMap<Id, ProcessDefinition> = state
        .config
        .read()
        .await
        .processes
        .iter()
        .map(|process| (process.id.clone(), process.clone()))
        .collect();
    let records = state.runtime.process_records.read().await.clone();
    if records.is_empty() {
        return;
    }

    let mut recovered_records = HashMap::new();
    for (process_id, record) in records {
        let Some(process) = processes_by_id.get(&process_id) else {
            continue;
        };
        // launchd owns these — never resurrect a stale Karvon pid record (which
        // may even be pid 1). The launchd monitor reconciles true status on boot.
        if process.launchd.is_some() {
            continue;
        }
        let process_group_id = normalized_process_group_id(&record);
        let live_pid = live_pid_for_record(&record).await;
        match live_pid {
            Some(live_pid) => {
                let mut next_record = record.clone();
                next_record.process_id = process_id.clone();
                next_record.project_id = process.project_id.clone();
                next_record.pid = live_pid;
                next_record.process_group_id = process_group_id;
                if next_record.command.trim().is_empty() {
                    next_record.command = process.command.clone();
                }
                recovered_records.insert(process_id.clone(), next_record.clone());
                track_runtime_process(&state, next_record.clone()).await;

                let mut runtime = state
                    .runtime
                    .states
                    .read()
                    .await
                    .get(&process_id)
                    .cloned()
                    .unwrap_or_else(|| ProcessRuntimeState::stopped(process_id.clone()));
                runtime.pid = Some(live_pid);
                runtime.started_at = Some(next_record.started_at);
                runtime.stopped_at = None;
                runtime.exit_code = None;
                runtime.last_error = None;
                runtime.memory_usage = None;
                runtime.health_status = Some(HealthStatus::Unknown);
                runtime.port_bindings = detect_process_ports(process);
                runtime.current_status = ProcessStatus::Running;
                set_runtime(&app, &state, runtime, "process_started").await;
                append_log(
                    &app,
                    &state,
                    process,
                    StreamType::System,
                    LogLevel::Info,
                    format!("Recovered running process group {process_group_id}"),
                )
                .await;
                spawn_memory_monitor(
                    app.clone(),
                    state.clone(),
                    process.project_id.clone(),
                    process_id.clone(),
                    live_pid,
                );
            }
            None => {
                let mut runtime = ProcessRuntimeState::stopped(process_id.clone());
                runtime.stopped_at = Some(Utc::now());
                set_runtime(&app, &state, runtime, "process_stopped").await;
                append_log(
                    &app,
                    &state,
                    process,
                    StreamType::System,
                    LogLevel::Info,
                    format!("Previous process group {process_group_id} is no longer running"),
                )
                .await;
            }
        }
    }

    replace_runtime_process_records(&state, recovered_records).await;
    let _ = persist_runtime_processes(&app, &state).await;
}

pub async fn sync_external_processes(app: AppHandle, state: AppState) {
    let mut tracked_groups: HashSet<u32> = state
        .runtime
        .process_records
        .read()
        .await
        .values()
        .map(normalized_process_group_id)
        .collect();
    let processes = {
        let config = state.config.read().await;
        config
            .processes
            .iter()
            .filter_map(|process| {
                // launchd-supervised processes are reconciled by start_launchd_monitor,
                // not adopted as external here.
                if process.launchd.is_some() {
                    return None;
                }
                let project = config
                    .projects
                    .iter()
                    .find(|project| project.id == process.project_id)?;
                if let Some(machine_id) = &process.machine_id {
                    let machine = config
                        .machines
                        .iter()
                        .find(|machine| &machine.id == machine_id);
                    if let Some(machine) = machine {
                        if !machine.is_default_local {
                            return None;
                        }
                    }
                }
                let cwd = resolve_working_directory(project, process).ok()?;
                let command_tokens = process_command_tokens(process).ok()?;
                Some((project.clone(), process.clone(), cwd, command_tokens))
            })
            .collect::<Vec<_>>()
    };
    if processes.is_empty() {
        return;
    }

    let rows = platform::list_live_processes().await;
    if rows.is_empty() {
        return;
    }

    let mut adopted = false;
    for (project, process, configured_cwd, command_tokens) in processes {
        let already_active = state
            .runtime
            .states
            .read()
            .await
            .get(&process.id)
            .map(|runtime| {
                matches!(
                    runtime.current_status,
                    ProcessStatus::Running
                        | ProcessStatus::Starting
                        | ProcessStatus::Queued
                        | ProcessStatus::Stopping
                )
            })
            .unwrap_or(false);
        if already_active {
            continue;
        }

        let Some(row) =
            find_external_process_match(&rows, &tracked_groups, &configured_cwd, &command_tokens)
                .await
        else {
            continue;
        };

        let record = RuntimeProcessRecord {
            process_id: process.id.clone(),
            project_id: project.id.clone(),
            pid: row.pid,
            process_group_id: row.process_group_id,
            started_at: Utc::now(),
            command: display_command(&command_tokens),
        };
        track_runtime_process(&state, record.clone()).await;
        tracked_groups.insert(row.process_group_id);

        let mut runtime = state
            .runtime
            .states
            .read()
            .await
            .get(&process.id)
            .cloned()
            .unwrap_or_else(|| ProcessRuntimeState::stopped(process.id.clone()));
        runtime.pid = Some(row.pid);
        runtime.started_at = Some(record.started_at);
        runtime.stopped_at = None;
        runtime.exit_code = None;
        runtime.last_error = None;
        runtime.memory_usage = None;
        runtime.health_status = Some(HealthStatus::Unknown);
        runtime.port_bindings = detect_process_ports(&process);
        runtime.current_status = ProcessStatus::Running;
        set_runtime(&app, &state, runtime, "process_started").await;
        append_log(
            &app,
            &state,
            &process,
            StreamType::System,
            LogLevel::Info,
            format!("Adopted running process group {}", row.process_group_id),
        )
        .await;
        spawn_memory_monitor(
            app.clone(),
            state.clone(),
            process.project_id.clone(),
            process.id.clone(),
            row.pid,
        );
        adopted = true;
    }

    if adopted {
        let _ = persist_runtime_processes(&app, &state).await;
    }
}

pub async fn list_external_project_processes(
    state: AppState,
    project_id: Id,
) -> ApiResponse<Vec<ExternalProcess>> {
    let project_root = {
        let config = state.config.read().await;
        match config
            .projects
            .iter()
            .find(|project| project.id == project_id)
        {
            Some(project) => project.root_path.clone(),
            None => {
                return ApiResponse::err(ApiError::new(
                    "PROJECT_NOT_FOUND",
                    "Project not found",
                    false,
                ))
            }
        }
    };

    let tracked_groups: HashSet<u32> = state
        .runtime
        .process_records
        .read()
        .await
        .values()
        .map(normalized_process_group_id)
        .collect();

    let rows = platform::list_live_processes().await;
    if rows.is_empty() {
        return ApiResponse::ok(vec![]);
    }

    let cwds = platform::all_process_cwds().await;
    let self_pid = std::process::id();

    let mut rows_by_group: HashMap<u32, Vec<ExternalProcessRow>> = HashMap::new();
    for row in &rows {
        rows_by_group
            .entry(row.process_group_id)
            .or_default()
            .push(row.clone());
    }

    let mut leaders: Vec<ExternalProcessRow> = Vec::new();
    let mut seen_groups: HashSet<u32> = HashSet::new();
    for row in &rows {
        if row.pid == self_pid {
            continue;
        }
        if tracked_groups.contains(&row.process_group_id) {
            continue;
        }
        let Some(cwd) = cwds.get(&row.pid) else {
            continue;
        };
        if !cwd_matches_root(cwd, &project_root) {
            continue;
        }
        if !seen_groups.insert(row.process_group_id) {
            continue;
        }
        leaders.push(row.clone());
    }

    let leader_pids: Vec<u32> = leaders.iter().map(|r| r.pid).collect();
    let ports_by_pid = platform::listening_ports_for_pids(&leader_pids).await;

    let mut results = Vec::with_capacity(leaders.len());
    for leader in leaders {
        let cwd = cwds.get(&leader.pid).cloned().unwrap_or_default();
        let children: Vec<crate::models::ExternalProcessChild> = rows_by_group
            .get(&leader.process_group_id)
            .map(|group_rows| {
                group_rows
                    .iter()
                    .filter(|r| r.pid != leader.pid)
                    .map(|r| crate::models::ExternalProcessChild {
                        pid: r.pid,
                        command: r.command.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ports = ports_by_pid.get(&leader.pid).cloned().unwrap_or_default();
        results.push(ExternalProcess {
            pid: leader.pid,
            process_group_id: leader.process_group_id,
            command: leader.command,
            cwd,
            user: leader.user,
            started_at: leader.started_at,
            etime: leader.etime,
            cpu_percent: leader.cpu_percent,
            memory_kb: leader.memory_kb,
            ports,
            children,
        });
    }
    ApiResponse::ok(results)
}

pub async fn stop_external_process(state: AppState, process_group_id: u32) -> ApiResponse<bool> {
    let tracked = state
        .runtime
        .process_records
        .read()
        .await
        .values()
        .any(|record| normalized_process_group_id(record) == process_group_id);
    if tracked {
        return ApiResponse::err(ApiError::new(
            "PROCESS_TRACKED",
            "This process is managed by the orchestrator. Use the regular Stop button.",
            false,
        ));
    }
    if !process_group_exists(process_group_id) {
        return ApiResponse::ok(true);
    }

    let stop_timeout_ms = state.config.read().await.settings.stop_timeout_ms;
    if let Err(error) = term_process_group(process_group_id) {
        return ApiResponse::err(ApiError::with_details(
            "STOP_FAILED",
            "Failed to signal process",
            error.to_string(),
            true,
        ));
    }
    sleep(Duration::from_millis(stop_timeout_ms)).await;
    if process_group_exists(process_group_id) {
        let _ = force_kill_process_group(process_group_id);
    }
    ApiResponse::ok(true)
}

pub async fn find_process_on_port(port: u16) -> ApiResponse<Option<ExternalProcess>> {
    let Some((pid, process_group_id, command)) = platform::find_listener_on_port(port).await else {
        return ApiResponse::ok(None);
    };
    let cwd = platform::process_cwd(pid).await.unwrap_or_default();
    ApiResponse::ok(Some(ExternalProcess {
        pid,
        process_group_id,
        command,
        cwd,
        user: String::new(),
        started_at: String::new(),
        etime: String::new(),
        cpu_percent: 0.0,
        memory_kb: 0,
        ports: Vec::new(),
        children: Vec::new(),
    }))
}

pub async fn shutdown_tracked_processes(app: AppHandle, state: AppState) {
    let stop_timeout_ms = state.config.read().await.settings.stop_timeout_ms;
    let records = state.runtime.process_records.read().await.clone();
    if records.is_empty() {
        return;
    }
    let tracked_process_ids: HashSet<Id> = records.keys().cloned().collect();
    let process_group_ids: HashSet<u32> =
        records.values().map(normalized_process_group_id).collect();

    for process_group_id in &process_group_ids {
        let _ = term_process_group(*process_group_id);
    }

    sleep(Duration::from_millis(stop_timeout_ms)).await;

    for process_group_id in &process_group_ids {
        if process_group_exists(*process_group_id) {
            let _ = force_kill_process_group(*process_group_id);
        }
    }
    sleep(Duration::from_millis(200)).await;

    let mut surviving_records = HashMap::new();
    for (process_id, mut record) in records {
        let process_group_id = normalized_process_group_id(&record);
        if let Some(live_pid) = platform::live_process_in_group(process_group_id).await {
            record.pid = live_pid;
            record.process_group_id = process_group_id;
            surviving_records.insert(process_id, record);
        }
    }

    replace_runtime_process_records(&state, surviving_records.clone()).await;
    let _ = persist_runtime_processes(&app, &state).await;
    let now = Utc::now();
    let mut states = state.runtime.states.write().await;
    for runtime in states.values_mut() {
        if let Some(record) = surviving_records.get(&runtime.process_id) {
            runtime.pid = Some(record.pid);
            runtime.current_status = ProcessStatus::Running;
            runtime.stopped_at = None;
        } else if tracked_process_ids.contains(&runtime.process_id) {
            runtime.pid = None;
            runtime.current_status = ProcessStatus::Stopped;
            runtime.stopped_at = Some(now);
            runtime.memory_usage = None;
        }
    }
}

async fn track_runtime_process(state: &AppState, record: RuntimeProcessRecord) {
    state
        .runtime
        .pids
        .write()
        .await
        .insert(record.process_id.clone(), record.process_group_id);
    state
        .runtime
        .process_records
        .write()
        .await
        .insert(record.process_id.clone(), record);
}

async fn update_tracked_process_pid(
    state: &AppState,
    process_id: &str,
    pid: u32,
    process_group_id: u32,
) -> bool {
    let updated = if let Some(record) = state
        .runtime
        .process_records
        .write()
        .await
        .get_mut(process_id)
    {
        if normalized_process_group_id(record) == process_group_id {
            record.pid = pid;
            record.process_group_id = process_group_id;
            true
        } else {
            false
        }
    } else {
        false
    };
    if updated {
        state
            .runtime
            .pids
            .write()
            .await
            .insert(process_id.to_string(), process_group_id);
    }
    updated
}

async fn untrack_runtime_process(state: &AppState, process_id: &str) {
    state.runtime.pids.write().await.remove(process_id);
    state
        .runtime
        .process_records
        .write()
        .await
        .remove(process_id);
}

async fn current_tracked_process_group(state: &AppState, process_id: &str) -> Option<u32> {
    state
        .runtime
        .process_records
        .read()
        .await
        .get(process_id)
        .map(normalized_process_group_id)
}

async fn untrack_runtime_process_if_group(
    state: &AppState,
    process_id: &str,
    process_group_id: u32,
) -> bool {
    let removed = {
        let mut records = state.runtime.process_records.write().await;
        match records.get(process_id).map(normalized_process_group_id) {
            Some(current_group_id) if current_group_id == process_group_id => {
                records.remove(process_id);
                true
            }
            _ => false,
        }
    };

    if removed {
        let mut pids = state.runtime.pids.write().await;
        if pids.get(process_id).copied() == Some(process_group_id) {
            pids.remove(process_id);
        }
    }

    removed
}

async fn replace_runtime_process_records(
    state: &AppState,
    records: HashMap<Id, RuntimeProcessRecord>,
) {
    let pids = records
        .iter()
        .map(|(process_id, record)| (process_id.clone(), normalized_process_group_id(record)))
        .collect();
    *state.runtime.pids.write().await = pids;
    *state.runtime.process_records.write().await = records;
}

async fn persist_runtime_processes(app: &AppHandle, state: &AppState) -> Result<(), ApiError> {
    let records = state.runtime.process_records.read().await.clone();
    storage::save_runtime_processes(app, &records)
}

fn normalized_process_group_id(record: &RuntimeProcessRecord) -> u32 {
    if record.process_group_id == 0 {
        record.pid
    } else {
        record.process_group_id
    }
}

fn stop_request_key(process_id: &str, process_group_id: u32) -> String {
    format!("{process_id}:{process_group_id}")
}

async fn mark_stop_requested(state: &AppState, process_id: &str, process_group_id: u32) {
    state
        .runtime
        .stopping_processes
        .write()
        .await
        .insert(stop_request_key(process_id, process_group_id));
}

async fn stop_was_requested(state: &AppState, process_id: &str, process_group_id: u32) -> bool {
    state
        .runtime
        .stopping_processes
        .read()
        .await
        .contains(&stop_request_key(process_id, process_group_id))
}

async fn clear_stop_requested(state: &AppState, process_id: &str, process_group_id: u32) {
    state
        .runtime
        .stopping_processes
        .write()
        .await
        .remove(&stop_request_key(process_id, process_group_id));
}

async fn clear_stop_requests_for_process(state: &AppState, process_id: &str) {
    let prefix = format!("{process_id}:");
    state
        .runtime
        .stopping_processes
        .write()
        .await
        .retain(|key| !key.starts_with(&prefix));
}

async fn terminate_process_group_gracefully(process_group_id: u32, stop_timeout_ms: u64) {
    let should_wait = match term_process_group(process_group_id) {
        Ok(()) => true,
        Err(platform::GroupError::NotFound) => false,
        Err(_) => true,
    };
    if !should_wait {
        return;
    }

    sleep(Duration::from_millis(stop_timeout_ms)).await;
    if process_group_exists(process_group_id) {
        let _ = force_kill_process_group(process_group_id);
    }
}

// The `pgid <= 1` guard is shared by all three wrappers: pgid 0 would target the
// CALLER's own group (killing the orchestrator and every managed process) and 1
// is init. A value <= 1 here means a missing/garbage id, not a real child group,
// so it is reported as "already gone". See MEDIA_GUARD_TECHDEBT_PLAN P2. The
// actual OS call is delegated to the platform layer (POSIX `killpg` on unix,
// `taskkill /T` on Windows).
fn term_process_group(process_group_id: u32) -> Result<(), platform::GroupError> {
    if process_group_id <= 1 {
        return Err(platform::GroupError::NotFound);
    }
    platform::terminate_group(process_group_id)
}

fn force_kill_process_group(process_group_id: u32) -> Result<(), platform::GroupError> {
    if process_group_id <= 1 {
        return Err(platform::GroupError::NotFound);
    }
    platform::force_kill_group(process_group_id)
}

fn process_group_exists(process_group_id: u32) -> bool {
    if process_group_id <= 1 {
        return false;
    }
    platform::group_exists(process_group_id)
}

async fn find_external_process_match(
    rows: &[ExternalProcessRow],
    tracked_groups: &HashSet<u32>,
    configured_cwd: &str,
    command_tokens: &[String],
) -> Option<ExternalProcessRow> {
    for row in rows {
        if tracked_groups.contains(&row.process_group_id) {
            continue;
        }
        if !command_tokens_match(command_tokens, &row.command) {
            continue;
        }
        let Some(cwd) = platform::process_cwd(row.pid).await else {
            continue;
        };
        if cwd_matches_root(&cwd, configured_cwd) {
            return Some(row.clone());
        }
    }
    None
}

fn cwd_matches_root(candidate_cwd: &str, configured_cwd: &str) -> bool {
    let candidate = canonical_or_original(candidate_cwd);
    let configured = canonical_or_original(configured_cwd);
    candidate.starts_with(configured)
}

fn canonical_or_original(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn command_tokens_match(configured_tokens: &[String], candidate_command: &str) -> bool {
    if configured_tokens.is_empty() {
        return false;
    }
    let Ok(candidate_tokens) = split_command_words(candidate_command) else {
        return false;
    };
    if candidate_tokens.len() < configured_tokens.len() {
        return false;
    }
    configured_tokens
        .iter()
        .zip(candidate_tokens.iter())
        .enumerate()
        .all(|(index, (configured, candidate))| {
            let configured = normalize_command_dashes(configured);
            let candidate = normalize_command_dashes(candidate);
            if index == 0 {
                command_name_matches(&configured, &candidate)
            } else {
                configured == candidate
            }
        })
}

fn command_name_matches(configured: &str, candidate: &str) -> bool {
    configured == candidate
        || Path::new(candidate)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name == configured)
            .unwrap_or(false)
}

async fn live_pid_for_record(record: &RuntimeProcessRecord) -> Option<u32> {
    let process_group_id = normalized_process_group_id(record);
    if process_is_live_in_group(record.pid, process_group_id).await {
        Some(record.pid)
    } else {
        platform::live_process_in_group(process_group_id).await
    }
}

async fn process_is_live_in_group(pid: u32, process_group_id: u32) -> bool {
    platform::process_info_for_pid(pid)
        .await
        .map(|(found_process_group_id, stat)| {
            found_process_group_id == process_group_id && platform::is_live_stat(&stat)
        })
        .unwrap_or(false)
}

pub async fn restart_process(
    app: AppHandle,
    state: AppState,
    process_id: Id,
) -> ApiResponse<ProcessRuntimeState> {
    // launchd-supervised: a restart is a single `launchctl kickstart -k`, which
    // does the kill+relaunch atomically; never spawn a competing child.
    let launchd_process = {
        let config = state.config.read().await;
        config
            .processes
            .iter()
            .find(|process| process.id == process_id)
            .filter(|process| process.launchd.is_some())
            .cloned()
    };
    if let Some(process) = launchd_process {
        let sup = process.launchd.clone().expect("launchd present");
        {
            let mut states = state.runtime.states.write().await;
            let runtime = states
                .entry(process_id.clone())
                .or_insert_with(|| ProcessRuntimeState::stopped(process_id.clone()));
            runtime.restart_count += 1;
        }
        return match restart_launchd_process(&app, &state, &process, &sup).await {
            Ok(runtime) => ApiResponse::ok(runtime),
            Err(error) => ApiResponse::err(error),
        };
    }

    let existing = state.runtime.states.read().await.get(&process_id).cloned();
    if matches!(
        existing.map(|state| state.current_status),
        Some(ProcessStatus::Running | ProcessStatus::Starting | ProcessStatus::Stopping)
    ) {
        let response = stop_process(app.clone(), state.clone(), process_id.clone()).await;
        if !response.success {
            return response;
        }
        let stop_timeout_ms = state.config.read().await.settings.stop_timeout_ms;
        wait_for_processes_to_stop(&state, &[process_id.clone()], stop_timeout_ms).await;
    }
    {
        let mut states = state.runtime.states.write().await;
        let runtime = states
            .entry(process_id.clone())
            .or_insert_with(|| ProcessRuntimeState::stopped(process_id.clone()));
        runtime.restart_count += 1;
    }
    start_process(app, state, process_id).await
}

async fn wait_for_processes_to_stop(state: &AppState, process_ids: &[Id], stop_timeout_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(stop_timeout_ms.saturating_add(1_000));
    loop {
        let still_stopping = {
            let states = state.runtime.states.read().await;
            process_ids.iter().any(|process_id| {
                states
                    .get(process_id)
                    .map(|runtime| {
                        matches!(
                            runtime.current_status,
                            ProcessStatus::Running
                                | ProcessStatus::Starting
                                | ProcessStatus::Queued
                                | ProcessStatus::Stopping
                        )
                    })
                    .unwrap_or(false)
            })
        };
        if !still_stopping || Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub async fn start_project(
    app: AppHandle,
    state: AppState,
    project_id: Id,
) -> ApiResponse<ProjectDetail> {
    let processes = ordered_processes(&state, &project_id).await;
    for process in processes
        .into_iter()
        .filter(|process| process.auto_start || process.visible)
    {
        let response = start_process(app.clone(), state.clone(), process.id.clone()).await;
        if !response.success {
            return ApiResponse::err(response.error.unwrap_or_else(|| {
                ApiError::new(
                    "COMMAND_EXECUTION_FAILED",
                    "Unable to start project process",
                    true,
                )
            }));
        }
        if let Some(delay) = process.startup_delay_ms {
            sleep(Duration::from_millis(delay)).await;
        }
    }
    match get_project_detail(&state, &project_id).await {
        Ok(detail) => ApiResponse::ok(detail),
        Err(error) => ApiResponse::err(error),
    }
}

pub async fn start_auto_start_processes(
    app: AppHandle,
    state: AppState,
    project_id: Id,
) -> ApiResponse<ProjectDetail> {
    let processes = ordered_processes(&state, &project_id).await;
    for process in processes.into_iter().filter(|process| process.auto_start) {
        let response = start_process(app.clone(), state.clone(), process.id.clone()).await;
        if !response.success {
            return ApiResponse::err(response.error.unwrap_or_else(|| {
                ApiError::new(
                    "COMMAND_EXECUTION_FAILED",
                    "Unable to start marked project process",
                    true,
                )
            }));
        }
        if let Some(delay) = process.startup_delay_ms {
            sleep(Duration::from_millis(delay)).await;
        }
    }
    match get_project_detail(&state, &project_id).await {
        Ok(detail) => ApiResponse::ok(detail),
        Err(error) => ApiResponse::err(error),
    }
}

pub async fn start_marked_projects_on_launch(app: AppHandle, state: AppState) {
    let mut projects = {
        let config = state.config.read().await;
        if !config.settings.auto_start_marked_projects {
            return;
        }
        config
            .projects
            .iter()
            .filter(|project| project.auto_start)
            .cloned()
            .collect::<Vec<Project>>()
    };
    projects.sort_by_key(|project| project.startup_order);
    for project in projects {
        let processes = ordered_processes(&state, &project.id).await;
        for process in processes.into_iter().filter(|process| process.auto_start) {
            let already_active = state
                .runtime
                .states
                .read()
                .await
                .get(&process.id)
                .map(|runtime| {
                    matches!(
                        runtime.current_status,
                        ProcessStatus::Running
                            | ProcessStatus::Starting
                            | ProcessStatus::Queued
                            | ProcessStatus::Stopping
                    )
                })
                .unwrap_or(false);
            if already_active {
                continue;
            }
            let _ = start_process(app.clone(), state.clone(), process.id.clone()).await;
            if let Some(delay) = process.startup_delay_ms {
                sleep(Duration::from_millis(delay)).await;
            }
        }
    }
}

pub async fn stop_project(
    app: AppHandle,
    state: AppState,
    project_id: Id,
) -> ApiResponse<ProjectDetail> {
    let mut processes = ordered_processes(&state, &project_id).await;
    processes.reverse();
    for process in processes {
        let _ = stop_process(app.clone(), state.clone(), process.id).await;
    }
    match get_project_detail(&state, &project_id).await {
        Ok(detail) => ApiResponse::ok(detail),
        Err(error) => ApiResponse::err(error),
    }
}

pub async fn restart_project(
    app: AppHandle,
    state: AppState,
    project_id: Id,
) -> ApiResponse<ProjectDetail> {
    let process_ids: Vec<Id> = ordered_processes(&state, &project_id)
        .await
        .into_iter()
        .map(|process| process.id)
        .collect();
    let stopped = stop_project(app.clone(), state.clone(), project_id.clone()).await;
    if !stopped.success {
        return stopped;
    }
    let stop_timeout_ms = state.config.read().await.settings.stop_timeout_ms;
    wait_for_processes_to_stop(&state, &process_ids, stop_timeout_ms).await;
    start_project(app, state, project_id).await
}

pub async fn restart_failed_processes(
    app: AppHandle,
    state: AppState,
    project_id: Option<Id>,
) -> ApiResponse<Vec<ProcessRuntimeState>> {
    let failed_processes: Vec<ProcessDefinition> = {
        let config = state.config.read().await;
        let states = state.runtime.states.read().await;
        config
            .processes
            .iter()
            .filter(|process| {
                project_id
                    .as_ref()
                    .map(|id| &process.project_id == id)
                    .unwrap_or(true)
            })
            .filter(|process| {
                states
                    .get(&process.id)
                    .map(|runtime| {
                        matches!(
                            runtime.current_status,
                            ProcessStatus::Failed
                                | ProcessStatus::Crashed
                                | ProcessStatus::WaitingDependency
                                | ProcessStatus::Blocked
                        )
                    })
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    };

    for process in failed_processes {
        let _ = restart_process(app.clone(), state.clone(), process.id).await;
    }
    ApiResponse::ok(
        state
            .runtime
            .states
            .read()
            .await
            .values()
            .cloned()
            .collect(),
    )
}

pub async fn get_runtime_state(
    state: AppState,
    process_id: Id,
) -> ApiResponse<ProcessRuntimeState> {
    ApiResponse::ok(
        state
            .runtime
            .states
            .read()
            .await
            .get(&process_id)
            .cloned()
            .unwrap_or_else(|| ProcessRuntimeState::stopped(process_id)),
    )
}

pub async fn get_all_runtime_states(state: AppState) -> ApiResponse<Vec<ProcessRuntimeState>> {
    ApiResponse::ok(
        state
            .runtime
            .states
            .read()
            .await
            .values()
            .cloned()
            .collect(),
    )
}

pub async fn get_log_history(
    state: AppState,
    filters: Option<LogHistoryFilters>,
) -> ApiResponse<Vec<LogEntry>> {
    let filters = filters.unwrap_or(LogHistoryFilters {
        project_id: None,
        process_id: None,
        limit: Some(1000),
        since: Some(log_history_since()),
    });
    let limit = filters.limit.unwrap_or(1000);
    let logs = state.runtime.logs.read().await;
    ApiResponse::ok(
        logs.iter()
            .filter(|log| {
                filters
                    .since
                    .as_ref()
                    .map(|since| log.timestamp >= *since)
                    .unwrap_or(true)
            })
            .filter(|log| {
                filters
                    .project_id
                    .as_ref()
                    .map(|id| &log.project_id == id)
                    .unwrap_or(true)
            })
            .filter(|log| {
                filters
                    .process_id
                    .as_ref()
                    .map(|id| &log.process_id == id)
                    .unwrap_or(true)
            })
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    )
}

pub async fn clear_log_history(
    app: AppHandle,
    state: AppState,
    project_id: Option<Id>,
) -> ApiResponse<bool> {
    {
        let mut logs = state.runtime.logs.write().await;
        match project_id.as_deref() {
            Some(project_id) => logs.retain(|log| log.project_id != project_id),
            None => logs.clear(),
        }
    }
    let _log_history_io = state.runtime.log_history_io.lock().await;
    if let Err(error) = storage::clear_log_history(&app, project_id.as_deref()) {
        return ApiResponse::err(error);
    }
    ApiResponse::ok(true)
}

pub async fn export_logs(
    state: AppState,
    filters: Option<LogHistoryFilters>,
) -> ApiResponse<String> {
    let filters = filters.unwrap_or(LogHistoryFilters {
        project_id: None,
        process_id: None,
        limit: None,
        since: None,
    });
    let logs = state.runtime.logs.read().await;
    let selected: Vec<LogEntry> = logs
        .iter()
        .filter(|log| {
            filters
                .project_id
                .as_ref()
                .map(|id| &log.project_id == id)
                .unwrap_or(true)
        })
        .filter(|log| {
            filters
                .process_id
                .as_ref()
                .map(|id| &log.process_id == id)
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    match serde_json::to_string_pretty(&selected) {
        Ok(content) => ApiResponse::ok(content),
        Err(error) => ApiResponse::err(ApiError::with_details(
            "CONFIG_SERIALIZATION_FAILED",
            "Unable to export logs",
            error,
            false,
        )),
    }
}

pub async fn run_process_health_check(
    app: AppHandle,
    state: AppState,
    process_id: Id,
) -> ApiResponse<ProcessRuntimeState> {
    let (project, process) = {
        let config = state.config.read().await;
        let process = match config
            .processes
            .iter()
            .find(|process| process.id == process_id)
            .cloned()
        {
            Some(process) => process,
            None => {
                return ApiResponse::err(ApiError::new(
                    "PROCESS_NOT_FOUND",
                    "Process not found",
                    false,
                ))
            }
        };
        let project = config
            .projects
            .iter()
            .find(|project| project.id == process.project_id)
            .cloned();
        (project, process)
    };
    let cwd = process
        .working_directory
        .as_deref()
        .or(project.as_ref().map(|project| project.root_path.as_str()));
    let remote_machine = resolve_remote_machine(&state, &process).await;
    let status = match health::run_health_check(
        &process.health_check,
        cwd,
        remote_machine.as_ref(),
    )
    .await
    {
        Ok(status) => status,
        Err(error) => {
            append_log(
                &app,
                &state,
                &process,
                StreamType::System,
                LogLevel::Warn,
                error.message.clone(),
            )
            .await;
            HealthStatus::Unhealthy
        }
    };
    let mut runtime = state
        .runtime
        .states
        .read()
        .await
        .get(&process_id)
        .cloned()
        .unwrap_or_else(|| ProcessRuntimeState::stopped(process_id.clone()));
    runtime.health_status = Some(status);
    runtime.last_heartbeat = Some(Utc::now());
    set_runtime(&app, &state, runtime.clone(), "process_health_changed").await;
    ApiResponse::ok(runtime)
}

pub async fn get_health_summary(
    state: AppState,
    project_id: Option<Id>,
) -> ApiResponse<HashMap<String, usize>> {
    let ids: HashSet<Id> = {
        let config = state.config.read().await;
        config
            .processes
            .iter()
            .filter(|process| {
                project_id
                    .as_ref()
                    .map(|id| &process.project_id == id)
                    .unwrap_or(true)
            })
            .map(|process| process.id.clone())
            .collect()
    };
    let mut summary = HashMap::from([
        ("healthy".to_string(), 0_usize),
        ("unhealthy".to_string(), 0_usize),
        ("unknown".to_string(), 0_usize),
    ]);
    for runtime in state
        .runtime
        .states
        .read()
        .await
        .values()
        .filter(|runtime| ids.contains(&runtime.process_id))
    {
        let bucket = match runtime.health_status {
            Some(HealthStatus::Healthy) => "healthy",
            Some(HealthStatus::Unhealthy | HealthStatus::Degraded) => "unhealthy",
            _ => "unknown",
        };
        *summary.entry(bucket.to_string()).or_insert(0) += 1;
    }
    ApiResponse::ok(summary)
}

pub async fn dashboard_summary(state: AppState) -> DashboardSummary {
    let config = state.config.read().await;
    let states = state.runtime.states.read().await;
    let logs = state.runtime.logs.read().await;
    DashboardSummary {
        project_count: config.projects.len(),
        process_count: config.processes.len(),
        running_process_count: states
            .values()
            .filter(|state| matches!(state.current_status, ProcessStatus::Running))
            .count(),
        failed_process_count: states
            .values()
            .filter(|state| {
                matches!(
                    state.current_status,
                    ProcessStatus::Failed
                        | ProcessStatus::Crashed
                        | ProcessStatus::Blocked
                        | ProcessStatus::WaitingDependency
                )
            })
            .count(),
        port_conflict_count: detect_port_conflicts(states.values().collect()).len(),
        auto_start_project_count: config
            .projects
            .iter()
            .filter(|project| project.auto_start)
            .count(),
        recent_problem_logs: logs
            .iter()
            .filter(|log| matches!(log.level, LogLevel::Warn | LogLevel::Error))
            .rev()
            .take(12)
            .cloned()
            .collect(),
    }
}

pub async fn detect_ports_in_use(state: AppState) -> ApiResponse<Vec<PortBinding>> {
    let states = state.runtime.states.read().await;
    ApiResponse::ok(detect_port_conflicts(states.values().collect()))
}

fn spawn_log_reader<R>(
    app: AppHandle,
    state: AppState,
    process: ProcessDefinition,
    stream: StreamType,
    reader: R,
    is_remote: bool,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        let level = if matches!(stream, StreamType::Stderr) {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if is_remote && matches!(stream, StreamType::Stderr) {
                        if let Some(remote_pid) = ssh_executor::parse_remote_pid_marker(&line) {
                            record_remote_pid(&state, &process.id, remote_pid).await;
                            append_log(
                                &app,
                                &state,
                                &process,
                                StreamType::System,
                                LogLevel::Debug,
                                format!("Remote pid {remote_pid} captured"),
                            )
                            .await;
                            continue;
                        }
                    }
                    let clean_line = if is_remote {
                        line.trim_end_matches('\r').to_string()
                    } else {
                        line
                    };
                    append_log(
                        &app,
                        &state,
                        &process,
                        stream.clone(),
                        level.clone(),
                        clean_line,
                    )
                    .await;
                }
                Ok(None) => {
                    append_log(
                        &app,
                        &state,
                        &process,
                        StreamType::System,
                        LogLevel::Debug,
                        format!("{:?} stream closed", stream),
                    )
                    .await;
                    break;
                }
                Err(err) => {
                    append_log(
                        &app,
                        &state,
                        &process,
                        StreamType::System,
                        LogLevel::Warn,
                        format!("{:?} reader error: {err}", stream),
                    )
                    .await;
                    break;
                }
            }
        }
        flush_process_log_batch(&app, &state, &process.id).await;
    });
}

async fn append_log(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    stream: StreamType,
    level: LogLevel,
    message: impl Into<String>,
) {
    let message = message.into();
    let entry = LogEntry {
        id: storage::id("log"),
        process_id: process.id.clone(),
        project_id: process.project_id.clone(),
        timestamp: Utc::now(),
        stream: stream.clone(),
        level,
        raw: Some(message.clone()),
        message,
    };
    {
        let retention = state.config.read().await.settings.log_retention_lines;
        let mut logs = state.runtime.logs.write().await;
        logs.push_back(entry.clone());
        while logs.len() > retention {
            logs.pop_front();
        }
    }
    match stream {
        StreamType::System => {
            persist_log_entry(app, state, &entry).await;
            if let Err(err) = app.emit("process_log", entry) {
                eprintln!("[log] emit process_log failed: {err}");
            }
        }
        StreamType::Stdout | StreamType::Stderr => {
            let batcher = get_or_create_batcher(state, &process.id).await;
            let drained = {
                let mut buffer = batcher.lock().await;
                buffer.push(entry);
                if buffer.len() >= LOG_BATCH_FLUSH_LIMIT {
                    Some(std::mem::take(&mut *buffer))
                } else {
                    None
                }
            };
            if let Some(entries) = drained {
                emit_log_batch(app, state, entries).await;
            }
        }
    }
}

async fn persist_log_entry(app: &AppHandle, state: &AppState, entry: &LogEntry) {
    let _io = state.runtime.log_history_io.lock().await;
    if let Err(err) = storage::append_log_entry(app, entry) {
        eprintln!(
            "[log] append_log_entry failed: {} ({})",
            err.message, err.code
        );
    }
}

async fn persist_log_entries(app: &AppHandle, state: &AppState, entries: &[LogEntry]) {
    if entries.is_empty() {
        return;
    }
    let _io = state.runtime.log_history_io.lock().await;
    for entry in entries {
        if let Err(err) = storage::append_log_entry(app, entry) {
            eprintln!(
                "[log] append_log_entry batch failed: {} ({})",
                err.message, err.code
            );
            break;
        }
    }
}

async fn get_or_create_batcher(state: &AppState, process_id: &str) -> Arc<Mutex<Vec<LogEntry>>> {
    {
        let map = state.runtime.log_batchers.read().await;
        if let Some(existing) = map.get(process_id) {
            return existing.clone();
        }
    }
    let mut map = state.runtime.log_batchers.write().await;
    map.entry(process_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

async fn emit_log_batch(app: &AppHandle, state: &AppState, entries: Vec<LogEntry>) {
    if entries.is_empty() {
        return;
    }
    persist_log_entries(app, state, &entries).await;
    if let Err(err) = app.emit("process_log_batch", entries) {
        eprintln!("[log] emit process_log_batch failed: {err}");
    }
}

async fn flush_process_log_batch(app: &AppHandle, state: &AppState, process_id: &str) {
    let batcher = {
        let map = state.runtime.log_batchers.read().await;
        map.get(process_id).cloned()
    };
    let Some(batcher) = batcher else {
        return;
    };
    let drained = {
        let mut buffer = batcher.lock().await;
        if buffer.is_empty() {
            return;
        }
        std::mem::take(&mut *buffer)
    };
    emit_log_batch(app, state, drained).await;
}

pub fn start_log_batch_flusher(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            sleep(Duration::from_millis(LOG_BATCH_FLUSH_INTERVAL_MS)).await;
            let batchers: Vec<(Id, Arc<Mutex<Vec<LogEntry>>>)> = {
                let map = state.runtime.log_batchers.read().await;
                map.iter()
                    .map(|(id, batcher)| (id.clone(), batcher.clone()))
                    .collect()
            };
            for (_process_id, batcher) in batchers {
                let drained = {
                    let mut buffer = batcher.lock().await;
                    if buffer.is_empty() {
                        continue;
                    }
                    std::mem::take(&mut *buffer)
                };
                emit_log_batch(&app, &state, drained).await;
            }
        }
    });
}

pub async fn record_frontend_error(
    app: &AppHandle,
    state: &AppState,
    record: crate::models::FrontendErrorRecord,
) {
    {
        let mut errors = state.runtime.frontend_errors.write().await;
        if errors.len() >= crate::state::FRONTEND_ERROR_RETENTION {
            errors.pop_front();
        }
        errors.push_back(record.clone());
    }
    let mut detail = record.message.clone();
    if let Some(stack) = record.stack.as_ref().filter(|stack| !stack.is_empty()) {
        detail.push_str("\n");
        detail.push_str(stack);
    }
    if let Some(component_stack) = record
        .component_stack
        .as_ref()
        .filter(|stack| !stack.is_empty())
    {
        detail.push_str("\nComponentStack:\n");
        detail.push_str(component_stack);
    }
    let entry = LogEntry {
        id: storage::id("log"),
        process_id: "_frontend".to_string(),
        project_id: "_app".to_string(),
        timestamp: record.timestamp,
        stream: StreamType::System,
        level: LogLevel::Error,
        raw: Some(detail.clone()),
        message: format!("[frontend:{}] {}", record.source, record.message),
    };
    {
        let retention = state.config.read().await.settings.log_retention_lines;
        let mut logs = state.runtime.logs.write().await;
        logs.push_back(entry.clone());
        while logs.len() > retention {
            logs.pop_front();
        }
    }
    persist_log_entry(app, state, &entry).await;
    if let Err(err) = app.emit("process_log", entry) {
        eprintln!("[log] emit frontend error failed: {err}");
    }
}

pub async fn recent_frontend_errors(state: &AppState) -> Vec<crate::models::FrontendErrorRecord> {
    state
        .runtime
        .frontend_errors
        .read()
        .await
        .iter()
        .cloned()
        .collect()
}

pub fn start_log_history_pruner(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            sleep(Duration::from_secs(60)).await;
            let _log_history_io = state.runtime.log_history_io.lock().await;
            if let Err(err) = storage::prune_log_history(&app, log_history_since()) {
                eprintln!(
                    "[log] prune_log_history failed: {} ({})",
                    err.message, err.code
                );
            }
        }
    });
}

async fn mark_spawn_failure(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    runtime: &mut ProcessRuntimeState,
    details: String,
) {
    runtime.current_status = ProcessStatus::Failed;
    runtime.stopped_at = Some(Utc::now());
    runtime.last_error = Some(details.clone());
    runtime.health_status = Some(HealthStatus::Unknown);
    append_log(
        app,
        state,
        process,
        StreamType::System,
        LogLevel::Error,
        details,
    )
    .await;
    set_runtime(app, state, runtime.clone(), "process_failed").await;
}

async fn set_runtime(app: &AppHandle, state: &AppState, runtime: ProcessRuntimeState, event: &str) {
    {
        let mut states = state.runtime.states.write().await;
        states.insert(runtime.process_id.clone(), runtime.clone());
    }
    if let Err(err) = app.emit(event, runtime.clone()) {
        eprintln!("[runtime] emit {event} failed: {err}");
    }
    maybe_notify_runtime_event(app, state, event, &runtime).await;
}

async fn maybe_notify_runtime_event(
    app: &AppHandle,
    state: &AppState,
    event: &str,
    runtime: &ProcessRuntimeState,
) {
    if event != "process_failed" && event != "process_health_changed" {
        return;
    }

    let config = state.config.read().await;
    if !config.settings.notifications_enabled {
        return;
    }

    let Some(process) = config
        .processes
        .iter()
        .find(|process| process.id == runtime.process_id)
        .cloned()
    else {
        return;
    };

    let project_name = config
        .projects
        .iter()
        .find(|project| project.id == process.project_id)
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "Project".to_string());
    drop(config);

    let should_notify = match event {
        "process_failed" => true,
        "process_health_changed" => matches!(
            runtime.health_status,
            Some(HealthStatus::Unhealthy | HealthStatus::Degraded)
        ),
        _ => false,
    };
    if !should_notify {
        return;
    }

    let title = if event == "process_failed" {
        format!("{} failed", process.name)
    } else {
        format!("{} health degraded", process.name)
    };
    let body = runtime
        .last_error
        .as_deref()
        .map(|error| format!("{}: {error}", project_name))
        .unwrap_or_else(|| format!("{}: status changed", project_name));
    let _ = app.notification().builder().title(title).body(body).show();
}

async fn missing_dependency(state: &AppState, process: &ProcessDefinition) -> Option<String> {
    if process.depends_on.is_empty() {
        return None;
    }
    let config = state.config.read().await;
    let states = state.runtime.states.read().await;
    for key in &process.depends_on {
        let dependency = config
            .processes
            .iter()
            .find(|candidate| candidate.project_id == process.project_id && candidate.key == *key);
        match dependency.and_then(|dependency| states.get(&dependency.id)) {
            Some(runtime) if matches!(runtime.current_status, ProcessStatus::Running) => {}
            _ => return Some(key.clone()),
        }
    }
    None
}

async fn resolve_remote_machine(state: &AppState, process: &ProcessDefinition) -> Option<Machine> {
    let machine_id = process.machine_id.as_ref()?;
    let config = state.config.read().await;
    let machine = config
        .machines
        .iter()
        .find(|machine| &machine.id == machine_id)?
        .clone();
    if machine.is_default_local {
        None
    } else {
        Some(machine)
    }
}

async fn record_remote_pid(state: &AppState, process_id: &str, remote_pid: u32) {
    let mut remote_pids = state.runtime.remote_pids.write().await;
    remote_pids.insert(process_id.to_string(), remote_pid);
}

async fn cleanup_remote_process_after_exit(
    state: &AppState,
    machine: Option<&Machine>,
    process_id: &str,
) {
    let remote_pid = state
        .runtime
        .remote_pids
        .write()
        .await
        .remove(process_id);
    if let (Some(machine), Some(remote_pid)) = (machine, remote_pid) {
        let _ = ssh_executor::kill_remote_process(machine, remote_pid, "KILL").await;
    }
}

fn resolve_working_directory(
    project: &Project,
    process: &ProcessDefinition,
) -> Result<String, ApiError> {
    resolve_working_directory_with_locality(project, process, false)
}

fn resolve_working_directory_with_locality(
    project: &Project,
    process: &ProcessDefinition,
    is_remote: bool,
) -> Result<String, ApiError> {
    let cwd = process
        .working_directory
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| project.root_path.clone());
    if !is_remote && !Path::new(&cwd).exists() {
        return Err(ApiError::with_details(
            "INVALID_PROJECT_PATH",
            "Working directory does not exist",
            cwd,
            false,
        ));
    }
    Ok(cwd)
}

fn process_command_tokens(process: &ProcessDefinition) -> Result<Vec<String>, ApiError> {
    let mut tokens = split_command_words(&process.command).map_err(|error| {
        ApiError::with_details(
            "INVALID_PROCESS_DEFINITION",
            "Command could not be parsed",
            error,
            false,
        )
    })?;
    tokens.extend(
        process
            .args
            .iter()
            .map(|arg| normalize_command_dashes(arg).trim().to_string())
            .filter(|arg| !arg.is_empty()),
    );
    if tokens.is_empty() {
        return Err(ApiError::new(
            "INVALID_PROCESS_DEFINITION",
            "Command is required",
            false,
        ));
    }
    Ok(tokens)
}

pub(crate) fn direct_process_command(tokens: &[String]) -> Command {
    let mut command = Command::new(&tokens[0]);
    command.args(&tokens[1..]);
    command
}

pub(crate) fn shell_process_command(tokens: &[String]) -> Command {
    platform::shell_command(tokens)
}

pub(crate) fn configure_process_command(
    command: &mut Command,
    cwd: &str,
    env: &HashMap<String, String>,
    memory_limit_mb: Option<u64>,
) {
    // Put each managed command in its own process group / console group so shells
    // and the workers they spawn can be terminated together.
    platform::set_process_group(command);
    command.current_dir(cwd);
    command.envs(effective_process_env(env));
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(memory_limit_mb) = memory_limit_mb {
        platform::apply_memory_limit(command, mb_to_bytes(memory_limit_mb));
    }
}

fn effective_process_env(process_env: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env = process_env.clone();
    if !env.contains_key("PATH") {
        let home = env::var("HOME").or_else(|_| env::var("USERPROFILE")).ok();
        env.insert(
            "PATH".to_string(),
            platform::default_process_path(env::var("PATH").ok(), home),
        );
    }
    env
}

fn spawn_memory_monitor(
    app: AppHandle,
    state: AppState,
    project_id: Id,
    process_id: Id,
    mut pid: u32,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            sleep(Duration::from_secs(2)).await;
            let tracked_record = state
                .runtime
                .process_records
                .read()
                .await
                .get(&process_id)
                .cloned();
            let Some(tracked_record) = tracked_record else {
                break;
            };
            if tracked_record.pid != pid {
                pid = tracked_record.pid;
            }

            let Some((memory_usage, cpu_usage)) = platform::read_process_metrics(pid).await else {
                let process_group_id = normalized_process_group_id(&tracked_record);
                if let Some(live_pid) = platform::live_process_in_group(process_group_id).await {
                    update_tracked_process_pid(&state, &process_id, live_pid, process_group_id)
                        .await;
                    let _ = persist_runtime_processes(&app, &state).await;
                    pid = live_pid;
                    continue;
                }
                untrack_runtime_process(&state, &process_id).await;
                let _ = persist_runtime_processes(&app, &state).await;
                if let Some((_, process)) =
                    config_project_process_pair(&state, &project_id, &process_id).await
                {
                    // A recovered process has no child.wait task, so this memory
                    // monitor is its ONLY crash detector. Distinguish an operator
                    // stop from a real crash and, on a crash, schedule the same
                    // auto-restart the wait path would — otherwise a recovered
                    // process that crashes stays down even under RestartPolicy
                    // Always/OnFailure. See MEDIA_GUARD_TECHDEBT_PLAN P2.
                    let prior_status = state
                        .runtime
                        .states
                        .read()
                        .await
                        .get(&process_id)
                        .map(|r| r.current_status.clone());
                    let operator_stop = stop_was_requested(&state, &process_id, process_group_id)
                        .await
                        || matches!(
                            prior_status,
                            Some(ProcessStatus::Stopping | ProcessStatus::Stopped)
                        );

                    let mut runtime = ProcessRuntimeState::stopped(process_id.clone());
                    runtime.stopped_at = Some(Utc::now());
                    if operator_stop {
                        set_runtime(&app, &state, runtime, "process_stopped").await;
                        append_log(
                            &app,
                            &state,
                            &process,
                            StreamType::System,
                            LogLevel::Info,
                            format!("Recovered process group {process_group_id} exited"),
                        )
                        .await;
                    } else {
                        runtime.current_status = ProcessStatus::Crashed;
                        runtime.last_error =
                            Some("Recovered process group exited unexpectedly".to_string());
                        set_runtime(&app, &state, runtime, "process_failed").await;
                        append_log(
                            &app,
                            &state,
                            &process,
                            StreamType::System,
                            LogLevel::Error,
                            format!("Recovered process group {process_group_id} crashed"),
                        )
                        .await;
                        schedule_auto_restart_if_eligible(&app, &state, &process).await;
                    }
                }
                break;
            };

            let Some((project, process)) =
                config_project_process_pair(&state, &project_id, &process_id).await
            else {
                break;
            };

            update_process_metrics(&app, &state, &process_id, memory_usage, cpu_usage).await;
            append_metric_sample(&state, &process_id, memory_usage, cpu_usage).await;

            if let Some(limit_mb) = process.memory_limit_mb {
                let limit_bytes = mb_to_bytes(limit_mb);
                if memory_usage > limit_bytes {
                    fail_process_for_memory_limit(
                        &app,
                        &state,
                        &process,
                        pid,
                        memory_usage,
                        limit_bytes,
                    )
                    .await;
                    break;
                }
            }

            if let Some(limit_mb) = project.memory_limit_mb {
                let limit_bytes = mb_to_bytes(limit_mb);
                let total_usage = project_memory_usage(&state, &project.id).await;
                if total_usage > limit_bytes {
                    fail_project_for_memory_limit(&app, &state, &project, total_usage, limit_bytes)
                        .await;
                    break;
                }
            }
        }
    });
}

async fn config_project_process_pair(
    state: &AppState,
    project_id: &str,
    process_id: &str,
) -> Option<(Project, ProcessDefinition)> {
    let config = state.config.read().await;
    let project = config
        .projects
        .iter()
        .find(|project| project.id == project_id)
        .cloned()?;
    let process = config
        .processes
        .iter()
        .find(|process| process.id == process_id)
        .cloned()?;
    Some((project, process))
}

async fn update_process_metrics(
    app: &AppHandle,
    state: &AppState,
    process_id: &str,
    memory_usage: u64,
    cpu_usage: Option<f64>,
) {
    let Some(mut runtime) = state.runtime.states.read().await.get(process_id).cloned() else {
        return;
    };
    if !matches!(
        runtime.current_status,
        ProcessStatus::Running | ProcessStatus::Starting
    ) {
        return;
    }
    let prev_memory = runtime.memory_usage;
    let prev_cpu = runtime.cpu_usage;
    runtime.memory_usage = Some(memory_usage);
    if cpu_usage.is_some() {
        runtime.cpu_usage = cpu_usage;
    }
    if !metrics_delta_significant(prev_memory, memory_usage, prev_cpu, cpu_usage) {
        return;
    }
    set_runtime(app, state, runtime, "process_metrics_changed").await;
}

fn metrics_delta_significant(
    prev_memory: Option<u64>,
    new_memory: u64,
    prev_cpu: Option<f64>,
    new_cpu: Option<f64>,
) -> bool {
    let memory_changed = match prev_memory {
        None => true,
        Some(prev) => {
            let threshold = (prev / 100).max(1_048_576);
            new_memory.abs_diff(prev) >= threshold
        }
    };
    let cpu_changed = match (prev_cpu, new_cpu) {
        (None, Some(_)) | (Some(_), None) => true,
        (None, None) => false,
        (Some(prev), Some(next)) => (next - prev).abs() >= 0.5,
    };
    memory_changed || cpu_changed
}

const METRICS_HISTORY_WINDOW_SECONDS: i64 = 600;
const METRICS_HISTORY_HARD_CAP: usize = 400;

async fn append_metric_sample(
    state: &AppState,
    process_id: &str,
    memory_usage: u64,
    cpu_usage: Option<f64>,
) {
    let now = Utc::now();
    let cutoff = now - ChronoDuration::seconds(METRICS_HISTORY_WINDOW_SECONDS);
    let sample = MetricSample {
        timestamp: now,
        cpu_usage,
        memory_usage: Some(memory_usage),
    };
    let mut history = state.runtime.metrics_history.write().await;
    let buffer = history.entry(process_id.to_string()).or_default();
    buffer.push_back(sample);
    while buffer.front().map_or(false, |s| s.timestamp < cutoff) {
        buffer.pop_front();
    }
    while buffer.len() > METRICS_HISTORY_HARD_CAP {
        buffer.pop_front();
    }
}

async fn project_memory_usage(state: &AppState, project_id: &str) -> u64 {
    let process_ids: HashSet<Id> = {
        let config = state.config.read().await;
        config
            .processes
            .iter()
            .filter(|process| process.project_id == project_id)
            .map(|process| process.id.clone())
            .collect()
    };
    state
        .runtime
        .states
        .read()
        .await
        .values()
        .filter(|runtime| process_ids.contains(&runtime.process_id))
        .filter(|runtime| {
            matches!(
                runtime.current_status,
                ProcessStatus::Running | ProcessStatus::Starting
            )
        })
        .filter_map(|runtime| runtime.memory_usage)
        .sum()
}

async fn fail_process_for_memory_limit(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    pid: u32,
    usage_bytes: u64,
    limit_bytes: u64,
) {
    let details = format!(
        "Process memory limit exceeded: {} used over {} limit",
        format_bytes(usage_bytes),
        format_bytes(limit_bytes)
    );
    append_log(
        app,
        state,
        process,
        StreamType::System,
        LogLevel::Error,
        details.clone(),
    )
    .await;
    mark_process_memory_failure(app, state, process, details, usage_bytes).await;
    let _ = force_kill_process_group(pid);
}

async fn fail_project_for_memory_limit(
    app: &AppHandle,
    state: &AppState,
    project: &Project,
    usage_bytes: u64,
    limit_bytes: u64,
) {
    let process_ids: HashSet<Id> = {
        let config = state.config.read().await;
        config
            .processes
            .iter()
            .filter(|process| process.project_id == project.id)
            .map(|process| process.id.clone())
            .collect()
    };
    let already_triggered = state
        .runtime
        .states
        .read()
        .await
        .values()
        .filter(|runtime| process_ids.contains(&runtime.process_id))
        .any(|runtime| {
            runtime
                .last_error
                .as_deref()
                .map(|error| error.starts_with("Project memory limit exceeded"))
                .unwrap_or(false)
        });
    if already_triggered {
        return;
    }

    let processes = {
        let config = state.config.read().await;
        config
            .processes
            .iter()
            .filter(|process| process.project_id == project.id)
            .cloned()
            .collect::<Vec<_>>()
    };
    let pids = state.runtime.pids.read().await.clone();
    let details = format!(
        "Project memory limit exceeded: {} used over {} limit",
        format_bytes(usage_bytes),
        format_bytes(limit_bytes)
    );
    for process in processes {
        append_log(
            app,
            state,
            &process,
            StreamType::System,
            LogLevel::Error,
            details.clone(),
        )
        .await;
        let memory_usage = state
            .runtime
            .states
            .read()
            .await
            .get(&process.id)
            .and_then(|runtime| runtime.memory_usage)
            .unwrap_or(0);
        mark_process_memory_failure(app, state, &process, details.clone(), memory_usage).await;
        if let Some(pid) = pids.get(&process.id) {
            let _ = force_kill_process_group(*pid);
        }
    }
}

async fn mark_process_memory_failure(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
    details: String,
    usage_bytes: u64,
) {
    let mut runtime = state
        .runtime
        .states
        .read()
        .await
        .get(&process.id)
        .cloned()
        .unwrap_or_else(|| ProcessRuntimeState::stopped(process.id.clone()));
    runtime.current_status = ProcessStatus::Failed;
    runtime.last_error = Some(details);
    runtime.memory_usage = Some(usage_bytes);
    runtime.health_status = Some(HealthStatus::Unknown);
    set_runtime(app, state, runtime, "process_failed").await;
}

fn mb_to_bytes(limit_mb: u64) -> u64 {
    limit_mb.saturating_mul(1024).saturating_mul(1024)
}

fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1024.0 / 1024.0;
    if mb < 1024.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.2} GB", mb / 1024.0)
    }
}

pub(crate) fn split_command_words(input: &str) -> Result<Vec<String>, String> {
    let input = normalize_command_dashes(input.trim());
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(character) = chars.next() {
        match quote {
            Some(active_quote) => {
                if character == active_quote {
                    quote = None;
                } else if character == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    } else {
                        current.push(character);
                    }
                } else {
                    current.push(character);
                }
            }
            None => {
                if character.is_whitespace() {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                } else if character == '\'' || character == '"' {
                    quote = Some(character);
                } else if character == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    } else {
                        current.push(character);
                    }
                } else {
                    current.push(character);
                }
            }
        }
    }

    if let Some(active_quote) = quote {
        return Err(format!("Unclosed {active_quote} quote in command"));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

pub(crate) fn normalize_command_dashes(value: &str) -> String {
    value.replace('—', "--").replace('–', "-").replace('−', "-")
}

pub(crate) fn display_command(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| shell_quote(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(
                character,
                '-' | '_' | '.' | '/' | ':' | '@' | '%' | '=' | '+'
            )
    }) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn detect_process_ports(process: &ProcessDefinition) -> Vec<PortBinding> {
    let mut ports = vec![];
    for arg in &process.args {
        if let Some(value) = arg
            .strip_prefix("--port=")
            .and_then(|value| value.parse::<u16>().ok())
        {
            ports.push(PortBinding {
                host: "127.0.0.1".to_string(),
                port: value,
                protocol: "unknown".to_string(),
            });
        }
    }
    if let crate::models::HealthCheck::Tcp { host, port, .. } = &process.health_check {
        ports.push(PortBinding {
            host: host.clone(),
            port: *port,
            protocol: "tcp".to_string(),
        });
    }
    if let crate::models::HealthCheck::Http { url, .. } = &process.health_check {
        if let Some((host, port)) = parse_http_host_port(url) {
            ports.push(PortBinding {
                host,
                port,
                protocol: "http".to_string(),
            });
        }
    }
    ports
}

fn parse_http_host_port(url: &str) -> Option<(String, u16)> {
    let stripped = url.strip_prefix("http://")?;
    let host_port = stripped.split('/').next()?;
    if let Some((host, port)) = host_port.split_once(':') {
        Some((host.to_string(), port.parse().ok()?))
    } else {
        Some((host_port.to_string(), 80))
    }
}

fn detect_port_conflicts(states: Vec<&ProcessRuntimeState>) -> Vec<PortBinding> {
    let mut seen = HashMap::<u16, PortBinding>::new();
    let mut conflicts = vec![];
    for binding in states
        .into_iter()
        .flat_map(|state| state.port_bindings.iter())
    {
        if seen.contains_key(&binding.port) {
            conflicts.push(binding.clone());
        } else {
            seen.insert(binding.port, binding.clone());
        }
    }
    conflicts
}

pub fn derive_project_status(states: &[ProcessRuntimeState]) -> ProjectStatus {
    if states.is_empty() {
        return ProjectStatus::Stopped;
    }
    let failed = states
        .iter()
        .filter(|state| {
            matches!(
                state.current_status,
                ProcessStatus::Failed
                    | ProcessStatus::Crashed
                    | ProcessStatus::Blocked
                    | ProcessStatus::WaitingDependency
            )
        })
        .count();
    let running = states
        .iter()
        .filter(|state| matches!(state.current_status, ProcessStatus::Running))
        .count();
    let starting = states.iter().any(|state| {
        matches!(
            state.current_status,
            ProcessStatus::Starting | ProcessStatus::Queued
        )
    });
    let stopped = states
        .iter()
        .filter(|state| {
            matches!(
                state.current_status,
                ProcessStatus::Stopped | ProcessStatus::Idle
            )
        })
        .count();

    if failed == states.len() {
        ProjectStatus::Failed
    } else if failed > 0 {
        ProjectStatus::Degraded
    } else if starting {
        ProjectStatus::Starting
    } else if running == states.len() {
        ProjectStatus::Running
    } else if stopped == states.len() {
        ProjectStatus::Stopped
    } else {
        ProjectStatus::Partial
    }
}

async fn ordered_processes(state: &AppState, project_id: &str) -> Vec<ProcessDefinition> {
    let processes: Vec<ProcessDefinition> = state
        .config
        .read()
        .await
        .processes
        .iter()
        .filter(|process| process.project_id == project_id)
        .cloned()
        .collect();
    let by_key: HashMap<String, ProcessDefinition> = processes
        .iter()
        .map(|process| (process.key.clone(), process.clone()))
        .collect();
    let by_id: HashMap<String, ProcessDefinition> = processes
        .iter()
        .map(|process| (process.id.clone(), process.clone()))
        .collect();
    let mut visited = HashSet::new();
    let mut output = vec![];
    for process in processes {
        visit_process(&process, &by_key, &by_id, &mut visited, &mut output);
    }
    output
}

fn visit_process(
    process: &ProcessDefinition,
    by_key: &HashMap<String, ProcessDefinition>,
    by_id: &HashMap<String, ProcessDefinition>,
    visited: &mut HashSet<Id>,
    output: &mut Vec<ProcessDefinition>,
) {
    if visited.contains(&process.id) {
        return;
    }
    for key in &process.depends_on {
        if let Some(dependency) = by_key.get(key) {
            visit_process(dependency, by_key, by_id, visited, output);
        }
    }
    if let Some(process) = by_id.get(&process.id) {
        visited.insert(process.id.clone());
        output.push(process.clone());
    }
}

fn retry_eligible(policy: &RestartPolicy, current_count: u32) -> bool {
    match policy.kind {
        RestartPolicyKind::Never => false,
        RestartPolicyKind::Always | RestartPolicyKind::OnFailure => true,
        RestartPolicyKind::LimitedRetries => policy
            .max_retries
            .map(|max| current_count < max)
            .unwrap_or(false),
    }
}

fn compute_restart_delay_ms(base_ms: u64, attempt: u32) -> u64 {
    let base = base_ms.max(500);
    let exponent = attempt.min(RESTART_BACKOFF_MAX_EXPONENT);
    base.saturating_mul(1u64 << exponent)
        .min(RESTART_BACKOFF_CAP_MS)
}

async fn schedule_auto_restart_if_eligible(
    app: &AppHandle,
    state: &AppState,
    process: &ProcessDefinition,
) {
    let policy = process.restart_policy.clone();
    let (prior_count, started_at) = state
        .runtime
        .states
        .read()
        .await
        .get(&process.id)
        .map(|r| (r.restart_count, r.started_at))
        .unwrap_or((0, None));

    // Reset the backoff/retry budget when the process had been running stably
    // before this crash. Otherwise restart_count only ever grows, so one crash
    // after a long, healthy uptime inherits the max 192s backoff (and can
    // exhaust a LimitedRetries budget). See MEDIA_GUARD_TECHDEBT_PLAN P2.
    let ran_stably = started_at
        .map(|s| {
            Utc::now().signed_duration_since(s).num_milliseconds() >= RESTART_STABLE_RESET_MS as i64
        })
        .unwrap_or(false);
    let current_count = if ran_stably && prior_count > 0 {
        if let Some(runtime) = state.runtime.states.write().await.get_mut(&process.id) {
            runtime.restart_count = 0;
        }
        0
    } else {
        prior_count
    };

    if !retry_eligible(&policy, current_count) {
        if matches!(policy.kind, RestartPolicyKind::LimitedRetries) {
            if let Some(max) = policy.max_retries {
                append_log(
                    app,
                    state,
                    process,
                    StreamType::System,
                    LogLevel::Warn,
                    format!("Auto-restart limit reached ({max}); not retrying"),
                )
                .await;
            }
        }
        return;
    }

    let base_delay = policy.retry_delay_ms.unwrap_or(3000);
    let delay_ms = compute_restart_delay_ms(base_delay, current_count);
    append_log(
        app,
        state,
        process,
        StreamType::System,
        LogLevel::Info,
        format!(
            "Auto-restart scheduled in {delay_ms}ms (attempt {})",
            current_count + 1
        ),
    )
    .await;

    let app_handle = app.clone();
    let state_handle = state.clone();
    let process_id = process.id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(delay_ms));
        tauri::async_runtime::block_on(async move {
            let status = state_handle
                .runtime
                .states
                .read()
                .await
                .get(&process_id)
                .map(|r| r.current_status.clone());
            if !matches!(
                status,
                Some(ProcessStatus::Crashed | ProcessStatus::Failed)
            ) {
                return;
            }

            let updated_runtime = {
                let mut states = state_handle.runtime.states.write().await;
                states.get_mut(&process_id).map(|r| {
                    r.restart_count += 1;
                    r.clone()
                })
            };
            if let Some(runtime) = updated_runtime {
                set_runtime(&app_handle, &state_handle, runtime, "process_failed").await;
            }

            if let Err(error) =
                start_process_inner(app_handle, state_handle, process_id.clone()).await
            {
                eprintln!(
                    "auto-restart start_process_inner failed for {process_id}: {} ({})",
                    error.message, error.code
                );
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn launchd(label: &str, domain: Option<&str>) -> LaunchdSupervision {
        LaunchdSupervision {
            label: label.to_string(),
            domain: domain.map(|d| d.to_string()),
        }
    }

    #[test]
    fn launchd_target_defaults_domain() {
        assert_eq!(
            launchd_target(&launchd("uz.blaze.mediaguard-agent", None)),
            "gui/501/uz.blaze.mediaguard-agent"
        );
        assert_eq!(
            launchd_target(&launchd("foo.bar", Some("gui/502"))),
            "gui/502/foo.bar"
        );
    }

    #[test]
    fn launchd_tokens_safe_rejects_injection() {
        assert!(launchd_tokens_safe(&launchd("uz.blaze.mediaguard-telegram-collector", None)));
        assert!(!launchd_tokens_safe(&launchd("foo; rm -rf /", None)));
        assert!(!launchd_tokens_safe(&launchd("foo$(whoami)", None)));
        assert!(!launchd_tokens_safe(&launchd("ok.label", Some("gui/501; echo"))));
    }

    #[test]
    fn parse_launchctl_pid_extracts_pid() {
        let out = "{\n\t\"LimitLoadToSessionType\" = \"Aqua\";\n\t\"LastExitStatus\" = 15;\n\t\"PID\" = 34468;\n\t\"Label\" = \"uz.blaze.mediaguard-agent\";\n};";
        assert_eq!(parse_launchctl_pid(out), Some(34468));
    }

    #[test]
    fn parse_launchctl_pid_absent_when_not_running() {
        let out = "{\n\t\"LastExitStatus\" = 0;\n\t\"Label\" = \"foo\";\n};";
        assert_eq!(parse_launchctl_pid(out), None);
    }

    #[test]
    fn command_match_accepts_exact_command() {
        assert!(command_tokens_match(
            &tokens(&["npm", "run", "dev"]),
            "npm run dev"
        ));
    }

    #[test]
    fn command_match_accepts_configured_prefix_with_executable_path() {
        assert!(command_tokens_match(
            &tokens(&["npm", "run", "dev"]),
            "/opt/homebrew/bin/npm run dev -- --host 127.0.0.1"
        ));
    }

    #[test]
    fn command_match_rejects_different_command() {
        assert!(!command_tokens_match(
            &tokens(&["npm", "run", "dev"]),
            "npm run preview"
        ));
    }

    #[test]
    fn command_match_normalizes_dash_variants() {
        assert!(command_tokens_match(
            &tokens(&["vite", "--host"]),
            "vite —host 127.0.0.1"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn default_process_path_includes_herd_bin_first() {
        let path = platform::default_process_path(
            Some("/custom/bin:/usr/bin".to_string()),
            Some("/Users/example".to_string()),
        );
        let entries: Vec<_> = path.split(':').collect();

        assert_eq!(
            entries.first().copied(),
            Some("/Users/example/Library/Application Support/Herd/bin")
        );
        assert!(entries.contains(&"/opt/homebrew/bin"));
        assert!(entries.contains(&"/usr/local/bin"));
        assert!(entries.contains(&"/custom/bin"));
    }

    #[cfg(unix)]
    #[test]
    fn default_process_path_deduplicates_entries() {
        let path = platform::default_process_path(
            Some("/usr/local/bin:/opt/homebrew/bin:/custom/bin:/custom/bin".to_string()),
            Some("/Users/example".to_string()),
        );
        let entries: Vec<_> = path.split(':').collect();

        assert_eq!(
            entries
                .iter()
                .filter(|entry| **entry == "/opt/homebrew/bin")
                .count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| **entry == "/custom/bin")
                .count(),
            1
        );
    }

    #[test]
    fn effective_process_env_preserves_explicit_path() {
        let mut process_env = HashMap::new();
        process_env.insert("PATH".to_string(), "/custom/php/bin".to_string());
        process_env.insert("APP_ENV".to_string(), "local".to_string());

        let env = effective_process_env(&process_env);

        assert_eq!(env.get("PATH").map(String::as_str), Some("/custom/php/bin"));
        assert_eq!(env.get("APP_ENV").map(String::as_str), Some("local"));
    }

    #[cfg(unix)]
    #[test]
    fn effective_process_env_keeps_unrelated_vars() {
        let mut process_env = HashMap::new();
        process_env.insert("APP_ENV".to_string(), "local".to_string());

        let env = effective_process_env(&process_env);

        assert_eq!(env.get("APP_ENV").map(String::as_str), Some("local"));
        assert!(env
            .get("PATH")
            .is_some_and(|path| path.contains("/Library/Application Support/Herd/bin")));
    }

    #[test]
    fn cwd_match_accepts_exact_directory() {
        assert!(cwd_matches_root(
            "/tmp/karvon/project",
            "/tmp/karvon/project"
        ));
    }

    #[test]
    fn cwd_match_accepts_child_directory() {
        assert!(cwd_matches_root(
            "/tmp/karvon/project/packages/api",
            "/tmp/karvon/project"
        ));
    }

    #[test]
    fn cwd_match_rejects_sibling_directory() {
        assert!(!cwd_matches_root(
            "/tmp/karvon/project-api",
            "/tmp/karvon/project"
        ));
    }

    fn policy(kind: RestartPolicyKind, max_retries: Option<u32>) -> RestartPolicy {
        RestartPolicy {
            kind,
            max_retries,
            retry_delay_ms: None,
        }
    }

    #[test]
    fn retry_eligible_never_policy_returns_false() {
        assert!(!retry_eligible(&policy(RestartPolicyKind::Never, None), 0));
        assert!(!retry_eligible(&policy(RestartPolicyKind::Never, Some(5)), 0));
    }

    #[test]
    fn retry_eligible_on_failure_always_retries() {
        assert!(retry_eligible(&policy(RestartPolicyKind::OnFailure, None), 0));
        assert!(retry_eligible(&policy(RestartPolicyKind::OnFailure, None), 999));
    }

    #[test]
    fn retry_eligible_always_policy_always_retries() {
        assert!(retry_eligible(&policy(RestartPolicyKind::Always, None), 0));
        assert!(retry_eligible(&policy(RestartPolicyKind::Always, None), 999));
    }

    #[test]
    fn retry_eligible_limited_retries_respects_max() {
        assert!(retry_eligible(
            &policy(RestartPolicyKind::LimitedRetries, Some(3)),
            0
        ));
        assert!(retry_eligible(
            &policy(RestartPolicyKind::LimitedRetries, Some(3)),
            2
        ));
        assert!(!retry_eligible(
            &policy(RestartPolicyKind::LimitedRetries, Some(3)),
            3
        ));
        assert!(!retry_eligible(
            &policy(RestartPolicyKind::LimitedRetries, Some(3)),
            10
        ));
    }

    #[test]
    fn retry_eligible_limited_retries_with_no_max_does_not_retry() {
        assert!(!retry_eligible(
            &policy(RestartPolicyKind::LimitedRetries, None),
            0
        ));
        assert!(!retry_eligible(
            &policy(RestartPolicyKind::LimitedRetries, None),
            10_000
        ));
    }

    #[test]
    fn compute_restart_delay_grows_exponentially_then_caps() {
        assert_eq!(compute_restart_delay_ms(3000, 0), 3000);
        assert_eq!(compute_restart_delay_ms(3000, 1), 6000);
        assert_eq!(compute_restart_delay_ms(3000, 2), 12000);
        assert_eq!(compute_restart_delay_ms(3000, 6), RESTART_BACKOFF_CAP_MS);
        assert_eq!(compute_restart_delay_ms(3000, 20), RESTART_BACKOFF_CAP_MS);
    }

    #[test]
    fn compute_restart_delay_enforces_minimum_base() {
        assert_eq!(compute_restart_delay_ms(100, 0), 500);
        assert_eq!(compute_restart_delay_ms(0, 0), 500);
    }
}
