use crate::{
    deploy,
    models::{
        ApiError, ApiResponse, AppConfig, AppSettings, DashboardSummary, DeployHistoryEntry,
        DeployRunState, DeployScript, DeployScriptFormInput, Id, Machine,
        MachineConnectionResult, MachineFormInput, MetricSample, ProcessDefinition,
        ProcessFormInput, Project, ProjectFormInput, ValidationResult, Workspace,
    },
    platform, process_manager,
    state::{app_state, AppState},
    storage,
};
use chrono::Utc;
use std::{collections::HashSet, path::Path};
use tauri::AppHandle;

#[tauri::command]
pub async fn get_config() -> ApiResponse<AppConfig> {
    let state = app_state();
    let config = state.config.read().await.clone();
    ApiResponse::ok(config)
}

#[tauri::command]
pub async fn list_workspaces() -> ApiResponse<Vec<Workspace>> {
    let state = app_state();
    let workspaces = state.config.read().await.workspaces.clone();
    ApiResponse::ok(workspaces)
}

#[tauri::command]
pub async fn create_workspace(app: AppHandle, input: WorkspaceInput) -> ApiResponse<Workspace> {
    let state = app_state();
    let now = Utc::now();
    let workspace = Workspace {
        id: storage::id("workspace"),
        name: input.name,
        description: input.description,
        created_at: now,
        updated_at: now,
        is_default: false,
    };
    let mut config = state.config.write().await;
    config.workspaces.push(workspace.clone());
    save_response(&app, &config, workspace)
}

#[tauri::command]
pub async fn update_workspace(app: AppHandle, workspace: Workspace) -> ApiResponse<Workspace> {
    let state = app_state();
    let mut config = state.config.write().await;
    let mut updated = workspace;
    updated.updated_at = Utc::now();
    if let Some(item) = config
        .workspaces
        .iter_mut()
        .find(|item| item.id == updated.id)
    {
        *item = updated.clone();
        save_response(&app, &config, updated)
    } else {
        ApiResponse::err(ApiError::new(
            "WORKSPACE_NOT_FOUND",
            "Workspace not found",
            false,
        ))
    }
}

#[tauri::command]
pub async fn delete_workspace(app: AppHandle, workspace_id: Id) -> ApiResponse<bool> {
    let state = app_state();
    let mut config = state.config.write().await;
    if config
        .workspaces
        .iter()
        .any(|workspace| workspace.id == workspace_id && workspace.is_default)
    {
        return ApiResponse::err(ApiError::new(
            "DEFAULT_WORKSPACE_LOCKED",
            "Default workspace cannot be deleted",
            false,
        ));
    }
    config
        .workspaces
        .retain(|workspace| workspace.id != workspace_id);
    config
        .projects
        .retain(|project| project.workspace_id != workspace_id);
    save_response(&app, &config, true)
}

#[tauri::command]
pub async fn list_machines() -> ApiResponse<Vec<Machine>> {
    let state = app_state();
    let machines = state.config.read().await.machines.clone();
    ApiResponse::ok(machines)
}

#[tauri::command]
pub async fn create_machine(app: AppHandle, input: MachineFormInput) -> ApiResponse<Machine> {
    if input.name.trim().is_empty()
        || input.hostname.trim().is_empty()
        || input.ssh_user.trim().is_empty()
    {
        return ApiResponse::err(ApiError::new(
            "INVALID_MACHINE_INPUT",
            "Name, hostname, and SSH user are required",
            false,
        ));
    }
    let state = app_state();
    let now = Utc::now();
    let machine = Machine {
        id: storage::id("machine"),
        name: input.name.trim().to_string(),
        hostname: input.hostname.trim().to_string(),
        ssh_user: input.ssh_user.trim().to_string(),
        ssh_port: input.ssh_port,
        ssh_key_path: input.ssh_key_path.filter(|p| !p.trim().is_empty()),
        is_default_local: false,
        created_at: now,
        updated_at: now,
    };
    let mut config = state.config.write().await;
    config.machines.push(machine.clone());
    save_response(&app, &config, machine)
}

#[tauri::command]
pub async fn update_machine(app: AppHandle, machine: Machine) -> ApiResponse<Machine> {
    let state = app_state();
    let mut config = state.config.write().await;
    let mut updated = machine;
    updated.updated_at = Utc::now();
    if let Some(existing) = config
        .machines
        .iter_mut()
        .find(|item| item.id == updated.id)
    {
        if existing.is_default_local {
            updated.is_default_local = true;
            updated.id = existing.id.clone();
        }
        *existing = updated.clone();
        save_response(&app, &config, updated)
    } else {
        ApiResponse::err(ApiError::new(
            "MACHINE_NOT_FOUND",
            "Machine not found",
            false,
        ))
    }
}

