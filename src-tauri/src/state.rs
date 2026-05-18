use crate::models::{
    AppConfig, DeployRunState, FrontendErrorRecord, Id, LogEntry, MetricSample,
    ProcessRuntimeState, RuntimeProcessRecord,
};
use std::sync::OnceLock;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};
use tokio::sync::{watch, Mutex, RwLock};

pub const FRONTEND_ERROR_RETENTION: usize = 100;

#[derive(Clone)]
pub struct RuntimeRegistry {
    pub states: Arc<RwLock<HashMap<Id, ProcessRuntimeState>>>,
    pub logs: Arc<RwLock<VecDeque<LogEntry>>>,
    pub pids: Arc<RwLock<HashMap<Id, u32>>>,
    pub process_records: Arc<RwLock<HashMap<Id, RuntimeProcessRecord>>>,
    pub stopping_processes: Arc<RwLock<HashSet<Id>>>,
    pub log_history_io: Arc<Mutex<()>>,
    pub metrics_history: Arc<RwLock<HashMap<Id, VecDeque<MetricSample>>>>,
    pub frontend_errors: Arc<RwLock<VecDeque<FrontendErrorRecord>>>,
    pub log_batchers: Arc<RwLock<HashMap<Id, Arc<Mutex<Vec<LogEntry>>>>>>,
    pub remote_pids: Arc<RwLock<HashMap<Id, u32>>>,
    pub deploy_states: Arc<RwLock<HashMap<Id, DeployRunState>>>,
    pub deploy_cancel: Arc<RwLock<HashMap<Id, watch::Sender<bool>>>>,
    pub deploy_log_buffers: Arc<RwLock<HashMap<Id, Vec<LogEntry>>>>,
    pub deploy_history_io: Arc<Mutex<()>>,
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
            logs: Arc::new(RwLock::new(VecDeque::from(logs))),
            pids: Arc::new(RwLock::new(pids)),
            process_records: Arc::new(RwLock::new(process_records)),
            stopping_processes: Arc::new(RwLock::new(HashSet::new())),
            log_history_io: Arc::new(Mutex::new(())),
            metrics_history: Arc::new(RwLock::new(HashMap::new())),
            frontend_errors: Arc::new(RwLock::new(VecDeque::with_capacity(
                FRONTEND_ERROR_RETENTION,
            ))),
            log_batchers: Arc::new(RwLock::new(HashMap::new())),
            remote_pids: Arc::new(RwLock::new(HashMap::new())),
            deploy_states: Arc::new(RwLock::new(HashMap::new())),
            deploy_cancel: Arc::new(RwLock::new(HashMap::new())),
            deploy_log_buffers: Arc::new(RwLock::new(HashMap::new())),
            deploy_history_io: Arc::new(Mutex::new(())),
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
