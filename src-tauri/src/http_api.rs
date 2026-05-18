use crate::deploy;
use crate::mediaguard_preset;
use crate::models::{
    ActivityType, ApiError, ApiResponse, AppConfig, AppSettings, DeployHistoryEntry, DeployScript,
    DeployScriptFormInput, FrontendErrorRecord, Id, LogHistoryFilters, Machine, MachineFormInput,
    ProcessDefinition, ProcessFormInput, ProcessRuntimeState, Project, ProjectDetail,
    ProjectFormInput, ValidationResult, Workspace,
};
use crate::process_manager;
use crate::state::AppState;
use crate::storage;
use axum::{
    extract::{Path, Query, Request, State as AxumState},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::Path as StdPath;
use std::sync::Arc;
use tauri::AppHandle;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct HttpApiState {
    app: AppHandle,
    state: AppState,
    token: Arc<String>,
}

pub fn start_http_server(app: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        if let Err(err) = run_server(app, state).await {
            eprintln!("[http-api] server stopped: {err}");
        }
    });
}

async fn run_server(app: AppHandle, state: AppState) -> Result<(), String> {
    let (enabled, port, host) = {
        let config = state.config.read().await;
        (
            config.settings.http_api_enabled,
            config.settings.http_api_port,
            config.settings.http_api_bind_host.clone(),
        )
    };

    if !enabled {
        println!("[http-api] disabled in settings");
        return Ok(());
    }

    let token = ensure_token(&app, &state).await;

    let api_state = HttpApiState {
        app: app.clone(),
        state: state.clone(),
        token: Arc::new(token.clone()),
    };

    let router = build_router(api_state);

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;

    println!(
        "[http-api] listening on {}:{}, token: {}",
        host, port, token
    );

    axum::serve(listener, router)
        .await
        .map_err(|e| e.to_string())
}

