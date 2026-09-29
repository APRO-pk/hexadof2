/**
 * The application shell: top bar, command palette, and notifications.
 *
 * The shell exposes navigation and status only. Technical controls live in the
 * contextual panels of each workspace, because a top bar that accumulates
 * technical settings stops being readable.
 */

import { Component, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  BrandMark,
  IconAnalysis,
  IconBell,
  IconCommand,
  IconDashboard,
  IconDynamics,
  IconError,
  IconHelp,
  IconInfo,
  IconMoon,
  IconProjects,
  IconSettings,
  IconSun,
  IconTelemetry,
  IconWarning,
} from "./Icons";
import { Badge, Button, Hint, Notice } from "./ui";
import { HIDDEN_SECTIONS, useAdvancedMode, useStore, type Section } from "../lib/store";
import { formatRelative } from "../lib/format";

interface NavEntry {
  id: Section;
  label: string;
  icon: React.ReactNode;
  hint: string;
}

const NAV: NavEntry[] = [
  { id: "overview", label: "Overview", icon: <IconDashboard />, hint: "Project status and next steps" },
  { id: "dynamics", label: "Dynamics", icon: <IconDynamics />, hint: "Configure and run a 6-DOF simulation" },
  { id: "telemetry", label: "Live Telemetry", icon: <IconTelemetry />, hint: "Serial connection and recording" },
  { id: "analysis", label: "Flight Analysis", icon: <IconAnalysis />, hint: "Import, replay, and compare" },
  { id: "projects", label: "Projects", icon: <IconProjects />, hint: "Create and open projects" },
  { id: "settings", label: "Settings", icon: <IconSettings />, hint: "Application preferences" },
];

/**
 * The product name and the mode switch.
 *
 * They sit at the left of the bar, because the switch changes the whole
 * interface rather than the screen underneath it.
 */
function Brand() {
  const advanced = useAdvancedMode();
  const setAdvancedMode = useStore((s) => s.setAdvancedMode);

  return (
    <div className="brand">
      <BrandMark className="brand-mark" />
      <div className="col" style={{ gap: 0 }}>
        <span className="brand-name">HexaDOF</span>
        <span className="brand-sub">APRO Works</span>
      </div>
      <button
        type="button"
        className="mode-switch"
        role="switch"
        aria-checked={advanced}
        onClick={() => void setAdvancedMode(!advanced)}
        title={
          advanced
            ? "Advanced: every control is shown. Switch to Simple to keep only the basic workflow."
            : "Simple: solver, environment, hardware, sensor and storage controls are hidden. Switch to Advanced to see them."
        }
      >
        <span className={`mode-switch-track ${advanced ? "on" : ""}`} aria-hidden="true">
          <span className="mode-switch-thumb" />
        </span>
        <span className="mode-switch-label">{advanced ? "Advanced" : "Simple"}</span>
      </button>
    </div>
  );
}

/** The workspace tabs, centred in the top bar. */
function TopNav() {
  const section = useStore((s) => s.section);
  const setSection = useStore((s) => s.setSection);
  const savedRuns = useStore((s) => s.savedRuns);
  const flightSessions = useStore((s) => s.flightSessions);
  const telemetrySessions = useStore((s) => s.telemetrySessions);
  const advanced = useAdvancedMode();

  const counts: Partial<Record<Section, number>> = {
    dynamics: savedRuns.length,
    telemetry: telemetrySessions.length,
    analysis: flightSessions.length,
  };

  const visible = NAV.filter((entry) => advanced || !HIDDEN_SECTIONS.includes(entry.id));

  return (
    <nav className="topnav" aria-label="Workspaces">
      {visible.map((entry) => (
        <button
          key={entry.id}
          type="button"
          className="topnav-item"
          aria-current={section === entry.id ? "page" : undefined}
          onClick={() => setSection(entry.id)}
          title={entry.hint}
        >
          {entry.icon}
          {/* The label is dropped on a narrow window, so the title and the
              accessible name both keep the meaning. */}
          <span className="nav-label">{entry.label}</span>
          {counts[entry.id] !== undefined && counts[entry.id]! > 0 && (
            <span className="nav-badge">{counts[entry.id]}</span>
          )}
        </button>
      ))}
    </nav>
  );
}

