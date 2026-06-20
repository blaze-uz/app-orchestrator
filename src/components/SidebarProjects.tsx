import type { LucideIcon } from "lucide-react";
import {
  ChevronDown,
  ChevronRight,
  FolderKanban,
  LayoutDashboard,
  Play,
  RotateCcw,
  ScrollText,
  Search,
  ServerCog,
  Settings,
  Square
} from "lucide-react";
import { useEffect, useState } from "react";
import { RuntimeDot } from "./RuntimeDot";
import { formatMemory } from "../lib/memory";
import { modKeyLabel } from "../lib/platform";
import { FAILED_STATUSES, isRuntimeBusy } from "../lib/status";
import { formatRelativeTime } from "../lib/time";
import { useOrchestratorStore } from "../stores/orchestratorStore";
import type { ProcessDefinition, ProcessRuntimeState, Project, ViewKey } from "../types/domain";

const viewRows: Array<{ key: ViewKey; label: string; icon: LucideIcon }> = [
  { key: "dashboard", label: "Status", icon: LayoutDashboard },
  { key: "projects", label: "Projects", icon: FolderKanban },
  { key: "machines", label: "Machines", icon: ServerCog },
  { key: "logs", label: "Logs", icon: ScrollText }
];

export function SidebarProjects() {
  const view = useOrchestratorStore((state) => state.view);
  const projects = useOrchestratorStore((state) => state.projects);
  const processes = useOrchestratorStore((state) => state.processes);
  const runtimeStates = useOrchestratorStore((state) => state.runtimeStates);
  const dashboard = useOrchestratorStore((state) => state.dashboard);
  const selectedProjectId = useOrchestratorStore((state) => state.selectedProjectId);
  const selectedProcessId = useOrchestratorStore((state) => state.selectedProcessId);
  const filters = useOrchestratorStore((state) => state.projectFilters);
  const setProjectFilters = useOrchestratorStore((state) => state.setProjectFilters);
  const selectView = useOrchestratorStore((state) => state.selectView);
  const selectProject = useOrchestratorStore((state) => state.selectProject);
  const selectProcess = useOrchestratorStore((state) => state.selectProcess);
  const startProcess = useOrchestratorStore((state) => state.startProcess);
  const restartProcess = useOrchestratorStore((state) => state.restartProcess);
  const stopProcess = useOrchestratorStore((state) => state.stopProcess);
  const [collapsedProjectIds, setCollapsedProjectIds] = useState<Record<string, boolean>>({});
  const [processShortcutMode, setProcessShortcutMode] = useState(false);

  const selectedProject = projects.find((project) => project.id === selectedProjectId);
  const selectedProcesses = processes.filter((process) => process.projectId === selectedProjectId);
  const query = filters.query.toLowerCase();

  const filteredProjects = projects.filter((project) => {
    if (!query) return true;
    return `${project.name} ${project.rootPath} ${project.tags.join(" ")}`.toLowerCase().includes(query);
  });
  const selectedProjectExpanded = selectedProject ? !collapsedProjectIds[selectedProject.id] : false;
  const visibleProcessNumbers = new Map<string, number>();
  let visibleProcessNumber = 1;

  for (const project of filteredProjects) {
    const expanded = project.id === selectedProjectId && !collapsedProjectIds[project.id];
    if (!expanded) continue;

    const projectProcesses = processes.filter((process) => process.projectId === project.id);
    const groupedProcesses = groupProjectProcesses(projectProcesses);
    for (const process of [...groupedProcesses.agents, ...groupedProcesses.commands]) {
      visibleProcessNumbers.set(process.id, visibleProcessNumber);
      visibleProcessNumber += 1;
    }
  }

  useEffect(() => {
    const hideProcessShortcuts = () => setProcessShortcutMode(false);

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Meta" || isEditableShortcutTarget(event.target)) return;
      setProcessShortcutMode(true);
    };

    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key === "Meta") hideProcessShortcuts();
    };

    const onVisibilityChange = () => {
      if (document.visibilityState !== "visible") hideProcessShortcuts();
    };

    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", hideProcessShortcuts);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", hideProcessShortcuts);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, []);

  const handleProjectSelect = (projectId: string) => {
    const isSelected = projectId === selectedProjectId;
    const isExpanded = isSelected && !collapsedProjectIds[projectId];

    setCollapsedProjectIds((current) => {
      const next = { ...current };
      if (isSelected && isExpanded) {
        next[projectId] = true;
      } else {
        delete next[projectId];
      }
      return next;
    });

    void selectProject(projectId);
  };

  return (
    <aside className="sidebar">
      <div className="app-brand">
        <img src="/app-logo.png" alt="" aria-hidden="true" />
        <div>
          <strong>Karvon</strong>
          <span>Process control</span>
        </div>
      </div>

      <label className="sidebar-search">
        <Search size={13} />
        <input value={filters.query} onChange={(event) => setProjectFilters({ query: event.target.value })} placeholder="Filter processes..." />
        <kbd>\</kbd>
      </label>

      <button className="solo-project-row primary" type="button" onClick={() => (selectedProject ? handleProjectSelect(selectedProject.id) : selectView("projects"))}>
        {selectedProjectExpanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        <span className="solo-mark" style={{ backgroundColor: selectedProject?.color || undefined }}>
          {projectMark(selectedProject)}
        </span>
        <strong>{selectedProject?.name ?? "Karvon"}</strong>
        {selectedProject ? <ProjectStatusBadges processes={selectedProcesses} runtimeStates={runtimeStates} /> : null}
        <small>{runningMeta(selectedProject, selectedProcesses, runtimeStates)}</small>
      </button>

      <div className="solo-section">
        <SectionTitle label="Views" count={viewRows.length} />
        {viewRows.map((item) => {
          const Icon = item.icon;
          return (
            <button key={item.key} className={`solo-row ${view === item.key ? "active" : ""}`} type="button" onClick={() => selectView(item.key)}>
              <Icon size={13} />
              <span>{item.label}</span>
              {item.key === "dashboard" ? <small>{dashboard?.runningProcessCount ?? 0}</small> : null}
            </button>
          );
        })}
      </div>

      <div className="solo-section grow">
        <SectionTitle label="Projects" count={filteredProjects.length} />
        <div className="solo-scroll solo-project-tree">
          {filteredProjects.map((project) => {
            const projectProcesses = processes.filter((process) => process.projectId === project.id);
            const selected = project.id === selectedProjectId;
            const expanded = selected && !collapsedProjectIds[project.id];
            const groupedProcesses = groupProjectProcesses(projectProcesses);

            return (
              <div key={project.id} className="solo-project-node">
                <ProjectRow
                  project={project}
                  processes={projectProcesses}
                  runtimeStates={runtimeStates}
                  selected={selected}
                  expanded={expanded}
                  onSelect={() => handleProjectSelect(project.id)}
                />
                {expanded ? (
                  <div className="solo-tree-children">
                    <span className="solo-tree-branch" aria-hidden="true" />
                    <ProcessGroup
                      label="Agents"
                      processes={groupedProcesses.agents}
                      selectedProcessId={selectedProcessId}
                      runtimeStates={runtimeStates}
                      processNumbers={visibleProcessNumbers}
                      shortcutMode={processShortcutMode}
                      onSelect={selectProcess}
                      onStart={startProcess}
                      onRestart={restartProcess}
                      onStop={stopProcess}
                    />
                    <ProcessGroup
                      label="Commands"
                      processes={groupedProcesses.commands}
                      selectedProcessId={selectedProcessId}
                      runtimeStates={runtimeStates}
                      processNumbers={visibleProcessNumbers}
                      shortcutMode={processShortcutMode}
                      onSelect={selectProcess}
                      onStart={startProcess}
                      onRestart={restartProcess}
                      onStop={stopProcess}
                    />
                    {!projectProcesses.length ? <p className="sidebar-empty tree-empty">No processes in this project.</p> : null}
                  </div>
                ) : null}
              </div>
            );
          })}
          {!filteredProjects.length ? <p className="sidebar-empty">No projects match the filter.</p> : null}
        </div>
      </div>

      <div className="sidebar-footer">
        <button className="solo-footer-row" type="button" title="Command palette" onClick={() => window.dispatchEvent(new Event("open-command-palette"))}>
          <kbd>{modKeyLabel} K</kbd>
          <span>to launch actions</span>
        </button>
        <button className={`solo-footer-row ${view === "settings" ? "active" : ""}`} type="button" onClick={() => selectView("settings")}>
          <Settings size={13} />
          <span>Settings</span>
        </button>
      </div>
    </aside>
  );
}

