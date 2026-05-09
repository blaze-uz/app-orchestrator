import type {
  ActivityEvent,
  ApiResponse,
  AppConfig,
  AppSettings,
  DashboardSummary,
  ID,
  LogEntry,
  ProcessDefinition,
  ProcessFormInput,
  ProcessRuntimeState,
  Project,
  ProjectDetail,
  ProjectFormInput,
  ValidationResult,
  Workspace
} from "../types/domain";
import { deriveProjectStatus, FAILED_STATUSES } from "./status";

type EventHandler<T> = (payload: T) => void;

const now = () => new Date().toISOString();
const id = (prefix: string) => `${prefix}_${Math.random().toString(36).slice(2, 10)}`;

const defaultWorkspace: Workspace = {
  id: "workspace_default",
  name: "Default Workspace",
  description: "Local developer services",
  createdAt: now(),
  updatedAt: now(),
  isDefault: true
};

const defaultSettings: AppSettings = {
  theme: "dark",
  launchOnLogin: false,
  autoStartMarkedProjects: false,
  logRetentionLines: 5000,
  notificationsEnabled: false,
  stopTimeoutMs: 5000
};

const demoProject: Project = {
  id: "project_mediaguard",
  workspaceId: defaultWorkspace.id,
  name: "MediaGuard Local",
  slug: "mediaguard-local",
  description: "Demo multi-process workspace",
  rootPath: "/Users/demo/Herd/mediaguard",
  icon: "Shield",
  color: "#32d583",
  tags: ["laravel", "collector", "queue"],
  autoStart: true,
  startupOrder: 1,
  memoryLimitMb: 2048,
  createdAt: now(),
  updatedAt: now()
};

const demoProcesses: ProcessDefinition[] = [
  {
    id: "process_api",
    projectId: demoProject.id,
    name: "Laravel API",
    key: "api",
    command: "php",
    args: ["artisan", "serve", "--port=8000"],
    workingDirectory: demoProject.rootPath,
    env: { APP_ENV: "local" },
    memoryLimitMb: 512,
    autoStart: true,
    restartPolicy: { kind: "on-failure", maxRetries: 3, retryDelayMs: 1500 },
    dependsOn: [],
    healthCheck: { kind: "http", url: "http://127.0.0.1:8000/up", method: "GET", expectedStatus: 200, timeoutMs: 2000 },
    logMode: "combined",
    group: "web",
    visible: true,
    createdAt: now(),
    updatedAt: now()
  },
  {
    id: "process_queue",
    projectId: demoProject.id,
    name: "Queue Worker",
    key: "queue",
    command: "php",
    args: ["artisan", "queue:work"],
    workingDirectory: demoProject.rootPath,
    env: { APP_ENV: "local" },
    memoryLimitMb: 384,
    autoStart: true,
    restartPolicy: { kind: "always", retryDelayMs: 2000 },
    dependsOn: ["api"],
    healthCheck: { kind: "none" },
    logMode: "combined",
    group: "workers",
    visible: true,
    createdAt: now(),
    updatedAt: now()
  },
  {
    id: "process_collector",
    projectId: demoProject.id,
    name: "Telegram Collector",
    key: "telegram_collector",
    command: "node",
    args: ["workers/telegram.js"],
    workingDirectory: demoProject.rootPath,
    env: { NODE_ENV: "development", TELEGRAM_TOKEN: "redacted" },
    memoryLimitMb: 768,
    autoStart: false,
    restartPolicy: { kind: "limited-retries", maxRetries: 2, retryDelayMs: 3000 },
    dependsOn: ["queue"],
    healthCheck: { kind: "none" },
    logMode: "split",
    group: "collectors",
    visible: true,
    createdAt: now(),
    updatedAt: now()
  }
];

const initialRuntime: ProcessRuntimeState[] = demoProcesses.map((process) => ({
  processId: process.id,
  restartCount: 0,
  healthStatus: "unknown",
  portBindings: process.key === "api" ? [{ host: "127.0.0.1", port: 8000, protocol: "http" }] : [],
  currentStatus: "stopped"
}));

function activity(type: ActivityEvent["type"], message: string, level: ActivityEvent["level"] = "info", projectId?: ID, processId?: ID): ActivityEvent {
  return { id: id("activity"), timestamp: now(), type, message, level, projectId, processId };
}

