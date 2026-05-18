use crate::{
    models::{
        ApiError, DeployHistoryEntry, DeployRunState, DeployScript, DeployScriptResult,
        DeployScriptStatus, DeployStage, DeployStatus, Id, LogEntry, LogLevel, Machine, Project,
        StreamType,
    },
    process_manager,
    ssh_executor,
    state::AppState,
    storage,
};
use chrono::Utc;
use std::{
    collections::HashMap,
    io::ErrorKind,
    path::Path,
    process::Stdio,
};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::watch,
};

const DEPLOY_PROCESS_ID_PREFIX: &str = "deploy:";
const DEPLOY_HISTORY_LIMIT: usize = 5;

pub fn deploy_log_process_id(script_id: &str) -> String {
    format!("{DEPLOY_PROCESS_ID_PREFIX}{script_id}")
}

pub async fn get_state(state: &AppState, project_id: &str) -> Option<DeployRunState> {
    state
        .runtime
        .deploy_states
        .read()
        .await
        .get(project_id)
        .cloned()
}

pub async fn all_states(state: &AppState) -> Vec<DeployRunState> {
    state
        .runtime
        .deploy_states
        .read()
        .await
        .values()
        .cloned()
        .collect()
}

pub async fn start_deployment(
    app: AppHandle,
    state: AppState,
    project_id: Id,
) -> Result<DeployRunState, ApiError> {
    {
        let existing = state.runtime.deploy_states.read().await;
        if let Some(run) = existing.get(&project_id) {
            if matches!(run.status, DeployStatus::Running) {
                return Err(ApiError::new(
                    "DEPLOY_ALREADY_RUNNING",
                    "A deployment is already running for this project",
                    false,
                ));
            }
        }
    }

    let (project, scripts) = {
        let config = state.config.read().await;
        let project = config
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
            .ok_or_else(|| ApiError::new("PROJECT_NOT_FOUND", "Project not found", false))?;
        let mut scripts: Vec<DeployScript> = config
            .deploy_scripts
            .iter()
            .filter(|script| script.project_id == project_id)
            .cloned()
            .collect();
        scripts.sort_by_key(|script| (stage_rank(script.stage), script.order));
        (project, scripts)
    };

    if scripts.is_empty() {
        return Err(ApiError::new(
            "DEPLOY_NO_SCRIPTS",
            "No deploy scripts are configured for this project",
            false,
        ));
    }

    let (cancel_tx, cancel_rx) = watch::channel(false);
    state
        .runtime
        .deploy_cancel
        .write()
        .await
        .insert(project_id.clone(), cancel_tx);

    let run_id = storage::id("dep");
    state
        .runtime
        .deploy_log_buffers
        .write()
        .await
        .insert(run_id.clone(), Vec::new());

    let now = Utc::now();
    let mut run = DeployRunState {
        run_id: run_id.clone(),
        project_id: project_id.clone(),
        status: DeployStatus::Running,
        current_script_id: None,
        started_at: Some(now),
        completed_at: None,
        script_results: scripts
            .iter()
            .map(|script| DeployScriptResult {
                script_id: script.id.clone(),
                status: DeployScriptStatus::Pending,
                exit_code: None,
                started_at: None,
                completed_at: None,
                error: None,
            })
            .collect(),
        last_error: None,
    };
    persist_state(&app, &state, run.clone()).await;

    let app_clone = app.clone();
    let state_clone = state.clone();
    let project_clone = project.clone();
    let scripts_clone = scripts.clone();
    let cancel_rx_clone = cancel_rx.clone();
    let project_id_clone = project_id.clone();

    tauri::async_runtime::spawn(async move {
        run = execute_pipeline(
            app_clone.clone(),
            state_clone.clone(),
            project_clone,
            scripts_clone,
            run,
            cancel_rx_clone,
        )
        .await;
        state_clone
            .runtime
            .deploy_cancel
            .write()
            .await
            .remove(&project_id_clone);
        finalize_deploy_history(&app_clone, &state_clone, &run).await;
        persist_state(&app_clone, &state_clone, run).await;
    });

    Ok(state
        .runtime
        .deploy_states
        .read()
        .await
        .get(&project_id)
        .cloned()
        .unwrap_or_else(|| DeployRunState::idle(project_id)))
}