function SectionTitle({ label, count }: { label: string; count?: number }) {
  return (
    <div className="solo-section-title">
      <span>{label}</span>
      {count !== undefined ? <small>{count}</small> : null}
    </div>
  );
}

function ProjectRow({
  project,
  processes,
  runtimeStates,
  selected,
  expanded,
  onSelect
}: {
  project: Project;
  processes: ProcessDefinition[];
  runtimeStates: Record<string, ProcessRuntimeState>;
  selected: boolean;
  expanded: boolean;
  onSelect: () => void;
}) {
  return (
    <button className={`solo-project-row ${selected ? "active" : ""}`} type="button" onClick={onSelect}>
      {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      <span className="solo-mark" style={{ backgroundColor: project.color || undefined }}>
        {projectMark(project)}
      </span>
      <span>{project.name}</span>
      <ProjectStatusBadges processes={processes} runtimeStates={runtimeStates} />
      <small>{projectMeta(project, processes, runtimeStates)}</small>
    </button>
  );
}

function ProcessGroup({
  label,
  processes,
  selectedProcessId,
  runtimeStates,
  processNumbers,
  shortcutMode,
  onSelect,
  onStart,
  onRestart,
  onStop
}: {
  label: string;
  processes: ProcessDefinition[];
  selectedProcessId?: string;
  runtimeStates: Record<string, ProcessRuntimeState>;
  processNumbers: Map<string, number>;
  shortcutMode: boolean;
  onSelect: (processId: string) => void;
  onStart: (processId: string) => void | Promise<void>;
  onRestart: (processId: string) => void | Promise<void>;
  onStop: (processId: string) => void | Promise<void>;
}) {
  if (!processes.length) return null;
  return (
    <div className="solo-tree-group">
      <div className="solo-tree-group-label">{label}</div>
      {processes.map((process) => (
        <ProcessRow
          key={process.id}
          process={process}
          runtime={runtimeStates[process.id]}
          selected={process.id === selectedProcessId}
          shortcutNumber={processNumbers.get(process.id)}
          shortcutMode={shortcutMode}
          onSelect={() => onSelect(process.id)}
          onStart={() => onStart(process.id)}
          onRestart={() => onRestart(process.id)}
          onStop={() => onStop(process.id)}
        />
      ))}
    </div>
  );
}

function ProcessRow({
  process,
  runtime,
  selected,
  shortcutNumber,
  shortcutMode,
  onSelect,
  onStart,
  onRestart,
  onStop
}: {
  process: ProcessDefinition;
  runtime?: ProcessRuntimeState;
  selected: boolean;
  shortcutNumber?: number;
  shortcutMode: boolean;
  onSelect: () => void;
  onStart: () => void | Promise<void>;
  onRestart: () => void | Promise<void>;
  onStop: () => void | Promise<void>;
}) {
  const shortcutsVisible = shortcutMode && shortcutNumber !== undefined;
  const processStarted = isProcessStarted(runtime);

  return (
    <div className={`solo-process-row ${selected ? "active" : ""} ${shortcutsVisible ? "shortcut-mode" : ""}`}>
      <button className="solo-process-main" type="button" onClick={onSelect}>
        <RuntimeDot status={runtime?.currentStatus} />
        <span>{process.name}</span>
      </button>
      <small className="solo-process-meta">{processMeta(process, runtime)}</small>
      <span className="solo-process-controls">
        {shortcutsVisible ? (
          <button
            className="solo-process-shortcut"
            type="button"
            title={`Open ${process.name}`}
            aria-label={`Open ${process.name}`}
            onClick={(event) => {
              event.stopPropagation();
              onSelect();
            }}
          >
            {shortcutNumber}
          </button>
        ) : null}
        <span className={`solo-process-hover-actions ${processStarted ? "started" : "not-started"}`}>
          {processStarted ? (
            <>
              <button
                className="solo-process-action"
                disabled={isRuntimeBusy(runtime?.currentStatus)}
                type="button"
                title="Restart process"
                aria-label={`Restart ${process.name}`}
                onClick={(event) => {
                  event.stopPropagation();
                  void onRestart();
                }}
              >
                <RotateCcw size={13} />
              </button>
              <button
                className="solo-process-action"
                disabled={isRuntimeBusy(runtime?.currentStatus)}
                type="button"
                title="Stop process"
                aria-label={`Stop ${process.name}`}
                onClick={(event) => {
                  event.stopPropagation();
                  void onStop();
                }}
              >
                <Square size={13} />
              </button>
            </>
          ) : (
            <button
              className="solo-process-action"
              disabled={isRuntimeBusy(runtime?.currentStatus)}
              type="button"
              title="Start process"
              aria-label={`Start ${process.name}`}
              onClick={(event) => {
                event.stopPropagation();
                void onStart();
              }}
            >
              <Play size={13} />
            </button>
          )}
        </span>
      </span>
    </div>
  );
}

function isProcessStarted(runtime?: ProcessRuntimeState) {
  if (!runtime) return false;
  return runtime.currentStatus !== "idle" && runtime.currentStatus !== "stopped";
}

function isEditableShortcutTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || Boolean(target.closest("input, textarea, select, [contenteditable='true'], [contenteditable='']"));
}