async fn ensure_token(app: &AppHandle, state: &AppState) -> String {
    {
        let config = state.config.read().await;
        if let Some(token) = &config.settings.http_api_token {
            if !token.is_empty() {
                return token.clone();
            }
        }
    }
    let new_token = generate_token();
    let mut config = state.config.write().await;
    config.settings.http_api_token = Some(new_token.clone());
    if let Err(err) = storage::save_config(app, &config) {
        eprintln!("[http-api] failed to persist token: {}", err.message);
    }
    new_token
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn build_router(state: HttpApiState) -> Router {
    let public = Router::new()
        .route("/api/v1/health", get(health_handler))
        .with_state(state.clone());

    let protected = Router::new()
        // ---- config / settings ----
        .route("/api/v1/config", get(get_config_handler))
        .route("/api/v1/config/import", post(import_config_handler))
        .route("/api/v1/config/export", get(export_config_handler))
        .route("/api/v1/settings", put(update_settings_handler))
        .route(
            "/api/v1/presets/media-guard",
            post(apply_media_guard_preset_handler),
        )
        // ---- workspaces ----
        .route("/api/v1/workspaces", get(list_workspaces_handler))
        .route("/api/v1/workspaces", post(create_workspace_handler))
        .route("/api/v1/workspaces/:id", patch(update_workspace_handler))
        .route("/api/v1/workspaces/:id", delete(delete_workspace_handler))
        // ---- machines ----
        .route("/api/v1/machines", get(list_machines_handler))
        .route("/api/v1/machines", post(create_machine_handler))
        .route("/api/v1/machines/:id", patch(update_machine_handler))
        .route("/api/v1/machines/:id", delete(delete_machine_handler))
        .route(
            "/api/v1/machines/:id/test",
            post(test_machine_connection_handler),
        )
        // ---- projects ----
        .route("/api/v1/projects", get(list_projects))
        .route("/api/v1/projects", post(create_project_handler))
        .route("/api/v1/projects/:id", get(get_project))
        .route("/api/v1/projects/:id", patch(update_project_handler))
        .route("/api/v1/projects/:id", delete(delete_project_handler))
        .route("/api/v1/projects/:id/start", post(start_project_handler))
        .route(
            "/api/v1/projects/:id/start-auto",
            post(start_auto_start_handler),
        )
        .route("/api/v1/projects/:id/stop", post(stop_project_handler))
        .route(
            "/api/v1/projects/:id/restart",
            post(restart_project_handler),
        )
        .route(
            "/api/v1/projects/:id/processes",
            get(list_project_processes_handler),
        )
        .route(
            "/api/v1/projects/:id/external-processes",
            get(list_external_project_processes_handler),
        )
        .route(
            "/api/v1/projects/:id/validate-path",
            post(validate_project_path_handler),
        )
        // ---- process definitions ----
        .route(
            "/api/v1/process-definitions",
            post(create_process_definition_handler),
        )
        .route(
            "/api/v1/process-definitions/:id",
            patch(update_process_definition_handler),
        )
        .route(
            "/api/v1/process-definitions/:id",
            delete(delete_process_definition_handler),
        )
        // ---- process runtime ----
        .route("/api/v1/processes", get(list_processes))
        .route("/api/v1/processes/:id", get(get_process))
        .route("/api/v1/processes/:id/metrics", get(get_process_metrics))
        .route("/api/v1/processes/:id/start", post(start_process_handler))
        .route("/api/v1/processes/:id/stop", post(stop_process_handler))
        .route(
            "/api/v1/processes/:id/restart",
            post(restart_process_handler),
        )
        .route(
            "/api/v1/processes/:id/health-check",
            post(run_health_handler),
        )
        .route(
            "/api/v1/processes/restart-failed",
            post(restart_failed_handler),
        )
        // ---- external processes ----
        .route(
            "/api/v1/external-processes/:gid/stop",
            post(stop_external_process_handler),
        )
        .route("/api/v1/ports", get(detect_ports_handler))
        .route("/api/v1/ports/:port", get(find_process_on_port_handler))
        // ---- logs ----
        .route("/api/v1/logs", get(get_logs))
        .route("/api/v1/logs", delete(clear_logs_handler))
        .route("/api/v1/logs/export", get(export_logs_handler))
        // ---- deploy scripts ----
        .route(
            "/api/v1/projects/:id/deploy-scripts",
            get(list_deploy_scripts_handler),
        )
        .route("/api/v1/deploy-scripts", post(create_deploy_script_handler))
        .route(
            "/api/v1/deploy-scripts/:id",
            patch(update_deploy_script_handler),
        )
        .route(
            "/api/v1/deploy-scripts/:id",
            delete(delete_deploy_script_handler),
        )
        .route(
            "/api/v1/projects/:id/deploy-scripts/reorder",
            post(reorder_deploy_scripts_handler),
        )
        // ---- deploys ----
        .route("/api/v1/projects/:id/deploys", get(list_deploys_handler))
        .route("/api/v1/projects/:id/deploy", post(deploy_project_handler))
        .route(
            "/api/v1/projects/:id/cancel-deploy",
            post(cancel_deploy_handler),
        )
        .route(
            "/api/v1/projects/:id/deploy-state",
            get(get_deploy_state_handler),
        )
        .route("/api/v1/deploys", get(list_all_deploys_handler))
        .route("/api/v1/deploys/:run_id", get(get_deploy_handler))
        // ---- observability ----
        .route("/api/v1/health-summary", get(health_summary_handler))
        .route("/api/v1/dashboard", get(dashboard_handler))
        .route("/api/v1/activity", get(activity_handler))
        // ---- frontend errors ----
        .route("/api/v1/frontend-errors", get(list_frontend_errors_handler))
        .route(
            "/api/v1/frontend-errors",
            post(record_frontend_error_handler),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    public.merge(protected).layer(CorsLayer::permissive())
}

async fn auth_middleware(
    AxumState(state): AxumState<HttpApiState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let Some(auth_header) = headers.get("authorization") else {
        return unauthorized("missing authorization header");
    };
    let Ok(auth_str) = auth_header.to_str() else {
        return unauthorized("invalid authorization header encoding");
    };
    let Some(token) = auth_str.strip_prefix("Bearer ") else {
        return unauthorized("expected Bearer scheme");
    };
    if token != state.token.as_str() {
        return unauthorized("invalid token");
    }
    next.run(request).await
}

fn unauthorized(message: &str) -> Response {
    let body = json!({
        "success": false,
        "error": {
            "code": "UNAUTHORIZED",
            "message": message,
            "retryable": false,
        }
    });
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

fn into_response<T: Serialize>(resp: ApiResponse<T>) -> Response {
    if resp.success {
        (StatusCode::OK, Json(resp)).into_response()
    } else {
        let code = resp
            .error
            .as_ref()
            .map(|e| e.code.as_str())
            .unwrap_or("UNKNOWN");
        let status = if code.contains("NOT_FOUND") {
            StatusCode::NOT_FOUND
        } else if code.contains("INVALID")
            || code.contains("VALIDATION")
            || code.contains("LOCKED")
            || code.contains("IN_USE")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(resp)).into_response()
    }
}

// ===== helpers =====

async fn persist_config(app: &AppHandle, config: &AppConfig) -> Result<(), ApiError> {
    storage::save_config(app, config)
}

fn trim_activity(config: &mut AppConfig) {
    const MAX: usize = 500;
    if config.activity.len() > MAX {
        config.activity.truncate(MAX);
    }
}

// ===== public health =====

async fn health_handler() -> Json<Value> {
    Json(json!({
        "ok": true,
        "name": "app-orchestrator",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ===== config / settings =====

async fn get_config_handler(AxumState(s): AxumState<HttpApiState>) -> Response {
    let config = s.state.config.read().await.clone();
    into_response(ApiResponse::ok(config))
}

async fn import_config_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(payload): Json<AppConfig>,
) -> Response {
    let migrated = storage::migrate_config(payload);
    {
        let mut guard = s.state.config.write().await;
        *guard = migrated.clone();
        guard.activity.insert(
            0,
            storage::activity(
                ActivityType::ConfigImported,
                "Configuration imported via HTTP API",
                "info",
                None,
                None,
            ),
        );
        trim_activity(&mut guard);
    }

    let config_snapshot = s.state.config.read().await.clone();
    let mut states = s.state.runtime.states.write().await;
    states.clear();
    for process in &config_snapshot.processes {
        states.insert(
            process.id.clone(),
            ProcessRuntimeState::stopped(process.id.clone()),
        );
    }
    drop(states);

    if let Err(err) = persist_config(&s.app, &config_snapshot).await {
        return into_response(ApiResponse::<AppConfig>::err(err));
    }
    into_response(ApiResponse::ok(config_snapshot))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportConfigQuery {
    #[serde(default)]
    redact_secrets: bool,
}

async fn export_config_handler(
    AxumState(s): AxumState<HttpApiState>,
    Query(query): Query<ExportConfigQuery>,
) -> Response {
    let mut config = s.state.config.read().await.clone();
    if query.redact_secrets {
        for process in &mut config.processes {
            for (key, value) in process.env.iter_mut() {
                let k = key.to_lowercase();
                if k.contains("token")
                    || k.contains("secret")
                    || k.contains("password")
                    || k.contains("key")
                {
                    *value = "REDACTED".to_string();
                }
            }
        }
    }
    match serde_json::to_string_pretty(&config) {
        Ok(content) => into_response(ApiResponse::ok(content)),
        Err(err) => into_response(ApiResponse::<String>::err(ApiError::with_details(
            "CONFIG_SERIALIZATION_FAILED",
            "Unable to export config",
            err,
            false,
        ))),
    }
}

async fn update_settings_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(settings): Json<AppSettings>,
) -> Response {
    let mut config = s.state.config.write().await;
    config.settings = settings.clone();
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<AppSettings>::err(err));
    }
    into_response(ApiResponse::ok(settings))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaGuardPresetBody {
    base_path: Option<String>,
}

async fn apply_media_guard_preset_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(body): Json<MediaGuardPresetBody>,
) -> Response {
    let mut config = s.state.config.write().await;
    mediaguard_preset::apply(&mut config, body.base_path);
    config.activity.insert(
        0,
        storage::activity(
            ActivityType::ConfigImported,
            "MediaGuard preset applied via HTTP API",
            "info",
            None,
            None,
        ),
    );
    trim_activity(&mut config);
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<AppConfig>::err(err));
    }
    into_response(ApiResponse::ok(config.clone()))
}

// ===== workspaces =====

async fn list_workspaces_handler(AxumState(s): AxumState<HttpApiState>) -> Response {
    let workspaces = s.state.config.read().await.workspaces.clone();
    into_response(ApiResponse::ok(workspaces))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceInput {
    name: String,
    description: Option<String>,
}

async fn create_workspace_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(input): Json<WorkspaceInput>,
) -> Response {
    let now = Utc::now();
    let workspace = Workspace {
        id: storage::id("workspace"),
        name: input.name,
        description: input.description,
        created_at: now,
        updated_at: now,
        is_default: false,
    };
    let mut config = s.state.config.write().await;
    config.workspaces.push(workspace.clone());
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<Workspace>::err(err));
    }
    into_response(ApiResponse::ok(workspace))
}

