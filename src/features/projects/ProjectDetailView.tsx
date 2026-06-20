import { AlertTriangle, Check, Copy, Edit3, FolderOpen, Play, Plus, RefreshCw, RotateCcw, Square, Trash2, X } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { StatusBadge } from "../../components/StatusBadge";
import { useConfirm } from "../../components/ConfirmDialog";
import { RuntimeDot } from "../../components/RuntimeDot";
import { api } from "../../lib/api";
import { formatMemory, formatMemoryLimit, normalizeMemoryLimitMb, parseMemoryLimitInput } from "../../lib/memory";
import { ensureNotificationPermission } from "../../lib/notifications";
import { fileManagerName, revealLabel } from "../../lib/platform";
import { isRuntimeBusy } from "../../lib/status";
import { envToText, formatPath, formatRelativeTime, normalizeCliText, parseEnvInput, parseListInput } from "../../lib/time";
import { useOrchestratorStore } from "../../stores/orchestratorStore";
import type { ExternalProcess, ProcessDefinition, ProcessFormInput, ProcessRuntimeState } from "../../types/domain";
import { DeploySection } from "./DeploySection";

const technicalInputProps = {
  autoCapitalize: "off",
  autoCorrect: "off",
  spellCheck: false
} as const;

function defaultProcess(projectId: string): ProcessFormInput {
  return {
    projectId,
    name: "",
    key: "",
    command: "",
    args: [],
    workingDirectory: "",
    env: {},
    memoryLimitMb: undefined,
    autoStart: true,
    restartPolicy: { kind: "never" },
    dependsOn: [],
    healthCheck: { kind: "none" },
    logMode: "combined",
    group: "",
    visible: true
  };
}