/**
 * Keeps a fault in one screen from taking the whole window with it.
 *
 * Without a boundary a single unexpected value blanks the application, including
 * the navigation, so the only way out is to kill the process. The shell stays,
 * the screen is replaced by an explanation, and the reader can move to another
 * screen or try the same one again.
 */
class PageBoundary extends Component<
  { name: string; children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidUpdate(previous: { name: string }) {
    if (previous.name !== this.props.name && this.state.error) {
      this.setState({ error: null });
    }
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="page">
        <div className="panel">
          <header className="panel-header">
            <h2 className="panel-title">This screen stopped rendering</h2>
          </header>
          <div className="panel-body">
            <Notice
              level="error"
              title={`The ${this.props.name} screen failed`}
              detail={error.message || String(error)}
            />
            <Hint>
              The rest of the application is still running, and nothing in the
              open project was changed by this. Moving to another screen, or
              opening this one again, usually clears it. This is a defect worth
              reporting.
            </Hint>
            <div className="row">
              <Button variant="primary" onClick={() => this.setState({ error: null })}>
                Try again
              </Button>
            </div>
          </div>
        </div>
      </div>
    );
  }
}

function TopBar() {
  const project = useStore((s) => s.project);
  const theme = useStore((s) => s.theme);
  const toggleTheme = useStore((s) => s.toggleTheme);
  const setPaletteOpen = useStore((s) => s.setPaletteOpen);
  const running = useStore((s) => s.running);
  const progress = useStore((s) => s.progress);
  const telemetryStatus = useStore((s) => s.telemetryStatus);
  const settingsWarnings = useStore((s) => s.settingsWarnings);
  const saveProject = useStore((s) => s.saveProject);
  const toasts = useStore((s) => s.toasts);

  return (
    <header className="app-topbar">
      <div className="topbar-left">
        <Brand />
      </div>

      <TopNav />

      <div className="topbar-right">
        {/* The project name only: the full path is long, and it is one tooltip
            away here and spelled out on the Overview screen. */}
        <div className="topbar-project">
          <span className="topbar-project-name truncate" title={project?.root ?? ""}>
            {project ? project.name : "No project is open"}
          </span>
        </div>

        {project?.dirty && (
          <span className="row" title="The project has unsaved changes">
            <span className="dirty-dot" />
            <span className="meta">Unsaved</span>
          </span>
        )}

        {running && (
          <Badge tone="accent">
            Running {progress ? `${Math.round(progress.fraction * 100)}%` : ""}
          </Badge>
        )}

        <span className="row" title={telemetryStatus?.connected ? telemetryStatus.port : "No device connected"}>
          <span
            className="dirty-dot"
            style={{
              background: telemetryStatus?.connected
                ? telemetryStatus.recording
                  ? "var(--error)"
                  : "var(--success)"
                : "var(--text-muted)",
            }}
          />
        {/* The device name sits beside the dot only when the bar has room; the
            dot and the tooltip carry the state either way. */}
        <span className="meta topbar-device-label">
          {telemetryStatus?.connected
            ? `${telemetryStatus.port} ${telemetryStatus.recording ? "recording" : "connected"}`
            : "No device"}
        </span>
      </span>

      {project && (
        <Button size="small" onClick={() => void saveProject()} title="Write the project file">
          Save
        </Button>
      )}

      <Button
        size="icon"
        variant="ghost"
        icon={<IconCommand />}
        onClick={() => setPaletteOpen(true)}
        title="Open the command palette (Ctrl and K)"
        aria-label="Command palette"
      />

      <Button
        size="icon"
        variant="ghost"
        icon={theme === "dark" ? <IconSun /> : <IconMoon />}
        onClick={() => void toggleTheme()}
        title={theme === "dark" ? "Switch to the light theme" : "Switch to the dark theme"}
        aria-label="Toggle theme"
      />

      {/* The count sits on the bell rather than beside it, so a notification
          never widens the cluster and never pushes it under the tabs. */}
      <span
        className="bell"
        title={
          toasts.length > 0
            ? `${toasts.length} recent notification(s)`
            : settingsWarnings.length > 0
              ? `${settingsWarnings.length} settings warning(s)`
              : "No new notifications"
        }
      >
        <IconBell style={{ width: 16, height: 16, color: "var(--text-secondary)" }} />
        {toasts.length + settingsWarnings.length > 0 && (
          <span className="nav-badge bell-count">{toasts.length + settingsWarnings.length}</span>
        )}
      </span>

      <Button
        size="icon"
        variant="ghost"
        icon={<IconHelp />}
        title="HexaDOF simulates, observes, records, replays, and compares six-degree-of-freedom motion. It is an analysis and instrumentation tool, and its results do not certify flight safety. Press Ctrl and K for the command list."
        aria-label="Help"
      />
      </div>
    </header>
  );
}