async fn update_workspace_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
    Json(mut input): Json<Workspace>,
) -> Response {
    let mut config = s.state.config.write().await;
    let Some(existing) = config.workspaces.iter_mut().find(|w| w.id == id) else {
        return into_response(ApiResponse::<Workspace>::err(ApiError::new(
            "WORKSPACE_NOT_FOUND",
            "Workspace not found",
            false,
        )));
    };
    input.id = id;
    input.updated_at = Utc::now();
    input.is_default = existing.is_default;
    *existing = input.clone();
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<Workspace>::err(err));
    }
    into_response(ApiResponse::ok(input))
}

async fn delete_workspace_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let mut config = s.state.config.write().await;
    if config
        .workspaces
        .iter()
        .any(|w| w.id == id && w.is_default)
    {
        return into_response(ApiResponse::<bool>::err(ApiError::new(
            "DEFAULT_WORKSPACE_LOCKED",
            "Default workspace cannot be deleted",
            false,
        )));
    }
    let before = config.workspaces.len();
    config.workspaces.retain(|w| w.id != id);
    if config.workspaces.len() == before {
        return into_response(ApiResponse::<bool>::err(ApiError::new(
            "WORKSPACE_NOT_FOUND",
            "Workspace not found",
            false,
        )));
    }
    config.projects.retain(|p| p.workspace_id != id);
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<bool>::err(err));
    }
    into_response(ApiResponse::ok(true))
}

