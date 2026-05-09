import { Clipboard, HeartPulse, Play, RotateCcw, Square } from "lucide-react";
import { LiveLogViewer } from "../../components/LiveLogViewer";
import { StatusBadge } from "../../components/StatusBadge";
import { useConfirm } from "../../components/ConfirmDialog";
import { RuntimeDot } from "../../components/RuntimeDot";
import { formatMemory, formatMemoryLimit } from "../../lib/memory";
import { isRuntimeBusy } from "../../lib/status";
import { formatClock, formatRelativeTime } from "../../lib/time";
import { useOrchestratorStore } from "../../stores/orchestratorStore";

export function ProcessDetailPanel() {
  const projects = useOrchestratorStore((state) => state.projects);
  const processes = useOrchestratorStore((state) => state.processes);
  const runtimeStates = useOrchestratorStore((state) => state.runtimeStates);
  const selectedProcessId = useOrchestratorStore((state) => state.selectedProcessId);
  const logs = useOrchestratorStore((state) => state.logs);
  const logFilters = useOrchestratorStore((state) => state.logFilters);
  const setLogFilters = useOrchestratorStore((state) => state.setLogFilters);
  const startProcess = useOrchestratorStore((state) => state.startProcess);
  const stopProcess = useOrchestratorStore((state) => state.stopProcess);
  const restartProcess = useOrchestratorStore((state) => state.restartProcess);
  const runHealthCheck = useOrchestratorStore((state) => state.runHealthCheck);
  const clearLogs = useOrchestratorStore((state) => state.clearLogs);
  const exportLogs = useOrchestratorStore((state) => state.exportLogs);
  const confirm = useConfirm();
  const process = processes.find((item) => item.id === selectedProcessId) ?? processes[0];
  const runtime = process ? runtimeStates[process.id] : undefined;
  const project = projects.find((item) => item.id === process?.projectId);
  const processLogs = logs.filter((log) => log.processId === process?.id);

  if (!process) {
    return (
      <main className="empty-state">
        <Clipboard size={24} />
        <span>Select a process to inspect.</span>
      </main>
    );
  }

  const fullCommand = `${process.command} ${process.args.join(" ")}`.trim();

  const copyCommand = async () => {
    await navigator.clipboard.writeText(fullCommand);
  };

  const exportProcessLogs = async () => {
    const content = await exportLogs();
    await navigator.clipboard.writeText(content);
  };

  return (
    <main className="page process-inspector-page">
      <header className="process-inspector-header">
        <div className="process-title-cell">
          <RuntimeDot status={runtime?.currentStatus} />
          <span>
            <small>{project?.name ?? "Project"}</small>
            <strong>{process.name}</strong>
          </span>
        </div>
        <StatusBadge status={runtime?.currentStatus ?? "stopped"} />
        <div className="icon-toolbar process-header-actions">
          <button disabled={isRuntimeBusy(runtime?.currentStatus)} type="button" onClick={() => startProcess(process.id)} title="Start process">
            <Play size={14} />
          </button>
          <button disabled={isRuntimeBusy(runtime?.currentStatus)} type="button" onClick={() => stopProcess(process.id)} title="Stop process">
            <Square size={14} />
          </button>
          <button disabled={isRuntimeBusy(runtime?.currentStatus)} type="button" onClick={() => restartProcess(process.id)} title="Restart process">
            <RotateCcw size={14} />
          </button>
          <button type="button" onClick={() => runHealthCheck(process.id)} title="Run health check">
            <HeartPulse size={14} />
          </button>
          <button type="button" onClick={copyCommand} title="Copy command">
            <Clipboard size={14} />
          </button>
        </div>
      </header>

      <section className="process-summary-strip" aria-label="Process runtime summary">
        <SummaryCell label="Key" value={process.key} />
        <SummaryCell label="PID" value={runtime?.pid ? String(runtime.pid) : "n/a"} />
        <SummaryCell label="Started" value={runtime?.startedAt ? formatRelativeTime(runtime.startedAt) : "idle"} />
        <SummaryCell label="Restarts" value={String(runtime?.restartCount ?? 0)} />
        <SummaryCell label="RAM" value={runtime?.memoryUsage ? formatMemory(runtime.memoryUsage) : "0 MB"} />
        <SummaryCell label="Health" value={runtime?.healthStatus ?? "unknown"} />
      </section>

      <section className="process-inspector-grid">
        <LiveLogViewer
          logs={processLogs}
          paused={logFilters.paused}
          liveTail={logFilters.liveTail}
          onPausedChange={(paused) => setLogFilters({ paused })}
          onLiveTailChange={(liveTail) => setLogFilters({ liveTail })}
          onClear={async () => {
            if (!project) return;
            const ok = await confirm({
              title: "Clear logs for this project?",
              message: "Logs will be removed from memory and on-disk history.",
              confirmLabel: "Clear logs",
              danger: true,
            });
            if (ok) void clearLogs(project.id);
          }}
          onExport={exportProcessLogs}
        />

        <aside className="process-detail-sidebar">
          <div className="compact-section-heading">
            <h3>Command</h3>
            <button type="button" onClick={copyCommand} title="Copy command">
              <Clipboard size={13} />
            </button>
          </div>
          <div className="process-detail-list">
            <DetailRow label="Full command" value={fullCommand} mono />
            <DetailRow label="Working directory" value={process.workingDirectory || project?.rootPath || "Project root"} mono />
            <DetailRow label="Restart policy" value={process.restartPolicy.kind} />
            <DetailRow label="RAM limit" value={formatMemoryLimit(process.memoryLimitMb)} />
            <DetailRow label="Dependencies" value={process.dependsOn.length ? process.dependsOn.join(", ") : "None"} />
            <DetailRow label="Log mode" value={process.logMode} />
          </div>

          <div className="compact-section-heading">
            <h3>Runtime</h3>
            <small>{runtime?.currentStatus ?? "stopped"}</small>
          </div>
          <div className="process-detail-list">
            <DetailRow label="PID" value={runtime?.pid ? String(runtime.pid) : "n/a"} />
            <DetailRow label="Started" value={runtime?.startedAt ? `${formatClock(runtime.startedAt)} (${formatRelativeTime(runtime.startedAt)})` : "n/a"} />
            <DetailRow label="Stopped" value={runtime?.stoppedAt ? formatClock(runtime.stoppedAt) : "n/a"} />
            <DetailRow label="Memory" value={runtime?.memoryUsage ? formatMemory(runtime.memoryUsage) : "0 MB"} />
            <DetailRow label="Exit code" value={runtime?.exitCode !== undefined ? String(runtime.exitCode) : "n/a"} />
            <DetailRow label="Last error" value={runtime?.lastError ?? "None"} />
          </div>

          <div className="compact-section-heading">
            <h3>Environment</h3>
            <small>{Object.keys(process.env).length} vars</small>
          </div>
          <div className="process-env-list">
            {Object.entries(process.env).length ? (
              Object.entries(process.env).map(([key, value]) => <DetailRow key={key} label={key} value={/(token|secret|password|key)/i.test(key) ? "••••••••" : value} mono />)
            ) : (
              <p className="compact-empty">No custom env values.</p>
            )}
          </div>
        </aside>
      </section>
    </main>
  );
}

function SummaryCell({ label, value }: { label: string; value: string }) {
  return (
    <div className="process-summary-cell">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function DetailRow({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="process-detail-row">
      <span>{label}</span>
      <strong className={mono ? "mono-value" : undefined}>{value}</strong>
    </div>
  );
}