function groupProjectProcesses(projectProcesses: ProcessDefinition[]) {
  const agents = projectProcesses.filter(isAgentProcess);
  const agentIds = new Set(agents.map((process) => process.id));
  return {
    agents,
    commands: projectProcesses.filter((process) => !agentIds.has(process.id))
  };
}

function isAgentProcess(process: ProcessDefinition) {
  return /(codex|claude|gemini|aider|amp|goose|agent)/i.test(`${process.name} ${process.key} ${process.command} ${process.group ?? ""}`);
}

function projectMark(project?: Project) {
  if (!project) return "LO";
  const source = (project.icon ?? project.name.slice(0, 2)).trim();
  return (source || "??").slice(0, 4).toUpperCase();
}

function ProjectStatusBadges({
  processes,
  runtimeStates
}: {
  processes: ProcessDefinition[];
  runtimeStates: Record<string, ProcessRuntimeState>;
}) {
  if (!processes.length) return null;
  let running = 0;
  let failed = 0;
  let busy = 0;
  for (const process of processes) {
    const status = runtimeStates[process.id]?.currentStatus;
    if (!status) continue;
    if (status === "running") running += 1;
    else if (FAILED_STATUSES.has(status)) failed += 1;
    else if (isRuntimeBusy(status)) busy += 1;
  }
  if (!running && !failed && !busy) return null;
  return (
    <span className="solo-project-badges" aria-hidden="true">
      {running > 0 ? <span className="solo-badge good" title={`${running} running`}>{running}</span> : null}
      {failed > 0 ? <span className="solo-badge bad" title={`${failed} failed`}>{failed}</span> : null}
      {busy > 0 ? <span className="solo-badge busy" title={`${busy} transitioning`}>{busy}</span> : null}
    </span>
  );
}

