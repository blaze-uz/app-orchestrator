import { AlertTriangle, CirclePower, FolderKanban, TerminalSquare, Zap } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { StatusBadge } from "../../components/StatusBadge";
import { RuntimeDot } from "../../components/RuntimeDot";
import { formatMemory } from "../../lib/memory";
import { deriveProjectStatus, FAILED_STATUSES } from "../../lib/status";
import { formatPath, formatRelativeTime } from "../../lib/time";
import { useOrchestratorStore } from "../../stores/orchestratorStore";
import type { ProcessDefinition, ProcessRuntimeState, Project } from "../../types/domain";

export function DashboardOverview() {
  const dashboard = useOrchestratorStore((state) => state.dashboard);
  const projects = useOrchestratorStore((state) => state.projects);
  const processes = useOrchestratorStore((state) => state.processes);
  const runtimeStates = useOrchestratorStore((state) => state.runtimeStates);
  const selectProject = useOrchestratorStore((state) => state.selectProject);
  const selectProcess = useOrchestratorStore((state) => state.selectProcess);

  const projectRows = projects.map((project) => {
    const projectProcesses = processes.filter((process) => process.projectId === project.id);
    const states = projectProcesses.map((process) => runtimeStates[process.id]).filter(Boolean);
    return {
      project,
      processCount: projectProcesses.length,
      running: states.filter((state) => state.currentStatus === "running").length,
      failed: states.filter((state) => FAILED_STATUSES.has(state.currentStatus)).length,
      status: deriveProjectStatus(states)
    };
  });

  const processRows = processes.map((process) => ({
    process,
    project: projects.find((project) => project.id === process.projectId),
    runtime: runtimeStates[process.id]
  }));

  const runningProcessCount = dashboard?.runningProcessCount ?? processRows.filter(({ runtime }) => runtime?.currentStatus === "running").length;
  const failedProcessCount = dashboard?.failedProcessCount ?? processRows.filter(({ runtime }) => FAILED_STATUSES.has(runtime?.currentStatus ?? "idle")).length;
  const processTotal = dashboard?.processCount ?? processes.length;
  const projectTotal = dashboard?.projectCount ?? projects.length;
  const autoStartTotal = dashboard?.autoStartProjectCount ?? projects.filter((project) => project.autoStart).length;
  const headlineStatus = failedProcessCount ? "failed" : runningProcessCount ? "running" : "stopped";

  return (
    <main className="page dashboard-page compact-dashboard">
      <header className="compact-page-header">
        <div>
          <p className="eyebrow">Status</p>
          <h2>Runtime status</h2>
          <p>Live health across local projects and commands.</p>
        </div>
        <div className="compact-header-summary" aria-label={`${runningProcessCount} of ${processTotal} processes running`}>
          <RuntimeDot status={headlineStatus} />
          <strong>{runningProcessCount}</strong>
          <small>of {processTotal} running</small>
        </div>
      </header>

      <section className="compact-status-strip" aria-label="Workspace status">
        <StatusMetric icon={FolderKanban} label="Projects" value={projectTotal} />
        <StatusMetric icon={TerminalSquare} label="Processes" value={processTotal} />
        <StatusMetric icon={Zap} label="Running" value={runningProcessCount} tone="good" />
        <StatusMetric icon={AlertTriangle} label="Failed" value={failedProcessCount} tone={failedProcessCount ? "bad" : "neutral"} />
        <StatusMetric icon={CirclePower} label="Auto-start" value={autoStartTotal} />
      </section>

      <section className="compact-runtime-grid">
        <section className="status-panel">
          <div className="compact-section-heading">
            <h3>Projects</h3>
            <small>{projectRows.length} total</small>
          </div>
          <div className="status-table project-status-table">
            <div className="status-table-head">
              <span>Project</span>
              <span>Status</span>
              <span>Running</span>
            </div>
            {projectRows.map(({ project, processCount, running, failed, status }) => (
              <button key={project.id} className="status-table-row project-status-row" type="button" onClick={() => selectProject(project.id)}>
                <span className="project-status-main">
                  <span className="project-dot" style={{ backgroundColor: project.color ?? "#31d07f" }} />
                  <span>
                    <strong>{project.name}</strong>
                    <small>{formatPath(project.rootPath)}</small>
                  </span>
                </span>
                <StatusBadge status={status} />
                <span className="status-count">
                  <strong>{running}/{processCount}</strong>
                  {failed ? <small className="danger-text">{failed} failed</small> : <small>{processCount} total</small>}
                </span>
              </button>
            ))}
            {!projectRows.length ? <p className="compact-empty">No projects configured.</p> : null}
          </div>
        </section>

        <section className="status-panel">
          <div className="compact-section-heading">
            <h3>Processes</h3>
            <small>{processes.length} total</small>
          </div>
          <div className="status-table process-status-table">
            <div className="status-table-head">
              <span>Process</span>
              <span>Status</span>
              <span>Runtime</span>
            </div>
            {processRows.map(({ process, project, runtime }) => (
              <button key={process.id} className="status-table-row process-status-row" type="button" onClick={() => selectProcess(process.id)}>
                <ProcessIdentity process={process} project={project} runtime={runtime} />
                <StatusBadge status={runtime?.currentStatus ?? "stopped"} />
                <span className="runtime-meta">
                  <strong>{runtime?.pid ? `PID ${runtime.pid}` : process.group ?? process.key}</strong>
                  <small>{runtimeDetail(runtime)}</small>
                </span>
              </button>
            ))}
            {!processRows.length ? <p className="compact-empty">No processes configured.</p> : null}
          </div>
        </section>
      </section>
    </main>
  );
}

function StatusMetric({ icon: Icon, label, value, tone = "neutral" }: { icon: LucideIcon; label: string; value: number; tone?: "neutral" | "good" | "bad" }) {
  return (
    <div className={`compact-metric ${tone}`} aria-label={`${label}: ${value}`}>
      <Icon size={16} />
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ProcessIdentity({ process, project, runtime }: { process: ProcessDefinition; project?: Project; runtime?: ProcessRuntimeState }) {
  return (
    <span className="process-status-main">
      <RuntimeDot status={runtime?.currentStatus} />
      <span>
        <strong>{process.name}</strong>
        <small>{project ? project.name : process.command}</small>
      </span>
    </span>
  );
}

function runtimeDetail(runtime?: ProcessRuntimeState) {
  if (!runtime) return "idle";
  if (typeof runtime.memoryUsage === "number") return `${formatMemory(runtime.memoryUsage)} RAM`;
  if (runtime.currentStatus === "running" && runtime.startedAt) return formatRelativeTime(runtime.startedAt);
  if (runtime.lastError) return runtime.lastError;
  return runtime.currentStatus.replace("_", " ");
}