#[tauri::command]
pub async fn delete_machine(app: AppHandle, machine_id: Id) -> ApiResponse<bool> {
    let state = app_state();
    let mut config = state.config.write().await;
    let target = config.machines.iter().find(|m| m.id == machine_id).cloned();
    let Some(target) = target else {
        return ApiResponse::err(ApiError::new(
            "MACHINE_NOT_FOUND",
            "Machine not found",
            false,
        ));
    };
    if target.is_default_local {
        return ApiResponse::err(ApiError::new(
            "DEFAULT_MACHINE_LOCKED",
            "The default local machine cannot be deleted",
            false,
        ));
    }
    let referencing: Vec<String> = config
        .processes
        .iter()
        .filter(|process| process.machine_id.as_deref() == Some(machine_id.as_str()))
        .map(|process| process.name.clone())
        .collect();
    if !referencing.is_empty() {
        return ApiResponse::err(ApiError::with_details(
            "MACHINE_IN_USE",
            "Machine is referenced by one or more processes",
            referencing.join(", "),
            false,
        ));
    }
    config.machines.retain(|m| m.id != machine_id);
    save_response(&app, &config, true)
}

#[tauri::command]
pub async fn test_machine_connection(machine_id: Id) -> ApiResponse<MachineConnectionResult> {
    let state = app_state();
    let machine = state
        .config
        .read()
        .await
        .machines
        .iter()
        .find(|m| m.id == machine_id)
        .cloned();
    let Some(machine) = machine else {
        return ApiResponse::err(ApiError::new(
            "MACHINE_NOT_FOUND",
            "Machine not found",
            false,
        ));
    };
    if machine.is_default_local {
        return ApiResponse::ok(MachineConnectionResult {
            ok: true,
            latency_ms: 0,
            detail: "local".to_string(),
        });
    }
    ApiResponse::ok(crate::ssh_executor::test_connection(&machine).await)
}

#[tauri::command]
pub async fn list_projects() -> ApiResponse<Vec<Project>> {
    let state = app_state();
    let projects = state.config.read().await.projects.clone();
    ApiResponse::ok(projects)
}

#[tauri::command]
pub async fn create_project(app: AppHandle, input: ProjectFormInput) -> ApiResponse<Project> {
    let state = app_state();
    if input.name.trim().is_empty() {
        return ApiResponse::err(ApiError::new(
            "VALIDATION_FAILED",
            "Project name is required",
            false,
        ));
    }
    if matches!(input.memory_limit_mb, Some(0)) {
        return ApiResponse::err(ApiError::new(
            "VALIDATION_FAILED",
            "Project memory limit must be greater than 0 MB",
            false,
        ));
    }
    if !Path::new(&input.root_path).exists() {
        return ApiResponse::err(ApiError::with_details(
            "INVALID_PROJECT_PATH",
            "Project root path does not exist",
            input.root_path,
            false,
        ));
    }
    let mut config = state.config.write().await;
    let workspace_id = config
        .workspaces
        .iter()
        .find(|workspace| workspace.is_default)
        .map(|workspace| workspace.id.clone())
        .or_else(|| {
            config
                .workspaces
                .first()
                .map(|workspace| workspace.id.clone())
        })
        .unwrap_or_else(|| "workspace_default".to_string());
    let now = Utc::now();
    let project = Project {
        id: storage::id("project"),
        workspace_id,
        name: input.name.clone(),
        slug: storage::slugify(&input.name),
        description: input.description,
        root_path: input.root_path,
        icon: None,
        color: None,
        tags: input.tags,
        auto_start: input.auto_start,
        startup_order: input.startup_order,
        memory_limit_mb: input.memory_limit_mb,
        auto_restart_on_deploy: true,
        auto_deploy: true,
        machine_id: None,
        created_at: now,
        updated_at: now,
    };
    config.projects.push(project.clone());
    config.last_selected_project_id = Some(project.id.clone());
    config.activity.insert(
        0,
        storage::activity(
            crate::models::ActivityType::ProjectCreated,
            format!("{} created", project.name),
            "info",
            Some(project.id.clone()),
            None,
        ),
    );
    save_response(&app, &config, project)
}