function projectMeta(project: Project, processes: ProcessDefinition[], runtimeStates: Record<string, ProcessRuntimeState>) {
  if (!processes.length) return project.autoStart ? "auto" : "";
  const states = processes.map((process) => runtimeStates[process.id]).filter(Boolean);
  const running = states.filter((state) => state.currentStatus === "running").length;
  if (running > 0) return projectMemoryMeta(states);
  return `${running}/${processes.length}`;
}

function processMeta(process: ProcessDefinition, runtime?: ProcessRuntimeState) {
  if (runtime?.currentStatus === "running") {
    const stats = runtimeStats(runtime);
    return stats || "0 MB";
  }
  const stats = runtimeStats(runtime);
  if (stats && runtime?.currentStatus === "starting") return stats;
  if (runtime?.pid) return `pid ${runtime.pid}`;
  return process.group ?? process.key;
}

function runtimeStats(runtime?: ProcessRuntimeState) {
  if (!runtime) return "";
  const stats: string[] = [];
  if (typeof runtime.cpuUsage === "number") stats.push(`${runtime.cpuUsage.toFixed(1)}%`);
  if (typeof runtime.memoryUsage === "number") stats.push(formatMemory(runtime.memoryUsage));
  return stats.join(" · ");
}

function runningMeta(project: Project | undefined, processes: ProcessDefinition[], runtimeStates: ReturnType<typeof useOrchestratorStore.getState>["runtimeStates"]) {
  if (!project) return "0/0";
  const states = processes.map((process) => runtimeStates[process.id]).filter(Boolean);
  const running = states.filter((state) => state.currentStatus === "running").length;
  if (running > 0) return projectMemoryMeta(states);
  return `${running}/${processes.length}`;
}

function projectMemoryMeta(states: ProcessRuntimeState[]) {
  const runningStates = states.filter((state) => state.currentStatus === "running");
  const totalMemory = runningStates.reduce((total, state) => total + (state.memoryUsage ?? 0), 0);
  return formatMemory(totalMemory);
}