export function ProjectDetailView() {
  const selectedProjectId = useOrchestratorStore((state) => state.selectedProjectId);
  const projects = useOrchestratorStore((state) => state.projects);
  const processes = useOrchestratorStore((state) => state.processes);
  const runtimeStates = useOrchestratorStore((state) => state.runtimeStates);
  const selectProcess = useOrchestratorStore((state) => state.selectProcess);
  const updateProject = useOrchestratorStore((state) => state.updateProject);
  const createProcess = useOrchestratorStore((state) => state.createProcess);
  const updateProcess = useOrchestratorStore((state) => state.updateProcess);
  const deleteProcess = useOrchestratorStore((state) => state.deleteProcess);
  const deleteProject = useOrchestratorStore((state) => state.deleteProject);
  const startProcess = useOrchestratorStore((state) => state.startProcess);
  const stopProcess = useOrchestratorStore((state) => state.stopProcess);
  const restartProcess = useOrchestratorStore((state) => state.restartProcess);
  const startProject = useOrchestratorStore((state) => state.startProject);
  const startAutoStartProcesses = useOrchestratorStore((state) => state.startAutoStartProcesses);
  const stopProject = useOrchestratorStore((state) => state.stopProject);
  const restartProject = useOrchestratorStore((state) => state.restartProject);
  const settings = useOrchestratorStore((state) => state.settings);
  const updateSettings = useOrchestratorStore((state) => state.updateSettings);
  const externalProcesses = useOrchestratorStore((state) => state.externalProcesses);
  const loadExternalProcesses = useOrchestratorStore((state) => state.loadExternalProcesses);
  const stopExternalProcess = useOrchestratorStore((state) => state.stopExternalProcess);
  const logs = useOrchestratorStore((state) => state.logs);
  const machines = useOrchestratorStore((state) => state.machines);
  const confirm = useConfirm();
  const [formOpen, setFormOpen] = useState(false);
  const [draft, setDraft] = useState<ProcessFormInput | null>(null);
  const [processFormError, setProcessFormError] = useState<string>();
  const [editingProjectName, setEditingProjectName] = useState(false);
  const [projectNameDraft, setProjectNameDraft] = useState("");
  const [iconDraft, setIconDraft] = useState("");
  const [externalDetailsKey, setExternalDetailsKey] = useState<number | null>(null);

  const project = useMemo(() => projects.find((item) => item.id === selectedProjectId), [projects, selectedProjectId]);
  const projectProcesses = useMemo(() => processes.filter((process) => process.projectId === selectedProjectId), [processes, selectedProjectId]);
  const projectExternals = selectedProjectId ? externalProcesses[selectedProjectId] ?? [] : [];
  const conflictingPorts = useMemo(() => {
    if (!selectedProjectId) return [] as number[];
    const ports = new Set<number>();
    for (const log of logs) {
      if (log.projectId !== selectedProjectId) continue;
      const port = extractConflictingPort(log.message);
      if (port) ports.add(port);
    }
    return [...ports];
  }, [logs, selectedProjectId]);
  const [portHolders, setPortHolders] = useState<Record<number, ExternalProcess | null | "loading">>({});
  const fetchedPortsRef = useRef<Set<number>>(new Set());

  useEffect(() => {
    setPortHolders({});
    fetchedPortsRef.current.clear();
  }, [selectedProjectId]);

  useEffect(() => {
    let cancelled = false;
    for (const port of conflictingPorts) {
      if (fetchedPortsRef.current.has(port)) continue;
      fetchedPortsRef.current.add(port);
      setPortHolders((prev) => ({ ...prev, [port]: "loading" }));
      api.findProcessOnPort(port).then((response) => {
        if (cancelled) return;
        const holder = response.success ? response.data ?? null : null;
        setPortHolders((prev) => ({ ...prev, [port]: holder }));
      });
    }
    return () => {
      cancelled = true;
    };
  }, [conflictingPorts]);

  const refreshPortHolder = async (port: number) => {
    setPortHolders((prev) => ({ ...prev, [port]: "loading" }));
    const response = await api.findProcessOnPort(port);
    const holder = response.success ? response.data ?? null : null;
    setPortHolders((prev) => ({ ...prev, [port]: holder }));
  };
  const processStates = useMemo(
    () => projectProcesses.map((process) => runtimeStates[process.id]).filter((state): state is ProcessRuntimeState => Boolean(state)),
    [projectProcesses, runtimeStates]
  );
  const runningCount = processStates.filter((state) => state.currentStatus === "running").length;
  const projectMemoryUsage = processStates.reduce((total, state) => total + (state.memoryUsage ?? 0), 0);

  useEffect(() => {
    if (!project) return;
    setProjectNameDraft(project.name);
    setIconDraft(project.icon ?? project.name.slice(0, 2).toUpperCase());
    setEditingProjectName(false);
  }, [project?.id, project?.name, project?.icon]);

  if (!project) {
    return (
      <main className="empty-state">
        <FolderOpen size={24} />
        <span>Select a project from the sidebar.</span>
      </main>
    );
  }

  const submitProcess = async () => {
    if (!draft) return;

    const name = draft.name.trim();
    const key = draft.key.trim();
    const command = draft.command.trim();

    if (!name || !key || !command) {
      setProcessFormError("Name, Key, and Command fields are required.");
      return;
    }

    if (projectProcesses.some((process) => process.key === key)) {
      setProcessFormError(`Process key "${key}" already exists in this project.`);
      return;
    }

    const existingKeys = new Set(projectProcesses.map((process) => process.key));
    const missingDependency = draft.dependsOn.find((dependency) => !existingKeys.has(dependency));
    if (missingDependency) {
      setProcessFormError(`Dependency "${missingDependency}" does not exist in this project.`);
      return;
    }

    setProcessFormError(undefined);
    const created = await createProcess({
      ...draft,
      name,
      key,
      command,
      memoryLimitMb: normalizeMemoryLimitMb(draft.memoryLimitMb),
      workingDirectory: draft.workingDirectory?.trim() || undefined,
      group: draft.group?.trim() || undefined
    });

    if (created) {
      setFormOpen(false);
      setDraft(null);
      return;
    }

    setProcessFormError("Process could not be created. Check the app error message for details.");
  };

  const copyProjectPath = async () => {
    await navigator.clipboard.writeText(project.rootPath);
  };

  const saveProjectName = async () => {
    const name = projectNameDraft.trim();
    if (!name || name === project.name) {
      setProjectNameDraft(project.name);
      setEditingProjectName(false);
      return;
    }
    await updateProject({ ...project, name });
    setEditingProjectName(false);
  };

  const patchNotifications = async (notificationsEnabled: boolean) => {
    if (!settings) return;
    if (notificationsEnabled && !(await ensureNotificationPermission())) return;
    await updateSettings({ ...settings, notificationsEnabled });
  };

  const commitIcon = async () => {
    const icon = iconDraft.trim().slice(0, 4);
    setIconDraft(icon || project.name.slice(0, 2).toUpperCase());
    await updateProject({ ...project, icon: icon || undefined });
  };

  return (
    <main className="solo-project-show">
      <header className="solo-project-topbar">
        <div className="solo-project-title">
          {editingProjectName ? (
            <span className="solo-project-name-editor">
              <input
                value={projectNameDraft}
                autoFocus
                onChange={(event) => setProjectNameDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void saveProjectName();
                  if (event.key === "Escape") {
                    setProjectNameDraft(project.name);
                    setEditingProjectName(false);
                  }
                }}
              />
              <button type="button" onClick={saveProjectName} title="Save project name">
                <Check size={13} />
              </button>
              <button
                type="button"
                onClick={() => {
                  setProjectNameDraft(project.name);
                  setEditingProjectName(false);
                }}
                title="Cancel"
              >
                <X size={13} />
              </button>
            </span>
          ) : (
            <>
              <h2>{project.name}</h2>
              <button type="button" onClick={() => setEditingProjectName(true)} title="Edit project name">
                <Edit3 size={13} />
              </button>
            </>
          )}
          <span className={`solo-running-pill ${runningCount > 0 ? "active" : "idle"}`}>
            <span />
            {runningCount}/{projectProcesses.length} Running
          </span>
        </div>
        <div className="solo-project-actions">
          <button type="button" onClick={() => startAutoStartProcesses(project.id)} disabled={!projectProcesses.some((process) => process.autoStart)}>
            <Play size={14} />
            Start auto-starting
          </button>
          <button type="button" onClick={() => startProject(project.id)}>
            <Play size={14} />
            Start all
          </button>
          <button type="button" onClick={() => restartProject(project.id)} title="Restart all">
            <RotateCcw size={14} />
          </button>
          <button type="button" onClick={async () => {
            if (await confirm({ title: "Stop all processes in this project?", confirmLabel: "Stop all", danger: true })) {
              void stopProject(project.id);
            }
          }} title="Stop all">
            <Square size={14} />
          </button>
        </div>
      </header>

      <div className="solo-project-scroll">
        <div className="solo-project-column">
          <SoloSection title="Overview">
            <div className="solo-detail-card">
              <SoloDetailRow
                title="Directory"
                subtitle={formatPath(project.rootPath)}
                actions={
                  <>
                    <button type="button" onClick={copyProjectPath} title="Copy directory">
                      <Copy size={14} />
                    </button>
                    <button type="button" onClick={() => api.openProjectFolderInFinder(project.id)} title="Open directory">
                      <FolderOpen size={14} />
                    </button>
                  </>
                }
              />
              <SoloDetailRow
                title="Config"
                subtitle="Project configuration file"
                value="None"
                actions={
                  <button type="button" onClick={() => setFormOpen(true)}>
                    Create process
                  </button>
                }
              />
              <SoloDetailRow title="Commands" subtitle="Running and total command count" value={`${runningCount} Running  -  ${projectProcesses.length} Total`} tone={runningCount ? "good" : "neutral"} />
              <SoloDetailRow title="RAM" subtitle="Current combined process memory" value={project.memoryLimitMb ? `${formatMemory(projectMemoryUsage)} / ${formatMemoryLimit(project.memoryLimitMb)}` : formatMemory(projectMemoryUsage)} />
            </div>
          </SoloSection>

          <SoloSection title="Settings" variant="settings">
            <div className="solo-detail-card">
              <SoloDetailRow
                title="Auto Start"
                subtitle="Auto-start commands will run when this app launches"
                actions={<SoloSwitch checked={project.autoStart} onChange={(autoStart) => updateProject({ ...project, autoStart })} />}
              />
              <SoloDetailRow
                title="Project RAM cap"
                subtitle="Stop project commands when combined usage crosses this cap"
                value={formatMemoryLimit(project.memoryLimitMb)}
                actions={<MemoryLimitInput value={project.memoryLimitMb} onCommit={(memoryLimitMb) => updateProject({ ...project, memoryLimitMb })} />}
              />
              <SoloDetailRow
                title="Icon"
                subtitle="Display a small icon next to the project name"
                actions={
                  <span className="solo-icon-control">
                    <span className="solo-mark" style={{ backgroundColor: project.color ?? undefined }}>
                      {(project.icon ?? project.name.slice(0, 2)).slice(0, 4).toUpperCase()}
                    </span>
                    <input
                      className="solo-icon-input"
                      value={iconDraft}
                      maxLength={4}
                      onChange={(event) => setIconDraft(event.target.value)}
                      onBlur={commitIcon}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") event.currentTarget.blur();
                      }}
                      title="Project icon text"
                    />
                    <input
                      className="solo-color-input"
                      type="color"
                      value={project.color ?? "#32d583"}
                      onChange={(event) => updateProject({ ...project, color: event.target.value })}
                      title="Project color"
                    />
                  </span>
                }
              />
            </div>
          </SoloSection>

          <SoloSection title="Notifications" variant="settings">
            <div className="solo-detail-card">
              <SoloDetailRow
                title="Crash & exit alerts"
                subtitle="Get notified when commands crash or exit unexpectedly"
                actions={<SoloSwitch checked={settings?.notificationsEnabled ?? false} onChange={patchNotifications} />}
              />
              <SoloDetailRow
                title="Health check alerts"
                subtitle="Use the global notification setting for degraded commands"
                actions={<SoloSwitch checked={settings?.notificationsEnabled ?? false} onChange={patchNotifications} />}
              />
            </div>
          </SoloSection>

          {conflictingPorts.length ? (
            <SoloSection title="Port conflicts">
              <div className="solo-detail-card">
                {conflictingPorts.map((port) => {
                  const holder = portHolders[port];
                  return (
                    <PortConflictRow
                      key={port}
                      port={port}
                      holder={holder}
                      onRefresh={() => void refreshPortHolder(port)}
                      onStop={async () => {
                        if (!holder || holder === "loading") return;
                        const ok = await confirm({
                          title: `Stop process holding port ${port}?`,
                          message: `${holder.command} (pid ${holder.pid})\n${holder.cwd || ""}`,
                          confirmLabel: "Stop",
                          danger: true,
                        });
                        if (!ok) return;
                        await stopExternalProcess(project.id, holder.processGroupId);
                        void refreshPortHolder(port);
                      }}
                    />
                  );
                })}
              </div>
            </SoloSection>
          ) : null}

          <SoloSection
            title="Commands"
            action={
              <button
                type="button"
                onClick={() => {
                  setDraft(defaultProcess(project.id));
                  setProcessFormError(undefined);
                  setFormOpen(!formOpen);
                }}
              >
                <Plus size={14} />
                Add command
              </button>
            }
          >
            {formOpen && draft ? (
              <div className="editor-panel inline solo-command-editor">
                <div className="form-grid">
                  <label>
                    Name<span className="required-marker" aria-hidden="true">*</span>
                    <input required value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="Vite dev server" />
                  </label>
                  <label>
                    Key<span className="required-marker" aria-hidden="true">*</span>
                    <input required {...technicalInputProps} value={draft.key} onChange={(event) => setDraft({ ...draft, key: event.target.value })} placeholder="vite" />
                  </label>
                  <label>
                    Command<span className="required-marker" aria-hidden="true">*</span>
                    <input required {...technicalInputProps} value={draft.command} onChange={(event) => setDraft({ ...draft, command: normalizeCliText(event.target.value) })} placeholder="npm" />
                  </label>
                  <label>
                    Args
                    <input {...technicalInputProps} value={draft.args.join(", ")} onChange={(event) => setDraft({ ...draft, args: parseListInput(event.target.value) })} placeholder="run, dev" />
                  </label>
                  <label>
                    Working directory
                    <input {...technicalInputProps} value={draft.workingDirectory} onChange={(event) => setDraft({ ...draft, workingDirectory: event.target.value })} placeholder={project.rootPath} />
                  </label>
                  <label>
                    Depends on keys
                    <input {...technicalInputProps} value={draft.dependsOn.join(", ")} onChange={(event) => setDraft({ ...draft, dependsOn: parseListInput(event.target.value) })} placeholder="api, redis" />
                  </label>
                  <label>
                    Group
                    <input {...technicalInputProps} value={draft.group} onChange={(event) => setDraft({ ...draft, group: event.target.value })} placeholder="workers" />
                  </label>
                  <label>
                    Env
                    <textarea {...technicalInputProps} value={envToText(draft.env)} onChange={(event) => setDraft({ ...draft, env: parseEnvInput(event.target.value) })} placeholder="NODE_ENV=development" />
                  </label>
                  <label>
                    Memory limit (MB)
                    <input
                      type="number"
                      min={64}
                      step={64}
                      value={draft.memoryLimitMb ?? ""}
                      onChange={(event) => setDraft({ ...draft, memoryLimitMb: normalizeMemoryLimitMb(event.target.valueAsNumber) })}
                      placeholder="Off"
                    />
                  </label>
                  <label>
                    Machine
                    <select
                      value={draft.machineId ?? ""}
                      onChange={(event) => setDraft({ ...draft, machineId: event.target.value || undefined })}
                    >
                      <option value="">{machines.find((machine) => machine.isDefaultLocal)?.name ?? "Local"} (local)</option>
                      {machines.filter((machine) => !machine.isDefaultLocal).map((machine) => (
                        <option key={machine.id} value={machine.id}>
                          {machine.name} ({machine.sshUser}@{machine.hostname})
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="checkbox-line">
                    <input type="checkbox" checked={draft.autoStart} onChange={(event) => setDraft({ ...draft, autoStart: event.target.checked })} />
                    Include in project start
                  </label>
                </div>
                {processFormError ? <div className="form-error">{processFormError}</div> : null}
                <div className="editor-actions">
                  <button type="button" onClick={submitProcess}>
                    Create process
                  </button>
                </div>
              </div>
            ) : null}

            <div className="solo-detail-card">
              {projectProcesses.map((process) => {
                const machine = process.machineId ? machines.find((item) => item.id === process.machineId) : undefined;
                const machineLabel = machine && !machine.isDefaultLocal ? machine.name : undefined;
                return (
                  <CommandRow
                    key={process.id}
                    process={process}
                    runtime={runtimeStates[process.id]}
                    machineLabel={machineLabel}
                    onSelect={() => selectProcess(process.id)}
                    onStart={() => startProcess(process.id)}
                    onStop={() => stopProcess(process.id)}
                    onRestart={() => restartProcess(process.id)}
                    onMemoryLimitCommit={(memoryLimitMb) => updateProcess({ ...process, memoryLimitMb })}
                    onDelete={async () => {
                      const ok = await confirm({
                        title: `Delete ${process.name}?`,
                        message: "The process definition will be removed. A running instance will be stopped.",
                        confirmLabel: "Delete",
                        danger: true,
                      });
                      if (ok) void deleteProcess(process.id);
                    }}
                  />
                );
              })}
              {!projectProcesses.length ? <p className="solo-empty-row">No commands configured.</p> : null}
            </div>
          </SoloSection>

          <DeploySection project={project} />

          <SoloSection
            title="Other processes in this project"
            action={
              <button
                type="button"
                onClick={() => void loadExternalProcesses(project.id)}
                title="Refresh external process list"
              >
                <RefreshCw size={14} />
                Refresh
              </button>
            }
          >
            <div className="solo-detail-card">
              {projectExternals.length ? (
                projectExternals.map((external) => (
                  <ExternalProcessRow
                    key={external.processGroupId}
                    external={external}
                    onOpen={() => setExternalDetailsKey(external.processGroupId)}
                    onStop={async () => {
                      const ok = await confirm({
                        title: `Stop process ${external.pid}?`,
                        message: `This was started outside the app. Sends SIGTERM, then SIGKILL after the stop timeout.\n\n${external.command}`,
                        confirmLabel: "Stop",
                        danger: true,
                      });
                      if (ok) void stopExternalProcess(project.id, external.processGroupId);
                    }}
                  />
                ))
              ) : (
                <p className="solo-empty-row">No untracked processes running in this project.</p>
              )}
            </div>
          </SoloSection>
          {(() => {
            const open = projectExternals.find((p) => p.processGroupId === externalDetailsKey);
            if (!open) return null;
            return (
              <ExternalProcessDetailsModal
                external={open}
                onClose={() => setExternalDetailsKey(null)}
                onStop={async () => {
                  const ok = await confirm({
                    title: `Stop process ${open.pid}?`,
                    message: `This was started outside the app. Sends SIGTERM, then SIGKILL after the stop timeout.\n\n${open.command}`,
                    confirmLabel: "Stop",
                    danger: true,
                  });
                  if (ok) {
                    setExternalDetailsKey(null);
                    void stopExternalProcess(project.id, open.processGroupId);
                  }
                }}
              />
            );
          })()}

          <button
            className="solo-remove-project"
            type="button"
            onClick={async () => {
              const ok = await confirm({
                title: `Delete ${project.name}?`,
                message: "This removes the project and all of its process definitions. Running processes will be stopped.",
                confirmLabel: "Delete",
                danger: true,
              });
              if (ok) void deleteProject(project.id);
            }}
          >
            <Trash2 size={14} />
            Remove project
          </button>
        </div>
      </div>
    </main>
  );
}

function SoloSection({ title, action, children, variant = "default" }: { title: string; action?: ReactNode; children: ReactNode; variant?: "default" | "settings" }) {
  return (
    <section className={variant === "settings" ? "solo-detail-section settings-like" : "solo-detail-section"}>
      <div className="solo-detail-section-heading">
        <span>{title}</span>
        {action}
      </div>
      {children}
    </section>
  );
}

function SoloDetailRow({
  title,
  subtitle,
  value,
  tone = "neutral",
  actions
}: {
  title: string;
  subtitle: string;
  value?: string;
  tone?: "neutral" | "good";
  actions?: ReactNode;
}) {
  return (
    <div className="solo-detail-row">
      <span className="solo-detail-row-copy">
        <strong>{title}</strong>
        <small>{subtitle}</small>
      </span>
      {value ? <span className={`solo-detail-value ${tone}`}>{value}</span> : null}
      {actions ? <span className="solo-detail-actions">{actions}</span> : null}
    </div>
  );
}

function CommandRow({
  process,
  runtime,
  machineLabel,
  onSelect,
  onStart,
  onStop,
  onRestart,
  onMemoryLimitCommit,
  onDelete
}: {
  process: ProcessDefinition;
  runtime?: ProcessRuntimeState;
  machineLabel?: string;
  onSelect: () => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onMemoryLimitCommit: (memoryLimitMb?: number) => void;
  onDelete: () => void;
}) {
  return (
    <div className="solo-command-row">
      <button type="button" className="solo-command-main" onClick={onSelect}>
        <RuntimeDot status={runtime?.currentStatus} />
        <span>
          <strong>{process.name}</strong>
          <small>
            {formatProcessCommand(process)}
            {machineLabel ? <span className="process-machine-badge">{machineLabel}</span> : null}
          </small>
        </span>
      </button>
      <StatusBadge status={runtime?.currentStatus ?? "stopped"} />
      <MemoryLimitInput value={process.memoryLimitMb} onCommit={onMemoryLimitCommit} compact />
      <span className="solo-command-meta">
        {runtime?.memoryUsage
          ? formatMemory(runtime.memoryUsage)
          : runtime?.pid
            ? `pid ${runtime.pid}`
            : runtime?.startedAt
              ? formatRelativeTime(runtime.startedAt)
              : process.autoStart
                ? "auto"
                : "manual"}
      </span>
      <span className="solo-command-actions">
        <button disabled={isRuntimeBusy(runtime?.currentStatus)} type="button" onClick={onStart} title="Start">
          <Play size={14} />
        </button>
        <button disabled={isRuntimeBusy(runtime?.currentStatus)} type="button" onClick={onStop} title="Stop">
          <Square size={14} />
        </button>
        <button disabled={isRuntimeBusy(runtime?.currentStatus)} type="button" onClick={onRestart} title="Restart">
          <RotateCcw size={14} />
        </button>
        <button type="button" onClick={onDelete} title="Delete">
          <Trash2 size={14} />
        </button>
      </span>
    </div>
  );
}

function MemoryLimitInput({
  value,
  onCommit,
  compact = false
}: {
  value?: number;
  onCommit: (memoryLimitMb?: number) => void;
  compact?: boolean;
}) {
  const [draft, setDraft] = useState(value ? String(value) : "");

  useEffect(() => {
    setDraft(value ? String(value) : "");
  }, [value]);

  const commit = () => {
    const next = parseMemoryLimitInput(draft);
    setDraft(next ? String(next) : "");
    if (next !== value) onCommit(next);
  };

  return (
    <label className={`memory-limit-field ${compact ? "compact" : ""}`}>
      <span>RAM</span>
      <input
        type="number"
        min={64}
        step={64}
        value={draft}
        onBlur={commit}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
        }}
        placeholder="Off"
      />
    </label>
  );
}

