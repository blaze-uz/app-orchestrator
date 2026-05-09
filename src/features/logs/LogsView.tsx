import { Download, Search, Trash2 } from "lucide-react";
import { LiveLogViewer } from "../../components/LiveLogViewer";
import { useConfirm } from "../../components/ConfirmDialog";
import { useOrchestratorStore } from "../../stores/orchestratorStore";
import type { LogLevel, StreamType } from "../../types/domain";

export function LogsView() {
  const projects = useOrchestratorStore((state) => state.projects);
  const processes = useOrchestratorStore((state) => state.processes);
  const logs = useOrchestratorStore((state) => state.logs);
  const filters = useOrchestratorStore((state) => state.logFilters);
  const setLogFilters = useOrchestratorStore((state) => state.setLogFilters);
  const clearLogs = useOrchestratorStore((state) => state.clearLogs);
  const exportLogs = useOrchestratorStore((state) => state.exportLogs);
  const confirm = useConfirm();

  const visibleLogs = logs
    .filter((log) => !filters.projectId || log.projectId === filters.projectId)
    .filter((log) => !filters.processId || log.processId === filters.processId)
    .filter((log) => !filters.stream || filters.stream === "all" || log.stream === filters.stream)
    .filter((log) => !filters.level || filters.level === "all" || log.level === filters.level)
    .filter((log) => !filters.query || log.message.toLowerCase().includes(filters.query.toLowerCase()));

  const visibleProcesses = processes.filter((process) => !filters.projectId || process.projectId === filters.projectId);

  const exportVisible = async () => {
    const content = await exportLogs();
    await navigator.clipboard.writeText(content);
  };

  return (
    <main className="page">
      <header className="page-header">
        <div>
          <p className="eyebrow">Logs</p>
          <h2>Search and tail</h2>
          <p className="muted">Filter by project, process, stream, level, and text without blocking live updates.</p>
        </div>
        <div className="header-actions">
          <button type="button" onClick={exportVisible}>
            <Download size={16} />
            Export
          </button>
          <button
            type="button"
            onClick={async () => {
              const ok = await confirm({
                title: "Clear all local logs?",
                message: "Logs across every project will be removed from memory and on-disk history.",
                confirmLabel: "Clear logs",
                danger: true,
              });
              if (ok) void clearLogs();
            }}
          >
            <Trash2 size={16} />
            Clear
          </button>
        </div>
      </header>

      <section className="log-filter-bar">
        <label className="search-field wide">
          <Search size={16} />
          <input value={filters.query} onChange={(event) => setLogFilters({ query: event.target.value })} placeholder="Search log text" />
        </label>
        <select value={filters.projectId ?? ""} onChange={(event) => setLogFilters({ projectId: event.target.value || undefined, processId: undefined })}>
          <option value="">All projects</option>
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.name}
            </option>
          ))}
        </select>
        <select value={filters.processId ?? ""} onChange={(event) => setLogFilters({ processId: event.target.value || undefined })}>
          <option value="">All processes</option>
          {visibleProcesses.map((process) => (
            <option key={process.id} value={process.id}>
              {process.name}
            </option>
          ))}
        </select>
        <select value={filters.stream ?? "all"} onChange={(event) => setLogFilters({ stream: event.target.value as StreamType | "all" })}>
          <option value="all">All streams</option>
          <option value="stdout">stdout</option>
          <option value="stderr">stderr</option>
          <option value="system">system</option>
        </select>
        <select value={filters.level ?? "all"} onChange={(event) => setLogFilters({ level: event.target.value as LogLevel | "all" })}>
          <option value="all">All levels</option>
          <option value="info">info</option>
          <option value="warn">warn</option>
          <option value="error">error</option>
          <option value="debug">debug</option>
        </select>
      </section>

      <LiveLogViewer
        logs={visibleLogs}
        paused={filters.paused}
        liveTail={filters.liveTail}
        onPausedChange={(paused) => setLogFilters({ paused })}
        onLiveTailChange={(liveTail) => setLogFilters({ liveTail })}
        onClear={async () => {
          const ok = await confirm({
            title: filters.projectId ? "Clear logs for the selected project?" : "Clear all local logs?",
            confirmLabel: "Clear logs",
            danger: true,
          });
          if (ok) void clearLogs(filters.projectId);
        }}
        onExport={exportVisible}
      />
    </main>
  );
}