// ===== machines =====

async fn list_machines_handler(AxumState(s): AxumState<HttpApiState>) -> Response {
    let machines = s.state.config.read().await.machines.clone();
    into_response(ApiResponse::ok(machines))
}

async fn create_machine_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(input): Json<MachineFormInput>,
) -> Response {
    if input.name.trim().is_empty()
        || input.hostname.trim().is_empty()
        || input.ssh_user.trim().is_empty()
    {
        return into_response(ApiResponse::<Machine>::err(ApiError::new(
            "INVALID_MACHINE_INPUT",
            "Name, hostname, and SSH user are required",
            false,
        )));
    }
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
    let mut config = s.state.config.write().await;
    config.machines.push(machine.clone());
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<Machine>::err(err));
    }
    into_response(ApiResponse::ok(machine))
}

async fn update_machine_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
    Json(mut input): Json<Machine>,
) -> Response {
    let mut config = s.state.config.write().await;
    let Some(existing) = config.machines.iter_mut().find(|m| m.id == id) else {
        return into_response(ApiResponse::<Machine>::err(ApiError::new(
            "MACHINE_NOT_FOUND",
            "Machine not found",
            false,
        )));
    };
    input.id = id;
    input.updated_at = Utc::now();
    if existing.is_default_local {
        input.is_default_local = true;
    }
    *existing = input.clone();
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<Machine>::err(err));
    }
    into_response(ApiResponse::ok(input))
}

async fn delete_machine_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let mut config = s.state.config.write().await;
    let target = config.machines.iter().find(|m| m.id == id).cloned();
    let Some(target) = target else {
        return into_response(ApiResponse::<bool>::err(ApiError::new(
            "MACHINE_NOT_FOUND",
            "Machine not found",
            false,
        )));
    };
    if target.is_default_local {
        return into_response(ApiResponse::<bool>::err(ApiError::new(
            "DEFAULT_MACHINE_LOCKED",
            "The default local machine cannot be deleted",
            false,
        )));
    }
    let referencing: Vec<String> = config
        .processes
        .iter()
        .filter(|p| p.machine_id.as_deref() == Some(id.as_str()))
        .map(|p| p.name.clone())
        .collect();
    if !referencing.is_empty() {
        return into_response(ApiResponse::<bool>::err(ApiError::with_details(
            "MACHINE_IN_USE",
            "Machine is referenced by one or more processes",
            referencing.join(", "),
            false,
        )));
    }
    config.machines.retain(|m| m.id != id);
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<bool>::err(err));
    }
    into_response(ApiResponse::ok(true))
}

async fn test_machine_connection_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let machine = s
        .state
        .config
        .read()
        .await
        .machines
        .iter()
        .find(|m| m.id == id)
        .cloned();
    let Some(machine) = machine else {
        return into_response(ApiResponse::<crate::models::MachineConnectionResult>::err(
            ApiError::new("MACHINE_NOT_FOUND", "Machine not found", false),
        ));
    };
    if machine.is_default_local {
        return into_response(ApiResponse::ok(crate::models::MachineConnectionResult {
            ok: true,
            latency_ms: 0,
            detail: "local".to_string(),
        }));
    }
    let result = crate::ssh_executor::test_connection(&machine).await;
    into_response(ApiResponse::ok(result))
}

// ===== projects =====

