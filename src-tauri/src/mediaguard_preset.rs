use crate::models::{
    AppConfig, HealthCheck, LogMode, ProcessDefinition, Project, RestartPolicy, RestartPolicyKind,
};
use chrono::{DateTime, Utc};
use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
};

const WORKSPACE_ID: &str = "workspace_default";
const PROJECT_WEB: &str = "project_media_guard_web";
const PROJECT_AGENT: &str = "project_media_guard_collector_agent";
const PROJECT_TELEGRAM: &str = "project_media_guard_telegram";
const PROJECT_INSTAGRAM: &str = "project_media_guard_instagram";
const PROJECT_YOUTUBE: &str = "project_media_guard_youtube";
const PROJECT_FACEBOOK: &str = "project_media_guard_facebook";
const PROJECT_ANALYZER: &str = "project_media_guard_analizer";

const OLD_WEB_PROJECT_ID: &str = "project_5d59221441524aadab582d075159b90d";

const PROJECT_FOLDERS: [&str; 7] = [
    "media-guard-web",
    "media-guard-collector-agent",
    "media-guard-telegram",
    "media-guard-instagram",
    "media-guard-youtube",
    "media-guard-facebook",
    "media-guard-analizer",
];

pub fn default_base_path() -> String {
    env::var("HOME")
        .map(|home| PathBuf::from(home).join("Herd"))
        .unwrap_or_else(|_| PathBuf::from("Herd"))
        .to_string_lossy()
        .to_string()
}

pub fn apply(config: &mut AppConfig, base_path: Option<String>) {
    let now = Utc::now();
    let base_path = base_path
        .filter(|path| !path.trim().is_empty())
        .unwrap_or_else(default_base_path);

    let old_projects = config.projects.clone();
    let old_processes = config.processes.clone();
    let desired_project_ids = desired_project_ids();
    let removed_project_ids: HashSet<_> = old_projects
        .iter()
        .filter(|project| is_mediaguard_project(project, &desired_project_ids))
        .map(|project| project.id.clone())
        .collect();

    config
        .projects
        .retain(|project| !is_mediaguard_project(project, &desired_project_ids));
    config.processes.retain(|process| {
        !desired_project_ids.contains(process.project_id.as_str())
            && !removed_project_ids.contains(process.project_id.as_str())
    });

    let projects = desired_projects(&base_path, &old_projects, now);
    let processes = desired_processes(&base_path, &old_projects, &old_processes, now);

    config.projects.extend(projects);
    config.processes.extend(processes);
    config.projects.sort_by_key(|project| project.startup_order);

    if config
        .last_selected_project_id
        .as_ref()
        .map(|id| removed_project_ids.contains(id.as_str()))
        .unwrap_or(true)
    {
        config.last_selected_project_id = Some(PROJECT_WEB.to_string());
    }
    if config
        .last_selected_process_id
        .as_ref()
        .map(|id| !config.processes.iter().any(|process| &process.id == id))
        .unwrap_or(false)
    {
        config.last_selected_process_id = None;
    }
}

fn desired_project_ids() -> HashSet<&'static str> {
    [
        PROJECT_WEB,
        PROJECT_AGENT,
        PROJECT_TELEGRAM,
        PROJECT_INSTAGRAM,
        PROJECT_YOUTUBE,
        PROJECT_FACEBOOK,
        PROJECT_ANALYZER,
    ]
    .into_iter()
    .collect()
}

