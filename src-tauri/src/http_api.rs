use crate::models::{ApiResponse, DeployHistoryEntry, Id, LogHistoryFilters, ProjectDetail};
use crate::process_manager;
use crate::state::AppState;
use crate::storage;
use axum::{
    extract::{Path, Query, Request, State as AxumState},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
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
        .route("/api/v1/processes", get(list_processes))
        .route("/api/v1/processes/:id", get(get_process))
        .route("/api/v1/processes/:id/metrics", get(get_process_metrics))
        .route("/api/v1/processes/:id/start", post(start_process_handler))
        .route("/api/v1/processes/:id/stop", post(stop_process_handler))
        .route("/api/v1/processes/:id/restart", post(restart_process_handler))
        .route(
            "/api/v1/processes/:id/health-check",
            post(run_health_handler),
        )
        .route("/api/v1/projects", get(list_projects))
        .route("/api/v1/projects/:id", get(get_project))
        .route("/api/v1/projects/:id/start", post(start_project_handler))
        .route("/api/v1/projects/:id/stop", post(stop_project_handler))
        .route(
            "/api/v1/projects/:id/restart",
            post(restart_project_handler),
        )
        .route("/api/v1/logs", get(get_logs))
        .route("/api/v1/projects/:id/deploys", get(list_deploys_handler))
        .route("/api/v1/deploys/:run_id", get(get_deploy_handler))
        .route("/api/v1/health-summary", get(health_summary_handler))
        .route("/api/v1/dashboard", get(dashboard_handler))
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
        } else if code.contains("INVALID") || code.contains("VALIDATION") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(resp)).into_response()
    }
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "ok": true,
        "name": "app-orchestrator",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

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

async fn list_projects(AxumState(s): AxumState<HttpApiState>) -> Response {
    let config = s.state.config.read().await;
    let projects = config.projects.clone();
    drop(config);
    into_response(ApiResponse::ok(projects))
}

async fn get_project(AxumState(s): AxumState<HttpApiState>, Path(id): Path<Id>) -> Response {
    process_manager::sync_external_processes(s.app.clone(), s.state.clone()).await;
    match process_manager::get_project_detail(&s.state, &id).await {
        Ok(detail) => into_response(ApiResponse::ok(detail)),
        Err(error) => into_response(ApiResponse::<ProjectDetail>::err(error)),
    }
}

async fn start_project_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(id): Path<Id>,
) -> Response {
    let resp = process_manager::start_project(s.app.clone(), s.state.clone(), id).await;
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

async fn get_deploy_handler(
    AxumState(s): AxumState<HttpApiState>,
    Path(run_id): Path<Id>,
) -> Response {
    let entry = storage::load_deploy_history(&s.app)
        .into_iter()
        .find(|entry| entry.run_id == run_id);
    into_response(ApiResponse::ok(entry))
}

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
