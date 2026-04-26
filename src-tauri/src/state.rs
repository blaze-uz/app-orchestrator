use crate::models::{AppConfig, Id, LogEntry, ProcessRuntimeState, RuntimeProcessRecord};
use std::sync::OnceLock;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct RuntimeRegistry {
    pub states: Arc<RwLock<HashMap<Id, ProcessRuntimeState>>>,
    pub logs: Arc<RwLock<Vec<LogEntry>>>,
    pub pids: Arc<RwLock<HashMap<Id, u32>>>,
    pub process_records: Arc<RwLock<HashMap<Id, RuntimeProcessRecord>>>,
    pub stopping_processes: Arc<RwLock<HashSet<Id>>>,
    pub log_history_io: Arc<Mutex<()>>,
}

impl RuntimeRegistry {
    pub fn new(
        config: &AppConfig,
        process_records: HashMap<Id, RuntimeProcessRecord>,
        logs: Vec<LogEntry>,
    ) -> Self {
        let states = config
            .processes
            .iter()
            .map(|process| {
                (
                    process.id.clone(),
                    ProcessRuntimeState::stopped(process.id.clone()),
                )
            })
            .collect();
        let pids = process_records
            .iter()
            .map(|(process_id, record)| (process_id.clone(), record.process_group_id))
            .collect();
        Self {
            states: Arc::new(RwLock::new(states)),
            logs: Arc::new(RwLock::new(logs)),
            pids: Arc::new(RwLock::new(pids)),
            process_records: Arc::new(RwLock::new(process_records)),
            stopping_processes: Arc::new(RwLock::new(HashSet::new())),
            log_history_io: Arc::new(Mutex::new(())),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub runtime: RuntimeRegistry,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        process_records: HashMap<Id, RuntimeProcessRecord>,
        logs: Vec<LogEntry>,
    ) -> Self {
        let runtime = RuntimeRegistry::new(&config, process_records, logs);
        Self {
            config: Arc::new(RwLock::new(config)),
            runtime,
        }
    }
}

static APP_STATE: OnceLock<AppState> = OnceLock::new();

pub fn set_global_state(state: AppState) {
    let _ = APP_STATE.set(state);
}

pub fn app_state() -> AppState {
    APP_STATE
        .get()
        .expect("app state must be initialized before commands run")
        .clone()
}
