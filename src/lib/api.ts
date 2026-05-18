import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ApiError,
  ApiResponse,
  AppConfig,
  AppSettings,
  DashboardSummary,
  DeployHistoryEntry,
  DeployRunState,
  DeployScript,
  DeployScriptFormInput,
  ExternalProcess,
  ID,
  LogEntry,
  LogHistoryRequest,
  Machine,
  MachineConnectionResult,
  MachineFormInput,
  MetricSample,
  ProcessDefinition,
  ProcessFormInput,
  ProcessRuntimeState,
  Project,
  ProjectDetail,
  ProjectFormInput,
  ValidationResult,
  Workspace
} from "../types/domain";
import { mockApi } from "./mockApi";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function command<T>(name: string, args?: Record<string, unknown>): Promise<ApiResponse<T>> {
  if (!isTauri) {
    const mockName = name.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
    const fn = (mockApi as unknown as Record<string, (...params: unknown[]) => Promise<ApiResponse<T>>>)[mockName];
    if (!fn) {
      return { success: false, error: { code: "MOCK_COMMAND_MISSING", message: `Mock command ${mockName} is not implemented`, retryable: false } };
    }
    return fn.call(mockApi, ...Object.values(args ?? {}));
  }
  return invoke<ApiResponse<T>>(name, args);
}

