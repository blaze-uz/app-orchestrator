use crate::models::{
    ActivityEvent, ActivityType, ApiError, AppConfig, Id, RuntimeProcessRecord,
    CURRENT_CONFIG_SCHEMA_VERSION,
};
use chrono::Utc;
use std::{
    collections::HashMap,
    fs,
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
    config.schema_version = CURRENT_CONFIG_SCHEMA_VERSION;
    config
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