interface PaletteAction {
  id: string;
  label: string;
  group: string;
  icon: React.ReactNode;
  run: () => void | Promise<void>;
  /**
   * The mode this command needs, when it drives a control the mode hides.
   *
   * A palette that offers an action the interface cannot carry out is a lie, so
   * a `simple` action is filtered out of Simple mode, and an `advanced` action
   * is offered only in Advanced mode.
   */
  mode?: "simple" | "advanced";
}

/** The command palette. */
function CommandPalette() {
  const open = useStore((s) => s.paletteOpen);
  const setOpen = useStore((s) => s.setPaletteOpen);
  const advanced = useAdvancedMode();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const store = useStore.getState;

  const actions = useMemo<PaletteAction[]>(() => {
    const s = store();
    return [
      {
        id: "new-project",
        label: "Create a new project",
        group: "Project",
        icon: <IconProjects />,
        run: () => s.setSection("projects"),
      },
      {
        id: "open-project",
        label: "Open a project",
        group: "Project",
        icon: <IconProjects />,
        run: () => s.setSection("projects"),
      },
      {
        id: "save-project",
        label: "Save the project",
        group: "Project",
        icon: <IconProjects />,
        run: () => s.saveProject(),
      },
      {
        id: "import-model",
        label: "Import a dynamics model",
        group: "Dynamics",
        icon: <IconDynamics />,
        run: () => s.setSection("dynamics"),
      },
      {
        id: "example-model",
        label: "Import the built-in example model",
        group: "Dynamics",
        icon: <IconDynamics />,
        run: () => s.importExampleModel(),
      },
      {
        id: "validate",
        label: "Run scenario validation",
        group: "Dynamics",
        icon: <IconDynamics />,
        run: () => s.validateScenario(),
      },
      {
        id: "run",
        label: "Start a simulation",
        group: "Dynamics",
        icon: <IconDynamics />,
        run: () => s.runSimulation(false),
      },
      {
        id: "cancel",
        label: "Stop the running simulation",
        group: "Dynamics",
        icon: <IconDynamics />,
        run: () => s.cancelSimulation(),
      },
      {
        id: "connect",
        label: "Connect a device",
        group: "Telemetry",
        icon: <IconTelemetry />,
        run: () => s.setSection("telemetry"),
        mode: "advanced",
      },
      {
        id: "scripted",
        label: "Attach the synthetic stream",
        group: "Telemetry",
        icon: <IconTelemetry />,
        run: () => s.connectScripted(),
        mode: "advanced",
      },
      {
        id: "record",
        label: "Start recording",
        group: "Telemetry",
        icon: <IconTelemetry />,
        run: () => s.startRecording(),
        mode: "advanced",
      },
      {
        id: "stop-record",
        label: "Stop recording and save",
        group: "Telemetry",
        icon: <IconTelemetry />,
        run: () => s.stopRecording("Telemetry session"),
        mode: "advanced",
      },
      {
        id: "import-flight",
        label: "Import a flight log",
        group: "Analysis",
        icon: <IconAnalysis />,
        run: () => s.setSection("analysis"),
      },
      {
        id: "compare",
        label: "Build a comparison",
        group: "Analysis",
        icon: <IconAnalysis />,
        run: () => s.buildComparison(),
      },
      {
        id: "marker",
        label: "Add an event marker",
        group: "Analysis",
        icon: <IconAnalysis />,
        run: () => s.setSection("analysis"),
      },
      {
        id: "report",
        label: "Export a report",
        group: "Analysis",
        icon: <IconAnalysis />,
        run: () => s.exportReport("HexaDOF report"),
      },
      {
        id: "theme",
        label: "Toggle the theme",
        group: "Appearance",
        icon: <IconSun />,
        run: () => s.toggleTheme(),
      },
      {
        id: "settings",
        label: "Open settings",
        group: "Appearance",
        icon: <IconSettings />,
        run: () => s.setSection("settings"),
      },
      {
        id: "mode-advanced",
        label: "Switch to Advanced mode",
        group: "Appearance",
        icon: <IconSettings />,
        run: () => s.setAdvancedMode(true),
        mode: "simple",
      },
      {
        id: "mode-simple",
        label: "Switch to Simple mode",
        group: "Appearance",
        icon: <IconSettings />,
        run: () => s.setAdvancedMode(false),
        mode: "advanced",
      },
    ];
  }, [store, open]);

  const filtered = useMemo(() => {
    // Commands that drive a control the mode hides are not offered.
    const available = actions.filter((action) =>
      action.mode === undefined ? true : action.mode === (advanced ? "advanced" : "simple"),
    );
    const needle = query.trim().toLowerCase();
    if (!needle) return available;
    return available.filter(
      (a) => a.label.toLowerCase().includes(needle) || a.group.toLowerCase().includes(needle),
    );
  }, [actions, query, advanced]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      // Focus after the overlay has mounted so the input is in the DOM.
      window.setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  useEffect(() => setActive(0), [query]);

  if (!open) return null;

  const grouped = filtered.reduce<Record<string, PaletteAction[]>>((acc, action) => {
    acc[action.group] = acc[action.group] ?? [];
    acc[action.group].push(action);
    return acc;
  }, {});

  const flat = Object.values(grouped).flat();

  return (
    <div
      className="overlay"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) setOpen(false);
      }}
    >
      <div className="modal" role="dialog" aria-modal="true" aria-label="Command palette">
        <input
          ref={inputRef}
          className="palette-input"
          placeholder="Type a command, then press Enter"
          value={query}
          aria-label="Search commands"
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setOpen(false);
            } else if (event.key === "ArrowDown") {
              event.preventDefault();
              setActive((i) => Math.min(flat.length - 1, i + 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((i) => Math.max(0, i - 1));
            } else if (event.key === "Enter") {
              event.preventDefault();
              const action = flat[active];
              if (action) {
                setOpen(false);
                void action.run();
              }
            }
          }}
        />
        <div className="palette-list">
          {Object.entries(grouped).map(([group, entries]) => (
            <div key={group}>
              <div className="palette-group">{group}</div>
              {entries.map((action) => {
                const index = flat.indexOf(action);
                return (
                  <button
                    key={action.id}
                    type="button"
                    className="palette-item"
                    data-active={index === active}
                    onMouseEnter={() => setActive(index)}
                    onClick={() => {
                      setOpen(false);
                      void action.run();
                    }}
                  >
                    {action.icon}
                    <span>{action.label}</span>
                  </button>
                );
              })}
            </div>
          ))}
          {flat.length === 0 && (
            <div className="empty">
              <span className="empty-title">No command matches</span>
              <span className="empty-detail">
                Try a shorter search, or a workspace name such as Dynamics or Telemetry.
              </span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Notification toasts. Errors stay longer, because they need reading. */
function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="toast-stack" role="status" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast toast-${toast.level}`}>
          <span style={{ marginTop: 2 }}>
            {toast.level === "error" ? (
              <IconError style={{ width: 16, height: 16, color: "var(--error)" }} />
            ) : toast.level === "warning" ? (
              <IconWarning style={{ width: 16, height: 16, color: "var(--warning)" }} />
            ) : (
              <IconInfo style={{ width: 16, height: 16, color: "var(--info)" }} />
            )}
          </span>
          <div className="grow">
            <div className="toast-title">{toast.title}</div>
            <div className="toast-detail">{toast.detail}</div>
          </div>
          <button
            type="button"
            className="toast-close"
            onClick={() => dismiss(toast.id)}
            aria-label="Dismiss notification"
          >
            x
          </button>
        </div>
      ))}
    </div>
  );
}

/** The persistent error panel, which keeps the last failure visible with its fix. */
export function ErrorBanner() {
  const error = useStore((s) => s.lastError);
  const clear = useStore((s) => s.clearError);
  const [showTechnical, setShowTechnical] = useState(false);
  if (!error) return null;
  return (
    <div
      className={`notice notice-${error.severity === "warning" ? "warning" : "error"}`}
      style={{ margin: "0 var(--space-3) var(--space-3)" }}
      role="alert"
    >
      <IconError />
      <div className="grow">
        <div className="notice-title">{error.title}</div>
        <div className="notice-detail">{error.detail}</div>
        {error.suggestion && <div className="issue-suggestion">{error.suggestion}</div>}
        {error.technical && (
          <>
            <button type="button" className="disclosure" onClick={() => setShowTechnical((v) => !v)}>
              {showTechnical ? "Hide technical detail" : "Show technical detail"}
            </button>
            {showTechnical && <pre className="issue-technical">{error.technical}</pre>}
          </>
        )}
        <div className="meta" style={{ marginTop: 4 }}>
          Code {error.code}
        </div>
      </div>
      <Button size="small" variant="ghost" onClick={clear}>
        Dismiss
      </Button>
    </div>
  );
}

/** Wrap a page so the shell is applied consistently. */
export function Shell({ children }: { children: React.ReactNode }) {
  const setPaletteOpen = useStore((s) => s.setPaletteOpen);
  const paletteOpen = useStore((s) => s.paletteOpen);
  const section = useStore((s) => s.section);
  const setSection = useStore((s) => s.setSection);

  // Global shortcuts. The palette shortcut is the one the product specifies.
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen(!paletteOpen);
      } else if (event.key === "Escape" && paletteOpen) {
        setPaletteOpen(false);
      } else if ((event.ctrlKey || event.metaKey) && event.key >= "1" && event.key <= "6") {
        event.preventDefault();
        const index = Number(event.key) - 1;
        if (NAV[index]) setSection(NAV[index].id);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [paletteOpen, setPaletteOpen, setSection]);

  return (
    <div className="app-shell">
      <TopBar />
      <main className="app-main" id={`workspace-${section}`}>
        <ErrorBanner />
        <PageBoundary name={NAV.find((entry) => entry.id === section)?.label ?? section}>
          {children}
        </PageBoundary>
      </main>
      <CommandPalette />
      <Toasts />
    </div>
  );
}

/** The status of the project, for the overview screen and the palette. */
export function useProjectStatusLine(): string {
  const project = useStore((s) => s.project);
  if (!project) return "No project is open";
  const updated = formatRelative(project.updated_at);
  return `${project.name}, updated ${updated}`;
}