function SoloSwitch({ checked, onChange }: { checked: boolean; onChange: (checked: boolean) => void }) {
  return (
    <label className={`solo-switch ${checked ? "checked" : ""}`}>
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span />
    </label>
  );
}

function formatProcessCommand(process: ProcessDefinition) {
  return [process.command, ...process.args].filter(Boolean).join(" ") || process.key;
}

function extractConflictingPort(message: string): number | null {
  if (!/address already in use|EADDRINUSE/i.test(message)) return null;
  const tuple = message.match(/,\s*(\d{2,5})\s*\)/);
  if (tuple) {
    const port = Number(tuple[1]);
    if (port > 0 && port < 65536) return port;
  }
  const colon = message.match(/:(\d{2,5})\b/);
  if (colon) {
    const port = Number(colon[1]);
    if (port > 0 && port < 65536) return port;
  }
  return null;
}

function PortConflictRow({
  port,
  holder,
  onStop,
  onRefresh
}: {
  port: number;
  holder: ExternalProcess | null | "loading" | undefined;
  onStop: () => void;
  onRefresh: () => void;
}) {
  return (
    <div className="solo-command-row">
      <span className="solo-command-main" style={{ cursor: "default" }}>
        <AlertTriangle size={14} />
        <span>
          <strong>Port {port}</strong>
          <small>
            {holder === "loading" || holder === undefined
              ? "Looking up holder…"
              : holder === null
                ? "No process found (already free)"
                : `${holder.command || "?"} — pid ${holder.pid}${holder.cwd ? ` — ${formatPath(holder.cwd)}` : ""}`}
          </small>
        </span>
      </span>
      <span className="solo-command-actions">
        <button type="button" onClick={onRefresh} title="Re-check">
          <RefreshCw size={14} />
        </button>
        <button type="button" onClick={onStop} disabled={!holder || holder === "loading"} title="Stop holder">
          <Square size={14} />
        </button>
      </span>
    </div>
  );
}