pub async fn cancel_deployment(
    app: AppHandle,
    state: AppState,
    project_id: Id,
) -> Result<DeployRunState, ApiError> {
    if let Some(sender) = state
        .runtime
        .deploy_cancel
        .read()
        .await
        .get(&project_id)
        .cloned()
    {
        let _ = sender.send(true);
    }
    let current = state
        .runtime
        .deploy_states
        .read()
        .await
        .get(&project_id)
        .cloned();
    if let Some(mut run) = current {
        if matches!(run.status, DeployStatus::Running) {
            run.status = DeployStatus::Cancelled;
            run.last_error = Some("Cancelled by user".to_string());
            run.completed_at = Some(Utc::now());
            persist_state(&app, &state, run.clone()).await;
        }
        Ok(run)
    } else {
        Ok(DeployRunState::idle(project_id))
    }
}

async fn execute_pipeline(
    app: AppHandle,
    state: AppState,
    project: Project,
    scripts: Vec<DeployScript>,
    mut run: DeployRunState,
    cancel_rx: watch::Receiver<bool>,
) -> DeployRunState {
    let mut last_stage: Option<DeployStage> = None;
    let mut pipeline_failed = false;
    let mut cancelled = false;
    let mut restart_attempted = false;

    for (index, script) in scripts.iter().enumerate() {
        if cancel_rx.borrow().clone() {
            cancelled = true;
            break;
        }
        if pipeline_failed {
            run.script_results[index].status = DeployScriptStatus::Skipped;
            run.script_results[index].error = Some("Skipped due to previous failure".into());
            persist_state(&app, &state, run.clone()).await;
            continue;
        }

        // Auto-restart between main and post stages.
        if matches!(last_stage, Some(DeployStage::Main))
            && matches!(script.stage, DeployStage::Post)
            && project.auto_restart_on_deploy
            && !restart_attempted
        {
            restart_attempted = true;
            run_auto_restart(&app, &state, &project).await;
        }

        run.current_script_id = Some(script.id.clone());
        run.script_results[index].status = DeployScriptStatus::Running;
        run.script_results[index].started_at = Some(Utc::now());
        persist_state(&app, &state, run.clone()).await;

        emit_log(
            &app,
            &state,
            &project.id,
            &script.id,
            StreamType::System,
            LogLevel::Info,
            format!("[{:?}] Running '{}'", script.stage, script.name),
        )
        .await;

        let result =
            execute_script(&app, &state, &project, script, cancel_rx.clone()).await;
        let exit_code = result.exit_code;
        let error_msg = result.error.clone();
        let ok = matches!(result.status, DeployScriptStatus::Success);

        run.script_results[index] = result;
        run.current_script_id = None;
        persist_state(&app, &state, run.clone()).await;

        if cancel_rx.borrow().clone() {
            cancelled = true;
            break;
        }

        if !ok && !script.continue_on_error {
            pipeline_failed = true;
            run.last_error = Some(error_msg.unwrap_or_else(|| {
                format!(
                    "Script '{}' failed{}",
                    script.name,
                    exit_code
                        .map(|code| format!(" (exit code {code})"))
                        .unwrap_or_default()
                )
            }));
        }

        last_stage = Some(script.stage);
    }

    // If main scripts succeeded but no post-deploy script exists, still run restart.
    if !cancelled
        && !pipeline_failed
        && !restart_attempted
        && project.auto_restart_on_deploy
        && scripts.iter().any(|script| matches!(script.stage, DeployStage::Main))
    {
        run_auto_restart(&app, &state, &project).await;
    }

    run.current_script_id = None;
    run.completed_at = Some(Utc::now());
    run.status = if cancelled {
        DeployStatus::Cancelled
    } else if pipeline_failed {
        DeployStatus::Failed
    } else {
        DeployStatus::Success
    };
    let status_label = match run.status {
        DeployStatus::Success => "succeeded",
        DeployStatus::Failed => "failed",
        DeployStatus::Cancelled => "cancelled",
        _ => "completed",
    };
    if let Some(last) = scripts.last() {
        emit_log(
            &app,
            &state,
            &project.id,
            &last.id,
            StreamType::System,
            LogLevel::Info,
            format!("Deployment {status_label}"),
        )
        .await;
    }
    run
}

