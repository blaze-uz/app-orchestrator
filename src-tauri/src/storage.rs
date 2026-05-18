use crate::models::{
    ActivityEvent, ActivityType, ApiError, AppConfig, DeployHistoryEntry, Id, LogEntry, Machine,
    RuntimeProcessRecord, CURRENT_CONFIG_SCHEMA_VERSION, DEFAULT_LOCAL_MACHINE_ID,
};
use chrono::{DateTime, Utc};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

pub fn config_path(app: &AppHandle) -> Result<PathBuf, ApiError> {
    let dir = app.path().app_config_dir().map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to resolve app config directory",
            error,
            false,
        )
    })?;
    fs::create_dir_all(&dir).map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to create app config directory",
            error,
            false,
        )
    })?;
    Ok(dir.join("config.json"))
}

pub fn load_config(app: &AppHandle) -> AppConfig {
    let Ok(path) = config_path(app) else {
        return AppConfig::default();
    };
    let _ = migrate_legacy_app_config_if_needed(&path);
    if !path.exists() {
        let config = AppConfig::default();
        let _ = save_config_to_path(&path, &config);
        return config;
    }
    let Ok(content) = fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    let source_schema_version = serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|value| {
            value
                .get("schemaVersion")
                .and_then(|schema| schema.as_u64())
        })
        .unwrap_or(0);

    let Ok(mut config) = serde_json::from_str::<AppConfig>(&content) else {
        return AppConfig::default();
    };

    if source_schema_version < CURRENT_CONFIG_SCHEMA_VERSION as u64 {
        config = migrate_config(config);
        if backup_config_before_migration(&path, &content).is_ok() {
            let _ = save_config_to_path(&path, &config);
        }
    } else {
        ensure_default_local_machine(&mut config);
    }

    config
}

pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), ApiError> {
    let path = config_path(app)?;
    save_config_to_path(&path, config)
}

pub fn runtime_pids_path(app: &AppHandle) -> Result<PathBuf, ApiError> {
    let dir = app.path().app_config_dir().map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to resolve app config directory",
            error,
            false,
        )
    })?;
    fs::create_dir_all(&dir).map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to create app config directory",
            error,
            false,
        )
    })?;
    Ok(dir.join("runtime-pids.json"))
}

pub fn log_history_path(app: &AppHandle) -> Result<PathBuf, ApiError> {
    let dir = app.path().app_config_dir().map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to resolve app config directory",
            error,
            false,
        )
    })?;
    fs::create_dir_all(&dir).map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to create app config directory",
            error,
            false,
        )
    })?;
    Ok(dir.join("log-history.jsonl"))
}

pub fn load_recent_logs(app: &AppHandle, since: DateTime<Utc>) -> Vec<LogEntry> {
    let Ok(path) = log_history_path(app) else {
        return vec![];
    };
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<LogEntry>(line).ok())
        .filter(|entry| entry.timestamp >= since)
        .collect()
}

pub fn append_log_entry(app: &AppHandle, entry: &LogEntry) -> Result<(), ApiError> {
    let path = log_history_path(app)?;
    let line = serde_json::to_string(entry).map_err(|error| {
        ApiError::with_details(
            "CONFIG_SERIALIZATION_FAILED",
            "Unable to serialize log entry",
            error,
            false,
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            ApiError::with_details(
                "CONFIG_WRITE_FAILED",
                "Unable to open log history",
                error,
                true,
            )
        })?;
    writeln!(file, "{line}").map_err(|error| {
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to append log history",
            error,
            true,
        )
    })
}

pub fn prune_log_history(app: &AppHandle, since: DateTime<Utc>) -> Result<(), ApiError> {
    rewrite_log_history(app, |entry| entry.timestamp >= since)
}

pub fn clear_log_history(app: &AppHandle, project_id: Option<&str>) -> Result<(), ApiError> {
    rewrite_log_history(app, |entry| {
        project_id
            .map(|project_id| entry.project_id != project_id)
            .unwrap_or(false)
    })
}

