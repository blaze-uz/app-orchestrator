use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Id = String;
pub const CURRENT_CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse<T>
where
    T: Serialize,
{
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl<T> ApiResponse<T>
where
    T: Serialize,
{
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: ApiError) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    pub retryable: bool,
}

impl ApiError {
    pub fn new(code: &str, message: &str, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            details: None,
            retryable,
        }
    }

    pub fn with_details(
        code: &str,
        message: &str,
        details: impl ToString,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            details: Some(details.to_string()),
            retryable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: Id,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub root_path: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub tags: Vec<String>,
    pub auto_start: bool,
    pub startup_order: i32,
    pub memory_limit_mb: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicyKind {
    Never,
    OnFailure,
    Always,
    LimitedRetries,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartPolicy {
    pub kind: RestartPolicyKind,
    pub max_retries: Option<u32>,
    pub retry_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HealthCheck {
    #[serde(rename = "none")]
    None,
    #[serde(rename_all = "camelCase")]
    #[serde(rename = "tcp")]
    Tcp {
        host: String,
        port: u16,
        timeout_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    #[serde(rename = "http")]
    Http {
        url: String,
        method: String,
        expected_status: u16,
        timeout_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    #[serde(rename = "custom")]
    Custom {
        command: String,
        args: Vec<String>,
        working_directory: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogMode {
    Combined,
    Split,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDefinition {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub key: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub env: HashMap<String, String>,
    pub memory_limit_mb: Option<u64>,
    pub auto_start: bool,
    pub restart_policy: RestartPolicy,
    pub startup_delay_ms: Option<u64>,
    pub depends_on: Vec<String>,
    pub health_check: HealthCheck,
    pub log_mode: LogMode,
    pub group: Option<String>,
    pub visible: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Idle,
    Queued,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
    Crashed,
    Blocked,
    WaitingDependency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Stopped,
    Starting,
    Running,
    Degraded,
    Failed,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Unhealthy,
    Degraded,
    Starting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortBinding {
    pub host: String,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRuntimeState {
    pub process_id: Id,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub last_error: Option<String>,
    pub restart_count: u32,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub cpu_usage: Option<f64>,
    pub memory_usage: Option<u64>,
    pub health_status: Option<HealthStatus>,
    pub port_bindings: Vec<PortBinding>,
    pub current_status: ProcessStatus,
}

impl ProcessRuntimeState {
    pub fn stopped(process_id: impl Into<Id>) -> Self {
        Self {
            process_id: process_id.into(),
            pid: None,
            started_at: None,
            stopped_at: None,
            exit_code: None,
            last_error: None,
            restart_count: 0,
            last_heartbeat: None,
            cpu_usage: None,
            memory_usage: None,
            health_status: Some(HealthStatus::Unknown),
            port_bindings: vec![],
            current_status: ProcessStatus::Stopped,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProcessRecord {
    pub process_id: Id,
    pub project_id: Id,
    pub pid: u32,
    pub process_group_id: u32,
    pub started_at: DateTime<Utc>,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamType {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: Id,
    pub process_id: Id,
    pub project_id: Id,
    pub timestamp: DateTime<Utc>,
    pub stream: StreamType,
    pub level: LogLevel,
    pub message: String,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: String,
    pub launch_on_login: bool,
    pub auto_start_marked_projects: bool,
    pub log_retention_lines: usize,
    pub project_storage_path: Option<String>,
    pub notifications_enabled: bool,
    pub stop_timeout_ms: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            launch_on_login: false,
            auto_start_marked_projects: false,
            log_retention_lines: 5000,
            project_storage_path: None,
            notifications_enabled: false,
            stop_timeout_ms: 5000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    ProjectCreated,
    ProjectUpdated,
    ProjectDeleted,
    ProcessCreated,
    ProcessUpdated,
    ProcessDeleted,
    ProcessStarted,
    ProcessStopped,
    ProcessFailed,
    HealthCheckFailed,
    RestartTriggered,
    ConfigImported,
    ConfigExported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: Id,
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub event_type: ActivityType,
    pub project_id: Option<Id>,
    pub process_id: Option<Id>,
    pub message: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default = "default_config_schema_version")]
    pub schema_version: u32,
    pub workspaces: Vec<Workspace>,
    pub projects: Vec<Project>,
    pub processes: Vec<ProcessDefinition>,
    pub settings: AppSettings,
    pub last_selected_project_id: Option<Id>,
    pub last_selected_process_id: Option<Id>,
    pub activity: Vec<ActivityEvent>,
}

pub fn default_config_schema_version() -> u32 {
    CURRENT_CONFIG_SCHEMA_VERSION
}

impl Default for AppConfig {
    fn default() -> Self {
        let now = Utc::now();
        let workspace = Workspace {
            id: "workspace_default".to_string(),
            name: "Default Workspace".to_string(),
            description: Some("Local developer services".to_string()),
            created_at: now,
            updated_at: now,
            is_default: true,
        };

        Self {
            schema_version: CURRENT_CONFIG_SCHEMA_VERSION,
            workspaces: vec![workspace],
            projects: vec![],
            processes: vec![],
            settings: AppSettings::default(),
            last_selected_project_id: None,
            last_selected_process_id: None,
            activity: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: Project,
    pub processes: Vec<ProcessDefinition>,
    pub runtime_states: Vec<ProcessRuntimeState>,
    pub recent_logs: Vec<LogEntry>,
    pub status: ProjectStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummary {
    pub project_count: usize,
    pub process_count: usize,
    pub running_process_count: usize,
    pub failed_process_count: usize,
    pub port_conflict_count: usize,
    pub auto_start_project_count: usize,
    pub recent_problem_logs: Vec<LogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFormInput {
    pub name: String,
    pub root_path: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub auto_start: bool,
    pub startup_order: i32,
    pub memory_limit_mb: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessFormInput {
    pub project_id: Id,
    pub name: String,
    pub key: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub env: HashMap<String, String>,
    pub memory_limit_mb: Option<u64>,
    pub auto_start: bool,
    pub restart_policy: RestartPolicy,
    pub startup_delay_ms: Option<u64>,
    pub depends_on: Vec<String>,
    pub health_check: HealthCheck,
    pub log_mode: LogMode,
    pub group: Option<String>,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogHistoryFilters {
    pub project_id: Option<Id>,
    pub process_id: Option<Id>,
    pub limit: Option<usize>,
    pub since: Option<DateTime<Utc>>,
}
