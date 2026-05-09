use crate::{
    mediaguard_preset,
    models::{
        ApiError, ApiResponse, AppConfig, AppSettings, DashboardSummary, Id, ProcessDefinition,
        ProcessFormInput, Project, ProjectFormInput, ValidationResult, Workspace,
    },
    process_manager,
    state::{app_state, AppState},
    storage,
};
use chrono::Utc;
use std::{collections::HashSet, path::Path, process::Command};
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
    match Command::new("open").arg(&project.root_path).spawn() {
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
pub async fn reveal_log_file_in_finder(app: AppHandle) -> ApiResponse<bool> {
    match storage::config_path(&app).and_then(|path| {
        Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| {
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
pub async fn apply_media_guard_preset(
    app: AppHandle,
    base_path: Option<String>,
) -> ApiResponse<AppConfig> {
    let state = app_state();
    let mut config = state.config.write().await;
    mediaguard_preset::apply(&mut config, base_path);
    config.activity.insert(
        0,
        storage::activity(
            crate::models::ActivityType::ConfigImported,
            "MediaGuard project preset synced",
            "info",
            None,
            None,
        ),
    );
    sync_runtime_registry_with_config(&app, &state, &config).await;
    save_response(&app, &config, config.clone())
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

async fn sync_runtime_registry_with_config(app: &AppHandle, state: &AppState, config: &AppConfig) {
    let process_ids: HashSet<_> = config
        .processes
        .iter()
        .map(|process| process.id.clone())
        .collect();
    {
        let mut states = state.runtime.states.write().await;
        states.retain(|process_id, _| process_ids.contains(process_id));
        for process in &config.processes {
            states
                .entry(process.id.clone())
                .or_insert_with(|| crate::models::ProcessRuntimeState::stopped(process.id.clone()));
        }
    }
    {
        let mut pids = state.runtime.pids.write().await;
        pids.retain(|process_id, _| process_ids.contains(process_id));
    }
    {
        let mut records = state.runtime.process_records.write().await;
        records.retain(|process_id, _| process_ids.contains(process_id));
        let _ = storage::save_runtime_processes(app, &records);
    }
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
