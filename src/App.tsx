import { useEffect, useState } from "react";
import { AlertTriangle, Loader2, X } from "lucide-react";
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
  const dismissError = useOrchestratorStore((state) => state.dismissError);
  const theme = useOrchestratorStore((state) => state.settings?.theme ?? "system");
  const [showErrorDetails, setShowErrorDetails] = useState(false);

  useEffect(() => {
    if (!lastError) {
      setShowErrorDetails(false);
      return;
    }
    const timer = window.setTimeout(() => dismissError(), 8000);
    return () => window.clearTimeout(timer);
  }, [lastError, dismissError]);

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
          <div className="error-toast" role="alert">
            <AlertTriangle size={16} />
            <div className="error-toast-body">
              <div className="error-toast-headline">
                {lastError.code ? <span className="error-toast-code">{lastError.code}</span> : null}
                <span>{lastError.message}</span>
              </div>
              {lastError.details ? (
                <button
                  type="button"
                  className="error-toast-toggle"
                  onClick={() => setShowErrorDetails((prev) => !prev)}
                >
                  {showErrorDetails ? "Hide details" : "Show details"}
                </button>
              ) : null}
              {showErrorDetails && lastError.details ? <pre className="error-toast-details">{lastError.details}</pre> : null}
            </div>
            <button type="button" className="error-toast-close" onClick={dismissError} title="Dismiss" aria-label="Dismiss error">
              <X size={14} />
            </button>
          </div>
        ) : null}
      </AppShell>
    </ConfirmProvider>
  );
}

export default App;
