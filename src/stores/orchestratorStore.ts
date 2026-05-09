import { create } from "zustand";
import { api, unwrap } from "../lib/api";
import { deriveProjectStatus } from "../lib/status";
import type {
  ActivityEvent,
  AppConfig,
  AppSettings,
  DashboardSummary,
  ExternalProcess,
  ID,
  LogEntry,
  LogFilters,
  ProcessDefinition,
  ProcessFormInput,
  ProcessRuntimeState,
  Project,
  ProjectFilters,
  ProjectFormInput,
  ViewKey,
  Workspace
} from "../types/domain";

interface ActionState {
  key: string;
  label: string;
}

interface OrchestratorState {
  booted: boolean;
  view: ViewKey;
  workspaces: Workspace[];
  projects: Project[];
  processes: ProcessDefinition[];
  runtimeStates: Record<ID, ProcessRuntimeState>;
  externalProcesses: Record<ID, ExternalProcess[]>;
  logs: LogEntry[];
  activity: ActivityEvent[];
  settings?: AppSettings;
  selectedWorkspaceId?: ID;
  selectedProjectId?: ID;
  selectedProcessId?: ID;
  projectFilters: ProjectFilters;
  logFilters: LogFilters;
  dashboard?: DashboardSummary;
  currentAction?: ActionState;
  lastError?: string;
  initialize: () => Promise<void>;
  refreshAll: () => Promise<void>;
  refreshDashboard: () => Promise<void>;
  selectView: (view: ViewKey) => void;
  selectProject: (projectId: ID) => Promise<void>;
  selectProcess: (processId: ID) => void;
  createProject: (input: ProjectFormInput) => Promise<void>;
  updateProject: (project: Project) => Promise<void>;
  deleteProject: (projectId: ID) => Promise<void>;
  createProcess: (input: ProcessFormInput) => Promise<boolean>;
  updateProcess: (process: ProcessDefinition) => Promise<void>;
  deleteProcess: (processId: ID) => Promise<void>;
  startProcess: (processId: ID) => Promise<void>;
  stopProcess: (processId: ID) => Promise<void>;
  restartProcess: (processId: ID) => Promise<void>;
  startProject: (projectId: ID) => Promise<void>;
  startAutoStartProcesses: (projectId: ID) => Promise<void>;
  stopProject: (projectId: ID) => Promise<void>;
  restartProject: (projectId: ID) => Promise<void>;
  restartFailed: (projectId?: ID) => Promise<void>;
  loadExternalProcesses: (projectId: ID) => Promise<void>;
  stopExternalProcess: (projectId: ID, processGroupId: number) => Promise<void>;
  runHealthCheck: (processId: ID) => Promise<void>;
  clearLogs: (projectId?: ID) => Promise<void>;
  applyMediaGuardPreset: (basePath?: string) => Promise<boolean>;
  importConfig: (config: AppConfig) => Promise<void>;
  exportConfig: (redactSecrets?: boolean) => Promise<string>;
  exportLogs: () => Promise<string>;
  updateSettings: (settings: AppSettings) => Promise<void>;
  setProjectFilters: (filters: Partial<ProjectFilters>) => void;
  setLogFilters: (filters: Partial<LogFilters>) => void;
}

const defaultProjectFilters: ProjectFilters = {
  query: "",
  runningOnly: false,
  failedOnly: false,
  autoStartOnly: false
};

const defaultLogFilters: LogFilters = {
  query: "",
  stream: "all",
  level: "all",
  liveTail: true,
  paused: false
};
const LOG_HISTORY_WINDOW_MS = 5 * 60 * 1000;

function recentLogHistorySince() {
  return new Date(Date.now() - LOG_HISTORY_WINDOW_MS).toISOString();
}

function mergeRuntime(states: ProcessRuntimeState[]) {
  return states.reduce<Record<ID, ProcessRuntimeState>>((acc, state) => {
    acc[state.processId] = state;
    return acc;
  }, {});
}

function mergeProcesses(existing: ProcessDefinition[], incoming: ProcessDefinition[]) {
  const merged = new Map(existing.map((process) => [process.id, process]));
  for (const process of incoming) merged.set(process.id, process);
  return Array.from(merged.values());
}