#[tauri::command]
pub async fn update_project(app: AppHandle, project: Project) -> ApiResponse<Project> {
    let state = app_state();
    if matches!(project.memory_limit_mb, Some(0)) {
        return ApiResponse::err(ApiError::new(
            "VALIDATION_FAILED",
            "Project memory limit must be greater than 0 MB",
            false,
        ));
    }
    let mut config = state.config.write().await;
    let mut updated = project;
    updated.updated_at = Utc::now();
    if let Some(item) = config
        .projects
        .iter_mut()
        .find(|item| item.id == updated.id)
    {
        *item = updated.clone();
        config.activity.insert(
            0,
            storage::activity(
                crate::models::ActivityType::ProjectUpdated,
                format!("{} updated", updated.name),
                "info",
                Some(updated.id.clone()),
                None,
            ),
        );
        save_response(&app, &config, updated)
    } else {
        ApiResponse::err(ApiError::new(
            "PROJECT_NOT_FOUND",
            "Project not found",
            false,
        ))
    }
}

#[tauri::command]
pub async fn delete_project(app: AppHandle, project_id: Id) -> ApiResponse<bool> {
    let state = app_state();
    let mut config = state.config.write().await;
    let process_ids: HashSet<_> = config
        .processes
        .iter()
        .filter(|process| process.project_id == project_id)
        .map(|process| process.id.clone())
        .collect();
    config.projects.retain(|project| project.id != project_id);
    config
        .processes
        .retain(|process| process.project_id != project_id);
    config.activity.insert(
        0,
        storage::activity(
            crate::models::ActivityType::ProjectDeleted,
            "Project deleted",
            "warn",
            Some(project_id.clone()),
            None,
        ),
    );
    drop(config);
    {
        let mut states = state.runtime.states.write().await;
        let mut pids = state.runtime.pids.write().await;
        let mut records = state.runtime.process_records.write().await;
        for process_id in process_ids {
            states.remove(&process_id);
            pids.remove(&process_id);
            records.remove(&process_id);
        }
        let _ = storage::save_runtime_processes(&app, &records);
    }
    let config = state.config.read().await;
    save_response(&app, &config, true)
}

#[tauri::command]
pub async fn get_project_detail(
    app: AppHandle,
    project_id: Id,
) -> ApiResponse<crate::models::ProjectDetail> {
    let state = app_state();
    process_manager::sync_external_processes(app, state.clone()).await;
    match process_manager::get_project_detail(&state, &project_id).await {
        Ok(detail) => ApiResponse::ok(detail),
        Err(error) => ApiResponse::err(error),
    }
}

#[tauri::command]
pub async fn list_processes_by_project(project_id: Id) -> ApiResponse<Vec<ProcessDefinition>> {
    let state = app_state();
    let processes = state
        .config
        .read()
        .await
        .processes
        .iter()
        .filter(|process| process.project_id == project_id)
        .cloned()
        .collect();
    ApiResponse::ok(processes)
}

#[tauri::command]
pub async fn create_process_definition(
    app: AppHandle,
    input: ProcessFormInput,
) -> ApiResponse<ProcessDefinition> {
    let state = app_state();
    let validation = validate_process_definition(&state, None, &input).await;
    if !validation.valid {
        return ApiResponse::err(ApiError::with_details(
            "INVALID_PROCESS_DEFINITION",
            "Process definition is invalid",
            validation.errors.join(", "),
            false,
        ));
    }
    let now = Utc::now();
    let process = ProcessDefinition {
        id: storage::id("process"),
        project_id: input.project_id,
        name: input.name,
        key: input.key,
        command: input.command,
        args: input.args,
        working_directory: input.working_directory,
        env: input.env,
        memory_limit_mb: input.memory_limit_mb,
        auto_start: input.auto_start,
        restart_policy: input.restart_policy,
        startup_delay_ms: input.startup_delay_ms,
        depends_on: input.depends_on,
        health_check: input.health_check,
        log_mode: input.log_mode,
        group: input.group,
        visible: input.visible,
        machine_id: input.machine_id,
        launchd: input.launchd,
        created_at: now,
        updated_at: now,
    };
    let mut config = state.config.write().await;
    config.processes.push(process.clone());
    config.last_selected_process_id = Some(process.id.clone());
    config.activity.insert(
        0,
        storage::activity(
            crate::models::ActivityType::ProcessCreated,
            format!("{} created", process.name),
            "info",
            Some(process.project_id.clone()),
            Some(process.id.clone()),
        ),
    );
    state.runtime.states.write().await.insert(
        process.id.clone(),
        crate::models::ProcessRuntimeState::stopped(process.id.clone()),
    );
    save_response(&app, &config, process)
}

