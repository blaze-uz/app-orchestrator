import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Edit3,
  Loader2,
  Play,
  Plus,
  Square,
  Trash2
} from "lucide-react";
import { useMemo, useState } from "react";
import { useConfirm } from "../../components/ConfirmDialog";
import { envToText, formatRelativeTime, normalizeCliText, parseEnvInput, parseListInput } from "../../lib/time";
import { useOrchestratorStore } from "../../stores/orchestratorStore";
import type {
  DeployScript,
  DeployScriptFormInput,
  DeployScriptResult,
  DeployStage,
  Project
} from "../../types/domain";

const technicalInputProps = {
  autoCapitalize: "off",
  autoCorrect: "off",
  spellCheck: false
} as const;

const STAGE_ORDER: DeployStage[] = ["pre", "main", "post"];

const STAGE_LABEL: Record<DeployStage, string> = {
  pre: "Pre-deploy",
  main: "Main",
  post: "Post-deploy"
};

const STAGE_HINT: Record<DeployStage, string> = {
  pre: "Run before the main pipeline (e.g. notify Slack, check disk).",
  main: "Core deployment steps (git pull, build, migrate).",
  post: "Run after restart (e.g. health checks, notifications)."
};

const EMPTY_SCRIPTS: DeployScript[] = [];

function emptyDraft(projectId: string, stage: DeployStage): DeployScriptFormInput {
  return {
    projectId,
    name: "",
    stage,
    command: "",
    args: [],
    workingDirectory: "",
    env: {},
    machineId: undefined,
    continueOnError: false
  };
}

function compareScripts(a: DeployScript, b: DeployScript) {
  return a.order - b.order;
}

function formatScriptCommand(script: DeployScript) {
  return [script.command, ...script.args].filter(Boolean).join(" ");
}