function ExternalProcessRow({
  external,
  onOpen,
  onStop,
}: {
  external: ExternalProcess;
  onOpen: () => void;
  onStop: () => void;
}) {
  return (
    <div
      className="solo-command-row solo-command-row-clickable"
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen();
        }
      }}
      title="Show full process details"
    >
      <span className="solo-command-main">
        <RuntimeDot status="running" />
        <span>
          <strong>{external.command || `pid ${external.pid}`}</strong>
          <small>{formatPath(external.cwd)}</small>
        </span>
      </span>
      <span className="solo-command-meta">pid {external.pid}</span>
      <span className="solo-command-actions">
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onStop();
          }}
          title="Stop"
        >
          <Square size={14} />
        </button>
      </span>
    </div>
  );
}

function ExternalProcessDetailsModal({
  external,
  onClose,
  onStop,
}: {
  external: ExternalProcess;
  onClose: () => void;
  onStop: () => void;
}) {
  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 1500);
    return () => clearTimeout(timer);
  }, [toast]);

  const flash = (message: string) => setToast(message);

  const copy = async (value: string, label: string) => {
    try {
      await navigator.clipboard.writeText(value);
      flash(`${label} copied`);
    } catch {
      flash("Copy failed");
    }
  };

  const revealInFinder = async () => {
    try {
      await api.openPathInFinder(external.cwd);
    } catch {
      flash(`Could not open in ${fileManagerName}`);
    }
  };

  const hasPorts = Array.isArray(external.ports) && external.ports.length > 0;
  const hasChildren = Array.isArray(external.children) && external.children.length > 0;
  const childList = hasChildren ? external.children! : [];
  const portList = hasPorts ? external.ports! : [];
  const commandTitle =
    (typeof external.command === "string" && external.command.trim().split(/\s+/)[0]) ||
    `pid ${external.pid}`;
  const memoryMb =
    external.memoryKb && external.memoryKb > 0
      ? `${(external.memoryKb / 1024).toFixed(1)} MB`
      : "—";
  const cpu =
    typeof external.cpuPercent === "number" ? `${external.cpuPercent.toFixed(1)}%` : "—";

  return (
    <div
      className="external-process-modal-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="external-process-modal-title"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="external-process-modal-panel">
        <div className="external-process-modal-header">
          <RuntimeDot status="running" />
          <div className="external-process-modal-heading">
            <h2 id="external-process-modal-title">{commandTitle}</h2>
            <span className="external-process-modal-subtitle">pid {external.pid} · pgid {external.processGroupId}</span>
          </div>
          <button
            type="button"
            className="external-process-modal-close"
            onClick={onClose}
            title="Close"
            aria-label="Close"
          >
            <X size={16} />
          </button>
        </div>

        <div className="external-process-modal-body">
          <section className="external-process-modal-section">
            <h3>Process</h3>
            <pre className="external-process-modal-command">{external.command || "—"}</pre>
            <dl className="external-process-modal-grid">
              <dt>User</dt>
              <dd>{external.user || "—"}</dd>
              <dt>Started</dt>
              <dd>{external.startedAt || "—"}</dd>
              <dt>Uptime</dt>
              <dd>{external.etime || "—"}</dd>
              <dt>CPU</dt>
              <dd>{cpu}</dd>
              <dt>Memory</dt>
              <dd>{memoryMb}</dd>
            </dl>
          </section>

          <section className="external-process-modal-section">
            <h3>Network</h3>
            {hasPorts ? (
              <ul className="external-process-port-list">
                {portList.map((port) => (
                  <li key={port} className="external-process-port-chip">{port}</li>
                ))}
              </ul>
            ) : (
              <p className="external-process-modal-empty">No listening TCP ports.</p>
            )}
          </section>

          <section className="external-process-modal-section">
            <h3>Working directory</h3>
            <div className="external-process-modal-cwd-row">
              <code className="external-process-modal-cwd">{external.cwd || "—"}</code>
              <button type="button" onClick={revealInFinder} disabled={!external.cwd}>
                <FolderOpen size={14} /> {revealLabel}
              </button>
            </div>
          </section>

          {hasChildren && (
            <section className="external-process-modal-section">
              <h3>Process group ({childList.length + 1} processes)</h3>
              <table className="external-process-children-table">
                <thead>
                  <tr>
                    <th>PID</th>
                    <th>Command</th>
                  </tr>
                </thead>
                <tbody>
                  {childList.map((child) => (
                    <tr key={child.pid}>
                      <td>{child.pid}</td>
                      <td>{child.command || "—"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          )}
        </div>

        <div className="external-process-modal-footer">
          <button type="button" onClick={() => void copy(String(external.pid), "PID")}>
            <Copy size={14} /> Copy PID
          </button>
          <button type="button" onClick={() => void copy(external.command, "Command")}>
            <Copy size={14} /> Copy command
          </button>
          <button type="button" className="external-process-modal-stop" onClick={onStop}>
            <Square size={14} /> Stop
          </button>
          <button type="button" onClick={onClose}>Close</button>
        </div>

        {toast && <div className="external-process-modal-toast">{toast}</div>}
      </div>
    </div>
  );
}