#[tauri::command]
pub async fn update_process_definition(
    app: AppHandle,
    process: ProcessDefinition,
) -> ApiResponse<ProcessDefinition> {
    let state = app_state();
    let input = ProcessFormInput {
        project_id: process.project_id.clone(),
        name: process.name.clone(),
        key: process.key.clone(),
        command: process.command.clone(),
        args: process.args.clone(),
        working_directory: process.working_directory.clone(),
        env: process.env.clone(),
        memory_limit_mb: process.memory_limit_mb,
        auto_start: process.auto_start,
        restart_policy: process.restart_policy.clone(),
        startup_delay_ms: process.startup_delay_ms,
        depends_on: process.depends_on.clone(),
        health_check: process.health_check.clone(),
        log_mode: process.log_mode.clone(),
        group: process.group.clone(),
        visible: process.visible,
        machine_id: process.machine_id.clone(),
        launchd: process.launchd.clone(),
    };
    let validation = validate_process_definition(&state, Some(&process.id), &input).await;
    if !validation.valid {
        return ApiResponse::err(ApiError::with_details(
            "INVALID_PROCESS_DEFINITION",
            "Process definition is invalid",
            validation.errors.join(", "),
            false,
        ));
    }
    let mut config = state.config.write().await;
    let mut updated = process;
    updated.updated_at = Utc::now();
    if let Some(item) = config
        .processes
        .iter_mut()
        .find(|item| item.id == updated.id)
    {
        *item = updated.clone();
        config.activity.insert(
            0,
            storage::activity(
                crate::models::ActivityType::ProcessUpdated,
                format!("{} updated", updated.name),
                "info",
                Some(updated.project_id.clone()),
                Some(updated.id.clone()),
            ),
        );
        save_response(&app, &config, updated)
    } else {
        ApiResponse::err(ApiError::new(
            "PROCESS_NOT_FOUND",
            "Process not found",
            false,
        ))
    }
}

#[tauri::command]
pub async fn delete_process_definition(app: AppHandle, process_id: Id) -> ApiResponse<bool> {
    let state = app_state();
    let mut config = state.config.write().await;
    let Some(process) = config
        .processes
        .iter()
        .find(|process| process.id == process_id)
        .cloned()
    else {
        return ApiResponse::err(ApiError::new(
            "PROCESS_NOT_FOUND",
            "Process not found",
            false,
        ));
    };
    config.processes.retain(|process| process.id != process_id);
    config.activity.insert(
        0,
        storage::activity(
            crate::models::ActivityType::ProcessDeleted,
            format!("{} deleted", process.name),
            "warn",
            Some(process.project_id),
            Some(process.id.clone()),
        ),
    );
    state.runtime.states.write().await.remove(&process.id);
    state
        .runtime
        .metrics_history
        .write()
        .await
        .remove(&process.id);
    {
        let mut pids = state.runtime.pids.write().await;
        let mut records = state.runtime.process_records.write().await;
        pids.remove(&process.id);
        records.remove(&process.id);
        let _ = storage::save_runtime_processes(&app, &records);
    }
    save_response(&app, &config, true)
}

#[tauri::command]
pub async fn start_process(
    app: AppHandle,
    process_id: Id,
) -> ApiResponse<crate::models::ProcessRuntimeState> {
    process_manager::start_process(app, app_state(), process_id).await
}

#[tauri::command]
pub async fn stop_process(
    app: AppHandle,
    process_id: Id,
) -> ApiResponse<crate::models::ProcessRuntimeState> {
    process_manager::stop_process(app, app_state(), process_id).await
}

#[tauri::command]
pub async fn restart_process(
    app: AppHandle,
    process_id: Id,
) -> ApiResponse<crate::models::ProcessRuntimeState> {
    process_manager::restart_process(app, app_state(), process_id).await
}

#[tauri::command]
pub async fn start_project(
    app: AppHandle,
    project_id: Id,
) -> ApiResponse<crate::models::ProjectDetail> {
    process_manager::start_project(app, app_state(), project_id).await
}

#[tauri::command]
pub async fn start_auto_start_processes(
    app: AppHandle,
    project_id: Id,
) -> ApiResponse<crate::models::ProjectDetail> {
    process_manager::start_auto_start_processes(app, app_state(), project_id).await
}

#[tauri::command]
pub async fn stop_project(
    app: AppHandle,
    project_id: Id,
) -> ApiResponse<crate::models::ProjectDetail> {
    process_manager::stop_project(app, app_state(), project_id).await
}