class MockApi {
  private config: AppConfig = {
    schemaVersion: 1,
    workspaces: [defaultWorkspace],
    projects: [demoProject],
    processes: demoProcesses,
    settings: defaultSettings,
    lastSelectedProjectId: demoProject.id,
    activity: [activity("project_created", "Demo workspace loaded", "info", demoProject.id)]
  };

  private runtime = new Map<ID, ProcessRuntimeState>(initialRuntime.map((state) => [state.processId, state]));
  private logs: LogEntry[] = [];
  private handlers = new Map<string, Set<EventHandler<unknown>>>();
  private intervals = new Map<ID, number>();

  on<T>(event: string, handler: EventHandler<T>): () => void {
    const set = this.handlers.get(event) ?? new Set<EventHandler<unknown>>();
    set.add(handler as EventHandler<unknown>);
    this.handlers.set(event, set);
    return () => set.delete(handler as EventHandler<unknown>);
  }

  private emit<T>(event: string, payload: T) {
    this.handlers.get(event)?.forEach((handler) => handler(payload));
  }

  private ok<T>(data: T): ApiResponse<T> {
    return { success: true, data };
  }

  private error<T>(code: string, message: string): ApiResponse<T> {
    return { success: false, error: { code, message, retryable: false } };
  }

  listWorkspaces() {
    return Promise.resolve(this.ok([...this.config.workspaces]));
  }

  createWorkspace(input: Pick<Workspace, "name" | "description">) {
    const workspace: Workspace = {
      id: id("workspace"),
      name: input.name,
      description: input.description,
      isDefault: false,
      createdAt: now(),
      updatedAt: now()
    };
    this.config.workspaces.push(workspace);
    return Promise.resolve(this.ok(workspace));
  }

  updateWorkspace(workspace: Workspace) {
    this.config.workspaces = this.config.workspaces.map((item) => (item.id === workspace.id ? { ...workspace, updatedAt: now() } : item));
    return Promise.resolve(this.ok(workspace));
  }

  deleteWorkspace(workspaceId: ID) {
    this.config.workspaces = this.config.workspaces.filter((workspace) => workspace.id !== workspaceId || workspace.isDefault);
    return Promise.resolve(this.ok(true));
  }

  listProjects() {
    return Promise.resolve(this.ok([...this.config.projects]));
  }