async fn execute_script(
    app: &AppHandle,
    state: &AppState,
    project: &Project,
    script: &DeployScript,
    cancel_rx: watch::Receiver<bool>,
) -> DeployScriptResult {
    let mut result = DeployScriptResult {
        script_id: script.id.clone(),
        status: DeployScriptStatus::Running,
        exit_code: None,
        started_at: Some(Utc::now()),
        completed_at: None,
        error: None,
    };

    let remote_machine = resolve_remote_machine(state, script).await;
    let is_remote = remote_machine.is_some();

    let cwd = match resolve_working_directory(project, script, is_remote) {
        Ok(cwd) => cwd,
        Err(error) => {
            result.status = DeployScriptStatus::Failed;
            result.completed_at = Some(Utc::now());
            result.error = Some(error.message.clone());
            emit_log(
                app,
                state,
                &project.id,
                &script.id,
                StreamType::System,
                LogLevel::Error,
                error.message,
            )
            .await;
            return result;
        }
    };

    let tokens = match build_command_tokens(script) {
        Ok(tokens) => tokens,
        Err(error) => {
            result.status = DeployScriptStatus::Failed;
            result.completed_at = Some(Utc::now());
            result.error = Some(error.message.clone());
            emit_log(
                app,
                state,
                &project.id,
                &script.id,
                StreamType::System,
                LogLevel::Error,
                error.message,
            )
            .await;
            return result;
        }
    };

    let mut child = match spawn_command(
        app,
        state,
        project,
        script,
        &tokens,
        &cwd,
        remote_machine.as_ref(),
    )
    .await
    {
        Ok(child) => child,
        Err(error) => {
            result.status = DeployScriptStatus::Failed;
            result.completed_at = Some(Utc::now());
            result.error = Some(error.message.clone());
            emit_log(
                app,
                state,
                &project.id,
                &script.id,
                StreamType::System,
                LogLevel::Error,
                error.message,
            )
            .await;
            return result;
        }
    };

    let pid = child.id();
    if let Some(stdout) = child.stdout.take() {
        spawn_reader(
            app.clone(),
            state.clone(),
            project.id.clone(),
            script.id.clone(),
            StreamType::Stdout,
            stdout,
            is_remote,
        );
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(
            app.clone(),
            state.clone(),
            project.id.clone(),
            script.id.clone(),
            StreamType::Stderr,
            stderr,
            is_remote,
        );
    }

    let mut cancel_rx_for_wait = cancel_rx;
    let exit_status = tokio::select! {
        status = child.wait() => status,
        _ = wait_for_cancel(&mut cancel_rx_for_wait) => {
            if let Some(pid) = pid {
                terminate_local_pid(pid);
            }
            let _ = child.wait().await;
            result.status = DeployScriptStatus::Failed;
            result.completed_at = Some(Utc::now());
            result.error = Some("Cancelled".into());
            emit_log(
                app,
                state,
                &project.id,
                &script.id,
                StreamType::System,
                LogLevel::Warn,
                "Cancelled by user",
            )
            .await;
            return result;
        }
    };

    result.completed_at = Some(Utc::now());
    match exit_status {
        Ok(status) => {
            let code = status.code();
            result.exit_code = code;
            if status.success() {
                result.status = DeployScriptStatus::Success;
                emit_log(
                    app,
                    state,
                    &project.id,
                    &script.id,
                    StreamType::System,
                    LogLevel::Info,
                    format!("'{}' completed (exit 0)", script.name),
                )
                .await;
            } else {
                result.status = DeployScriptStatus::Failed;
                let message = match code {
                    Some(code) => format!("'{}' exited with code {code}", script.name),
                    None => format!("'{}' terminated without exit code", script.name),
                };
                result.error = Some(message.clone());
                emit_log(
                    app,
                    state,
                    &project.id,
                    &script.id,
                    StreamType::System,
                    LogLevel::Error,
                    message,
                )
                .await;
            }
        }
        Err(error) => {
            result.status = DeployScriptStatus::Failed;
            let message = format!("Failed to wait for '{}': {error}", script.name);
            result.error = Some(message.clone());
            emit_log(
                app,
                state,
                &project.id,
                &script.id,
                StreamType::System,
                LogLevel::Error,
                message,
            )
            .await;
        }
    }

    result
}

async fn wait_for_cancel(cancel_rx: &mut watch::Receiver<bool>) {
    while !*cancel_rx.borrow() {
        if cancel_rx.changed().await.is_err() {
            return;
        }
    }
}