#[tauri::command]
pub async fn restart_project(
    app: AppHandle,
    project_id: Id,
) -> ApiResponse<crate::models::ProjectDetail> {
    process_manager::restart_project(app, app_state(), project_id).await
}

#[tauri::command]
pub async fn list_external_project_processes(
    project_id: Id,
) -> ApiResponse<Vec<crate::models::ExternalProcess>> {
    process_manager::list_external_project_processes(app_state(), project_id).await
}

#[tauri::command]
pub async fn stop_external_process(process_group_id: u32) -> ApiResponse<bool> {
    process_manager::stop_external_process(app_state(), process_group_id).await
}

#[tauri::command]
pub async fn find_process_on_port(
    port: u16,
) -> ApiResponse<Option<crate::models::ExternalProcess>> {
    process_manager::find_process_on_port(port).await
}

#[tauri::command]
pub async fn restart_failed_processes(
    app: AppHandle,
    project_id: Option<Id>,
) -> ApiResponse<Vec<crate::models::ProcessRuntimeState>> {
    process_manager::restart_failed_processes(app, app_state(), project_id).await
}

#[tauri::command]
pub async fn get_runtime_state(process_id: Id) -> ApiResponse<crate::models::ProcessRuntimeState> {
    process_manager::get_runtime_state(app_state(), process_id).await
}

#[tauri::command]
pub async fn get_all_runtime_states(
    app: AppHandle,
) -> ApiResponse<Vec<crate::models::ProcessRuntimeState>> {
    let state = app_state();
    process_manager::sync_external_processes(app, state.clone()).await;
    process_manager::get_all_runtime_states(state).await
}

#[tauri::command]
pub async fn get_process_metrics_history(process_id: Id) -> ApiResponse<Vec<MetricSample>> {
    let state = app_state();
    let history = state.runtime.metrics_history.read().await;
    let samples = history
        .get(&process_id)
        .map(|buffer| buffer.iter().cloned().collect())
        .unwrap_or_default();
    ApiResponse::ok(samples)
}

#[tauri::command]
pub async fn get_log_history(
    filters: Option<crate::models::LogHistoryFilters>,
) -> ApiResponse<Vec<crate::models::LogEntry>> {
    process_manager::get_log_history(app_state(), filters).await
}

#[tauri::command]
pub async fn clear_log_history(app: AppHandle, project_id: Option<Id>) -> ApiResponse<bool> {
    process_manager::clear_log_history(app, app_state(), project_id).await
}

#[tauri::command]
pub async fn subscribe_logs() -> ApiResponse<bool> {
    ApiResponse::ok(true)
}

#[tauri::command]
pub async fn unsubscribe_logs() -> ApiResponse<bool> {
    ApiResponse::ok(true)
}

#[tauri::command]
pub async fn export_logs(filters: Option<crate::models::LogHistoryFilters>) -> ApiResponse<String> {
    process_manager::export_logs(app_state(), filters).await
}

#[tauri::command]
pub async fn run_health_check(
    app: AppHandle,
    process_id: Id,
) -> ApiResponse<crate::models::ProcessRuntimeState> {
    process_manager::run_process_health_check(app, app_state(), process_id).await
}

#[tauri::command]
pub async fn get_health_summary(
    project_id: Option<Id>,
) -> ApiResponse<std::collections::HashMap<String, usize>> {
    process_manager::get_health_summary(app_state(), project_id).await
}

#[tauri::command]
pub async fn get_dashboard_summary(app: AppHandle) -> ApiResponse<DashboardSummary> {
    let state = app_state();
    process_manager::sync_external_processes(app, state.clone()).await;
    ApiResponse::ok(process_manager::dashboard_summary(state).await)
}

#[tauri::command]
pub async fn open_project_folder_in_finder(project_id: Id) -> ApiResponse<bool> {
    let state = app_state();
    let config = state.config.read().await;
    let Some(project) = config
        .projects
        .iter()
        .find(|project| project.id == project_id)
    else {
        return ApiResponse::err(ApiError::new(
            "PROJECT_NOT_FOUND",
            "Project not found",
            false,
        ));
    };
    match platform::open_path(Path::new(&project.root_path)) {
        Ok(_) => ApiResponse::ok(true),
        Err(error) => ApiResponse::err(ApiError::with_details(
            "COMMAND_EXECUTION_FAILED",
            "Unable to open project folder",
            error,
            true,
        )),
    }
}