async function safeAction<T>(set: (partial: Partial<OrchestratorState>) => void, action: ActionState, task: () => Promise<T>): Promise<T | undefined> {
  try {
    set({ currentAction: action, lastError: undefined });
    return await task();
  } catch (error) {
    set({ lastError: error instanceof Error ? error.message : String(error) });
    return undefined;
  } finally {
    set({ currentAction: undefined });
  }
}

export const useOrchestratorStore = create<OrchestratorState>((set, get) => ({
  booted: false,
  view: "dashboard",
  workspaces: [],
  projects: [],
  processes: [],
  runtimeStates: {},
  externalProcesses: {},
  logs: [],
  activity: [],
  projectFilters: defaultProjectFilters,
  logFilters: defaultLogFilters,
  initialize: async () => {
    if (get().booted) return;
    api.on<LogEntry>("process_log", (log) => {
      if (get().logFilters.paused) return;
      set((state) => ({
        logs: state.logs.some((entry) => entry.id === log.id)
          ? state.logs
          : [...state.logs.slice(-(state.settings?.logRetentionLines ?? 5000) + 1), log]
      }));
    });
    const updateRuntime = (runtime: ProcessRuntimeState) => {
      set((state) => ({ runtimeStates: { ...state.runtimeStates, [runtime.processId]: runtime } }));
      void get().refreshDashboard();
    };
    api.on<ProcessRuntimeState>("process_started", updateRuntime);
    api.on<ProcessRuntimeState>("process_stopped", updateRuntime);
    api.on<ProcessRuntimeState>("process_failed", updateRuntime);
    api.on<ProcessRuntimeState>("process_health_changed", updateRuntime);
    api.on<ProcessRuntimeState>("process_metrics_changed", (runtime) => {
      set((state) => ({ runtimeStates: { ...state.runtimeStates, [runtime.processId]: runtime } }));
    });
    await get().refreshAll();
    set({ booted: true });
  },
  refreshAll: async () => {
    await safeAction(set, { key: "refresh", label: "Refreshing workspace" }, async () => {
      const [config, workspaces, projects, runtimes, logs, dashboard] = await Promise.all([
        api.getConfig().then(unwrap),
        api.listWorkspaces().then(unwrap),
        api.listProjects().then(unwrap),
        api.getAllRuntimeStates().then(unwrap),
        api.getLogHistory({ limit: 1000, since: recentLogHistorySince() }).then(unwrap),
        api.getDashboardSummary().then(unwrap)
      ]);
      const selectedProjectId = config.lastSelectedProjectId ?? projects[0]?.id;
      set({
        workspaces,
        projects,
        processes: config.processes,
        runtimeStates: mergeRuntime(runtimes),
        logs,
        activity: config.activity,
        settings: config.settings,
        selectedWorkspaceId: config.workspaces.find((workspace) => workspace.isDefault)?.id ?? workspaces[0]?.id,
        selectedProjectId,
        selectedProcessId: config.lastSelectedProcessId,
        dashboard,
        view: selectedProjectId ? get().view : "dashboard"
      });
    });
  },
  refreshDashboard: async () => {
    try {
      set({ dashboard: await api.getDashboardSummary().then(unwrap) });
    } catch {
      // Dashboard updates are secondary to runtime actions.
    }
  },
  selectView: (view) => set({ view }),
  selectProject: async (projectId) => {
    await safeAction(set, { key: `select:${projectId}`, label: "Loading project" }, async () => {
      const detail = await api.getProjectDetail(projectId).then(unwrap);
      set((state) => ({
        selectedProjectId: projectId,
        selectedProcessId: detail.processes[0]?.id,
        processes: mergeProcesses(state.processes, detail.processes),
        runtimeStates: { ...state.runtimeStates, ...mergeRuntime(detail.runtimeStates) },
        view: "project"
      }));
    });
    void get().loadExternalProcesses(projectId);
  },
  selectProcess: (processId) => set({ selectedProcessId: processId, view: "process" }),
  createProject: async (input) => {
    await safeAction(set, { key: "create-project", label: "Creating project" }, async () => {
      const project = await api.createProject(input).then(unwrap);
      set((state) => ({ projects: [...state.projects, project], selectedProjectId: project.id, view: "project" }));
      await get().selectProject(project.id);
      await get().refreshDashboard();
    });
  },
  updateProject: async (project) => {
    await safeAction(set, { key: `update-project:${project.id}`, label: "Saving project" }, async () => {
      const saved = await api.updateProject(project).then(unwrap);
      set((state) => ({ projects: state.projects.map((item) => (item.id === saved.id ? saved : item)) }));
    });
  },
  deleteProject: async (projectId) => {
    await safeAction(set, { key: `delete-project:${projectId}`, label: "Deleting project" }, async () => {
      try {
        await api.stopProject(projectId).then(unwrap);
      } catch {
        // Deletion should still be possible if nothing is running or stop already failed.
      }
      await api.deleteProject(projectId).then(unwrap);
      const projects = get().projects.filter((project) => project.id !== projectId);
      set((state) => ({
        projects,
        processes: state.processes.filter((process) => process.projectId !== projectId),
        selectedProjectId: projects[0]?.id,
        selectedProcessId: undefined,
        view: projects[0] ? "project" : "dashboard"
      }));
      if (projects[0]) await get().selectProject(projects[0].id);
      await get().refreshDashboard();
    });
  },
  createProcess: async (input) => {
    const created = await safeAction(set, { key: "create-process", label: "Creating process" }, async () => {
      const process = await api.createProcessDefinition(input).then(unwrap);
      set((state) => ({ processes: [...state.processes, process], selectedProcessId: process.id }));
      await get().selectProject(input.projectId);
      return true;
    });
    return Boolean(created);
  },
  updateProcess: async (process) => {
    await safeAction(set, { key: `update-process:${process.id}`, label: "Saving process" }, async () => {
      const saved = await api.updateProcessDefinition(process).then(unwrap);
      set((state) => ({ processes: state.processes.map((item) => (item.id === saved.id ? saved : item)) }));
    });
  },
  deleteProcess: async (processId) => {
    await safeAction(set, { key: `delete-process:${processId}`, label: "Deleting process" }, async () => {
      await api.deleteProcessDefinition(processId).then(unwrap);
      set((state) => {
        const processes = state.processes.filter((process) => process.id !== processId);
        return { processes, selectedProcessId: processes[0]?.id };
      });
    });
  },
  startProcess: async (processId) => {
    await safeAction(set, { key: `start:${processId}`, label: "Starting process" }, async () => {
      const runtime = await api.startProcess(processId).then(unwrap);
      set((state) => ({ runtimeStates: { ...state.runtimeStates, [runtime.processId]: runtime } }));
      await get().refreshDashboard();
    });
  },
  stopProcess: async (processId) => {
    await safeAction(set, { key: `stop:${processId}`, label: "Stopping process" }, async () => {
      const runtime = await api.stopProcess(processId).then(unwrap);
      set((state) => ({ runtimeStates: { ...state.runtimeStates, [runtime.processId]: runtime } }));
      await get().refreshDashboard();
    });
  },
  restartProcess: async (processId) => {
    await safeAction(set, { key: `restart:${processId}`, label: "Restarting process" }, async () => {
      const runtime = await api.restartProcess(processId).then(unwrap);
      set((state) => ({ runtimeStates: { ...state.runtimeStates, [runtime.processId]: runtime } }));
      await get().refreshDashboard();
    });
  },
  startProject: async (projectId) => {
    await safeAction(set, { key: `start-project:${projectId}`, label: "Starting project" }, async () => {
      const detail = await api.startProject(projectId).then(unwrap);
      set((state) => ({ processes: mergeProcesses(state.processes, detail.processes), runtimeStates: { ...state.runtimeStates, ...mergeRuntime(detail.runtimeStates) } }));
      await get().refreshDashboard();
    });
  },
  startAutoStartProcesses: async (projectId) => {
    await safeAction(set, { key: `start-auto:${projectId}`, label: "Starting marked processes" }, async () => {
      const detail = await api.startAutoStartProcesses(projectId).then(unwrap);
      set((state) => ({ processes: mergeProcesses(state.processes, detail.processes), runtimeStates: { ...state.runtimeStates, ...mergeRuntime(detail.runtimeStates) } }));
      await get().refreshDashboard();
    });
  },
  stopProject: async (projectId) => {
    await safeAction(set, { key: `stop-project:${projectId}`, label: "Stopping project" }, async () => {
      const detail = await api.stopProject(projectId).then(unwrap);
      set((state) => ({ processes: mergeProcesses(state.processes, detail.processes), runtimeStates: { ...state.runtimeStates, ...mergeRuntime(detail.runtimeStates) } }));
      await get().refreshDashboard();
    });
  },
  restartProject: async (projectId) => {
    await safeAction(set, { key: `restart-project:${projectId}`, label: "Restarting project" }, async () => {
      const detail = await api.restartProject(projectId).then(unwrap);
      set((state) => ({ processes: mergeProcesses(state.processes, detail.processes), runtimeStates: { ...state.runtimeStates, ...mergeRuntime(detail.runtimeStates) } }));
      await get().refreshDashboard();
    });
  },
  restartFailed: async (projectId) => {
    await safeAction(set, { key: "restart-failed", label: "Restarting failed processes" }, async () => {
      const runtimes = await api.restartFailedProcesses(projectId).then(unwrap);
      set((state) => ({ runtimeStates: { ...state.runtimeStates, ...mergeRuntime(runtimes) } }));
      await get().refreshDashboard();
    });
  },
  loadExternalProcesses: async (projectId) => {
    try {
      const list = await api.listExternalProjectProcesses(projectId).then(unwrap);
      set((state) => ({ externalProcesses: { ...state.externalProcesses, [projectId]: list } }));
    } catch (error) {
      set({ lastError: error instanceof Error ? error.message : String(error) });
    }
  },
  stopExternalProcess: async (projectId, processGroupId) => {
    await safeAction(set, { key: `stop-external:${processGroupId}`, label: "Stopping process" }, async () => {
      await api.stopExternalProcess(processGroupId).then(unwrap);
      const list = await api.listExternalProjectProcesses(projectId).then(unwrap);
      set((state) => ({ externalProcesses: { ...state.externalProcesses, [projectId]: list } }));
    });
  },
  runHealthCheck: async (processId) => {
    await safeAction(set, { key: `health:${processId}`, label: "Running health check" }, async () => {
      const runtime = await api.runHealthCheck(processId).then(unwrap);
      set((state) => ({ runtimeStates: { ...state.runtimeStates, [runtime.processId]: runtime } }));
    });
  },
  clearLogs: async (projectId) => {
    await safeAction(set, { key: "clear-logs", label: "Clearing logs" }, async () => {
      await api.clearLogHistory(projectId).then(unwrap);
      set((state) => ({ logs: state.logs.filter((log) => projectId && log.projectId !== projectId) }));
    });
  },
  applyMediaGuardPreset: async (basePath) => {
    const applied = await safeAction(set, { key: "mediaguard-preset", label: "Syncing MediaGuard" }, async () => {
      await api.applyMediaGuardPreset(basePath).then(unwrap);
      await get().refreshAll();
      return true;
    });
    return Boolean(applied);
  },
  importConfig: async (config) => {
    await safeAction(set, { key: "import-config", label: "Importing config" }, async () => {
      await api.importConfig(config).then(unwrap);
      await get().refreshAll();
    });
  },
  exportConfig: async (redactSecrets = true) => api.exportConfig(redactSecrets).then(unwrap),
  exportLogs: async () => api.exportLogs().then(unwrap),
  updateSettings: async (settings) => {
    await safeAction(set, { key: "settings", label: "Saving settings" }, async () => {
      const saved = await api.updateSettings(settings).then(unwrap);
      set({ settings: saved });
    });
  },
  setProjectFilters: (filters) => set((state) => ({ projectFilters: { ...state.projectFilters, ...filters } })),
  setLogFilters: (filters) => set((state) => ({ logFilters: { ...state.logFilters, ...filters } }))
}));

export function selectCurrentProject(state: OrchestratorState): Project | undefined {
  return state.projects.find((project) => project.id === state.selectedProjectId);
}

export function selectCurrentProcess(state: OrchestratorState): ProcessDefinition | undefined {
  return state.processes.find((process) => process.id === state.selectedProcessId);
}

export function selectProjectStatus(state: OrchestratorState, projectId: ID) {
  const states = state.processes
    .filter((process) => process.projectId === projectId)
    .map((process) => state.runtimeStates[process.id])
    .filter(Boolean);
  return deriveProjectStatus(states);
}