fn rewrite_log_history(
    app: &AppHandle,
    retain: impl Fn(&LogEntry) -> bool,
) -> Result<(), ApiError> {
    let path = log_history_path(app)?;
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        ApiError::with_details(
            "CONFIG_READ_FAILED",
            "Unable to read log history",
            error,
            true,
        )
    })?;
    let retained = content
        .lines()
        .filter_map(|line| serde_json::from_str::<LogEntry>(line).ok())
        .filter(|entry| retain(entry))
        .collect::<Vec<_>>();
    if retained.is_empty() {
        fs::remove_file(&path).map_err(|error| {
            ApiError::with_details(
                "CONFIG_WRITE_FAILED",
                "Unable to clear log history",
                error,
                true,
            )
        })?;
        return Ok(());
    }

    let temp_path = path.with_extension("jsonl.tmp");
    let mut file = fs::File::create(&temp_path).map_err(|error| {
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to rewrite log history",
            error,
            true,
        )
    })?;
    for entry in retained {
        let line = serde_json::to_string(&entry).map_err(|error| {
            ApiError::with_details(
                "CONFIG_SERIALIZATION_FAILED",
                "Unable to serialize log entry",
                error,
                false,
            )
        })?;
        writeln!(file, "{line}").map_err(|error| {
            ApiError::with_details(
                "CONFIG_WRITE_FAILED",
                "Unable to rewrite log history",
                error,
                true,
            )
        })?;
    }
    fs::rename(&temp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to commit log history rewrite",
            error,
            true,
        )
    })
}

pub fn deploy_history_path(app: &AppHandle) -> Result<PathBuf, ApiError> {
    let dir = app.path().app_config_dir().map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to resolve app config directory",
            error,
            false,
        )
    })?;
    fs::create_dir_all(&dir).map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to create app config directory",
            error,
            false,
        )
    })?;
    Ok(dir.join("deploy-history.jsonl"))
}

pub fn load_deploy_history(app: &AppHandle) -> Vec<DeployHistoryEntry> {
    let Ok(path) = deploy_history_path(app) else {
        return vec![];
    };
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<DeployHistoryEntry>(line).ok())
        .collect()
}

pub fn append_deploy_history(
    app: &AppHandle,
    entry: &DeployHistoryEntry,
) -> Result<(), ApiError> {
    let path = deploy_history_path(app)?;
    let line = serde_json::to_string(entry).map_err(|error| {
        ApiError::with_details(
            "CONFIG_SERIALIZATION_FAILED",
            "Unable to serialize deploy history entry",
            error,
            false,
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            ApiError::with_details(
                "CONFIG_WRITE_FAILED",
                "Unable to open deploy history",
                error,
                true,
            )
        })?;
    writeln!(file, "{line}").map_err(|error| {
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to append deploy history",
            error,
            true,
        )
    })
}

pub fn prune_deploy_history_for_project(
    app: &AppHandle,
    project_id: &str,
    keep_per_project: usize,
) -> Result<(), ApiError> {
    let path = deploy_history_path(app)?;
    if !path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        ApiError::with_details(
            "CONFIG_READ_FAILED",
            "Unable to read deploy history",
            error,
            true,
        )
    })?;
    let entries: Vec<DeployHistoryEntry> = content
        .lines()
        .filter_map(|line| serde_json::from_str::<DeployHistoryEntry>(line).ok())
        .collect();

    let mut target_entries: Vec<&DeployHistoryEntry> = entries
        .iter()
        .filter(|entry| entry.project_id == project_id)
        .collect();
    if target_entries.len() <= keep_per_project {
        return Ok(());
    }
    target_entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    let keep_ids: std::collections::HashSet<&str> = target_entries
        .iter()
        .take(keep_per_project)
        .map(|entry| entry.run_id.as_str())
        .collect();

    let retained: Vec<&DeployHistoryEntry> = entries
        .iter()
        .filter(|entry| entry.project_id != project_id || keep_ids.contains(entry.run_id.as_str()))
        .collect();

    if retained.is_empty() {
        fs::remove_file(&path).map_err(|error| {
            ApiError::with_details(
                "CONFIG_WRITE_FAILED",
                "Unable to clear deploy history",
                error,
                true,
            )
        })?;
        return Ok(());
    }

    let temp_path = path.with_extension("jsonl.tmp");
    let mut file = fs::File::create(&temp_path).map_err(|error| {
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to rewrite deploy history",
            error,
            true,
        )
    })?;
    for entry in retained {
        let line = serde_json::to_string(entry).map_err(|error| {
            ApiError::with_details(
                "CONFIG_SERIALIZATION_FAILED",
                "Unable to serialize deploy history entry",
                error,
                false,
            )
        })?;
        writeln!(file, "{line}").map_err(|error| {
            ApiError::with_details(
                "CONFIG_WRITE_FAILED",
                "Unable to rewrite deploy history",
                error,
                true,
            )
        })?;
    }
    fs::rename(&temp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to commit deploy history rewrite",
            error,
            true,
        )
    })
}

