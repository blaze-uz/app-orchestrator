import { FolderOpen, FolderPlus, Play, RotateCcw, Square, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { StatusBadge } from "../../components/StatusBadge";
import { useConfirm } from "../../components/ConfirmDialog";
import { selectFolder } from "../../lib/folderPicker";
import { normalizeMemoryLimitMb } from "../../lib/memory";
import { deriveProjectStatus, FAILED_STATUSES } from "../../lib/status";
import { formatPath, parseListInput } from "../../lib/time";
import { useOrchestratorStore } from "../../stores/orchestratorStore";
import type { ProjectFormInput } from "../../types/domain";

const emptyProject: ProjectFormInput = {
  name: "",
  rootPath: "",
  description: "",
  tags: [],
  autoStart: false,
  startupOrder: 10,
  memoryLimitMb: undefined
};

export function ProjectsView() {
  const [formOpen, setFormOpen] = useState(false);
  const [draft, setDraft] = useState<ProjectFormInput>(emptyProject);
  const projects = useOrchestratorStore((state) => state.projects);
  const processes = useOrchestratorStore((state) => state.processes);
  const runtimeStates = useOrchestratorStore((state) => state.runtimeStates);
  const createProject = useOrchestratorStore((state) => state.createProject);
  const selectProject = useOrchestratorStore((state) => state.selectProject);
  const startProject = useOrchestratorStore((state) => state.startProject);
  const stopProject = useOrchestratorStore((state) => state.stopProject);
  const restartProject = useOrchestratorStore((state) => state.restartProject);
  const deleteProject = useOrchestratorStore((state) => state.deleteProject);
  const confirm = useConfirm();

  const rows = useMemo(
    () =>
      projects.map((project) => {
        const projectProcesses = processes.filter((process) => process.projectId === project.id);
        const states = projectProcesses.map((process) => runtimeStates[process.id]).filter(Boolean);
        return { project, processCount: projectProcesses.length, running: states.filter((state) => state.currentStatus === "running").length, failed: states.filter((state) => FAILED_STATUSES.has(state.currentStatus)).length, status: deriveProjectStatus(states) };
      }),
    [processes, projects, runtimeStates]
  );

  const submit = async () => {
    if (!draft.name.trim() || !draft.rootPath.trim()) return;
    await createProject(draft);
    setDraft(emptyProject);
    setFormOpen(false);
  };

  const chooseRootPath = async () => {
    const rootPath = await selectFolder(draft.rootPath);
    if (rootPath) setDraft({ ...draft, rootPath });
  };

  return (
    <main className="page">
      <header className="page-header">
        <div>
          <p className="eyebrow">Projects</p>
          <h2>Local services</h2>
          <p className="muted">Create project groups, attach process definitions, and control them from one view.</p>
        </div>
        <button type="button" onClick={() => setFormOpen(!formOpen)}>
          <FolderPlus size={16} />
          New project
        </button>
      </header>

      {formOpen ? (
        <section className="editor-panel">
          <div className="form-grid">
            <label>
              Name<span className="required-marker" aria-hidden="true">*</span>
              <input required value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="MediaGuard" />
            </label>
            <label>
              Root path<span className="required-marker" aria-hidden="true">*</span>
              <span className="path-picker">
                <input required value={draft.rootPath} readOnly placeholder="Select a local project folder" onClick={chooseRootPath} />
                <button type="button" onClick={chooseRootPath} title="Select project folder">
                  <FolderOpen size={16} />
                  Browse
                </button>
              </span>
            </label>
            <label>
              Description
              <input value={draft.description} onChange={(event) => setDraft({ ...draft, description: event.target.value })} placeholder="Local app and workers" />
            </label>
            <label>
              Tags
              <input value={draft.tags.join(", ")} onChange={(event) => setDraft({ ...draft, tags: parseListInput(event.target.value) })} placeholder="laravel, queue, collector" />
            </label>
            <label>
              Startup order
              <input type="number" value={draft.startupOrder} onChange={(event) => setDraft({ ...draft, startupOrder: Number(event.target.value) })} />
            </label>
            <label>
              Project RAM cap (MB)
              <input
                type="number"
                min={128}
                step={128}
                value={draft.memoryLimitMb ?? ""}
                onChange={(event) => setDraft({ ...draft, memoryLimitMb: normalizeMemoryLimitMb(event.target.valueAsNumber) })}
                placeholder="Off"
              />
            </label>
            <label className="checkbox-line">
              <input type="checkbox" checked={draft.autoStart} onChange={(event) => setDraft({ ...draft, autoStart: event.target.checked })} />
              Auto-start project on app launch
            </label>
          </div>
          <div className="editor-actions">
            <button type="button" onClick={submit}>
              Create project
            </button>
          </div>
        </section>
      ) : null}

      <section className="project-cards">
        {rows.map(({ project, processCount, running, failed, status }) => (
          <article key={project.id} className="project-card">
            <button type="button" className="project-card-main" onClick={() => selectProject(project.id)}>
              <span className="project-dot large" style={{ backgroundColor: project.color ?? "#32d583" }} />
              <span>
                <strong>{project.name}</strong>
                <small>{formatPath(project.rootPath)}</small>
              </span>
            </button>
            <div className="project-card-meta">
              <StatusBadge status={status} />
              <span>{processCount} processes</span>
              <span>{running} running</span>
              {failed ? <span className="danger-text">{failed} failed</span> : null}
            </div>
            <div className="inline-actions">
              <button type="button" onClick={() => startProject(project.id)} title="Start project">
                <Play size={16} />
              </button>
              <button type="button" onClick={() => stopProject(project.id)} title="Stop project">
                <Square size={16} />
              </button>
              <button type="button" onClick={() => restartProject(project.id)} title="Restart project">
                <RotateCcw size={16} />
              </button>
              <button
                className="danger-button"
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
                title="Delete project"
              >
                <Trash2 size={16} />
              </button>
            </div>
          </article>
        ))}
      </section>
    </main>
  );
}
