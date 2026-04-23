import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { Download, FolderOpen, RefreshCw, RotateCcw, Upload } from "lucide-react";
import { selectFolder } from "../../lib/folderPicker";
import {
  canUseAppUpdater,
  checkForAppUpdate,
  confirmUpdateInstall,
  getCurrentAppVersion,
  installAppUpdate,
  type Update,
  type UpdateProgress
} from "../../lib/updater";
import { useOrchestratorStore } from "../../stores/orchestratorStore";
import type { ActivityEvent, AppConfig, AppSettings } from "../../types/domain";

type SettingsTab = "appearance" | "notifications" | "automation" | "logging" | "storage" | "updates" | "config" | "activity";
type UpdateStatus = "idle" | "checking" | "available" | "current" | "installing" | "error";

const settingsTabs: Array<{ key: SettingsTab; label: string }> = [
  { key: "appearance", label: "Appearance" },
  { key: "notifications", label: "Notifications" },
  { key: "automation", label: "Automation" },
  { key: "logging", label: "Logging" },
  { key: "storage", label: "Storage" },
  { key: "updates", label: "Updates" },
  { key: "config", label: "Config" },
  { key: "activity", label: "Activity" }
];

export function SettingsView() {
  const settings = useOrchestratorStore((state) => state.settings);
  const projects = useOrchestratorStore((state) => state.projects);
  const updateSettings = useOrchestratorStore((state) => state.updateSettings);
  const exportConfig = useOrchestratorStore((state) => state.exportConfig);
  const importConfig = useOrchestratorStore((state) => state.importConfig);
  const activity = useOrchestratorStore((state) => state.activity);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [activeTab, setActiveTab] = useState<SettingsTab>("appearance");
  const [redactSecrets, setRedactSecrets] = useState(true);
  const [importError, setImportError] = useState<string>();
  const [appVersion, setAppVersion] = useState(__APP_VERSION__);
  const [availableUpdate, setAvailableUpdate] = useState<Update | null>(null);
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
  const [updateMessage, setUpdateMessage] = useState<string>();
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress>();

  useEffect(() => {
    let mounted = true;
    getCurrentAppVersion()
      .then((version) => {
        if (mounted) setAppVersion(version);
      })
      .catch(() => undefined);
    return () => {
      mounted = false;
    };
  }, []);

  if (!settings) {
    return (
      <main className="empty-state">
        <span>Settings unavailable.</span>
      </main>
    );
  }

  const patchSettings = async (patch: Partial<AppSettings>) => {
    await updateSettings({ ...settings, ...patch });
  };

  const downloadConfig = async () => {
    const content = await exportConfig(redactSecrets);
    const blob = new Blob([content], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "local-project-orchestrator.config.json";
    link.click();
    URL.revokeObjectURL(url);
  };

  const readImport = async (file?: File) => {
    if (!file) return;
    setImportError(undefined);
    try {
      const text = await file.text();
      const config = JSON.parse(text) as AppConfig;
      await importConfig(config);
    } catch (error) {
      setImportError(error instanceof Error ? error.message : "Import failed");
    }
  };

  const chooseStoragePath = async () => {
    const projectStoragePath = await selectFolder(settings.projectStoragePath, {
      title: "Select project storage folder",
      prompt: "Project storage folder path"
    });
    if (projectStoragePath) await patchSettings({ projectStoragePath });
  };

  const checkForUpdates = async () => {
    setUpdateStatus("checking");
    setUpdateMessage(undefined);
    setUpdateProgress(undefined);
    setAvailableUpdate(null);
    try {
      const update = await checkForAppUpdate();
      if (!update) {
        setUpdateStatus("current");
        setUpdateMessage(`Version ${appVersion} is current.`);
        return;
      }
      setAvailableUpdate(update);
      setUpdateStatus("available");
      setUpdateMessage(`Version ${update.version} is ready to install.`);
    } catch (error) {
      setUpdateStatus("error");
      setUpdateMessage(error instanceof Error ? error.message : "Unable to check for updates");
    }
  };

  const installAvailableUpdate = async () => {
    if (!availableUpdate) return;
    const confirmed = await confirmUpdateInstall(availableUpdate.version);
    if (!confirmed) return;
    setUpdateStatus("installing");
    setUpdateMessage("Downloading update...");
    setUpdateProgress({ downloadedBytes: 0 });
    try {
      await installAppUpdate(availableUpdate, (progress) => {
        setUpdateProgress(progress);
        setUpdateMessage(progress.percent === undefined ? "Downloading update..." : `Downloading update... ${progress.percent}%`);
      });
      setUpdateMessage("Relaunching...");
    } catch (error) {
      setUpdateStatus("error");
      setUpdateMessage(error instanceof Error ? error.message : "Unable to install update");
    }
  };

  const updateBusy = updateStatus === "checking" || updateStatus === "installing";

  return (
    <main className="page solo-settings-page">
      <header className="solo-settings-titlebar">
        <div className="solo-settings-title">
          <h2>Settings</h2>
          <span>{projects.length} projects</span>
        </div>
        <span className="solo-settings-esc">ESC</span>
      </header>

      <nav className="solo-settings-tabs" aria-label="Settings tabs">
        {settingsTabs.map((tab) => (
          <button key={tab.key} className={tab.key === activeTab ? "active" : ""} type="button" onClick={() => setActiveTab(tab.key)}>
            {tab.label}
          </button>
        ))}
      </nav>

      <section className="solo-settings-body">
        {activeTab === "appearance" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Application">
              <SettingsRow title="Theme" detail="App color scheme">
                <SegmentedControl
                  value={settings.theme}
                  options={[
                    { value: "light", label: "Light" },
                    { value: "dark", label: "Dark" },
                    { value: "system", label: "System" }
                  ]}
                  onChange={(theme) => patchSettings({ theme })}
                />
              </SettingsRow>
            </SettingsGroup>
          </SettingsTabPanel>
        ) : null}

        {activeTab === "notifications" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Notifications">
              <SettingsRow title="Crash & exit alerts" detail="Get notified when processes fail or exit unexpectedly">
                <Switch checked={settings.notificationsEnabled} onChange={(notificationsEnabled) => patchSettings({ notificationsEnabled })} />
              </SettingsRow>
              <SettingsRow title="Health check alerts" detail="Use the same notification setting for degraded services">
                <Switch checked={settings.notificationsEnabled} onChange={(notificationsEnabled) => patchSettings({ notificationsEnabled })} />
              </SettingsRow>
            </SettingsGroup>
          </SettingsTabPanel>
        ) : null}

        {activeTab === "automation" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Application">
              <SettingsRow title="Launch app on login" detail="Open the orchestrator when macOS starts">
                <Switch checked={settings.launchOnLogin} onChange={(launchOnLogin) => patchSettings({ launchOnLogin })} />
              </SettingsRow>
              <SettingsRow title="Start marked projects" detail="Auto-start projects marked for launch">
                <Switch checked={settings.autoStartMarkedProjects} onChange={(autoStartMarkedProjects) => patchSettings({ autoStartMarkedProjects })} />
              </SettingsRow>
            </SettingsGroup>
          </SettingsTabPanel>
        ) : null}

        {activeTab === "logging" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Logs">
              <SettingsRow title="Retained lines" detail="Maximum in-memory log history">
                <input
                  className="solo-settings-number"
                  type="number"
                  min={500}
                  max={50000}
                  step={500}
                  value={settings.logRetentionLines}
                  onChange={(event) => patchSettings({ logRetentionLines: clampNumber(event.target.valueAsNumber, 500, 50000, settings.logRetentionLines) })}
                />
              </SettingsRow>
              <SettingsRow title="Stop timeout" detail="Grace period before forced termination">
                <input
                  className="solo-settings-number"
                  type="number"
                  min={500}
                  max={30000}
                  step={500}
                  value={settings.stopTimeoutMs}
                  onChange={(event) => patchSettings({ stopTimeoutMs: clampNumber(event.target.valueAsNumber, 500, 30000, settings.stopTimeoutMs) })}
                />
              </SettingsRow>
            </SettingsGroup>
          </SettingsTabPanel>
        ) : null}

        {activeTab === "storage" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Project defaults">
              <SettingsRow title="Project storage path" detail={settings.projectStoragePath || "Default app config directory"}>
                <div className="solo-settings-actions">
                  <button type="button" onClick={chooseStoragePath}>
                    <FolderOpen size={14} />
                    Browse
                  </button>
                  <button type="button" onClick={() => patchSettings({ projectStoragePath: undefined })} title="Use default storage path">
                    <RotateCcw size={14} />
                  </button>
                </div>
              </SettingsRow>
            </SettingsGroup>
          </SettingsTabPanel>
        ) : null}

        {activeTab === "updates" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Updates">
              <SettingsRow title="Current version" detail={canUseAppUpdater ? "Desktop app" : "Browser preview"}>
                <span className="solo-settings-version">{appVersion}</span>
              </SettingsRow>
              <SettingsRow title="Update channel" detail="GitHub Releases">
                <div className="solo-settings-actions">
                  <button type="button" onClick={checkForUpdates} disabled={!canUseAppUpdater || updateBusy}>
                    <RefreshCw size={14} />
                    {updateStatus === "checking" ? "Checking" : "Check"}
                  </button>
                  {availableUpdate ? (
                    <button className="primary-action" type="button" onClick={installAvailableUpdate} disabled={updateBusy}>
                      <Download size={14} />
                      {updateStatus === "installing" ? "Installing" : "Install"}
                    </button>
                  ) : null}
                </div>
              </SettingsRow>
              {updateStatus === "installing" ? (
                <SettingsRow title="Download progress" detail={formatUpdateProgress(updateProgress)}>
                  <progress className="solo-settings-progress" max={100} value={updateProgress?.percent ?? 0} />
                </SettingsRow>
              ) : null}
            </SettingsGroup>
            {updateMessage ? <p className={updateStatus === "error" ? "solo-settings-error" : "solo-settings-note"}>{updateMessage}</p> : null}
          </SettingsTabPanel>
        ) : null}

        {activeTab === "config" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Config">
              <SettingsRow title="Redact secrets" detail="Hide token, secret, password, and key values on export">
                <Switch checked={redactSecrets} onChange={setRedactSecrets} />
              </SettingsRow>
              <SettingsRow title="Configuration file" detail="Export or import a portable JSON snapshot">
                <div className="solo-settings-actions">
                  <button type="button" onClick={downloadConfig}>
                    <Download size={14} />
                    Export
                  </button>
                  <button type="button" onClick={() => inputRef.current?.click()}>
                    <Upload size={14} />
                    Import
                  </button>
                  <input
                    ref={inputRef}
                    hidden
                    type="file"
                    accept="application/json"
                    onChange={(event) => {
                      const file = event.target.files?.[0];
                      event.target.value = "";
                      void readImport(file);
                    }}
                  />
                </div>
              </SettingsRow>
            </SettingsGroup>
            {importError ? <p className="solo-settings-error">{importError}</p> : null}
          </SettingsTabPanel>
        ) : null}

        {activeTab === "activity" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Activity">
              {activity.slice(0, 12).map((item) => (
                <ActivityRow key={item.id} item={item} />
              ))}
              {!activity.length ? <SettingsRow title="No activity yet" detail="Changes will appear here once workspace actions run" /> : null}
            </SettingsGroup>
          </SettingsTabPanel>
        ) : null}
      </section>
    </main>
  );
}