async fn list_projects(AxumState(s): AxumState<HttpApiState>) -> Response {
    let projects = s.state.config.read().await.projects.clone();
    into_response(ApiResponse::ok(projects))
}

async fn get_project(AxumState(s): AxumState<HttpApiState>, Path(id): Path<Id>) -> Response {
    process_manager::sync_external_processes(s.app.clone(), s.state.clone()).await;
    match process_manager::get_project_detail(&s.state, &id).await {
        Ok(detail) => into_response(ApiResponse::ok(detail)),
        Err(error) => into_response(ApiResponse::<ProjectDetail>::err(error)),
    }
}

async fn create_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(input): Json<ProjectFormInput>,
) -> Response {
    if input.name.trim().is_empty() {
        return into_response(ApiResponse::<Project>::err(ApiError::new(
            "VALIDATION_FAILED",
            "Project name is required",
            false,
        )));
    }
    if matches!(input.memory_limit_mb, Some(0)) {
        return into_response(ApiResponse::<Project>::err(ApiError::new(
            "VALIDATION_FAILED",
            "Project memory limit must be greater than 0 MB",
            false,
        )));
    }
    if !StdPath::new(&input.root_path).exists() {
        return into_response(ApiResponse::<Project>::err(ApiError::with_details(
            "INVALID_PROJECT_PATH",
            "Project root path does not exist",
            input.root_path.clone(),
            false,
        )));
    }
    let mut config = s.state.config.write().await;
    let workspace_id = config
        .workspaces
        .iter()
        .find(|w| w.is_default)
        .map(|w| w.id.clone())
        .or_else(|| config.workspaces.first().map(|w| w.id.clone()))
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
            ActivityType::ProjectCreated,
            format!("{} created via HTTP API", project.name),
            "info",
            Some(project.id.clone()),
            None,
        ),
    );
    trim_activity(&mut config);
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<Project>::err(err));
    }
    into_response(ApiResponse::ok(project))
}

async fn update_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
    Json(mut input): Json<Project>,
) -> Response {
    if matches!(input.memory_limit_mb, Some(0)) {
        return into_response(ApiResponse::<Project>::err(ApiError::new(
            "VALIDATION_FAILED",
            "Project memory limit must be greater than 0 MB",
            false,
        )));
    }
    let mut config = s.state.config.write().await;
    let Some(existing) = config.projects.iter_mut().find(|p| p.id == id) else {
        return into_response(ApiResponse::<Project>::err(ApiError::new(
            "PROJECT_NOT_FOUND",
            "Project not found",
            false,
        )));
    };
    input.id = id;
    input.updated_at = Utc::now();
    *existing = input.clone();
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<Project>::err(err));
    }
    into_response(ApiResponse::ok(input))
}

async fn delete_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let mut config = s.state.config.write().await;
    let before = config.projects.len();
    config.projects.retain(|p| p.id != id);
    if before == config.projects.len() {
        return into_response(ApiResponse::<bool>::err(ApiError::new(
            "PROJECT_NOT_FOUND",
            "Project not found",
            false,
        )));
    }
    let removed_process_ids: HashSet<Id> = config
        .processes
        .iter()
        .filter(|p| p.project_id == id)
        .map(|p| p.id.clone())
        .collect();
    config.processes.retain(|p| p.project_id != id);
    config.deploy_scripts.retain(|d| d.project_id != id);
    {
        let mut states = s.state.runtime.states.write().await;
        states.retain(|process_id, _| !removed_process_ids.contains(process_id));
    }
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<bool>::err(err));
    }
    into_response(ApiResponse::ok(true))
}