pub fn load_runtime_processes(
    app: &AppHandle,
    config: &AppConfig,
) -> HashMap<Id, RuntimeProcessRecord> {
    let Ok(path) = runtime_pids_path(app) else {
        return HashMap::new();
    };
    let Some(value) = fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
    else {
        return HashMap::new();
    };

    if let Ok(records) = serde_json::from_value::<HashMap<Id, RuntimeProcessRecord>>(value.clone())
    {
        return normalize_runtime_records(config, records);
    }

    let Some(object) = value.as_object() else {
        return HashMap::new();
    };

    let process_lookup: HashMap<_, _> = config
        .processes
        .iter()
        .map(|process| (process.id.as_str(), process))
        .collect();
    object
        .iter()
        .filter_map(|(process_id, value)| {
            let pid = value.as_u64()? as u32;
            let process = process_lookup.get(process_id.as_str())?;
            Some((
                process_id.clone(),
                RuntimeProcessRecord {
                    process_id: process_id.clone(),
                    project_id: process.project_id.clone(),
                    pid,
                    process_group_id: pid,
                    started_at: Utc::now(),
                    command: process.command.clone(),
                },
            ))
        })
        .collect()
}

pub fn save_runtime_processes(
    app: &AppHandle,
    records: &HashMap<Id, RuntimeProcessRecord>,
) -> Result<(), ApiError> {
    let path = runtime_pids_path(app)?;
    if records.is_empty() {
        if path.exists() {
            fs::remove_file(path).map_err(|error| {
                ApiError::with_details(
                    "CONFIG_WRITE_FAILED",
                    "Unable to remove runtime process registry",
                    error,
                    true,
                )
            })?;
        }
        return Ok(());
    }

    let content = serde_json::to_string_pretty(records).map_err(|error| {
        ApiError::with_details(
            "CONFIG_SERIALIZATION_FAILED",
            "Unable to serialize runtime process registry",
            error,
            false,
        )
    })?;
    fs::write(path, content).map_err(|error| {
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to write runtime process registry",
            error,
            true,
        )
    })
}

fn normalize_runtime_records(
    config: &AppConfig,
    records: HashMap<Id, RuntimeProcessRecord>,
) -> HashMap<Id, RuntimeProcessRecord> {
    let process_lookup: HashMap<_, _> = config
        .processes
        .iter()
        .map(|process| (process.id.as_str(), process))
        .collect();
    records
        .into_iter()
        .filter_map(|(process_id, mut record)| {
            let process = process_lookup.get(process_id.as_str())?;
            record.process_id = process_id.clone();
            record.project_id = process.project_id.clone();
            if record.process_group_id == 0 {
                record.process_group_id = record.pid;
            }
            if record.command.trim().is_empty() {
                record.command = process.command.clone();
            }
            Some((process_id, record))
        })
        .collect()
}

pub fn migrate_config(mut config: AppConfig) -> AppConfig {
    let previous_version = config.schema_version;
    config.schema_version = CURRENT_CONFIG_SCHEMA_VERSION;
    ensure_default_local_machine(&mut config);
    if previous_version < 5 {
        migrate_to_v5(&mut config);
    }
    config
}

fn migrate_to_v5(config: &mut AppConfig) {
    // Existing MediaGuard installs: enable auto-start so the new preset
    // (project.auto_start = true) actually fires on launch. New installs
    // already get this via AppSettings::default(), so this only flips the
    // flag for users upgrading from schema v4.
    let has_mediaguard_projects = config
        .projects
        .iter()
        .any(|project| project.id.starts_with("project_media_guard_"));
    if has_mediaguard_projects {
        config.settings.auto_start_marked_projects = true;
    }
}