function SettingsTabPanel({ children }: { children: ReactNode }) {
  return <div className="solo-settings-panel">{children}</div>;
}

function SettingsGroup({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="solo-settings-group">
      <p className="solo-settings-group-label">{label}</p>
      <div className="solo-settings-card">{children}</div>
    </section>
  );
}

function SettingsRow({ title, detail, children }: { title: string; detail: string; children?: ReactNode }) {
  return (
    <div className="solo-settings-row">
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
      {children ? <div className="solo-settings-control">{children}</div> : null}
    </div>
  );
}

function SegmentedControl<T extends string>({
  value,
  options,
  onChange
}: {
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
}) {
  return (
    <div className="solo-settings-segmented">
      {options.map((option) => (
        <button key={option.value} className={option.value === value ? "active" : ""} type="button" onClick={() => onChange(option.value)}>
          {option.label}
        </button>
      ))}
    </div>
  );
}

function Switch({ checked, onChange }: { checked: boolean; onChange: (checked: boolean) => void }) {
  return (
    <label className="solo-settings-switch">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span aria-hidden="true" />
    </label>
  );
}

function ActivityRow({ item }: { item: ActivityEvent }) {
  return (
    <div className={`solo-settings-row solo-settings-activity ${item.level}`}>
      <span>
        <strong>{item.message}</strong>
        <small>{new Date(item.timestamp).toLocaleTimeString()}</small>
      </span>
    </div>
  );
}

function clampNumber(value: number, min: number, max: number, fallback: number) {
  if (!Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, value));
}

function formatUpdateProgress(progress?: UpdateProgress) {
  if (!progress) return "Waiting for download";
  if (progress.percent !== undefined) return `${progress.percent}% of ${formatBytes(progress.totalBytes ?? 0)}`;
  return `${formatBytes(progress.downloadedBytes)} downloaded`;
}

function formatBytes(bytes: number) {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value.toFixed(value >= 10 || index === 0 ? 0 : 1)} ${units[index]}`;
}