#[tauri::command]
pub async fn open_path_in_finder(path: String) -> ApiResponse<bool> {
    if !std::path::Path::new(&path).is_absolute() {
        return ApiResponse::err(ApiError::new(
            "INVALID_PATH",
            "Path must be absolute",
            false,
        ));
    }
    match platform::open_path(Path::new(&path)) {
        Ok(_) => ApiResponse::ok(true),
        Err(error) => ApiResponse::err(ApiError::with_details(
            "COMMAND_EXECUTION_FAILED",
            "Unable to open path",
            error,
            true,
        )),
    }
}

#[tauri::command]
pub async fn reveal_log_file_in_finder(app: AppHandle) -> ApiResponse<bool> {
    match storage::config_path(&app).and_then(|path| {
        platform::reveal_path(&path).map_err(|error| {
            ApiError::with_details(
                "COMMAND_EXECUTION_FAILED",
                "Unable to reveal config file",
                error,
                true,
            )
        })
    }) {
        Ok(_) => ApiResponse::ok(true),
        Err(error) => ApiResponse::err(error),
    }
}

#[tauri::command]
pub async fn validate_project_path(root_path: String) -> ApiResponse<ValidationResult> {
    let exists = Path::new(&root_path).exists();
    ApiResponse::ok(ValidationResult {
        valid: exists,
        errors: if exists {
            vec![]
        } else {
            vec!["Path does not exist".to_string()]
        },
        warnings: if root_path.starts_with('/') {
            vec![]
        } else {
            vec!["Use an absolute macOS path".to_string()]
        },
    })
}

#[tauri::command]
pub async fn detect_ports_in_use() -> ApiResponse<Vec<crate::models::PortBinding>> {
    process_manager::detect_ports_in_use(app_state()).await
}

#[tauri::command]
pub async fn update_settings(app: AppHandle, settings: AppSettings) -> ApiResponse<AppSettings> {
    let state = app_state();
    let mut config = state.config.write().await;
    config.settings = settings.clone();
    save_response(&app, &config, settings)
}

#[tauri::command]
pub async fn import_config(app: AppHandle, config: AppConfig) -> ApiResponse<AppConfig> {
    let state = app_state();
    let config = storage::migrate_config(config);
    {
        let mut guard = state.config.write().await;
        *guard = config.clone();
        guard.activity.insert(
            0,
            storage::activity(
                crate::models::ActivityType::ConfigImported,
                "Configuration imported",
                "info",
                None,
                None,
            ),
        );
    }
    {
        let config = state.config.read().await;
        reset_runtime_registry_for_config(&app, &state, &config).await;
        save_response(&app, &config, config.clone())
    }
}

#[tauri::command]
pub async fn log_frontend_error(
    app: AppHandle,
    record: crate::models::FrontendErrorRecord,
) -> ApiResponse<bool> {
    let state = app_state();
    process_manager::record_frontend_error(&app, &state, record).await;
    ApiResponse::ok(true)
}

#[tauri::command]
pub async fn get_recent_frontend_errors() -> ApiResponse<Vec<crate::models::FrontendErrorRecord>> {
    let state = app_state();
    let errors = process_manager::recent_frontend_errors(&state).await;
    ApiResponse::ok(errors)
}

#[tauri::command]
pub async fn export_config(redact_secrets: bool) -> ApiResponse<String> {
    let state = app_state();
    let mut config = state.config.read().await.clone();
    if redact_secrets {
        for process in &mut config.processes {
            for (key, value) in process.env.iter_mut() {
                let key_lower = key.to_lowercase();
                if key_lower.contains("token")
                    || key_lower.contains("secret")
                    || key_lower.contains("password")
                    || key_lower.contains("key")
                {
                    *value = "REDACTED".to_string();
                }
            }
        }
    }
    match serde_json::to_string_pretty(&config) {
        Ok(content) => ApiResponse::ok(content),
        Err(error) => ApiResponse::err(ApiError::with_details(
            "CONFIG_SERIALIZATION_FAILED",
            "Unable to export config",
            error,
            false,
        )),
    }
}

