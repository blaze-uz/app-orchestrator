import { useEffect, useMemo, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { Activity, Columns3, FolderKanban, LayoutDashboard, Play, RotateCcw, Search, Settings, Square, TerminalSquare } from "lucide-react";
import { formatPath } from "../lib/time";
import { useOrchestratorStore } from "../stores/orchestratorStore";
import { useConfirm } from "./ConfirmDialog";

interface PaletteCommand {
  id: string;
  label: string;
  description: string;
  section: string;
  icon: LucideIcon;
  run: () => Promise<void> | void;
}

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const projects = useOrchestratorStore((state) => state.projects);
  const processes = useOrchestratorStore((state) => state.processes);
  const selectedProjectId = useOrchestratorStore((state) => state.selectedProjectId);
  const selectView = useOrchestratorStore((state) => state.selectView);
  const selectProject = useOrchestratorStore((state) => state.selectProject);
  const selectProcess = useOrchestratorStore((state) => state.selectProcess);
  const startProject = useOrchestratorStore((state) => state.startProject);
  const stopProject = useOrchestratorStore((state) => state.stopProject);
  const restartProject = useOrchestratorStore((state) => state.restartProject);
  const startProcess = useOrchestratorStore((state) => state.startProcess);
  const stopProcess = useOrchestratorStore((state) => state.stopProcess);
  const restartProcess = useOrchestratorStore((state) => state.restartProcess);
  const restartFailed = useOrchestratorStore((state) => state.restartFailed);
  const confirm = useConfirm();

  const selectedProject = projects.find((project) => project.id === selectedProjectId);

  useEffect(() => {
    const openPalette = () => setOpen(true);
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setOpen((current) => !current);
      }
      if (event.key === "Escape") {
        setOpen(false);
      }
    };

    window.addEventListener("open-command-palette", openPalette);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("open-command-palette", openPalette);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    window.requestAnimationFrame(() => inputRef.current?.focus());
  }, [open]);

  const commands = useMemo<PaletteCommand[]>(() => {
    const workspaceCommands: PaletteCommand[] = [
      {
        id: "nav-dashboard",
        label: "Open status",
        description: "Project and process runtime state",
        section: "Navigation",
        icon: LayoutDashboard,
        run: () => selectView("dashboard")
      },
      {
        id: "nav-projects",
        label: "Open projects",
        description: "Create and manage local project groups",
        section: "Navigation",
        icon: Columns3,
        run: () => selectView("projects")
      },
      {
        id: "nav-logs",
        label: "Open logs",
        description: "Search and tail process output",
        section: "Navigation",
        icon: TerminalSquare,
        run: () => selectView("logs")
      },
      {
        id: "nav-settings",
        label: "Open settings",
        description: "Startup, retention, import, and export",
        section: "Navigation",
        icon: Settings,
        run: () => selectView("settings")
      },
      {
        id: "workspace-start-all",
        label: "Start all projects",
        description: "Start every configured project in this workspace",
        section: "Workspace",
        icon: Play,
        run: async () => {
          for (const project of projects) await startProject(project.id);
        }
      },
      {
        id: "workspace-stop-all",
        label: "Stop all projects",
        description: "Stop every configured process after confirmation",
        section: "Workspace",
        icon: Square,
        run: async () => {
          const ok = await confirm({
            title: "Stop all project processes?",
            confirmLabel: "Stop all",
            danger: true,
          });
          if (!ok) return;
          for (const project of projects) await stopProject(project.id);
        }
      },
      {
        id: "workspace-restart-failed",
        label: "Restart failed processes",
        description: "Recover crashed or failed runtime units",
        section: "Workspace",
        icon: RotateCcw,
        run: () => restartFailed()
      }
    ];

    const currentProjectCommands: PaletteCommand[] = selectedProject
      ? [
          {
            id: `project-start-${selectedProject.id}`,
            label: `Start ${selectedProject.name}`,
            description: formatPath(selectedProject.rootPath),
            section: "Current project",
            icon: Play,
            run: () => startProject(selectedProject.id)
          },
          {
            id: `project-stop-${selectedProject.id}`,
            label: `Stop ${selectedProject.name}`,
            description: "Stop every process in the selected project",
            section: "Current project",
            icon: Square,
            run: async () => {
              const ok = await confirm({
                title: `Stop all processes in ${selectedProject.name}?`,
                confirmLabel: "Stop all",
                danger: true,
              });
              if (ok) await stopProject(selectedProject.id);
            }
          },
          {
            id: `project-restart-${selectedProject.id}`,
            label: `Restart ${selectedProject.name}`,
            description: "Restart all selected project processes",
            section: "Current project",
            icon: RotateCcw,
            run: () => restartProject(selectedProject.id)
          }
        ]
      : [];

    const projectCommands = projects.map<PaletteCommand>((project) => ({
      id: `select-project-${project.id}`,
      label: `Go to ${project.name}`,
      description: formatPath(project.rootPath),
      section: "Projects",
      icon: FolderKanban,
      run: () => selectProject(project.id)
    }));

    const processCommands = processes.flatMap<PaletteCommand>((process) => {
      const description = `${process.command} ${process.args.join(" ")}`.trim();
      return [
        {
          id: `open-process-${process.id}`,
          label: `Open ${process.name}`,
          description,
          section: "Processes",
          icon: Activity,
          run: () => selectProcess(process.id)
        },
        {
          id: `start-process-${process.id}`,
          label: `Start ${process.name}`,
          description,
          section: "Processes",
          icon: Play,
          run: async () => {
            selectProcess(process.id);
            await startProcess(process.id);
          }
        },
        {
          id: `stop-process-${process.id}`,
          label: `Stop ${process.name}`,
          description,
          section: "Processes",
          icon: Square,
          run: async () => {
            selectProcess(process.id);
            await stopProcess(process.id);
          }
        },
        {
          id: `restart-process-${process.id}`,
          label: `Restart ${process.name}`,
          description,
          section: "Processes",
          icon: RotateCcw,
          run: async () => {
            selectProcess(process.id);
            await restartProcess(process.id);
          }
        }
      ];
    });

    return [...workspaceCommands, ...currentProjectCommands, ...projectCommands, ...processCommands];
  }, [processes, projects, restartFailed, restartProcess, restartProject, selectProcess, selectProject, selectView, selectedProject, startProcess, startProject, stopProcess, stopProject]);

  const filteredCommands = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return commands;
    return commands.filter((command) => `${command.section} ${command.label} ${command.description}`.toLowerCase().includes(normalizedQuery));
  }, [commands, query]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query, open]);

  const runCommand = async (command?: PaletteCommand) => {
    if (!command) return;
    await command.run();
    setOpen(false);
    setQuery("");
  };

  const onInputKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (!filteredCommands.length) return;
      setActiveIndex((current) => Math.min(current + 1, filteredCommands.length - 1));
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      if (!filteredCommands.length) return;
      setActiveIndex((current) => Math.max(current - 1, 0));
    }
    if (event.key === "Enter") {
      event.preventDefault();
      void runCommand(filteredCommands[activeIndex]);
    }
  };

  if (!open) return null;

  return (
    <div className="command-palette-backdrop" role="presentation" onMouseDown={() => setOpen(false)}>
      <section className="command-palette" role="dialog" aria-modal="true" aria-label="Command palette" onMouseDown={(event) => event.stopPropagation()}>
        <label className="palette-search">
          <Search size={18} />
          <input ref={inputRef} value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={onInputKeyDown} placeholder="Jump to a project, start a stack, or open logs" />
          <kbd>Esc</kbd>
        </label>
        <div className="palette-results">
          {filteredCommands.map((command, index) => {
            const Icon = command.icon;
            return (
              <button
                key={command.id}
                className={index === activeIndex ? "active" : ""}
                type="button"
                onMouseEnter={() => setActiveIndex(index)}
                onClick={() => void runCommand(command)}
              >
                <Icon size={18} />
                <span>
                  <strong>{command.label}</strong>
                  <small>{command.description}</small>
                </span>
                <em>{command.section}</em>
              </button>
            );
          })}
          {!filteredCommands.length ? <p className="palette-empty">No matching command.</p> : null}
        </div>
        <footer className="palette-footer">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd>
            navigate
          </span>
          <span>
            <kbd>↵</kbd>
            select
          </span>
          <span>
            <kbd>esc</kbd>
            close
          </span>
        </footer>
      </section>
    </div>
  );
}