export function DeploySection({ project }: { project: Project }) {
  const machines = useOrchestratorStore((state) => state.machines);
  const allScripts = useOrchestratorStore((state) => state.deployScripts[project.id] ?? EMPTY_SCRIPTS);
  const deployState = useOrchestratorStore((state) => state.deployStates[project.id]);
  const createDeployScript = useOrchestratorStore((state) => state.createDeployScript);
  const updateDeployScript = useOrchestratorStore((state) => state.updateDeployScript);
  const deleteDeployScript = useOrchestratorStore((state) => state.deleteDeployScript);
  const reorderDeployScripts = useOrchestratorStore((state) => state.reorderDeployScripts);
  const deployProject = useOrchestratorStore((state) => state.deployProject);
  const cancelDeploy = useOrchestratorStore((state) => state.cancelDeploy);
  const updateProject = useOrchestratorStore((state) => state.updateProject);
  const currentAction = useOrchestratorStore((state) => state.currentAction);
  const confirm = useConfirm();

  const [draft, setDraft] = useState<DeployScriptFormInput | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [formError, setFormError] = useState<string>();
  const [collapsedStages, setCollapsedStages] = useState<Record<DeployStage, boolean>>({ pre: false, main: false, post: false });

  const scriptsByStage = useMemo(() => {
    const buckets: Record<DeployStage, DeployScript[]> = { pre: [], main: [], post: [] };
    for (const script of allScripts) buckets[script.stage].push(script);
    for (const stage of STAGE_ORDER) buckets[stage].sort(compareScripts);
    return buckets;
  }, [allScripts]);

  const resultByScript = useMemo(() => {
    const map = new Map<string, DeployScriptResult>();
    deployState?.scriptResults.forEach((result) => map.set(result.scriptId, result));
    return map;
  }, [deployState]);

  const status = deployState?.status ?? "idle";
  const isRunning = status === "running";
  const currentScript = isRunning && deployState?.currentScriptId
    ? allScripts.find((script) => script.id === deployState.currentScriptId)
    : undefined;

  const openCreate = (stage: DeployStage) => {
    setDraft(emptyDraft(project.id, stage));
    setEditingId(null);
    setFormError(undefined);
  };

  const openEdit = (script: DeployScript) => {
    setEditingId(script.id);
    setDraft({
      projectId: script.projectId,
      name: script.name,
      stage: script.stage,
      command: script.command,
      args: script.args,
      workingDirectory: script.workingDirectory ?? "",
      env: script.env,
      machineId: script.machineId,
      continueOnError: script.continueOnError
    });
    setFormError(undefined);
  };

  const cancelForm = () => {
    setDraft(null);
    setEditingId(null);
    setFormError(undefined);
  };

  const submitForm = async () => {
    if (!draft) return;
    const name = draft.name.trim();
    const command = draft.command.trim();
    if (!name || !command) {
      setFormError("Name and command are required.");
      return;
    }
    const cleaned: DeployScriptFormInput = {
      ...draft,
      name,
      command,
      workingDirectory: draft.workingDirectory?.trim() || undefined,
      machineId: draft.machineId?.trim() || undefined
    };

    if (editingId) {
      const existing = allScripts.find((script) => script.id === editingId);
      if (!existing) {
        setFormError("Script no longer exists.");
        return;
      }
      await updateDeployScript({
        ...existing,
        name: cleaned.name,
        stage: cleaned.stage,
        command: cleaned.command,
        args: cleaned.args,
        workingDirectory: cleaned.workingDirectory,
        env: cleaned.env,
        machineId: cleaned.machineId,
        continueOnError: cleaned.continueOnError
      });
      cancelForm();
      return;
    }

    const ok = await createDeployScript(cleaned);
    if (ok) cancelForm();
    else setFormError("Could not save deploy script.");
  };

  const move = async (stage: DeployStage, scriptId: string, delta: -1 | 1) => {
    const bucket = scriptsByStage[stage];
    const index = bucket.findIndex((item) => item.id === scriptId);
    if (index < 0) return;
    const next = index + delta;
    if (next < 0 || next >= bucket.length) return;
    const reordered = bucket.slice();
    const [moved] = reordered.splice(index, 1);
    reordered.splice(next, 0, moved);
    const fullOrder: string[] = [];
    for (const s of STAGE_ORDER) {
      const scripts = s === stage ? reordered : scriptsByStage[s];
      fullOrder.push(...scripts.map((script) => script.id));
    }
    await reorderDeployScripts(project.id, fullOrder);
  };

  const handleDeploy = async () => {
    if (isRunning) {
      const ok = await confirm({
        title: "Cancel deployment?",
        message: "The current script will be sent SIGTERM. Already completed scripts are not rolled back.",
        confirmLabel: "Cancel deploy",
        danger: true
      });
      if (ok) await cancelDeploy(project.id);
      return;
    }
    if (!allScripts.length) return;
    await deployProject(project.id);
  };

  const deployBusy = Boolean(currentAction?.key === `deploy:${project.id}` || currentAction?.key === `cancel-deploy:${project.id}`);

  return (
    <section className="solo-detail-section">
      <div className="solo-detail-section-heading">
        <span>Deploy</span>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <DeployStatusPill status={status} currentScriptName={currentScript?.name} startedAt={deployState?.startedAt} completedAt={deployState?.completedAt} />
          {isRunning ? (
            <button type="button" onClick={handleDeploy} disabled={deployBusy} title="Cancel deployment">
              <Square size={14} />
              Cancel
            </button>
          ) : (
            <button type="button" onClick={handleDeploy} disabled={deployBusy || allScripts.length === 0} title={allScripts.length ? "Run deploy pipeline" : "Add at least one deploy script first"}>
              <Play size={14} />
              Deploy now
            </button>
          )}
        </div>
      </div>

      <div className="solo-detail-card">
        <div className="solo-detail-row">
          <span className="solo-detail-row-copy">
            <strong>Auto-deploy on new commits</strong>
            <small>Polls main (or master) every minute and runs the deploy pipeline when a new commit appears.</small>
          </span>
          <span className="solo-detail-actions">
            <label className={`solo-switch ${project.autoDeploy ? "checked" : ""}`}>
              <input
                type="checkbox"
                checked={project.autoDeploy}
                onChange={(event) => updateProject({ ...project, autoDeploy: event.target.checked })}
              />
              <span />
            </label>
          </span>
        </div>
        <div className="solo-detail-row">
          <span className="solo-detail-row-copy">
            <strong>Auto-restart processes after main steps</strong>
            <small>When the main pipeline succeeds, all project processes restart automatically.</small>
          </span>
          <span className="solo-detail-actions">
            <label className={`solo-switch ${project.autoRestartOnDeploy ? "checked" : ""}`}>
              <input
                type="checkbox"
                checked={project.autoRestartOnDeploy}
                onChange={(event) => updateProject({ ...project, autoRestartOnDeploy: event.target.checked })}
              />
              <span />
            </label>
          </span>
        </div>
      </div>

      {STAGE_ORDER.map((stage) => {
        const scripts = scriptsByStage[stage];
        const collapsed = collapsedStages[stage] && stage !== "main";
        return (
          <div key={stage} className="solo-detail-card" style={{ marginTop: 8 }}>
            <div className="solo-detail-row" style={{ alignItems: "center" }}>
              <button
                type="button"
                className="solo-command-main"
                onClick={() => setCollapsedStages((prev) => ({ ...prev, [stage]: !prev[stage] }))}
                style={{ flex: 1 }}
              >
                {collapsed ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
                <span>
                  <strong>{STAGE_LABEL[stage]}</strong>
                  <small>{STAGE_HINT[stage]} · {scripts.length} {scripts.length === 1 ? "script" : "scripts"}</small>
                </span>
              </button>
              <span className="solo-command-actions">
                <button type="button" onClick={() => openCreate(stage)} title={`Add ${STAGE_LABEL[stage]} script`}>
                  <Plus size={14} />
                </button>
              </span>
            </div>
            {!collapsed && scripts.map((script, index) => {
              const result = resultByScript.get(script.id);
              const machine = script.machineId ? machines.find((m) => m.id === script.machineId) : undefined;
              const machineLabel = machine && !machine.isDefaultLocal ? machine.name : undefined;
              return (
                <DeployScriptRow
                  key={script.id}
                  script={script}
                  result={result}
                  isCurrent={currentScript?.id === script.id}
                  machineLabel={machineLabel}
                  canMoveUp={index > 0}
                  canMoveDown={index < scripts.length - 1}
                  onMoveUp={() => move(stage, script.id, -1)}
                  onMoveDown={() => move(stage, script.id, 1)}
                  onEdit={() => openEdit(script)}
                  onDelete={async () => {
                    const ok = await confirm({
                      title: `Delete '${script.name}'?`,
                      message: "Deploy script will be removed permanently.",
                      confirmLabel: "Delete",
                      danger: true
                    });
                    if (ok) await deleteDeployScript(script.id, project.id);
                  }}
                />
              );
            })}
            {!collapsed && !scripts.length ? (
              <p className="solo-empty-row">No {STAGE_LABEL[stage].toLowerCase()} scripts.</p>
            ) : null}
          </div>
        );
      })}

      {draft ? (
        <div className="editor-panel inline solo-command-editor" style={{ marginTop: 12 }}>
          <div className="form-grid">
            <label>
              Name<span className="required-marker" aria-hidden="true">*</span>
              <input
                required
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                placeholder="Git pull"
              />
            </label>
            <label>
              Stage
              <select
                value={draft.stage}
                onChange={(event) => setDraft({ ...draft, stage: event.target.value as DeployStage })}
              >
                {STAGE_ORDER.map((stage) => (
                  <option key={stage} value={stage}>
                    {STAGE_LABEL[stage]}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Command<span className="required-marker" aria-hidden="true">*</span>
              <input
                required
                {...technicalInputProps}
                value={draft.command}
                onChange={(event) => setDraft({ ...draft, command: normalizeCliText(event.target.value) })}
                placeholder="git"
              />
            </label>
            <label>
              Args
              <input
                {...technicalInputProps}
                value={draft.args.join(", ")}
                onChange={(event) => setDraft({ ...draft, args: parseListInput(event.target.value) })}
                placeholder="pull, --ff-only"
              />
            </label>
            <label>
              Working directory
              <input
                {...technicalInputProps}
                value={draft.workingDirectory ?? ""}
                onChange={(event) => setDraft({ ...draft, workingDirectory: event.target.value })}
                placeholder={project.rootPath}
              />
            </label>
            <label>
              Machine
              <select
                value={draft.machineId ?? ""}
                onChange={(event) => setDraft({ ...draft, machineId: event.target.value || undefined })}
              >
                <option value="">{machines.find((machine) => machine.isDefaultLocal)?.name ?? "Local"} (local)</option>
                {machines
                  .filter((machine) => !machine.isDefaultLocal)
                  .map((machine) => (
                    <option key={machine.id} value={machine.id}>
                      {machine.name} ({machine.sshUser}@{machine.hostname})
                    </option>
                  ))}
              </select>
            </label>
            <label>
              Env
              <textarea
                {...technicalInputProps}
                value={envToText(draft.env)}
                onChange={(event) => setDraft({ ...draft, env: parseEnvInput(event.target.value) })}
                placeholder="DEPLOY_ENV=production"
              />
            </label>
            <label className="checkbox-line">
              <input
                type="checkbox"
                checked={draft.continueOnError}
                onChange={(event) => setDraft({ ...draft, continueOnError: event.target.checked })}
              />
              Continue on error
            </label>
          </div>
          {formError ? <div className="form-error">{formError}</div> : null}
          <div className="editor-actions">
            <button type="button" onClick={cancelForm}>
              Cancel
            </button>
            <button type="button" onClick={submitForm}>
              {editingId ? "Save script" : "Create script"}
            </button>
          </div>
        </div>
      ) : null}
    </section>
  );
}

function DeployStatusPill({
  status,
  currentScriptName,
  startedAt,
  completedAt
}: {
  status: string;
  currentScriptName?: string;
  startedAt?: string;
  completedAt?: string;
}) {
  const label = (() => {
    switch (status) {
      case "running":
        return currentScriptName ? `Running: ${currentScriptName}` : "Running";
      case "success":
        return completedAt ? `Succeeded ${formatRelativeTime(completedAt)}` : "Succeeded";
      case "failed":
        return completedAt ? `Failed ${formatRelativeTime(completedAt)}` : "Failed";
      case "cancelled":
        return "Cancelled";
      default:
        return startedAt ? `Last run ${formatRelativeTime(startedAt)}` : "Idle";
    }
  })();
  const tone = (() => {
    switch (status) {
      case "running":
        return "active";
      case "success":
        return "active";
      case "failed":
      case "cancelled":
        return "failed";
      default:
        return "idle";
    }
  })();
  return (
    <span className={`solo-running-pill ${tone}`}>
      <span />
      {label}
    </span>
  );
}

function DeployScriptRow({
  script,
  result,
  isCurrent,
  machineLabel,
  canMoveUp,
  canMoveDown,
  onMoveUp,
  onMoveDown,
  onEdit,
  onDelete
}: {
  script: DeployScript;
  result?: DeployScriptResult;
  isCurrent: boolean;
  machineLabel?: string;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="solo-command-row">
      <span className="solo-command-main" style={{ cursor: "default" }}>
        <ResultDot status={result?.status} isCurrent={isCurrent} />
        <span>
          <strong>{script.name}</strong>
          <small>
            {formatScriptCommand(script)}
            {machineLabel ? <span className="process-machine-badge">{machineLabel}</span> : null}
            {script.continueOnError ? <span className="process-machine-badge">continue-on-error</span> : null}
          </small>
        </span>
      </span>
      <span className="solo-command-meta">
        {result?.completedAt ? formatRelativeTime(result.completedAt) : isCurrent ? "running…" : ""}
      </span>
      <span className="solo-command-actions">
        <button type="button" onClick={onMoveUp} disabled={!canMoveUp} title="Move up">
          <ChevronUp size={14} />
        </button>
        <button type="button" onClick={onMoveDown} disabled={!canMoveDown} title="Move down">
          <ChevronDown size={14} />
        </button>
        <button type="button" onClick={onEdit} title="Edit">
          <Edit3 size={14} />
        </button>
        <button type="button" onClick={onDelete} title="Delete">
          <Trash2 size={14} />
        </button>
      </span>
    </div>
  );
}

function ResultDot({ status, isCurrent }: { status?: DeployScriptResult["status"]; isCurrent: boolean }) {
  if (isCurrent || status === "running") {
    return <Loader2 size={14} className="spin" aria-label="running" />;
  }
  switch (status) {
    case "success":
      return <CheckCircle2 size={14} style={{ color: "#32d583" }} aria-label="success" />;
    case "failed":
      return <AlertCircle size={14} style={{ color: "#f97066" }} aria-label="failed" />;
    case "skipped":
      return <AlertCircle size={14} style={{ color: "#fdb022" }} aria-label="skipped" />;
    default:
      return <span style={{ display: "inline-block", width: 10, height: 10, borderRadius: "50%", border: "1px solid rgba(255,255,255,0.3)" }} />;
  }
}