export const api = {
  isTauri,
  on<T>(event: string, handler: (payload: T) => void) {
    if (!isTauri) return mockApi.on(event, handler);
    let dispose: () => void = () => undefined;
    listen<T>(event, (message) => handler(message.payload)).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose();
  },
  listWorkspaces: () => command<Workspace[]>("list_workspaces"),
  createWorkspace: (input: Pick<Workspace, "name" | "description">) => command<Workspace>("create_workspace", { input }),
  updateWorkspace: (workspace: Workspace) => command<Workspace>("update_workspace", { workspace }),
  deleteWorkspace: (workspaceId: ID) => command<boolean>("delete_workspace", { workspaceId }),
  listMachines: () => command<Machine[]>("list_machines"),
  createMachine: (input: MachineFormInput) => command<Machine>("create_machine", { input }),
  updateMachine: (machine: Machine) => command<Machine>("update_machine", { machine }),
  deleteMachine: (machineId: ID) => command<boolean>("delete_machine", { machineId }),
  testMachineConnection: (machineId: ID) => command<MachineConnectionResult>("test_machine_connection", { machineId }),
  listProjects: () => command<Project[]>("list_projects"),
  createProject: (input: ProjectFormInput) => command<Project>("create_project", { input }),
  updateProject: (project: Project) => command<Project>("update_project", { project }),
  deleteProject: (projectId: ID) => command<boolean>("delete_project", { projectId }),
  getProjectDetail: (projectId: ID) => command<ProjectDetail>("get_project_detail", { projectId }),
  listProcessesByProject: (projectId: ID) => command<ProcessDefinition[]>("list_processes_by_project", { projectId }),
  createProcessDefinition: (input: ProcessFormInput) => command<ProcessDefinition>("create_process_definition", { input }),
  updateProcessDefinition: (process: ProcessDefinition) => command<ProcessDefinition>("update_process_definition", { process }),
  deleteProcessDefinition: (processId: ID) => command<boolean>("delete_process_definition", { processId }),
  startProcess: (processId: ID) => command<ProcessRuntimeState>("start_process", { processId }),
  stopProcess: (processId: ID) => command<ProcessRuntimeState>("stop_process", { processId }),
  restartProcess: (processId: ID) => command<ProcessRuntimeState>("restart_process", { processId }),
  startProject: (projectId: ID) => command<ProjectDetail>("start_project", { projectId }),
  startAutoStartProcesses: (projectId: ID) => command<ProjectDetail>("start_auto_start_processes", { projectId }),
  stopProject: (projectId: ID) => command<ProjectDetail>("stop_project", { projectId }),
  restartProject: (projectId: ID) => command<ProjectDetail>("restart_project", { projectId }),
  restartFailedProcesses: (projectId?: ID) => command<ProcessRuntimeState[]>("restart_failed_processes", { projectId }),
  listExternalProjectProcesses: (projectId: ID) => command<ExternalProcess[]>("list_external_project_processes", { projectId }),
  stopExternalProcess: (processGroupId: number) => command<boolean>("stop_external_process", { processGroupId }),
  findProcessOnPort: (port: number) => command<ExternalProcess | null>("find_process_on_port", { port }),
  getRuntimeState: (processId: ID) => command<ProcessRuntimeState>("get_runtime_state", { processId }),
  getAllRuntimeStates: () => command<ProcessRuntimeState[]>("get_all_runtime_states"),
  getProcessMetricsHistory: (processId: ID) => command<MetricSample[]>("get_process_metrics_history", { processId }),
  getLogHistory: (filters?: LogHistoryRequest) => command<LogEntry[]>("get_log_history", { filters }),
  clearLogHistory: (projectId?: ID) => command<boolean>("clear_log_history", { projectId }),
  exportLogs: (filters?: { projectId?: ID; processId?: ID }) => command<string>("export_logs", { filters }),
  runHealthCheck: (processId: ID) => command<ProcessRuntimeState>("run_health_check", { processId }),
  getHealthSummary: (projectId?: ID) => command<{ healthy: number; unhealthy: number; unknown: number }>("get_health_summary", { projectId }),
  openProjectFolderInFinder: (projectId: ID) => command<boolean>("open_project_folder_in_finder", { projectId }),
  openPathInFinder: (path: string) => command<boolean>("open_path_in_finder", { path }),
  revealLogFileInFinder: () => command<boolean>("reveal_log_file_in_finder"),
  validateProjectPath: (rootPath: string) => command<ValidationResult>("validate_project_path", { rootPath }),
  detectPortsInUse: () => command<Array<{ host: string; port: number; process?: string }>>("detect_ports_in_use"),
  getConfig: () => command<AppConfig>("get_config"),
  updateSettings: (settings: AppSettings) => command<AppSettings>("update_settings", { settings }),
  applyMediaGuardPreset: (basePath?: string) => command<AppConfig>("apply_media_guard_preset", { basePath }),
  importConfig: (config: AppConfig) => command<AppConfig>("import_config", { config }),
  exportConfig: (redactSecrets = true) => command<string>("export_config", { redactSecrets }),
  exportConfigToPath: (path: string, redactSecrets = true) =>
    command<string>("export_config_to_path", { path, redactSecrets }),
  getDashboardSummary: () => command<DashboardSummary>("get_dashboard_summary"),
  getRecentFrontendErrors: () =>
    command<Array<{
      source: string;
      message: string;
      stack?: string;
      componentStack?: string;
      timestamp: string;
      url?: string;
    }>>("get_recent_frontend_errors"),
  listDeployScripts: (projectId: ID) => command<DeployScript[]>("list_deploy_scripts", { projectId }),
  createDeployScript: (input: DeployScriptFormInput) => command<DeployScript>("create_deploy_script", { input }),
  updateDeployScript: (script: DeployScript) => command<DeployScript>("update_deploy_script", { script }),
  deleteDeployScript: (scriptId: ID) => command<boolean>("delete_deploy_script", { scriptId }),
  reorderDeployScripts: (projectId: ID, orderedIds: ID[]) =>
    command<DeployScript[]>("reorder_deploy_scripts", { projectId, orderedIds }),
  deployProject: (projectId: ID) => command<DeployRunState>("deploy_project", { projectId }),
  cancelDeploy: (projectId: ID) => command<DeployRunState>("cancel_deploy", { projectId }),
  getDeployState: (projectId: ID) => command<DeployRunState | null>("get_deploy_state", { projectId }),
  getAllDeployStates: () => command<DeployRunState[]>("get_all_deploy_states"),
  listDeployHistory: (projectId: ID) =>
    command<DeployHistoryEntry[]>("list_deploy_history", { projectId }),
  getDeployHistoryEntry: (runId: ID) =>
    command<DeployHistoryEntry | null>("get_deploy_history_entry", { runId })
};

export class ApiCallError extends Error {
  readonly code: string;
  readonly details?: string;
  readonly retryable: boolean;

  constructor(apiError: ApiError) {
    super(apiError.message);
    this.name = "ApiCallError";
    this.code = apiError.code;
    this.details = apiError.details;
    this.retryable = apiError.retryable;
  }
}

export function unwrap<T>(response: ApiResponse<T>): T {
  if (!response.success || response.data === undefined) {
    throw new ApiCallError(
      response.error ?? { code: "UNKNOWN_ERROR", message: "Command failed", retryable: false }
    );
  }
  return response.data;
}
