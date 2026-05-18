export type ID = string;

export type StreamType = "stdout" | "stderr" | "system";
export type LogLevel = "info" | "warn" | "error" | "debug";
export type ProcessStatus =
  | "idle"
  | "queued"
  | "starting"
  | "running"
  | "stopping"
  | "stopped"
  | "failed"
  | "crashed"
  | "blocked"
  | "waiting_dependency";
export type ProjectStatus = "stopped" | "starting" | "running" | "degraded" | "failed" | "partial";
export type HealthStatus = "unknown" | "healthy" | "unhealthy" | "degraded" | "starting";
export type RestartPolicyKind = "never" | "on-failure" | "always" | "limited-retries";
export type HealthCheckKind = "none" | "tcp" | "http" | "custom";
export type LogMode = "combined" | "split";

export interface Workspace {
  id: ID;
  name: string;
  description?: string;
  createdAt: string;
  updatedAt: string;
  isDefault: boolean;
}

export interface Machine {
  id: ID;
  name: string;
  hostname: string;
  sshUser: string;
  sshPort: number;
  sshKeyPath?: string;
  isDefaultLocal: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface MachineFormInput {
  name: string;
  hostname: string;
  sshUser: string;
  sshPort: number;
  sshKeyPath?: string;
}

export interface MachineConnectionResult {
  ok: boolean;
  latencyMs: number;
  detail: string;
}

export interface Project {
  id: ID;
  workspaceId: ID;
  name: string;
  slug: string;
  description?: string;
  rootPath: string;
  icon?: string;
  color?: string;
  tags: string[];
  autoStart: boolean;
  startupOrder: number;
  memoryLimitMb?: number;
  autoRestartOnDeploy: boolean;
  autoDeploy: boolean;
  machineId?: ID;
  createdAt: string;
  updatedAt: string;
}

export interface RestartPolicy {
  kind: RestartPolicyKind;
  maxRetries?: number;
  retryDelayMs?: number;
}

export interface TcpHealthCheck {
  kind: "tcp";
  host: string;
  port: number;
  timeoutMs: number;
}

export interface HttpHealthCheck {
  kind: "http";
  url: string;
  method: "GET" | "POST" | "HEAD";
  expectedStatus: number;
  timeoutMs: number;
}

export interface CustomHealthCheck {
  kind: "custom";
  command: string;
  args: string[];
  workingDirectory?: string;
  timeoutMs?: number;
}

export interface NoHealthCheck {
  kind: "none";
}

export type HealthCheck = NoHealthCheck | TcpHealthCheck | HttpHealthCheck | CustomHealthCheck;

export interface ProcessDefinition {
  id: ID;
  projectId: ID;
  name: string;
  key: string;
  command: string;
  args: string[];
  workingDirectory?: string;
  env: Record<string, string>;
  memoryLimitMb?: number;
  autoStart: boolean;
  restartPolicy: RestartPolicy;
  startupDelayMs?: number;
  dependsOn: string[];
  healthCheck: HealthCheck;
  logMode: LogMode;
  group?: string;
  visible: boolean;
  machineId?: ID;
  createdAt: string;
  updatedAt: string;
}

export interface ExternalProcess {
  pid: number;
  processGroupId: number;
  command: string;
  cwd: string;
  user?: string;
  startedAt?: string;
  etime?: string;
  cpuPercent?: number;
  memoryKb?: number;
  ports?: number[];
  children?: ExternalProcessChild[];
}

export interface ExternalProcessChild {
  pid: number;
  command: string;
}

export interface PortBinding {
  host: string;
  port: number;
  protocol: "tcp" | "http" | "unknown";
}

export interface ProcessRuntimeState {
  processId: ID;
  pid?: number;
  startedAt?: string;
  stoppedAt?: string;
  exitCode?: number;
  lastError?: string;
  restartCount: number;
  lastHeartbeat?: string;
  cpuUsage?: number;
  memoryUsage?: number;
  healthStatus?: HealthStatus;
  portBindings: PortBinding[];
  currentStatus: ProcessStatus;
}

export interface MetricSample {
  timestamp: string;
  cpuUsage?: number;
  memoryUsage?: number;
}

export interface LogEntry {
  id: ID;
  processId: ID;
  projectId: ID;
  timestamp: string;
  stream: StreamType;
  level: LogLevel;
  message: string;
  raw?: string;
}

export interface AppSettings {
  theme: "system" | "light" | "dark";
  launchOnLogin: boolean;
  autoStartMarkedProjects: boolean;
  logRetentionLines: number;
  projectStoragePath?: string;
  notificationsEnabled: boolean;
  stopTimeoutMs: number;
}

export interface ActivityEvent {
  id: ID;
  timestamp: string;
  type:
    | "project_created"
    | "project_updated"
    | "project_deleted"
    | "process_created"
    | "process_updated"
    | "process_deleted"
    | "process_started"
    | "process_stopped"
    | "process_failed"
    | "health_check_failed"
    | "restart_triggered"
    | "config_imported"
    | "config_exported";
  projectId?: ID;
  processId?: ID;
  message: string;
  level: "info" | "warn" | "error";
}

export interface ProjectDetail {
  project: Project;
  processes: ProcessDefinition[];
  runtimeStates: ProcessRuntimeState[];
  recentLogs: LogEntry[];
  status: ProjectStatus;
}

export interface DashboardSummary {
  projectCount: number;
  processCount: number;
  runningProcessCount: number;
  failedProcessCount: number;
  portConflictCount: number;
  autoStartProjectCount: number;
  recentProblemLogs: LogEntry[];
}

export interface AppConfig {
  schemaVersion: number;
  workspaces: Workspace[];
  projects: Project[];
  processes: ProcessDefinition[];
  machines: Machine[];
  deployScripts: DeployScript[];
  settings: AppSettings;
  lastSelectedProjectId?: ID;
  lastSelectedProcessId?: ID;
  activity: ActivityEvent[];
}

export interface ApiError {
  code: string;
  message: string;
  details?: string;
  retryable: boolean;
}

export interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: ApiError;
}