#[tauri::command]
pub async fn export_config_to_path(path: String, redact_secrets: bool) -> ApiResponse<String> {
    let state = app_state();
    let mut config = state.config.read().await.clone();
    if redact_secrets {
        for process in &mut config.processes {
            for (key, value) in process.env.iter_mut() {
                let key_lower = key.to_lowercase();
                if key_lower.contains("token")
                    || key_lower.contains("secret")
                    || key_lower.contains("password")
                    || key_lower.contains("key")
                {
                    *value = "REDACTED".to_string();
                }
            }
        }
    }
    let content = match serde_json::to_string_pretty(&config) {
        Ok(content) => content,
        Err(error) => {
            return ApiResponse::err(ApiError::with_details(
                "CONFIG_SERIALIZATION_FAILED",
                "Unable to export config",
                error,
                false,
            ));
        }
    };
    match std::fs::write(&path, format!("{content}\n")) {
        Ok(_) => ApiResponse::ok(path),
        Err(error) => ApiResponse::err(ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to write config file",
            error,
            true,
        )),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInput {
    pub name: String,
    pub description: Option<String>,
}

const MAX_ACTIVITY_EVENTS: usize = 500;

fn save_response<T: serde::Serialize>(
    app: &AppHandle,
    config: &AppConfig,
    data: T,
) -> ApiResponse<T> {
    let config = trim_activity_for_save(config);
    match storage::save_config(app, &config) {
        Ok(_) => ApiResponse::ok(data),
        Err(error) => ApiResponse::err(error),
    }
}

fn trim_activity_for_save(config: &AppConfig) -> AppConfig {
    if config.activity.len() <= MAX_ACTIVITY_EVENTS {
        return config.clone();
    }
    let mut trimmed = config.clone();
    trimmed.activity.truncate(MAX_ACTIVITY_EVENTS);
    trimmed
}

async fn reset_runtime_registry_for_config(app: &AppHandle, state: &AppState, config: &AppConfig) {
    let mut states = state.runtime.states.write().await;
    states.clear();
    for process in &config.processes {
        states.insert(
            process.id.clone(),
            crate::models::ProcessRuntimeState::stopped(process.id.clone()),
        );
    }
    let mut pids = state.runtime.pids.write().await;
    let mut records = state.runtime.process_records.write().await;
    pids.clear();
    records.clear();
    let _ = storage::save_runtime_processes(app, &records);
}

async fn validate_process_definition(
    state: &AppState,
    current_id: Option<&str>,
    input: &ProcessFormInput,
) -> ValidationResult {
    let config = state.config.read().await;
    let processes: Vec<_> = config
        .processes
        .iter()
        .filter(|process| process.project_id == input.project_id)
        .collect();
    let mut errors = vec![];
    if input.name.trim().is_empty() {
        errors.push("Process name is required".to_string());
    }
    if input.key.trim().is_empty() {
        errors.push("Process key is required".to_string());
    }
    if input.command.trim().is_empty() {
        errors.push("Command is required".to_string());
    }
    if matches!(input.memory_limit_mb, Some(0)) {
        errors.push("Memory limit must be greater than 0 MB".to_string());
    }
    if processes
        .iter()
        .any(|process| process.key == input.key && Some(process.id.as_str()) != current_id)
    {
        errors.push("Process key must be unique in the project".to_string());
    }
    let keys: HashSet<_> = processes
        .iter()
        .map(|process| process.key.as_str())
        .collect();
    for dependency in &input.depends_on {
        if !keys.contains(dependency.as_str()) {
            errors.push(format!("Unknown dependency: {dependency}"));
        }
    }
    ValidationResult {
        valid: errors.is_empty(),
        errors,
        warnings: vec![],
    }
}

#[tauri::command]
pub async fn list_deploy_scripts(project_id: Id) -> ApiResponse<Vec<DeployScript>> {
    let state = app_state();
    let scripts: Vec<DeployScript> = state
        .config
        .read()
        .await
        .deploy_scripts
        .iter()
        .filter(|script| script.project_id == project_id)
        .cloned()
        .collect();
    ApiResponse::ok(scripts)
}

#[tauri::command]
pub async fn create_deploy_script(
    app: AppHandle,
    input: DeployScriptFormInput,
) -> ApiResponse<DeployScript> {
    if input.name.trim().is_empty() || input.command.trim().is_empty() {
        return ApiResponse::err(ApiError::new(
            "VALIDATION_FAILED",
            "Deploy script name and command are required",
            false,
        ));
    }
    let state = app_state();
    let now = Utc::now();
    let mut config = state.config.write().await;
    if !config
        .projects
        .iter()
        .any(|project| project.id == input.project_id)
    {
        return ApiResponse::err(ApiError::new(
            "PROJECT_NOT_FOUND",
            "Project not found",
            false,
        ));
    }
    let next_order = input.order.unwrap_or_else(|| {
        config
            .deploy_scripts
            .iter()
            .filter(|script| script.project_id == input.project_id && script.stage == input.stage)
            .map(|script| script.order)
            .max()
            .map(|max| max + 1)
            .unwrap_or(0)
    });
    let script = DeployScript {
        id: storage::id("deploy"),
        project_id: input.project_id,
        name: input.name.trim().to_string(),
        stage: input.stage,
        order: next_order,
        command: input.command.trim().to_string(),
        args: input.args,
        working_directory: input
            .working_directory
            .and_then(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }),
        env: input.env,
        machine_id: input.machine_id.and_then(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        }),
        continue_on_error: input.continue_on_error,
        created_at: now,
        updated_at: now,
    };
    config.deploy_scripts.push(script.clone());
    save_response(&app, &config, script)
}

