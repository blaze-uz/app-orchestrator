import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { Download, FolderOpen, RotateCcw, Upload } from "lucide-react";
import { getLaunchOnLoginEnabled, setLaunchOnLoginEnabled } from "../../lib/autostart";
import { selectFolder } from "../../lib/folderPicker";
import { ensureNotificationPermission } from "../../lib/notifications";
import { useOrchestratorStore } from "../../stores/orchestratorStore";
import type { ActivityEvent, AppConfig, AppSettings } from "../../types/domain";

type SettingsTab = "appearance" | "notifications" | "automation" | "logging" | "storage" | "config" | "activity";

const settingsTabs: Array<{ key: SettingsTab; label: string }> = [
  { key: "appearance", label: "Appearance" },
  { key: "notifications", label: "Notifications" },
  { key: "automation", label: "Automation" },
  { key: "logging", label: "Logging" },
  { key: "storage", label: "Storage" },
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
  const selectView = useOrchestratorStore((state) => state.selectView);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const autostartSyncedRef = useRef(false);
  const [activeTab, setActiveTab] = useState<SettingsTab>("appearance");
  const [redactSecrets, setRedactSecrets] = useState(true);
  const [importError, setImportError] = useState<string>();
  const [integrationError, setIntegrationError] = useState<string>();

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") selectView("dashboard");
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectView]);

  useEffect(() => {
    if (!settings || autostartSyncedRef.current) return;
    autostartSyncedRef.current = true;
    getLaunchOnLoginEnabled()
      .then((launchOnLogin) => {
        if (launchOnLogin !== settings.launchOnLogin) void updateSettings({ ...settings, launchOnLogin });
      })
      .catch(() => undefined);
  }, [settings, updateSettings]);

  if (!settings) {
    return (
      <main className="empty-state">
        <span>Settings unavailable.</span>
      </main>
    );
  }

  const patchSettings = async (patch: Partial<AppSettings>) => {
    setIntegrationError(undefined);
    await updateSettings({ ...settings, ...patch });
  };

  const toggleLaunchOnLogin = async (launchOnLogin: boolean) => {
    setIntegrationError(undefined);
    try {
      await setLaunchOnLoginEnabled(launchOnLogin);
      await patchSettings({ launchOnLogin });
    } catch (error) {
      setIntegrationError(error instanceof Error ? error.message : "Unable to update launch on login");
    }
  };

  const toggleNotifications = async (notificationsEnabled: boolean) => {
    setIntegrationError(undefined);
    if (notificationsEnabled && !(await ensureNotificationPermission())) {
      setIntegrationError("Notification permission was not granted.");
      return;
    }
    await patchSettings({ notificationsEnabled });
  };

  const downloadConfig = async () => {
    const content = await exportConfig(redactSecrets);
    const blob = new Blob([content], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "app-orchestrator.config.json";
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

  return (
    <main className="page solo-settings-page">
      <header className="solo-settings-titlebar">
        <div className="solo-settings-title">
          <h2>Settings</h2>
          <span>{projects.length} projects</span>
        </div>
        <span className="solo-settings-esc" title="Press Escape to return to dashboard">
          ESC
        </span>
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
              <SettingsRow title="Process alerts" detail="Crash, exit, and health check notifications">
                <Switch checked={settings.notificationsEnabled} onChange={toggleNotifications} />
              </SettingsRow>
            </SettingsGroup>
            {integrationError ? <p className="solo-settings-error">{integrationError}</p> : null}
          </SettingsTabPanel>
        ) : null}

        {activeTab === "automation" ? (
          <SettingsTabPanel>
            <SettingsGroup label="Application">
              <SettingsRow title="Launch app on login" detail="Open the orchestrator when macOS starts">
                <Switch checked={settings.launchOnLogin} onChange={toggleLaunchOnLogin} />
              </SettingsRow>
              <SettingsRow title="Start marked projects" detail="Auto-start projects marked for launch">
                <Switch checked={settings.autoStartMarkedProjects} onChange={(autoStartMarkedProjects) => patchSettings({ autoStartMarkedProjects })} />
              </SettingsRow>
            </SettingsGroup>
            {integrationError ? <p className="solo-settings-error">{integrationError}</p> : null}
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