fn build_command_tokens(script: &DeployScript) -> Result<Vec<String>, ApiError> {
    let mut tokens = process_manager::split_command_words(&script.command).map_err(|error| {
        ApiError::with_details(
            "INVALID_DEPLOY_SCRIPT",
            "Command could not be parsed",
            error,
            false,
        )
    })?;
    tokens.extend(
        script
            .args
            .iter()
            .map(|arg| process_manager::normalize_command_dashes(arg).trim().to_string())
            .filter(|arg| !arg.is_empty()),
    );
    if tokens.is_empty() {
        return Err(ApiError::new(
            "INVALID_DEPLOY_SCRIPT",
            "Command is required",
            false,
        ));
    }
    Ok(tokens)
}

fn resolve_working_directory(
    project: &Project,
    script: &DeployScript,
    is_remote: bool,
) -> Result<String, ApiError> {
    let cwd = script
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

async fn resolve_remote_machine(state: &AppState, script: &DeployScript) -> Option<Machine> {
    let machine_id = script.machine_id.as_deref()?;
    let machines = state.config.read().await.machines.clone();
    let machine = machines.iter().find(|machine| machine.id == machine_id)?;
    if machine.is_default_local {
        None
    } else {
        Some(machine.clone())
    }
}

async fn spawn_command(
    app: &AppHandle,
    state: &AppState,
    project: &Project,
    script: &DeployScript,
    tokens: &[String],
    cwd: &str,
    remote_machine: Option<&Machine>,
) -> Result<tokio::process::Child, ApiError> {
    let command_label = process_manager::display_command(tokens);
    let env = effective_env(script);

    if let Some(machine) = remote_machine {
        let mut command = ssh_executor::build_ssh_command(machine, tokens, Some(cwd), &env);
        emit_log(
            app,
            state,
            &project.id,
            &script.id,
            StreamType::System,
            LogLevel::Info,
            format!(
                "Connecting to {}@{}:{} via SSH",
                machine.ssh_user, machine.hostname, machine.ssh_port
            ),
        )
        .await;
        return command.spawn().map_err(|error| {
            ApiError::with_details(
                "COMMAND_EXECUTION_FAILED",
                "Unable to execute remote deploy command",
                format!("{command_label} (ssh {}): {error}", machine.hostname),
                true,
            )
        });
    }

    let mut command = process_manager::direct_process_command(tokens);
    configure_local_command(&mut command, cwd, &env);
    match command.spawn() {
        Ok(child) => Ok(child),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let mut shell_command = process_manager::shell_process_command(tokens);
            configure_local_command(&mut shell_command, cwd, &env);
            shell_command.spawn().map_err(|shell_error| {
                ApiError::with_details(
                    "COMMAND_EXECUTION_FAILED",
                    "Unable to execute deploy command",
                    format!("{command_label}: {shell_error}. Direct launch also failed: {error}"),
                    true,
                )
            })
        }
        Err(error) => Err(ApiError::with_details(
            "COMMAND_EXECUTION_FAILED",
            "Unable to execute deploy command",
            format!("{command_label}: {error}"),
            true,
        )),
    }
}

fn configure_local_command(command: &mut Command, cwd: &str, env: &HashMap<String, String>) {
    command.process_group(0);
    command.current_dir(cwd);
    command.envs(env);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
}

fn effective_env(script: &DeployScript) -> HashMap<String, String> {
    let mut env = script.env.clone();
    if !env.contains_key("PATH") {
        let mut paths: Vec<String> = vec![
            "/opt/homebrew/bin".into(),
            "/usr/local/bin".into(),
            "/usr/bin".into(),
            "/bin".into(),
            "/usr/sbin".into(),
            "/sbin".into(),
        ];
        if let Ok(home) = std::env::var("HOME") {
            paths.insert(0, format!("{home}/Library/Application Support/Herd/bin"));
        }
        if let Ok(inherited) = std::env::var("PATH") {
            for part in inherited.split(':') {
                let trimmed = part.trim();
                if !trimmed.is_empty() && !paths.iter().any(|existing| existing == trimmed) {
                    paths.push(trimmed.to_string());
                }
            }
        }
        env.insert("PATH".into(), paths.join(":"));
    }
    env
}

fn terminate_local_pid(pid: u32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM);
}