async fn start_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::start_project(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

async fn start_auto_start_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::start_auto_start_processes(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

async fn stop_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::stop_project(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

async fn restart_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::restart_project(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

async fn list_project_processes_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let processes: Vec<ProcessDefinition> = s
        .state
        .config
        .read()
        .await
        .processes
        .iter()
        .filter(|p| p.project_id == id)
        .cloned()
        .collect();
    into_response(ApiResponse::ok(processes))
}

async fn list_external_project_processes_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::list_external_project_processes(s.state.clone(), id).await;
    into_response(resp)
}

#[derive(Debug, Deserialize)]
struct ValidatePathBody {
    root_path: String,
}

async fn validate_project_path_handler(
    AxumState(_s): AxumState<HttpApiState>,
    Path(_id): Path<Id>,
    Json(body): Json<ValidatePathBody>,
) -> Response {
    let mut errors = vec![];
    let mut warnings = vec![];
    let path = StdPath::new(&body.root_path);
    if !path.exists() {
        errors.push(format!("Path does not exist: {}", body.root_path));
    } else if !path.is_dir() {
        errors.push(format!("Path is not a directory: {}", body.root_path));
    }
    if body.root_path.contains(" ") {
        warnings.push("Path contains spaces; may cause issues".to_string());
    }
    let result = ValidationResult {
        valid: errors.is_empty(),
        errors,
        warnings,
    };
    into_response(ApiResponse::ok(result))
}

// ===== process definitions =====

async fn create_process_definition_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(input): Json<ProcessFormInput>,
) -> Response {
    let now = Utc::now();
    if input.name.trim().is_empty()
        || input.key.trim().is_empty()
        || input.command.trim().is_empty()
    {
        return into_response(ApiResponse::<ProcessDefinition>::err(ApiError::new(
            "INVALID_PROCESS_DEFINITION",
            "name, key and command are required",
            false,
        )));
    }
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
        created_at: now,
        updated_at: now,
    };
    let mut config = s.state.config.write().await;
    config.processes.push(process.clone());
    config.last_selected_process_id = Some(process.id.clone());
    s.state.runtime.states.write().await.insert(
        process.id.clone(),
        ProcessRuntimeState::stopped(process.id.clone()),
    );
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<ProcessDefinition>::err(err));
    }
    into_response(ApiResponse::ok(process))
}

async fn update_process_definition_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
    Json(mut input): Json<ProcessDefinition>,
) -> Response {
    let mut config = s.state.config.write().await;
    let Some(existing) = config.processes.iter_mut().find(|p| p.id == id) else {
        return into_response(ApiResponse::<ProcessDefinition>::err(ApiError::new(
            "PROCESS_NOT_FOUND",
            "Process not found",
            false,
        )));
    };
    input.id = id;
    input.updated_at = Utc::now();
    *existing = input.clone();
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<ProcessDefinition>::err(err));
    }
    into_response(ApiResponse::ok(input))
}

async fn delete_process_definition_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let mut config = s.state.config.write().await;
    let before = config.processes.len();
    config.processes.retain(|p| p.id != id);
    if config.processes.len() == before {
        return into_response(ApiResponse::<bool>::err(ApiError::new(
            "PROCESS_NOT_FOUND",
            "Process not found",
            false,
        )));
    }
    {
        let mut states = s.state.runtime.states.write().await;
        states.remove(&id);
    }
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<bool>::err(err));
    }
    into_response(ApiResponse::ok(true))
}

// ===== process runtime =====

async fn list_processes(AxumState(s): AxumState<HttpApiState>) -> Response {
    process_manager::sync_external_processes(s.app.clone(), s.state.clone()).await;
    let resp = process_manager::get_all_runtime_states(s.state.clone()).await;
    into_response(resp)
}

async fn get_process(AxumState(s): AxumState<HttpApiState>, Path(id): Path<Id>) -> Response {
    let resp = process_manager::get_runtime_state(s.state.clone(), id).await;
    into_response(resp)
}

async fn get_process_metrics(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let history = s.state.runtime.metrics_history.read().await;
    let samples: Vec<_> = history
        .get(&id)
        .map(|buffer| buffer.iter().cloned().collect())
        .unwrap_or_default();
    drop(history);
    into_response(ApiResponse::ok(samples))
}