#[tauri::command]
pub async fn update_deploy_script(
    app: AppHandle,
    script: DeployScript,
) -> ApiResponse<DeployScript> {
    if script.name.trim().is_empty() || script.command.trim().is_empty() {
        return ApiResponse::err(ApiError::new(
            "VALIDATION_FAILED",
            "Deploy script name and command are required",
            false,
        ));
    }
    let state = app_state();
    let mut config = state.config.write().await;
    let mut updated = script;
    updated.updated_at = Utc::now();
    if let Some(item) = config
        .deploy_scripts
        .iter_mut()
        .find(|item| item.id == updated.id)
    {
        *item = updated.clone();
        save_response(&app, &config, updated)
    } else {
        ApiResponse::err(ApiError::new(
            "DEPLOY_SCRIPT_NOT_FOUND",
            "Deploy script not found",
            false,
        ))
    }
}

#[tauri::command]
pub async fn delete_deploy_script(app: AppHandle, script_id: Id) -> ApiResponse<bool> {
    let state = app_state();
    let mut config = state.config.write().await;
    let before = config.deploy_scripts.len();
    config.deploy_scripts.retain(|script| script.id != script_id);
    if before == config.deploy_scripts.len() {
        return ApiResponse::err(ApiError::new(
            "DEPLOY_SCRIPT_NOT_FOUND",
            "Deploy script not found",
            false,
        ));
    }
    save_response(&app, &config, true)
}

#[tauri::command]
pub async fn reorder_deploy_scripts(
    app: AppHandle,
    project_id: Id,
    ordered_ids: Vec<Id>,
) -> ApiResponse<Vec<DeployScript>> {
    let state = app_state();
    let mut config = state.config.write().await;
    let now = Utc::now();
    for (index, id) in ordered_ids.iter().enumerate() {
        if let Some(script) = config
            .deploy_scripts
            .iter_mut()
            .find(|script| script.id == *id && script.project_id == project_id)
        {
            script.order = index as i32;
            script.updated_at = now;
        }
    }
    let scripts: Vec<DeployScript> = config
        .deploy_scripts
        .iter()
        .filter(|script| script.project_id == project_id)
        .cloned()
        .collect();
    save_response(&app, &config, scripts)
}

#[tauri::command]
pub async fn deploy_project(app: AppHandle, project_id: Id) -> ApiResponse<DeployRunState> {
    let state = app_state();
    match deploy::start_deployment(app, state, project_id).await {
        Ok(run) => ApiResponse::ok(run),
        Err(error) => ApiResponse::err(error),
    }
}

#[tauri::command]
pub async fn cancel_deploy(app: AppHandle, project_id: Id) -> ApiResponse<DeployRunState> {
    let state = app_state();
    match deploy::cancel_deployment(app, state, project_id).await {
        Ok(run) => ApiResponse::ok(run),
        Err(error) => ApiResponse::err(error),
    }
}

#[tauri::command]
pub async fn get_deploy_state(project_id: Id) -> ApiResponse<Option<DeployRunState>> {
    let state = app_state();
    let run = deploy::get_state(&state, &project_id).await;
    ApiResponse::ok(run)
}

#[tauri::command]
pub async fn get_all_deploy_states() -> ApiResponse<Vec<DeployRunState>> {
    let state = app_state();
    let runs = deploy::all_states(&state).await;
    ApiResponse::ok(runs)
}

#[tauri::command]
pub async fn list_deploy_history(
    app: AppHandle,
    project_id: Id,
) -> ApiResponse<Vec<DeployHistoryEntry>> {
    let mut entries: Vec<DeployHistoryEntry> = storage::load_deploy_history(&app)
        .into_iter()
        .filter(|entry| entry.project_id == project_id)
        .collect();
    entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    ApiResponse::ok(entries)
}

#[tauri::command]
pub async fn get_deploy_history_entry(
    app: AppHandle,
    run_id: Id,
) -> ApiResponse<Option<DeployHistoryEntry>> {
    let entry = storage::load_deploy_history(&app)
        .into_iter()
        .find(|entry| entry.run_id == run_id);
    ApiResponse::ok(entry)
}