pub fn ensure_default_local_machine(config: &mut AppConfig) {
    let now = Utc::now();
    let has_default = config.machines.iter().any(|machine| machine.is_default_local);
    if !has_default {
        if let Some(existing) = config
            .machines
            .iter_mut()
            .find(|machine| machine.id == DEFAULT_LOCAL_MACHINE_ID)
        {
            existing.is_default_local = true;
            existing.updated_at = now;
        } else {
            config.machines.insert(0, Machine::default_local(now));
        }
    }
    let mut default_seen = false;
    for machine in &mut config.machines {
        if machine.is_default_local {
            if default_seen {
                machine.is_default_local = false;
            } else {
                default_seen = true;
            }
        }
    }
}

fn backup_config_before_migration(path: &Path, content: &str) -> Result<(), ApiError> {
    let backup_path = migration_backup_path(path);
    fs::write(backup_path, content).map_err(|error| {
        ApiError::with_details(
            "CONFIG_BACKUP_FAILED",
            "Unable to create config backup before migration",
            error,
            true,
        )
    })
}

fn migration_backup_path(path: &Path) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.json");
    path.with_file_name(format!("{filename}.backup-{timestamp}.json"))
}

fn save_config_to_path(path: &Path, config: &AppConfig) -> Result<(), ApiError> {
    let content = serde_json::to_string_pretty(config).map_err(|error| {
        ApiError::with_details(
            "CONFIG_SERIALIZATION_FAILED",
            "Unable to serialize config",
            error,
            false,
        )
    })?;
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, format!("{content}\n")).map_err(|error| {
        ApiError::with_details("CONFIG_WRITE_FAILED", "Unable to write config", error, true)
    })?;
    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to commit config update",
            error,
            true,
        )
    })
}

fn migrate_legacy_app_config_if_needed(path: &Path) -> Result<(), ApiError> {
    let Some(current_dir) = path.parent() else {
        return Ok(());
    };
    let Some(config_parent) = current_dir.parent() else {
        return Ok(());
    };
    let legacy_dir = config_parent.join("dev.local-project-orchestrator.app");
    let legacy_config_path = legacy_dir.join("config.json");
    if !legacy_config_path.exists() || !should_use_legacy_config(path, &legacy_config_path) {
        return Ok(());
    }

    fs::create_dir_all(current_dir).map_err(|error| {
        ApiError::with_details(
            "CONFIG_PATH_UNAVAILABLE",
            "Unable to create app config directory",
            error,
            false,
        )
    })?;
    if path.exists() {
        let backup_path = migration_backup_path(path);
        let _ = fs::copy(path, backup_path);
    }
    fs::copy(&legacy_config_path, path).map_err(|error| {
        ApiError::with_details(
            "CONFIG_WRITE_FAILED",
            "Unable to migrate legacy app config",
            error,
            true,
        )
    })?;

    let legacy_runtime_path = legacy_dir.join("runtime-pids.json");
    if legacy_runtime_path.exists() {
        let _ = fs::copy(legacy_runtime_path, current_dir.join("runtime-pids.json"));
    }
    Ok(())
}

fn should_use_legacy_config(current_path: &Path, legacy_path: &Path) -> bool {
    let legacy_score = config_content_score(legacy_path);
    if legacy_score == 0 {
        return false;
    }
    if !current_path.exists() {
        return true;
    }
    config_content_score(current_path) == 0
}

fn config_content_score(path: &Path) -> usize {
    let Some(value) = fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
    else {
        return 0;
    };
    ["projects", "processes", "activity"]
        .iter()
        .filter_map(|key| value.get(key).and_then(|items| items.as_array()))
        .map(Vec::len)
        .sum()
}

pub fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for ch in value.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn id(prefix: &str) -> Id {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

pub fn activity(
    event_type: ActivityType,
    message: impl Into<String>,
    level: &str,
    project_id: Option<Id>,
    process_id: Option<Id>,
) -> ActivityEvent {
    ActivityEvent {
        id: id("activity"),
        timestamp: Utc::now(),
        event_type,
        project_id,
        process_id,
        message: message.into(),
        level: level.to_string(),
    }
}