async fn start_process_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::start_process(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

async fn stop_process_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::stop_process(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

async fn restart_process_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::restart_process(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

async fn run_health_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp =
        process_manager::run_process_health_check(s.app.clone(), s.state.clone(), id).await;
    into_response(resp)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestartFailedQuery {
    project_id: Option<Id>,
}

async fn restart_failed_handler(
    AxumState(s): AxumState<HttpApiState>,
    Query(query): Query<RestartFailedQuery>,
) -> Response {
    let resp =
        process_manager::restart_failed_processes(s.app.clone(), s.state.clone(), query.project_id)
            .await;
    into_response(resp)
}

// ===== external processes =====

async fn stop_external_process_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(gid): Path<u32>,
) -> Response {
    let resp = process_manager::stop_external_process(s.state.clone(), gid).await;
    into_response(resp)
}

async fn detect_ports_handler(AxumState(s): AxumState<HttpApiState>) -> Response {
    let resp = process_manager::detect_ports_in_use(s.state.clone()).await;
    into_response(resp)
}

async fn find_process_on_port_handler(
    AxumState(_s): AxumState<HttpApiState>,
    Path(port): Path<u16>,
) -> Response {
    let resp = process_manager::find_process_on_port(port).await;
    into_response(resp)
}

// ===== logs =====

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogsQuery {
    project_id: Option<Id>,
    process_id: Option<Id>,
    limit: Option<usize>,
    since: Option<DateTime<Utc>>,
}

async fn get_logs(
    AxumState(s): AxumState<HttpApiState>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let filters = LogHistoryFilters {
        project_id: query.project_id,
        process_id: query.process_id,
        limit: query.limit,
        since: query.since,
    };
    let resp = process_manager::get_log_history(s.state.clone(), Some(filters)).await;
    into_response(resp)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClearLogsQuery {
    project_id: Option<Id>,
}

async fn clear_logs_handler(
    AxumState(s): AxumState<HttpApiState>,
    Query(query): Query<ClearLogsQuery>,
) -> Response {
    let mut logs = s.state.runtime.logs.write().await;
    if let Some(ref pid) = query.project_id {
        logs.retain(|log| log.project_id != *pid);
    } else {
        logs.clear();
    }
    into_response(ApiResponse::ok(true))
}

async fn export_logs_handler(
    AxumState(s): AxumState<HttpApiState>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let filters = LogHistoryFilters {
        project_id: query.project_id,
        process_id: query.process_id,
        limit: query.limit,
        since: query.since,
    };
    let resp = process_manager::get_log_history(s.state.clone(), Some(filters)).await;
    match resp.data {
        Some(entries) => match serde_json::to_string_pretty(&entries) {
            Ok(content) => into_response(ApiResponse::ok(content)),
            Err(err) => into_response(ApiResponse::<String>::err(ApiError::with_details(
                "LOG_EXPORT_FAILED",
                "Failed to serialize logs",
                err,
                false,
            ))),
        },
        None => into_response(ApiResponse::<String>::err(resp.error.unwrap_or_else(
            || ApiError::new("LOG_EXPORT_FAILED", "No logs returned", false),
        ))),
    }
}

// ===== deploy scripts =====

async fn list_deploy_scripts_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(project_id): Path<Id>,
) -> Response {
    let scripts: Vec<DeployScript> = s
        .state
        .config
        .read()
        .await
        .deploy_scripts
        .iter()
        .filter(|d| d.project_id == project_id)
        .cloned()
        .collect();
    into_response(ApiResponse::ok(scripts))
}

async fn create_deploy_script_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(input): Json<DeployScriptFormInput>,
) -> Response {
    if input.name.trim().is_empty() || input.command.trim().is_empty() {
        return into_response(ApiResponse::<DeployScript>::err(ApiError::new(
            "VALIDATION_FAILED",
            "Deploy script name and command are required",
            false,
        )));
    }
    let now = Utc::now();
    let mut config = s.state.config.write().await;
    if !config.projects.iter().any(|p| p.id == input.project_id) {
        return into_response(ApiResponse::<DeployScript>::err(ApiError::new(
            "PROJECT_NOT_FOUND",
            "Project not found",
            false,
        )));
    }
    let next_order = input.order.unwrap_or_else(|| {
        config
            .deploy_scripts
            .iter()
            .filter(|d| d.project_id == input.project_id && d.stage == input.stage)
            .map(|d| d.order)
            .max()
            .map(|m| m + 1)
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
        working_directory: input.working_directory.and_then(|v| {
            let t = v.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }),
        env: input.env,
        machine_id: input.machine_id.and_then(|v| {
            if v.trim().is_empty() {
                None
            } else {
                Some(v)
            }
        }),
        continue_on_error: input.continue_on_error,
        created_at: now,
        updated_at: now,
    };
    config.deploy_scripts.push(script.clone());
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<DeployScript>::err(err));
    }
    into_response(ApiResponse::ok(script))
}

async fn update_deploy_script_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
    Json(mut input): Json<DeployScript>,
) -> Response {
    if input.name.trim().is_empty() || input.command.trim().is_empty() {
        return into_response(ApiResponse::<DeployScript>::err(ApiError::new(
            "VALIDATION_FAILED",
            "Deploy script name and command are required",
            false,
        )));
    }
    let mut config = s.state.config.write().await;
    let Some(existing) = config.deploy_scripts.iter_mut().find(|d| d.id == id) else {
        return into_response(ApiResponse::<DeployScript>::err(ApiError::new(
            "DEPLOY_SCRIPT_NOT_FOUND",
            "Deploy script not found",
            false,
        )));
    };
    input.id = id;
    input.updated_at = Utc::now();
    *existing = input.clone();
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<DeployScript>::err(err));
    }
    into_response(ApiResponse::ok(input))
}