fn spawn_reader<R>(
    app: AppHandle,
    state: AppState,
    project_id: Id,
    script_id: Id,
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
                        if ssh_executor::parse_remote_pid_marker(&line).is_some() {
                            continue;
                        }
                    }
                    let clean_line = if is_remote {
                        line.trim_end_matches('\r').to_string()
                    } else {
                        line
                    };
                    emit_log(
                        &app,
                        &state,
                        &project_id,
                        &script_id,
                        stream.clone(),
                        level.clone(),
                        clean_line,
                    )
                    .await;
                }
                Ok(None) => break,
                Err(err) => {
                    emit_log(
                        &app,
                        &state,
                        &project_id,
                        &script_id,
                        StreamType::System,
                        LogLevel::Warn,
                        format!("Reader error: {err}"),
                    )
                    .await;
                    break;
                }
            }
        }
    });
}

async fn emit_log(
    app: &AppHandle,
    state: &AppState,
    project_id: &str,
    script_id: &str,
    stream: StreamType,
    level: LogLevel,
    message: impl Into<String>,
) {
    let message = message.into();
    let entry = LogEntry {
        id: storage::id("log"),
        process_id: deploy_log_process_id(script_id),
        project_id: project_id.to_string(),
        timestamp: Utc::now(),
        stream,
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
    let active_run_id = state
        .runtime
        .deploy_states
        .read()
        .await
        .get(project_id)
        .filter(|run| matches!(run.status, DeployStatus::Running) && !run.run_id.is_empty())
        .map(|run| run.run_id.clone());
    if let Some(run_id) = active_run_id {
        let mut buffers = state.runtime.deploy_log_buffers.write().await;
        if let Some(buffer) = buffers.get_mut(&run_id) {
            buffer.push(entry.clone());
        }
    }
    if let Err(err) = app.emit("process_log", entry) {
        eprintln!("[deploy] emit process_log failed: {err}");
    }
}

async fn finalize_deploy_history(app: &AppHandle, state: &AppState, run: &DeployRunState) {
    if run.run_id.is_empty() {
        return;
    }
    let logs = state
        .runtime
        .deploy_log_buffers
        .write()
        .await
        .remove(&run.run_id)
        .unwrap_or_default();
    let Some(started_at) = run.started_at else {
        return;
    };
    let entry = DeployHistoryEntry {
        run_id: run.run_id.clone(),
        project_id: run.project_id.clone(),
        status: run.status,
        started_at,
        completed_at: run.completed_at,
        script_results: run.script_results.clone(),
        last_error: run.last_error.clone(),
        trigger: None,
        logs,
    };
    let _guard = state.runtime.deploy_history_io.lock().await;
    if let Err(err) = storage::append_deploy_history(app, &entry) {
        eprintln!("[deploy] append deploy history failed: {}", err.message);
        return;
    }
    if let Err(err) =
        storage::prune_deploy_history_for_project(app, &run.project_id, DEPLOY_HISTORY_LIMIT)
    {
        eprintln!("[deploy] prune deploy history failed: {}", err.message);
    }
}

async fn persist_state(app: &AppHandle, state: &AppState, run: DeployRunState) {
    state
        .runtime
        .deploy_states
        .write()
        .await
        .insert(run.project_id.clone(), run.clone());
    if let Err(err) = app.emit("deploy_state_changed", run) {
        eprintln!("[deploy] emit deploy_state_changed failed: {err}");
    }
}

async fn run_auto_restart(app: &AppHandle, state: &AppState, project: &Project) {
    let response = process_manager::restart_project(app.clone(), state.clone(), project.id.clone()).await;
    let message = match response.success {
        true => format!("Auto-restarted processes in '{}'", project.name),
        false => format!(
            "Auto-restart skipped or partially failed: {}",
            response
                .error
                .as_ref()
                .map(|err| err.message.clone())
                .unwrap_or_else(|| "unknown error".into())
        ),
    };
    let level = if response.success {
        LogLevel::Info
    } else {
        LogLevel::Warn
    };
    let entry = LogEntry {
        id: storage::id("log"),
        process_id: format!("{DEPLOY_PROCESS_ID_PREFIX}restart"),
        project_id: project.id.clone(),
        timestamp: Utc::now(),
        stream: StreamType::System,
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
    if let Err(err) = app.emit("process_log", entry) {
        eprintln!("[deploy] emit process_log failed: {err}");
    }
}

fn stage_rank(stage: DeployStage) -> u8 {
    match stage {
        DeployStage::Pre => 0,
        DeployStage::Main => 1,
        DeployStage::Post => 2,
    }
}