export type ViewKey =
  | "dashboard"
  | "projects"
  | "project"
  | "process"
  | "logs"
  | "machines"
  | "settings";

export interface LogFilters {
  projectId?: ID;
  processId?: ID;
  stream?: StreamType | "all";
  level?: LogLevel | "all";
  query: string;
  liveTail: boolean;
  paused: boolean;
}

export interface LogHistoryRequest {
  projectId?: ID;
  processId?: ID;
  limit?: number;
  since?: string;
}

export interface ProjectFilters {
  query: string;
  runningOnly: boolean;
  failedOnly: boolean;
  autoStartOnly: boolean;
  tag?: string;
}

export interface ProjectFormInput {
  name: string;
  rootPath: string;
  description?: string;
  tags: string[];
  autoStart: boolean;
  startupOrder: number;
  memoryLimitMb?: number;
}

export interface ProcessFormInput {
  projectId: ID;
  name: string;
  key: string;
  command: string;
  args: string[];
  workingDirectory?: string;
  env: Record<string, string>;
  memoryLimitMb?: number;
  autoStart: boolean;
  restartPolicy: RestartPolicy;
  startupDelayMs?: number;
  dependsOn: string[];
  healthCheck: HealthCheck;
  logMode: LogMode;
  group?: string;
  visible: boolean;
  machineId?: ID;
}

export interface ValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
}

export type DeployStage = "pre" | "main" | "post";

export type DeployStatus = "idle" | "running" | "success" | "failed" | "cancelled";

export type DeployScriptStatus = "pending" | "running" | "success" | "failed" | "skipped";

export interface DeployScript {
  id: ID;
  projectId: ID;
  name: string;
  stage: DeployStage;
  order: number;
  command: string;
  args: string[];
  workingDirectory?: string;
  env: Record<string, string>;
  machineId?: ID;
  continueOnError: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface DeployScriptFormInput {
  projectId: ID;
  name: string;
  stage: DeployStage;
  order?: number;
  command: string;
  args: string[];
  workingDirectory?: string;
  env: Record<string, string>;
  machineId?: ID;
  continueOnError: boolean;
}

export interface DeployScriptResult {
  scriptId: ID;
  status: DeployScriptStatus;
  exitCode?: number;
  startedAt?: string;
  completedAt?: string;
  error?: string;
}

export interface DeployRunState {
  runId: ID;
  projectId: ID;
  status: DeployStatus;
  currentScriptId?: ID;
  startedAt?: string;
  completedAt?: string;
  scriptResults: DeployScriptResult[];
  lastError?: string;
}

export interface DeployHistoryEntry {
  runId: ID;
  projectId: ID;
  status: DeployStatus;
  startedAt: string;
  completedAt?: string;
  scriptResults: DeployScriptResult[];
  lastError?: string;
  trigger?: string;
  logs: LogEntry[];
}
