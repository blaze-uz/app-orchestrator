import { useEffect } from "react";
import { AlertTriangle, Loader2 } from "lucide-react";
import { AppShell } from "./components/AppShell";
import { CommandPalette } from "./components/CommandPalette";
import { ConfirmProvider } from "./components/ConfirmDialog";
import { DashboardOverview } from "./features/dashboard/DashboardOverview";
import { LogsView } from "./features/logs/LogsView";
import { ProcessDetailPanel } from "./features/processes/ProcessDetailPanel";
import { ProjectDetailView } from "./features/projects/ProjectDetailView";
import { ProjectsView } from "./features/projects/ProjectsView";
import { SettingsView } from "./features/settings/SettingsView";
import { applyThemePreference, subscribeToSystemThemeChange } from "./lib/theme";
import { useOrchestratorStore } from "./stores/orchestratorStore";

function App() {
  const booted = useOrchestratorStore((state) => state.booted);
  const initialize = useOrchestratorStore((state) => state.initialize);
  const view = useOrchestratorStore((state) => state.view);
  const currentAction = useOrchestratorStore((state) => state.currentAction);
  const lastError = useOrchestratorStore((state) => state.lastError);
  const theme = useOrchestratorStore((state) => state.settings?.theme ?? "system");

  useEffect(() => {
    void initialize();
  }, [initialize]);

  useEffect(() => {
    applyThemePreference(theme);
    if (theme !== "system") return;
    return subscribeToSystemThemeChange(() => applyThemePreference("system"));
  }, [theme]);

  const content = {
    dashboard: <DashboardOverview />,
    projects: <ProjectsView />,
    project: <ProjectDetailView />,
    process: <ProcessDetailPanel />,
    logs: <LogsView />,
    settings: <SettingsView />
  }[view];

  return (
    <ConfirmProvider>
      <AppShell>
        {!booted ? (
          <main className="empty-state">
            <Loader2 className="spin" size={24} />
            <span>Loading local workspace</span>
          </main>
        ) : (
          content
        )}

        <CommandPalette />

        {currentAction ? (
          <div className="action-toast">
            <Loader2 className="spin" size={16} />
            <span>{currentAction.label}</span>
          </div>
        ) : null}

        {lastError ? (
          <div className="error-toast">
            <AlertTriangle size={16} />
            <span>{lastError}</span>
          </div>
        ) : null}
      </AppShell>
    </ConfirmProvider>
  );
}

export default App;