async fn delete_deploy_script_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let mut config = s.state.config.write().await;
    let before = config.deploy_scripts.len();
    config.deploy_scripts.retain(|d| d.id != id);
    if before == config.deploy_scripts.len() {
        return into_response(ApiResponse::<bool>::err(ApiError::new(
            "DEPLOY_SCRIPT_NOT_FOUND",
            "Deploy script not found",
            false,
        )));
    }
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<bool>::err(err));
    }
    into_response(ApiResponse::ok(true))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderBody {
    ordered_ids: Vec<Id>,
}

async fn reorder_deploy_scripts_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(project_id): Path<Id>,
    Json(body): Json<ReorderBody>,
) -> Response {
    let mut config = s.state.config.write().await;
    let now = Utc::now();
    for (index, id) in body.ordered_ids.iter().enumerate() {
        if let Some(script) = config
            .deploy_scripts
            .iter_mut()
            .find(|d| d.id == *id && d.project_id == project_id)
        {
            script.order = index as i32;
            script.updated_at = now;
        }
    }
    let scripts: Vec<DeployScript> = config
        .deploy_scripts
        .iter()
        .filter(|d| d.project_id == project_id)
        .cloned()
        .collect();
    if let Err(err) = persist_config(&s.app, &config).await {
        return into_response(ApiResponse::<Vec<DeployScript>>::err(err));
    }
    into_response(ApiResponse::ok(scripts))
}

// ===== deploys =====

async fn list_deploys_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let mut entries: Vec<DeployHistoryEntry> = storage::load_deploy_history(&s.app)
        .into_iter()
        .filter(|entry| entry.project_id == id)
        .collect();
    entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    into_response(ApiResponse::ok(entries))
}

async fn list_all_deploys_handler(AxumState(s): AxumState<HttpApiState>) -> Response {
    let runs = deploy::all_states(&s.state).await;
    into_response(ApiResponse::ok(runs))
}

async fn deploy_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    match deploy::start_deployment(s.app.clone(), s.state.clone(), id).await {
        Ok(run) => into_response(ApiResponse::ok(run)),
        Err(error) => into_response(ApiResponse::<crate::models::DeployRunState>::err(error)),
    }
}

async fn cancel_deploy_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    match deploy::cancel_deployment(s.app.clone(), s.state.clone(), id).await {
        Ok(run) => into_response(ApiResponse::ok(run)),
        Err(error) => into_response(ApiResponse::<crate::models::DeployRunState>::err(error)),
    }
}

async fn get_deploy_state_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let run = deploy::get_state(&s.state, &id).await;
    into_response(ApiResponse::ok(run))
}

async fn get_deploy_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(run_id): Path<Id>,
) -> Response {
    let entry = storage::load_deploy_history(&s.app)
        .into_iter()
        .find(|entry| entry.run_id == run_id);
    into_response(ApiResponse::ok(entry))
}

// ===== observability =====

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HealthSummaryQuery {
    project_id: Option<Id>,
}

async fn health_summary_handler(
    AxumState(s): AxumState<HttpApiState>,
    Query(query): Query<HealthSummaryQuery>,
) -> Response {
    let resp = process_manager::get_health_summary(s.state.clone(), query.project_id).await;
    into_response(resp)
}

async fn dashboard_handler(AxumState(s): AxumState<HttpApiState>) -> Response {
    process_manager::sync_external_processes(s.app.clone(), s.state.clone()).await;
    let summary = process_manager::dashboard_summary(s.state.clone()).await;
    into_response(ApiResponse::ok(summary))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityQuery {
    limit: Option<usize>,
}

async fn activity_handler(
    AxumState(s): AxumState<HttpApiState>,
    Query(query): Query<ActivityQuery>,
) -> Response {
    let activity = s.state.config.read().await.activity.clone();
    let limit = query.limit.unwrap_or(50);
    let trimmed: Vec<_> = activity.into_iter().take(limit).collect();
    into_response(ApiResponse::ok(trimmed))
}

// ===== frontend errors =====

async fn list_frontend_errors_handler(AxumState(s): AxumState<HttpApiState>) -> Response {
    let errors = process_manager::recent_frontend_errors(&s.state).await;
    into_response(ApiResponse::ok(errors))
}

async fn record_frontend_error_handler(
    AxumState(s): AxumState<HttpApiState>,
    Json(record): Json<FrontendErrorRecord>,
) -> Response {
    process_manager::record_frontend_error(&s.app, &s.state, record).await;
    into_response(ApiResponse::ok(true))
}