  createProject(input: ProjectFormInput) {
    const workspaceId = this.config.workspaces.find((workspace) => workspace.isDefault)?.id ?? defaultWorkspace.id;
    const project: Project = {
      id: id("project"),
      workspaceId,
      name: input.name,
      slug: input.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, ""),
      description: input.description,
      rootPath: input.rootPath,
      tags: input.tags,
      autoStart: input.autoStart,
      startupOrder: input.startupOrder,
      memoryLimitMb: input.memoryLimitMb,
      createdAt: now(),
      updatedAt: now()
    };
    this.config.projects.push(project);
    this.config.activity.unshift(activity("project_created", `${project.name} created`, "info", project.id));
    this.emit("project_status_changed", { projectId: project.id, status: "stopped" });
    return Promise.resolve(this.ok(project));
  }

  updateProject(project: Project) {
    this.config.projects = this.config.projects.map((item) => (item.id === project.id ? { ...project, updatedAt: now() } : item));
    this.config.activity.unshift(activity("project_updated", `${project.name} updated`, "info", project.id));
    return Promise.resolve(this.ok(project));
  }

  deleteProject(projectId: ID) {
    this.config.projects = this.config.projects.filter((project) => project.id !== projectId);
    const processIds = this.config.processes.filter((process) => process.projectId === projectId).map((process) => process.id);
    this.config.processes = this.config.processes.filter((process) => process.projectId !== projectId);
    processIds.forEach((processId) => this.runtime.delete(processId));
    this.config.activity.unshift(activity("project_deleted", "Project deleted", "warn", projectId));
    return Promise.resolve(this.ok(true));
  }

  getProjectDetail(projectId: ID): Promise<ApiResponse<ProjectDetail>> {
    const project = this.config.projects.find((item) => item.id === projectId);
    if (!project) return Promise.resolve(this.error("PROJECT_NOT_FOUND", "Project not found"));
    const processes = this.config.processes.filter((process) => process.projectId === projectId);
    const runtimeStates = processes.map((process) => this.runtime.get(process.id) ?? this.defaultRuntime(process.id));
    return Promise.resolve(
      this.ok({
        project,
        processes,
        runtimeStates,
        recentLogs: this.logs.filter((log) => log.projectId === projectId).slice(-250),
        status: deriveProjectStatus(runtimeStates)
      })
    );
  }

  listProcessesByProject(projectId: ID) {
    return Promise.resolve(this.ok(this.config.processes.filter((process) => process.projectId === projectId)));
  }

  createProcessDefinition(input: ProcessFormInput) {
    const validation = this.validateProcess(input);
    if (!validation.valid) return Promise.resolve(this.error("INVALID_PROCESS_DEFINITION", validation.errors.join(", ")));
    const process: ProcessDefinition = { ...input, id: id("process"), createdAt: now(), updatedAt: now() };
    this.config.processes.push(process);
    this.runtime.set(process.id, this.defaultRuntime(process.id));
    this.config.activity.unshift(activity("process_created", `${process.name} created`, "info", process.projectId, process.id));
    return Promise.resolve(this.ok(process));
  }

  updateProcessDefinition(process: ProcessDefinition) {
    const validation = this.validateProcess(process);
    if (!validation.valid) return Promise.resolve(this.error("INVALID_PROCESS_DEFINITION", validation.errors.join(", ")));
    this.config.processes = this.config.processes.map((item) => (item.id === process.id ? { ...process, updatedAt: now() } : item));
    this.config.activity.unshift(activity("process_updated", `${process.name} updated`, "info", process.projectId, process.id));
    return Promise.resolve(this.ok(process));
  }

  deleteProcessDefinition(processId: ID) {
    const process = this.config.processes.find((item) => item.id === processId);
    if (!process) return Promise.resolve(this.error("PROCESS_NOT_FOUND", "Process not found"));
    this.stopProcess(processId);
    this.config.processes = this.config.processes.filter((item) => item.id !== processId);
    this.runtime.delete(processId);
    this.config.activity.unshift(activity("process_deleted", `${process.name} deleted`, "warn", process.projectId, process.id));
    return Promise.resolve(this.ok(true));
  }

  startProcess(processId: ID): Promise<ApiResponse<ProcessRuntimeState>> {
    const process = this.config.processes.find((item) => item.id === processId);
    if (!process) return Promise.resolve(this.error("PROCESS_NOT_FOUND", "Process not found"));
    const current = this.runtime.get(processId);
    if (current?.currentStatus === "running" || current?.currentStatus === "starting") {
      return Promise.resolve(this.error("PROCESS_ALREADY_RUNNING", "Process is already running"));
    }
    const missingDependency = process.dependsOn.find((key) => {
      const dependency = this.config.processes.find((candidate) => candidate.projectId === process.projectId && candidate.key === key);
      return !dependency || this.runtime.get(dependency.id)?.currentStatus !== "running";
    });
    if (missingDependency) {
      const state = { ...this.defaultRuntime(processId), currentStatus: "waiting_dependency" as const, lastError: `Waiting for ${missingDependency}` };
      this.runtime.set(processId, state);
      this.emit("process_failed", state);
      return Promise.resolve(this.ok(state));
    }

    const starting = { ...this.defaultRuntime(processId), currentStatus: "starting" as const, startedAt: now() };
    this.runtime.set(processId, starting);
    this.emit("process_started", starting);
    this.pushSystemLog(process, "Starting process");

    window.setTimeout(() => {
      const running = { ...starting, currentStatus: "running" as const, pid: Math.floor(2000 + Math.random() * 8000), healthStatus: "healthy" as const };
      this.runtime.set(processId, running);
      this.emit("process_started", running);
      this.pushSystemLog(process, `Process running with pid ${running.pid}`);
      const interval = window.setInterval(() => {
        const current = this.runtime.get(processId);
        if (!current || current.currentStatus !== "running") return;
        const memoryUsage = this.nextMemoryUsage(process, current.memoryUsage);
        const next = { ...current, memoryUsage };
        this.runtime.set(processId, next);
        this.emit("process_metrics_changed", next);
        if (this.enforceMemoryLimits(process, memoryUsage)) return;
        const line = `${process.key}: ${new Date().toLocaleTimeString()} heartbeat ${Math.floor(Math.random() * 99)}`;
        this.pushLog(process, "stdout", "info", line);
      }, 1800 + Math.random() * 1000);
      this.intervals.set(processId, interval);
    }, 650);

    this.config.activity.unshift(activity("process_started", `${process.name} start requested`, "info", process.projectId, process.id));
    return Promise.resolve(this.ok(starting));
  }

  stopProcess(processId: ID): Promise<ApiResponse<ProcessRuntimeState>> {
    const process = this.config.processes.find((item) => item.id === processId);
    if (!process) return Promise.resolve(this.error("PROCESS_NOT_FOUND", "Process not found"));
    const existing = this.runtime.get(processId) ?? this.defaultRuntime(processId);
    const stopping = { ...existing, currentStatus: "stopping" as const };
    this.runtime.set(processId, stopping);
    this.emit("process_stopped", stopping);
    this.pushSystemLog(process, "Stopping process");

    window.setTimeout(() => {
      const interval = this.intervals.get(processId);
      if (interval) window.clearInterval(interval);
      this.intervals.delete(processId);
      const stopped = { ...stopping, currentStatus: "stopped" as const, stoppedAt: now(), pid: undefined, exitCode: 0, memoryUsage: undefined };
      this.runtime.set(processId, stopped);
      this.emit("process_stopped", stopped);
      this.pushSystemLog(process, "Process stopped");
    }, 500);

    this.config.activity.unshift(activity("process_stopped", `${process.name} stop requested`, "info", process.projectId, process.id));
    return Promise.resolve(this.ok(stopping));
  }

  async restartProcess(processId: ID) {
    const state = this.runtime.get(processId);
    if (state?.currentStatus === "running" || state?.currentStatus === "starting") await this.stopProcess(processId);
    await new Promise((resolve) => window.setTimeout(resolve, 700));
    const result = await this.startProcess(processId);
    if (result.success && result.data) {
      this.runtime.set(processId, { ...result.data, restartCount: (state?.restartCount ?? 0) + 1 });
    }
    return result;
  }

  async startProject(projectId: ID) {
    const processes = this.orderedProcesses(projectId);
    for (const process of processes.filter((item) => item.visible || item.autoStart)) {
      await this.startProcess(process.id);
      if (process.startupDelayMs) await new Promise((resolve) => window.setTimeout(resolve, process.startupDelayMs));
    }
    return this.getProjectDetail(projectId);
  }

  async startAutoStartProcesses(projectId: ID) {
    const processes = this.orderedProcesses(projectId);
    for (const process of processes.filter((item) => item.autoStart)) {
      await this.startProcess(process.id);
      if (process.startupDelayMs) await new Promise((resolve) => window.setTimeout(resolve, process.startupDelayMs));
    }
    return this.getProjectDetail(projectId);
  }

  async stopProject(projectId: ID) {
    const processes = this.orderedProcesses(projectId).reverse();
    for (const process of processes) await this.stopProcess(process.id);
    return this.getProjectDetail(projectId);
  }

  async restartProject(projectId: ID) {
    await this.stopProject(projectId);
    await new Promise((resolve) => window.setTimeout(resolve, 800));
    return this.startProject(projectId);
  }

  async restartFailedProcesses(projectId?: ID) {
    const processes = this.config.processes.filter((process) => !projectId || process.projectId === projectId);
    const failed = processes.filter((process) => FAILED_STATUSES.has(this.runtime.get(process.id)?.currentStatus ?? "idle"));
    for (const process of failed) await this.restartProcess(process.id);
    return this.ok(await this.getAllRuntimeStates().then((result) => result.data ?? []));
  }

  listExternalProjectProcesses(_projectId: ID) {
    return Promise.resolve(this.ok([]));
  }

  stopExternalProcess(_processGroupId: number) {
    return Promise.resolve(this.ok(true));
  }

  getRuntimeState(processId: ID) {
    return Promise.resolve(this.ok(this.runtime.get(processId) ?? this.defaultRuntime(processId)));
  }

  getAllRuntimeStates() {
    return Promise.resolve(this.ok([...this.runtime.values()]));
  }

  getLogHistory(filters: { projectId?: ID; processId?: ID; limit?: number; since?: string } = {}) {
    const limit = filters.limit ?? 1000;
    const since = filters.since ? Date.parse(filters.since) : undefined;
    return Promise.resolve(
      this.ok(
        this.logs
          .filter((log) => since === undefined || Date.parse(log.timestamp) >= since)
          .filter((log) => !filters.projectId || log.projectId === filters.projectId)
          .filter((log) => !filters.processId || log.processId === filters.processId)
          .slice(-limit)
      )
    );
  }

  clearLogHistory(projectId?: ID) {
    this.logs = this.logs.filter((log) => projectId && log.projectId !== projectId);
    return Promise.resolve(this.ok(true));
  }

  exportLogs() {
    return Promise.resolve(this.ok(JSON.stringify(this.logs, null, 2)));
  }

  runHealthCheck(processId: ID) {
    const current = this.runtime.get(processId) ?? this.defaultRuntime(processId);
    const next = { ...current, healthStatus: current.currentStatus === "running" ? ("healthy" as const) : ("unknown" as const) };
    this.runtime.set(processId, next);
    this.emit("process_health_changed", next);
    return Promise.resolve(this.ok(next));
  }

  getHealthSummary(projectId?: ID) {
    const processIds = new Set(this.config.processes.filter((process) => !projectId || process.projectId === projectId).map((process) => process.id));
    const states = [...this.runtime.values()].filter((state) => processIds.has(state.processId));
    return Promise.resolve(
      this.ok({
        healthy: states.filter((state) => state.healthStatus === "healthy").length,
        unhealthy: states.filter((state) => state.healthStatus === "unhealthy").length,
        unknown: states.filter((state) => !state.healthStatus || state.healthStatus === "unknown").length
      })
    );
  }

  openProjectFolderInFinder(_projectId: ID) {
    return Promise.resolve(this.ok(true));
  }

  revealLogFileInFinder() {
    return Promise.resolve(this.ok(true));
  }

  validateProjectPath(rootPath: string): Promise<ApiResponse<ValidationResult>> {
    return Promise.resolve(this.ok({ valid: rootPath.startsWith("/"), errors: rootPath.startsWith("/") ? [] : ["Use an absolute path"], warnings: [] }));
  }

  detectPortsInUse() {
    return Promise.resolve(this.ok([]));
  }

  getConfig() {
    return Promise.resolve(this.ok(this.config));
  }

  updateSettings(settings: AppSettings) {
    this.config.settings = settings;
    return Promise.resolve(this.ok(settings));
  }

  applyMediaGuardPreset(_basePath?: string) {
    this.config.activity.unshift(activity("config_imported", "MediaGuard project preset synced"));
    return Promise.resolve(this.ok(this.config));
  }

  importConfig(config: AppConfig) {
    this.config = config;
    this.runtime.clear();
    config.processes.forEach((process) => this.runtime.set(process.id, this.defaultRuntime(process.id)));
    this.config.activity.unshift(activity("config_imported", "Configuration imported"));
    return Promise.resolve(this.ok(this.config));
  }

  exportConfig(redactSecrets = true) {
    const processes = this.config.processes.map((process) => ({
      ...process,
      env: redactSecrets
        ? Object.fromEntries(Object.entries(process.env).map(([key, value]) => [key, /(token|secret|password|key)/i.test(key) ? "REDACTED" : value]))
        : process.env
    }));
    return Promise.resolve(this.ok(JSON.stringify({ ...this.config, processes }, null, 2)));
  }

  getDashboardSummary(): Promise<ApiResponse<DashboardSummary>> {
    const states = [...this.runtime.values()];
    return Promise.resolve(
      this.ok({
        projectCount: this.config.projects.length,
        processCount: this.config.processes.length,
        runningProcessCount: states.filter((state) => state.currentStatus === "running").length,
        failedProcessCount: states.filter((state) => FAILED_STATUSES.has(state.currentStatus)).length,
        portConflictCount: 0,
        autoStartProjectCount: this.config.projects.filter((project) => project.autoStart).length,
        recentProblemLogs: this.logs.filter((log) => log.level === "warn" || log.level === "error").slice(-12)
      })
    );
  }

  private orderedProcesses(projectId: ID): ProcessDefinition[] {
    const processes = this.config.processes.filter((process) => process.projectId === projectId);
    const byKey = new Map(processes.map((process) => [process.key, process]));
    const visited = new Set<ID>();
    const output: ProcessDefinition[] = [];
    const visit = (process: ProcessDefinition) => {
      if (visited.has(process.id)) return;
      process.dependsOn.forEach((key) => {
        const dependency = byKey.get(key);
        if (dependency) visit(dependency);
      });
      visited.add(process.id);
      output.push(process);
    };
    processes.forEach(visit);
    return output;
  }

  private validateProcess(input: Pick<ProcessDefinition, "projectId" | "id" | "key" | "command" | "dependsOn"> | ProcessFormInput): ValidationResult {
    const errors: string[] = [];
    const processes = this.config.processes.filter((process) => process.projectId === input.projectId);
    const currentId = "id" in input ? input.id : undefined;
    if (!input.command.trim()) errors.push("Command is required");
    if (!input.key.trim()) errors.push("Key is required");
    if (processes.some((process) => process.key === input.key && process.id !== currentId)) errors.push("Process key must be unique in the project");
    const keys = new Set(processes.map((process) => process.key));
    input.dependsOn.forEach((key) => {
      if (!keys.has(key)) errors.push(`Unknown dependency: ${key}`);
    });
    return { valid: errors.length === 0, errors, warnings: [] };
  }

  private defaultRuntime(processId: ID): ProcessRuntimeState {
    return { processId, restartCount: 0, healthStatus: "unknown", portBindings: [], currentStatus: "stopped" };
  }

  private nextMemoryUsage(process: ProcessDefinition, current?: number) {
    const baseMb = process.memoryLimitMb ? Math.max(48, process.memoryLimitMb * 0.42) : 96;
    const currentMb = current ? current / 1024 / 1024 : baseMb;
    const driftMb = Math.round((Math.random() - 0.35) * 28);
    const nextMb = Math.max(24, currentMb + driftMb);
    return Math.round(nextMb * 1024 * 1024);
  }

  private enforceMemoryLimits(process: ProcessDefinition, memoryUsage: number) {
    if (process.memoryLimitMb && memoryUsage > process.memoryLimitMb * 1024 * 1024) {
      this.failForMemoryLimit(process, `Process memory limit exceeded: ${Math.round(memoryUsage / 1024 / 1024)} MB used over ${process.memoryLimitMb} MB limit`);
      return true;
    }
    const project = this.config.projects.find((item) => item.id === process.projectId);
    if (!project?.memoryLimitMb) return false;
    const processIds = new Set(this.config.processes.filter((item) => item.projectId === project.id).map((item) => item.id));
    const projectUsage = [...this.runtime.values()]
      .filter((state) => processIds.has(state.processId) && state.currentStatus === "running")
      .reduce((total, state) => total + (state.memoryUsage ?? 0), 0);
    if (projectUsage <= project.memoryLimitMb * 1024 * 1024) return false;
    const detail = `Project memory limit exceeded: ${Math.round(projectUsage / 1024 / 1024)} MB used over ${project.memoryLimitMb} MB limit`;
    this.config.processes.filter((item) => item.projectId === project.id).forEach((item) => this.failForMemoryLimit(item, detail));
    return true;
  }

  private failForMemoryLimit(process: ProcessDefinition, detail: string) {
    const interval = this.intervals.get(process.id);
    if (interval) window.clearInterval(interval);
    this.intervals.delete(process.id);
    const current = this.runtime.get(process.id) ?? this.defaultRuntime(process.id);
    const failed = { ...current, currentStatus: "failed" as const, stoppedAt: now(), pid: undefined, lastError: detail, healthStatus: "unknown" as const };
    this.runtime.set(process.id, failed);
    this.pushLog(process, "system", "error", detail);
    this.emit("process_failed", failed);
  }

  private pushSystemLog(process: ProcessDefinition, message: string) {
    this.pushLog(process, "system", "info", message);
  }

  private pushLog(process: ProcessDefinition, stream: LogEntry["stream"], level: LogEntry["level"], message: string) {
    const entry: LogEntry = { id: id("log"), processId: process.id, projectId: process.projectId, timestamp: now(), stream, level, message, raw: message };
    this.logs.push(entry);
    const retention = this.config.settings.logRetentionLines;
    if (this.logs.length > retention) this.logs = this.logs.slice(-retention);
    this.emit("process_log", entry);
  }
}

export const mockApi = new MockApi();