fn is_mediaguard_project(project: &Project, desired_project_ids: &HashSet<&str>) -> bool {
    if desired_project_ids.contains(project.id.as_str()) || project.id == OLD_WEB_PROJECT_ID {
        return true;
    }
    let root_folder = Path::new(&project.root_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    PROJECT_FOLDERS.contains(&root_folder) || matches!(project.slug.as_str(), "mediaguard-web")
}

fn desired_projects(base_path: &str, old_projects: &[Project], now: DateTime<Utc>) -> Vec<Project> {
    vec![
        project(
            PROJECT_WEB,
            "MediaGuard Web",
            "media-guard-web",
            "Laravel central app, queues, scheduler, and Vite UI.",
            base_path,
            "media-guard-web",
            "#31d07f",
            vec!["laravel", "central", "web"],
            10,
            old_projects,
            now,
        ),
        project(
            PROJECT_AGENT,
            "MediaGuard Collector Agent",
            "media-guard-collector-agent",
            "Agentic orchestration service for MediaGuard collectors.",
            base_path,
            "media-guard-collector-agent",
            "#6f8cff",
            vec!["python", "agent", "collector"],
            20,
            old_projects,
            now,
        ),
        project(
            PROJECT_TELEGRAM,
            "MediaGuard Telegram",
            "media-guard-telegram",
            "Go TDLib Telegram collector and internal management API.",
            base_path,
            "media-guard-telegram",
            "#69b7ff",
            vec!["go", "telegram", "collector"],
            30,
            old_projects,
            now,
        ),
        project(
            PROJECT_INSTAGRAM,
            "MediaGuard Instagram",
            "media-guard-instagram",
            "Python FastAPI Instagram collector.",
            base_path,
            "media-guard-instagram",
            "#d86fd8",
            vec!["python", "instagram", "collector"],
            40,
            old_projects,
            now,
        ),
        project(
            PROJECT_YOUTUBE,
            "MediaGuard YouTube",
            "media-guard-youtube",
            "Node.js YouTube collector, task server, and ops dashboard.",
            base_path,
            "media-guard-youtube",
            "#ff6b62",
            vec!["node", "youtube", "collector"],
            50,
            old_projects,
            now,
        ),
        project(
            PROJECT_FACEBOOK,
            "MediaGuard Facebook",
            "media-guard-facebook",
            "Node.js Facebook collector and ops dashboard.",
            base_path,
            "media-guard-facebook",
            "#3f8df6",
            vec!["node", "facebook", "collector"],
            60,
            old_projects,
            now,
        ),
        project(
            PROJECT_ANALYZER,
            "MediaGuard Analizer",
            "media-guard-analizer",
            "Python AI sentiment, embedding, report, and taxonomy workers.",
            base_path,
            "media-guard-analizer",
            "#8f7cff",
            vec!["python", "ai", "analizer"],
            70,
            old_projects,
            now,
        ),
    ]
}

fn desired_processes(
    _base_path: &str,
    old_projects: &[Project],
    old_processes: &[ProcessDefinition],
    now: DateTime<Utc>,
) -> Vec<ProcessDefinition> {
    vec![
        process(
            "process_media_guard_web_laravel_http",
            PROJECT_WEB,
            "Laravel HTTP server",
            "laravel-http",
            "php",
            vec!["artisan", "serve", "--host=127.0.0.1", "--port=8000"],
            HashMap::new(),
            true,
            Some(http_health("http://127.0.0.1:8000/up", 2000)),
            "web",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_web_vite",
            PROJECT_WEB,
            "Vite dev server",
            "vite",
            "npm",
            vec![
                "run",
                "dev",
                "--",
                "--host=127.0.0.1",
                "--port=5173",
                "--strictPort",
            ],
            HashMap::new(),
            true,
            Some(tcp_health(5173)),
            "frontend",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_web_queue_default",
            PROJECT_WEB,
            "Queue default",
            "queue-default",
            "php",
            vec![
                "artisan",
                "queue:work",
                "--queue=monitoring,default",
                "--tries=1",
                "--timeout=0",
            ],
            HashMap::new(),
            true,
            None,
            "queue",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_web_scheduler",
            PROJECT_WEB,
            "Laravel scheduler",
            "scheduler",
            "php",
            vec!["artisan", "schedule:work"],
            HashMap::new(),
            true,
            None,
            "scheduler",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_collector_agent_api",
            PROJECT_AGENT,
            "Collector Agent API",
            "agent-api",
            "./run.sh",
            vec!["backend"],
            HashMap::new(),
            true,
            Some(http_health("http://127.0.0.1:8090/health", 1500)),
            "agent",
            vec![],
            Some(3000),
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_telegram_collector",
            PROJECT_TELEGRAM,
            "Telegram collector API",
            "collector",
            "./run.sh",
            vec!["run"],
            env_map(vec![
                ("APP_PORT", "8080"),
                ("COLLECTOR_AGENT_BASE_URL", "http://127.0.0.1:8090"),
                ("COLLECTOR_PUBLIC_BASE_URL", "http://127.0.0.1:8080"),
                ("COLLECTOR_INSTANCE_NAME", "telegram-collector"),
            ]),
            true,
            Some(http_health("http://127.0.0.1:8080/health", 1500)),
            "collector",
            vec!["agent-api"],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_instagram_collector",
            PROJECT_INSTAGRAM,
            "Instagram collector API",
            "collector",
            "./.venv/bin/python",
            vec!["main.py"],
            env_map(vec![
                ("HOST", "127.0.0.1"),
                ("PORT", "8091"),
                ("COLLECTOR_AGENT_BASE_URL", "http://127.0.0.1:8090"),
                ("COLLECTOR_PUBLIC_BASE_URL", "http://127.0.0.1:8091"),
                ("COLLECTOR_INSTANCE_NAME", "instagram-collector"),
            ]),
            true,
            Some(http_health("http://127.0.0.1:8091/health", 1500)),
            "collector",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_youtube_collector",
            PROJECT_YOUTUBE,
            "YouTube collector API",
            "collector",
            "npm",
            vec!["start"],
            env_map(vec![
                ("COLLECTOR_BIND_HOST", "127.0.0.1"),
                ("COLLECTOR_BIND_PORT", "8082"),
                ("COLLECTOR_PUBLIC_BASE_URL", "http://127.0.0.1:8082"),
                ("COLLECTOR_AGENT_BASE_URL", "http://127.0.0.1:8090"),
                ("COLLECTOR_INSTANCE_NAME", "youtube-collector"),
            ]),
            true,
            Some(http_health("http://127.0.0.1:8082/health", 1500)),
            "collector",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_facebook_collector",
            PROJECT_FACEBOOK,
            "Facebook collector API",
            "collector",
            "npm",
            vec!["start"],
            env_map(vec![
                ("COLLECTOR_BIND_HOST", "127.0.0.1"),
                ("COLLECTOR_BIND_PORT", "8083"),
                ("COLLECTOR_PUBLIC_BASE_URL", "http://127.0.0.1:8083"),
                ("COLLECTOR_AGENT_BASE_URL", "http://127.0.0.1:8090"),
                ("COLLECTOR_INSTANCE_NAME", "facebook-collector"),
            ]),
            true,
            Some(http_health("http://127.0.0.1:8083/health", 1500)),
            "collector",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_analizer_sentiment_worker",
            PROJECT_ANALYZER,
            "Sentiment worker",
            "sentiment-worker",
            "./.venv/bin/python",
            vec!["-m", "app.main_sentiment"],
            HashMap::new(),
            true,
            None,
            "ai-workers",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_analizer_embedding_worker",
            PROJECT_ANALYZER,
            "Embedding worker",
            "embedding-worker",
            "./.venv/bin/python",
            vec!["-m", "app.main_embedding"],
            HashMap::new(),
            true,
            None,
            "ai-workers",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_analizer_report_worker",
            PROJECT_ANALYZER,
            "Report worker",
            "report-worker",
            "./.venv/bin/python",
            vec!["-m", "app.main_report"],
            HashMap::new(),
            true,
            None,
            "ai-workers",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
        process(
            "process_media_guard_analizer_taxonomy_worker",
            PROJECT_ANALYZER,
            "Taxonomy worker",
            "taxonomy-worker",
            "./.venv/bin/python",
            vec!["-m", "app.main_taxonomy"],
            HashMap::new(),
            true,
            None,
            "ai-workers",
            vec![],
            None,
            old_projects,
            old_processes,
            now,
        ),
    ]
}

fn project(
    id: &str,
    name: &str,
    slug: &str,
    description: &str,
    base_path: &str,
    folder: &str,
    color: &str,
    tags: Vec<&str>,
    startup_order: i32,
    old_projects: &[Project],
    now: DateTime<Utc>,
) -> Project {
    let root_path = PathBuf::from(base_path)
        .join(folder)
        .to_string_lossy()
        .to_string();
    let created_at = old_projects
        .iter()
        .find(|project| {
            project.id == id
                || (id == PROJECT_WEB && project.id == OLD_WEB_PROJECT_ID)
                || project.root_path == root_path
                || project.slug == slug
        })
        .map(|project| project.created_at)
        .unwrap_or(now);

    Project {
        id: id.to_string(),
        workspace_id: WORKSPACE_ID.to_string(),
        name: name.to_string(),
        slug: slug.to_string(),
        description: Some(description.to_string()),
        root_path,
        icon: None,
        color: Some(color.to_string()),
        tags: tags.into_iter().map(str::to_string).collect(),
        auto_start: false,
        startup_order,
        memory_limit_mb: None,
        created_at,
        updated_at: now,
    }
}

#[allow(clippy::too_many_arguments)]
fn process(
    id: &str,
    project_id: &str,
    name: &str,
    key: &str,
    command: &str,
    args: Vec<&str>,
    env: HashMap<String, String>,
    auto_start: bool,
    health_check: Option<HealthCheck>,
    group: &str,
    depends_on: Vec<&str>,
    startup_delay_ms: Option<u64>,
    old_projects: &[Project],
    old_processes: &[ProcessDefinition],
    now: DateTime<Utc>,
) -> ProcessDefinition {
    let created_at = old_processes
        .iter()
        .find(|process| {
            process.id == id
                || (process.key == key
                    && old_project_matches(old_projects, &process.project_id, project_id))
        })
        .map(|process| process.created_at)
        .unwrap_or(now);

    ProcessDefinition {
        id: id.to_string(),
        project_id: project_id.to_string(),
        name: name.to_string(),
        key: key.to_string(),
        command: command.to_string(),
        args: args.into_iter().map(str::to_string).collect(),
        working_directory: None,
        env,
        memory_limit_mb: None,
        auto_start,
        restart_policy: RestartPolicy {
            kind: RestartPolicyKind::OnFailure,
            max_retries: None,
            retry_delay_ms: Some(3000),
        },
        startup_delay_ms,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        health_check: health_check.unwrap_or(HealthCheck::None),
        log_mode: LogMode::Combined,
        group: Some(group.to_string()),
        visible: true,
        created_at,
        updated_at: now,
    }
}

fn old_project_matches(
    old_projects: &[Project],
    old_project_id: &str,
    desired_project_id: &str,
) -> bool {
    old_project_id == desired_project_id
        || (desired_project_id == PROJECT_WEB && old_project_id == OLD_WEB_PROJECT_ID)
        || old_projects.iter().any(|project| {
            project.id == old_project_id
                && is_project_folder_for_desired(&project.root_path, desired_project_id)
        })
}

fn is_project_folder_for_desired(root_path: &str, desired_project_id: &str) -> bool {
    let folder = Path::new(root_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    matches!(
        (desired_project_id, folder),
        (PROJECT_WEB, "media-guard-web")
            | (PROJECT_AGENT, "media-guard-collector-agent")
            | (PROJECT_TELEGRAM, "media-guard-telegram")
            | (PROJECT_INSTAGRAM, "media-guard-instagram")
            | (PROJECT_YOUTUBE, "media-guard-youtube")
            | (PROJECT_FACEBOOK, "media-guard-facebook")
            | (PROJECT_ANALYZER, "media-guard-analizer")
    )
}

fn env_map(items: Vec<(&str, &str)>) -> HashMap<String, String> {
    items
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn tcp_health(port: u16) -> HealthCheck {
    HealthCheck::Tcp {
        host: "127.0.0.1".to_string(),
        port,
        timeout_ms: 1200,
    }
}

fn http_health(url: &str, timeout_ms: u64) -> HealthCheck {
    HealthCheck::Http {
        url: url.to_string(),
        method: "GET".to_string(),
        expected_status: 200,
        timeout_ms,
    }
}
