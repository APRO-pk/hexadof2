/**
 * The application store.
 *
 * The store holds what a screen needs to survive being navigated away from: the
 * open project, the imported model, the scenario being edited, the loaded run, the
 * live telemetry state, the imported flight session, and the comparison. Commands
 * are issued from here so a page never talks to the backend directly and two pages
 * cannot disagree about the same value.
 */

import { create } from "zustand";
import * as api from "./api";
import { exampleFlight } from "./examples";
import {
  type CommandError,
  type ComparableChannel,
  type Comparison,
  type ComparisonRequest,
  type DeviceProfile,
  type DiscoveredProject,
  type EventTimelineView,
  type FlightChannelSummary,
  type FlightPreview,
  type FlightSessionSummary,
  type FlightSeries,
  type LiveFrame,
  type ModelView,
  type NoticeEvent,
  type PortView,
  type ProjectView,
  type ReplayData,
  type RunResultView,
  type SavedRunSummary,
  type SavedSessionSummary,
  type ScenarioRequest,
  type ScenarioValidation,
  type SensorValidation,
  type Settings,
  type SimulationProgressEvent,
  type TelemetryHealthEvent,
  type TelemetryStatus,
  type TimestampReport,
} from "./types";

export type Section =
  | "overview"
  | "dynamics"
  | "telemetry"
  | "analysis"
  | "projects"
  | "settings";

export interface Toast extends NoticeEvent {
  id: number;
  at: number;
}

/**
 * The sections Simple mode does not offer.
 *
 * Live telemetry is the hardware console: device profiles, packet formats,
 * channel mapping, calibration, and estimator settings. It is the deepest
 * instrumentation surface in the application and it only means anything with a
 * device attached, so it is the one section kept out of the reduced interface.
 * Every other screen stays, with its advanced controls hidden.
 */
export const HIDDEN_SECTIONS: Section[] = ["telemetry"];

/** Whether the interface is showing the full surface. */
export function useAdvancedMode(): boolean {
  return useStore((s) => s.settings?.general.advanced_mode ?? true);
}

/** The default scenario the Dynamics screen opens with. */
export function defaultScenario(): ScenarioRequest {
  return {
    name: "Vertical flight",
    description: "",
    mode: "six_dof",
    start_time: 0,
    end_time: 8,
    output_interval: 0.01,
    step: 0.0005,
    relative_tolerance: null,
    absolute_tolerance: null,
    position: [0, 0, 0],
    velocity: [0, 0, 0],
    euler: [0, -Math.PI / 2, 0],
    angular_velocity: [0, 0, 0],
    mass: 4.5,
    inertia: [0.09, 0.09, 0.012, 0, 0, 0],
    gravity: 9.80665,
    standard_atmosphere: true,
    air_density: 1.225,
    wind_speed: 0,
    wind_direction: [1, 0, 0],
    rail_length: 1.5,
    gravity_enabled: true,
    thrust: 180,
    burn_time: 2.4,
    thrust_application_point: [0, 0, 0],
    drag_coefficient: 0.45,
    // `auto` means the imported model's coefficients when it declares them, and
    // the built-in drag estimate when it does not.
    aero_source: "auto",
    lift_slope: 2,
    pitch_damping: -8,
    control_moment: null,
    control_duration: 0.5,
    ground_elevation: 0,
    save: false,
  };
}

/** The channels the plot panel starts with. */
export function defaultPlotChannels(): string[] {
  return [
    "altitude",
    "vertical_velocity",
    "speed",
    "acceleration",
    "roll",
    "pitch",
    "yaw",
    "angular_rate_x",
    "angular_rate_y",
    "angular_rate_z",
    "force",
    "moment",
    "angle_of_attack",
    "sideslip",
    "dynamic_pressure",
    "mass",
  ];
}

/** The default comparison request. */
export function defaultComparisonRequest(): ComparisonRequest {
  return {
    method: "first_sample",
    event_code: "apogee",
    offset_seconds: 0,
    correlation_half_width: 1,
    correlation_step: 0.01,
    channels: [],
    minimum_overlap_seconds: 0.1,
    attitude_warning_degrees: 10,
  };
}

interface AppState {
  /* navigation */
  section: Section;
  setSection: (section: Section) => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;

  /* notifications */
  toasts: Toast[];
  pushToast: (notice: NoticeEvent) => void;
  dismissToast: (id: number) => void;
  pushError: (error: CommandError) => void;
  lastError: CommandError | null;
  clearError: () => void;

  /* project */
  project: ProjectView | null;
  recentProjects: string[];
  discovered: DiscoveredProject[];
  busy: string | null;
  refreshProject: () => Promise<void>;
  createProject: (directory: string, name: string) => Promise<boolean>;
  openProject: (directory: string) => Promise<boolean>;
  saveProject: () => Promise<void>;
  discoverProjects: (directory: string) => Promise<void>;
  updateProject: (name?: string, description?: string) => Promise<void>;
  loadRecentProjects: () => Promise<void>;
  deleteRun: (name: string) => Promise<void>;
  deleteTelemetrySession: (name: string) => Promise<void>;
  deleteFlightSession: (name: string) => Promise<void>;

  /* settings */
  settings: Settings | null;
  settingsWarnings: string[];
  loadSettings: () => Promise<void>;
  saveSettings: (value: Settings) => Promise<boolean>;
  setAdvancedMode: (advanced: boolean) => Promise<void>;
  resetSettings: () => Promise<void>;

  /* model */
  models: ModelView[];
  activeModel: ModelView | null;
  loadModels: () => Promise<void>;
  importModelFromPath: (path: string, unitSystem?: string) => Promise<boolean>;
  importExampleModel: () => Promise<boolean>;

  /* scenario and simulation */
  scenario: ScenarioRequest;
  setScenario: (patch: Partial<ScenarioRequest>) => void;
  resetScenario: () => void;
  loadExampleFlight: (id: string) => Promise<boolean>;
  validation: ScenarioValidation | null;
  validateScenario: () => Promise<void>;
  runResult: RunResultView | null;
  running: boolean;
  progress: SimulationProgressEvent | null;
  setProgress: (progress: SimulationProgressEvent) => void;
  runSimulation: (save: boolean) => Promise<boolean>;
  cancelSimulation: () => Promise<void>;
  replay: ReplayData | null;
  loadReplay: () => Promise<void>;
  savedRuns: SavedRunSummary[];
  loadSavedRuns: () => Promise<void>;
  openSavedRun: (name: string) => Promise<boolean>;
  addMarker: (time: number, label: string) => Promise<void>;
  plotChannels: string[];
  setPlotChannels: (channels: string[]) => void;

  /* telemetry */
  ports: PortView[];
  refreshPorts: () => Promise<void>;
  telemetryStatus: TelemetryStatus | null;
  telemetryHealth: TelemetryHealthEvent | null;
  liveFrames: LiveFrame[];
  pollTelemetry: () => Promise<void>;
  connectPort: (portName: string, baudRate: number) => Promise<boolean>;
  connectScripted: () => Promise<boolean>;
  disconnectPort: () => Promise<void>;
  startRecording: () => Promise<void>;
  stopRecording: (label: string) => Promise<void>;
  sensorValidation: SensorValidation | null;
  runSensorValidation: (orientation?: string, expectedRate?: number) => Promise<void>;
  deviceProfiles: DeviceProfile[];
  loadProfiles: () => Promise<void>;
  saveProfile: (profile: DeviceProfile) => Promise<void>;
  telemetrySessions: SavedSessionSummary[];
  loadTelemetrySessions: () => Promise<void>;
  scriptedStream: boolean;

  /* flight */
  flightPreview: FlightPreview | null;
  flightPreviewPath: string | null;
  previewFlight: (path: string) => Promise<void>;
  timestampReport: TimestampReport | null;
  validateFlight: (path: string) => Promise<void>;
  flightSession: FlightSessionSummary | null;
  flightChannels: FlightChannelSummary[];
  importFlight: (path: string, label?: string, createSession?: boolean) => Promise<boolean>;
  refreshFlightAnalysis: () => Promise<void>;
  flightSeries: FlightSeries | null;
  loadFlightSeries: (name: string) => Promise<void>;
  flightTimeline: EventTimelineView | null;
  detectFlightEvents: () => Promise<void>;
  addFlightMarker: (time: number, label: string) => Promise<void>;
  flightReplay: ReplayData | null;
  loadFlightReplay: () => Promise<void>;
  flightSessions: SavedSessionSummary[];
  loadFlightSessions: () => Promise<void>;

  /* comparison */
  comparison: Comparison | null;
  comparableChannels: ComparableChannel[];
  comparisonRequest: ComparisonRequest;
  setComparisonRequest: (patch: Partial<ComparisonRequest>) => void;
  buildComparison: () => Promise<void>;
  loadComparableChannels: () => Promise<void>;
  comparisonMethods: [string, string, boolean][];
  loadComparisonMethods: () => Promise<void>;
  exportReport: (title: string) => Promise<void>;

  /* theme */
  theme: "dark" | "light";
  systemPrefersDark: boolean;
  setThemePreference: (theme: Settings["appearance"]["theme"]) => Promise<void>;
  applyAppearance: () => void;
  toggleTheme: () => Promise<void>;
}

let toastCounter = 0;

/** Read the operating system colour scheme preference. */
function readSystemPrefersDark(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return true;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/** Apply the theme, density, and reduced-motion attributes to the document root. */
function applyDocumentAttributes(
  theme: "dark" | "light",
  density: string,
  reducedMotion: boolean,
): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.dataset.theme = theme;
  root.dataset.density = density === "compact" ? "compact" : "comfortable";
  root.dataset.reducedMotion = reducedMotion ? "true" : "false";
}

export const useStore = create<AppState>((set, get) => ({
  /* ------------------------------------------------------------ navigation */
  section: "overview",
  setSection: (section) => set({ section }),
  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),

  /* --------------------------------------------------------- notifications */
  toasts: [],
  pushToast: (notice) => {
    const toast: Toast = { ...notice, id: (toastCounter += 1), at: Date.now() };
    set((state) => ({ toasts: [...state.toasts, toast].slice(-6) }));
    // A notice is transient; the error panel is what persists.
    const timeout = notice.level === "error" ? 9000 : 4500;
    window.setTimeout(() => get().dismissToast(toast.id), timeout);
  },
  dismissToast: (id) => set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
  pushError: (error) => {
    set({ lastError: error });
    get().pushToast({ level: error.severity === "warning" ? "warning" : "error", title: error.title, detail: error.detail });
  },
  lastError: null,
  clearError: () => set({ lastError: null }),

  /* ---------------------------------------------------------------- project */
  project: null,
  recentProjects: [],
  discovered: [],
  busy: null,

  refreshProject: async () => {
    const result = await api.attempt<ProjectView | null>("project_status");
    if (!result.ok) return;
    set({ project: result.value });
    // Everything scoped to the project is reloaded here, because starting with a
    // project already open is the normal case: the application remembers the last
    // one. Without this the saved runs, logs, and sessions are listed as empty
    // until something else happens to reload them.
    await Promise.all([
      get().loadModels(),
      get().loadSavedRuns(),
      get().loadFlightSessions(),
      get().loadTelemetrySessions(),
    ]);
  },

  createProject: async (directory, name) => {
    set({ busy: "Creating project" });
    const result = await api.attempt<ProjectView>("project_create", { directory, name });
    set({ busy: null });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({ project: result.value });
    get().pushToast({ level: "info", title: "Project created", detail: result.value.root });
    await get().loadModels();
    return true;
  },

  openProject: async (directory) => {
    set({ busy: "Opening project" });
    const result = await api.attempt<ProjectView>("project_open", { directory });
    set({ busy: null });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({ project: result.value, discovered: [] });
    for (const warning of result.value.warnings) {
      get().pushToast({ level: "warning", title: "Project note", detail: warning });
    }
    await Promise.all([
      get().loadModels(),
      get().loadSavedRuns(),
      get().loadProfiles(),
      get().loadTelemetrySessions(),
      get().loadFlightSessions(),
    ]);
    return true;
  },

  saveProject: async () => {
    const result = await api.attempt<ProjectView>("project_save");
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ project: result.value });
    get().pushToast({ level: "info", title: "Project saved", detail: result.value.name });
  },

  discoverProjects: async (directory) => {
    const result = await api.attempt<DiscoveredProject[]>("project_discover", {
      directory,
      maxDepth: 3,
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ discovered: result.value });
  },

  updateProject: async (name, description) => {
    const result = await api.attempt<ProjectView>("project_update", {
      name: name ?? null,
      description: description ?? null,
      save: true,
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ project: result.value });
  },

  loadRecentProjects: async () => {
    const result = await api.attempt<string[]>("project_recent");
    if (result.ok) set({ recentProjects: result.value });
  },

  deleteRun: async (name) => {
    const result = await api.attempt<number>("run_delete", { name });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    get().pushToast({ level: "info", title: "Run deleted", detail: `${name} was removed.` });
    await Promise.all([get().loadSavedRuns(), get().refreshProject()]);
  },

  deleteTelemetrySession: async (name) => {
    const result = await api.attempt<void>("telemetry_session_delete", { name });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    await Promise.all([get().loadTelemetrySessions(), get().refreshProject()]);
  },

  deleteFlightSession: async (name) => {
    const result = await api.attempt<void>("flight_session_delete", { name });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    await Promise.all([get().loadFlightSessions(), get().refreshProject()]);
  },

  /* --------------------------------------------------------------- settings */
  settings: null,
  settingsWarnings: [],

  loadSettings: async () => {
    const result = await api.attempt<Settings>("settings_get");
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ settings: result.value });
    const warnings = await api.attempt<string[]>("settings_warnings");
    if (warnings.ok) set({ settingsWarnings: warnings.value });
    get().applyAppearance();
  },

  saveSettings: async (value) => {
    const result = await api.attempt<string[]>("settings_set", { settings: value });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({ settings: value, settingsWarnings: result.value });
    get().applyAppearance();
    get().pushToast({ level: "info", title: "Settings saved", detail: "The settings file was updated." });
    return true;
  },

  /**
   * Switch between the reduced and the full interface.
   *
   * Simple mode keeps the end to end job and hides the controls that only matter
   * to someone tuning a model: solver settings, environment overrides, control
   * moments, channel selection, live hardware, sensor mapping and calibration,
   * log quality tooling, storage, and diagnostics.
   */
  setAdvancedMode: async (advanced) => {
    const current = get().settings;
    if (!current) return;
    if (current.general.advanced_mode === advanced) return;
    const next: Settings = {
      ...current,
      general: { ...current.general, advanced_mode: advanced },
    };
    // The mode is written straight away: a preference that silently reverted on
    // restart would be worse than no switch at all.
    await get().saveSettings(next);
    if (!advanced) {
      const hidden = HIDDEN_SECTIONS;
      if (hidden.includes(get().section)) {
        set({ section: "overview" });
      }
    }
  },

  resetSettings: async () => {
    const result = await api.attempt<Settings>("settings_reset");
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ settings: result.value });
    get().applyAppearance();
  },

  /* ------------------------------------------------------------------ model */
  models: [],
  activeModel: null,

  loadModels: async () => {
    const result = await api.attempt<ModelView[]>("model_list");
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({
      models: result.value,
      activeModel: result.value.length > 0 ? result.value[result.value.length - 1] : null,
    });
  },

  importModelFromPath: async (path, unitSystem = "si") => {
    set({ busy: "Importing model" });
    const result = await api.attempt<{ model: ModelView; usable: boolean }>("model_import", {
      request: { path, text: null, unit_system: unitSystem, mesh_scale: null },
    });
    set({ busy: null });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set((state) => ({
      models: [...state.models, result.value.model],
      activeModel: result.value.model,
    }));
    if (!result.value.usable) {
      get().pushToast({
        level: "error",
        title: "Model has blocking errors",
        detail: `${result.value.model.error_count} problem(s) must be fixed before a run.`,
      });
      return false;
    }
    get().pushToast({
      level: "info",
      title: "Model imported",
      detail: result.value.model.validation_summary,
    });
    return true;
  },

  importExampleModel: async () => {
    const result = await api.attempt<{ model: ModelView; usable: boolean }>("model_import_example");
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set((state) => ({
      models: [...state.models, result.value.model],
      activeModel: result.value.model,
    }));
    get().pushToast({
      level: "info",
      title: "Example model imported",
      detail: result.value.model.name,
    });
    return true;
  },

  /* ----------------------------------------------- scenario and simulation */
  scenario: defaultScenario(),
  setScenario: (patch) => set((state) => ({ scenario: { ...state.scenario, ...patch } })),
  resetScenario: () => set({ scenario: defaultScenario(), validation: null }),

  /**
   * Install one of the example flights, importing the example model when there
   * is none.
   *
   * The scenario is validated afterwards, so the Run button is live and the
   * reader sees the outcome of the numbers rather than a button that does
   * nothing until they press Validate themselves.
   */
  loadExampleFlight: async (id: string) => {
    const example = exampleFlight(id);
    if (!example) {
      get().pushError({
        code: "example.unknown",
        title: "That example does not exist",
        detail: `No example flight is registered under "${id}".`,
        severity: "error",
        suggestion: "Pick one from the list.",
      });
      return false;
    }
    if (!get().activeModel) {
      const imported = await get().importExampleModel();
      if (!imported) return false;
    }
    set({
      scenario: example.scenario,
      validation: null,
      plotChannels: example.channels.length > 0 ? example.channels : defaultPlotChannels(),
    });
    await get().validateScenario();
    get().pushToast({
      level: "info",
      title: `${example.name} loaded`,
      detail: example.summary,
    });
    return true;
  },
  validation: null,

  validateScenario: async () => {
    const result = await api.attempt<ScenarioValidation>("simulation_validate", {
      scenario: get().scenario,
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ validation: result.value });
    if (!result.value.can_run) {
      get().pushToast({
        level: "error",
        title: "The scenario cannot run",
        detail: result.value.errors[0] ?? "Fix the highlighted values.",
      });
    } else {
      get().pushToast({
        level: "info",
        title: "Scenario is valid",
        detail: `${result.value.expected_samples} output samples expected.`,
      });
    }
  },

  runResult: null,
  running: false,
  progress: null,
  setProgress: (progress) => set({ progress }),

  runSimulation: async (save) => {
    set({ running: true, progress: null });
    const scenario: ScenarioRequest = { ...get().scenario, save };
    const result = await api.attempt<RunResultView>("simulation_run", {
      scenario,
      channels: { channels: get().plotChannels, maximum_samples: 4000 },
    });
    set({ running: false });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({ runResult: result.value });
    const warnings = result.value.warnings;
    if (warnings.length > 0) {
      get().pushToast({
        level: warnings.some((w) => w.blocking) ? "error" : "warning",
        title: `${warnings.length} run warning(s)`,
        detail: warnings[0].title,
      });
    } else {
      get().pushToast({
        level: "info",
        title: "Run complete",
        detail: `${result.value.summary.sample_count} samples over ${result.value.summary.duration.toFixed(3)} s.`,
      });
    }
    await Promise.all([get().loadReplay(), get().loadSavedRuns()]);
    return true;
  },

  cancelSimulation: async () => {
    await api.attempt<boolean>("simulation_cancel");
  },

  replay: null,
  loadReplay: async () => {
    const result = await api.attempt<ReplayData>("simulation_replay_data", {
      maximumSamples: 2000,
    });
    if (!result.ok) return;
    set({ replay: result.value });
  },

  savedRuns: [],
  loadSavedRuns: async () => {
    if (!get().project) {
      set({ savedRuns: [] });
      return;
    }
    const result = await api.attempt<SavedRunSummary[]>("simulation_list_saved");
    if (result.ok) set({ savedRuns: result.value });
  },

  openSavedRun: async (name) => {
    const result = await api.attempt<RunResultView>("simulation_open_saved", {
      name,
      channels: { channels: get().plotChannels, maximum_samples: 4000 },
    });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({ runResult: result.value });
    await get().loadReplay();
    get().pushToast({ level: "info", title: "Run loaded", detail: name });
    return true;
  },

  addMarker: async (time, label) => {
    const result = await api.attempt<import("./types").FlightEvent[]>("simulation_add_marker", {
      time,
      label,
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set((state) =>
      state.runResult ? { runResult: { ...state.runResult, events: result.value } } : {},
    );
    get().pushToast({ level: "info", title: "Marker added", detail: `${label} at ${time.toFixed(3)} s` });
  },

  plotChannels: defaultPlotChannels(),
  setPlotChannels: (plotChannels) => set({ plotChannels }),

  /* -------------------------------------------------------------- telemetry */
  ports: [],
  refreshPorts: async () => {
    const result = await api.attempt<PortView[]>("serial_list_ports");
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ ports: result.value });
  },

  telemetryStatus: null,
  telemetryHealth: null,
  liveFrames: [],

  pollTelemetry: async () => {
    const status = await api.attempt<TelemetryStatus | null>("telemetry_status");
    if (status.ok) set({ telemetryStatus: status.value });
    if (!status.ok || !status.value?.connected) {
      set({ liveFrames: [] });
      return;
    }
    const frames = await api.attempt<LiveFrame[]>("telemetry_poll", { limit: 64 });
    if (!frames.ok) return;
    if (frames.value.length > 0) {
      // Keep a bounded history so a long session cannot exhaust memory in the
      // renderer. The full-rate recording is unaffected.
      set((state) => ({ liveFrames: [...state.liveFrames, ...frames.value].slice(-600) }));
    }
  },

  connectPort: async (portName, baudRate) => {
    set({ busy: "Opening port" });
    const result = await api.attempt<TelemetryStatus>("serial_connect", {
      request: {
        port_name: portName,
        profile_id: null,
        profile: null,
        baud_rate: baudRate,
        buffer_samples: null,
        display_rate_hz: null,
      },
    });
    set({ busy: null });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({ telemetryStatus: result.value, liveFrames: [] });
    get().pushToast({
      level: "info",
      title: "Port opened",
      detail: `${result.value.port} at ${result.value.baud_rate} baud`,
    });
    return true;
  },

  connectScripted: async () => {
    const result = await api.attempt<TelemetryStatus>("telemetry_connect_scripted", {
      lines: 900,
      profile: null,
      displayRateHz: null,
    });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({ telemetryStatus: result.value, liveFrames: [], scriptedStream: true });
    get().pushToast({
      level: "warning",
      title: "Synthetic stream attached",
      detail:
        "A generated stream is being read so the console can be exercised without hardware. This is not recorded flight data.",
    });
    return true;
  },

  disconnectPort: async () => {
    const result = await api.attempt<TelemetryStatus>("serial_disconnect");
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ telemetryStatus: result.value, liveFrames: [], scriptedStream: false });
  },

  startRecording: async () => {
    const result = await api.attempt<TelemetryStatus>("telemetry_start_recording");
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ telemetryStatus: result.value });
    get().pushToast({ level: "info", title: "Recording started", detail: "Every packet is being recorded." });
  },

  stopRecording: async (label) => {
    const result = await api.attempt<TelemetryStatus>("telemetry_stop_recording", {
      label,
      save: true,
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ telemetryStatus: result.value });
    await get().loadTelemetrySessions();
    get().pushToast({
      level: "info",
      title: "Recording stopped",
      detail: `${result.value.recorded_samples} samples recorded.`,
    });
  },

  sensorValidation: null,
  runSensorValidation: async (orientation, expectedRate) => {
    const result = await api.attempt<SensorValidation>("telemetry_validate", {
      declaredOrientation: orientation ?? null,
      expectedRateHz: expectedRate ?? null,
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ sensorValidation: result.value });
  },

  deviceProfiles: [],
  loadProfiles: async () => {
    if (!get().project) {
      set({ deviceProfiles: await api.attempt<DeviceProfile[]>("device_profile_examples").then((r) => (r.ok ? r.value : [])) });
      return;
    }
    const result = await api.attempt<DeviceProfile[]>("device_profile_list");
    if (result.ok) set({ deviceProfiles: result.value });
  },

  saveProfile: async (profile) => {
    const result = await api.attempt<DeviceProfile[]>("device_profile_save", { profile });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ deviceProfiles: result.value });
    get().pushToast({ level: "info", title: "Profile saved", detail: profile.name });
  },

  telemetrySessions: [],
  loadTelemetrySessions: async () => {
    if (!get().project) {
      set({ telemetrySessions: [] });
      return;
    }
    const result = await api.attempt<SavedSessionSummary[]>("telemetry_session_list");
    if (result.ok) set({ telemetrySessions: result.value });
  },
  scriptedStream: false,

  /* ----------------------------------------------------------------- flight */
  flightPreview: null,
  flightPreviewPath: null,

  previewFlight: async (path) => {
    const result = await api.attempt<FlightPreview>("flight_preview", {
      request: baseFlightRequest(path),
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ flightPreview: result.value, flightPreviewPath: path });
  },

  timestampReport: null,

  validateFlight: async (path) => {
    const result = await api.attempt<TimestampReport>("flight_validate", {
      request: baseFlightRequest(path),
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ timestampReport: result.value });
    get().pushToast({
      level: result.value.strictly_increasing ? "info" : "warning",
      title: "Timestamp validation finished",
      detail: `${result.value.sample_count} samples at ${result.value.effective_rate_hz.toFixed(2)} Hz.`,
    });
  },

  flightSession: null,
  flightChannels: [],

  importFlight: async (path, label, createSession = true) => {
    set({ busy: "Importing flight log" });
    const result = await api.attempt<import("./types").FlightImportResult>("flight_import", {
      request: {
        ...baseFlightRequest(path),
        label: label ?? null,
        create_session: createSession,
      },
    });
    set({ busy: null });
    if (!result.ok) {
      get().pushError(result.error);
      return false;
    }
    set({
      flightSession: result.value.session,
      timestampReport: null,
    });
    const failed = result.value.report.stages.filter((s) => s.status === "Failed");
    if (failed.length > 0) {
      get().pushToast({
        level: "error",
        title: "Import stopped",
        detail: result.value.report.primary_error ?? failed[0].message,
      });
      return false;
    }
    get().pushToast({
      level: "info",
      title: "Flight log imported",
      detail: result.value.artifact_name
        ? `Saved as ${result.value.artifact_name}.`
        : `${result.value.session?.sample_count ?? 0} samples.`,
    });
    await get().refreshFlightAnalysis();
    return true;
  },

  refreshFlightAnalysis: async () => {
    const [channels, serialised, report] = await Promise.all([
      api.attempt<FlightChannelSummary[]>("flight_channels"),
      api.attempt<FlightSeries>("flight_series", {
        name: null,
        role: null,
        maximumSamples: 600,
      }),
      api.attempt<TimestampReport>("flight_validate_session"),
    ]);
    set({
      flightChannels: channels.ok ? channels.value : [],
      flightSeries: serialised.ok ? serialised.value : null,
      timestampReport: report.ok ? report.value : null,
    });
  },

  flightSeries: null,

  loadFlightSeries: async (name) => {
    const result = await api.attempt<FlightSeries>("flight_series", {
      name,
      role: null,
      maximumSamples: 600,
    });
    if (result.ok) set({ flightSeries: result.value });
  },

  flightTimeline: null,

  detectFlightEvents: async () => {
    const result = await api.attempt<EventTimelineView>("flight_detect_events", { options: null });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ flightTimeline: result.value });
  },

  addFlightMarker: async (time, label) => {
    const result = await api.attempt<EventTimelineView>("flight_add_marker", { time, label });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ flightTimeline: result.value });
    // The replay strip carries the session events, so it is reloaded to show the
    // marker at the position it was placed.
    await get().loadFlightReplay();
    get().pushToast({
      level: "info",
      title: "Marker added",
      detail: `${label} at ${time.toFixed(3)} s`,
    });
  },

  flightReplay: null,

  loadFlightReplay: async () => {
    const result = await api.attempt<ReplayData>("flight_replay_data", {
      maximumSamples: 1200,
      initialAttitude: [1, 0, 0, 0],
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ flightReplay: result.value });
  },

  flightSessions: [],
  loadFlightSessions: async () => {
    if (!get().project) {
      set({ flightSessions: [] });
      return;
    }
    const result = await api.attempt<SavedSessionSummary[]>("flight_session_list");
    if (result.ok) set({ flightSessions: result.value });
  },

  /* ------------------------------------------------------------- comparison */
  comparison: null,
  comparableChannels: [],
  comparisonRequest: defaultComparisonRequest(),
  setComparisonRequest: (patch) =>
    set((state) => ({ comparisonRequest: { ...state.comparisonRequest, ...patch } })),

  loadComparableChannels: async () => {
    const result = await api.attempt<ComparableChannel[]>("comparison_channels");
    if (result.ok) set({ comparableChannels: result.value });
  },

  comparisonMethods: [],
  loadComparisonMethods: async () => {
    const result = await api.attempt<[string, string, boolean][]>("comparison_methods");
    if (result.ok) set({ comparisonMethods: result.value });
  },

  buildComparison: async () => {
    if (!get().runResult) {
      get().pushToast({
        level: "warning",
        title: "No simulation result",
        detail: "Run a simulation, or open a saved run, before comparing.",
      });
      return;
    }
    if (!get().flightSession) {
      get().pushToast({
        level: "warning",
        title: "No flight log",
        detail: "Import a flight log before comparing.",
      });
      return;
    }
    set({ busy: "Building comparison" });
    const result = await api.attempt<Comparison>("comparison_build", {
      request: get().comparisonRequest,
    });
    set({ busy: null });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    set({ comparison: result.value });
    const severe = result.value.warnings.filter((w) => w.severe).length;
    get().pushToast({
      level: severe > 0 ? "warning" : "info",
      title: "Comparison built",
      detail:
        severe > 0
          ? `${severe} important warning(s). Read them before trusting the numbers.`
          : "Open the metrics table for the detail.",
    });
  },

  exportReport: async (title) => {
    const result = await api.attempt<string>("report_export", {
      request: {
        title,
        include_comparison: true,
        include_simulation: true,
        include_flight: true,
        include_events: true,
      },
    });
    if (!result.ok) {
      get().pushError(result.error);
      return;
    }
    get().pushToast({ level: "info", title: "Report exported", detail: result.value });
  },

  /* ------------------------------------------------------------------ theme */
  theme: "dark",
  systemPrefersDark: readSystemPrefersDark(),

  applyAppearance: () => {
    const settings = get().settings;
    const preference = settings?.appearance.theme ?? "system";
    const resolved =
      preference === "system" ? (get().systemPrefersDark ? "dark" : "light") : preference;
    set({ theme: resolved });
    applyDocumentAttributes(
      resolved,
      settings?.appearance.density ?? "comfortable",
      settings?.appearance.reduced_motion ?? false,
    );
  },

  setThemePreference: async (theme) => {
    const current = get().settings;
    if (!current) return;
    const next: Settings = {
      ...current,
      appearance: { ...current.appearance, theme },
    };
    await get().saveSettings(next);
  },

  toggleTheme: async () => {
    const next = get().theme === "dark" ? "light" : "dark";
    await get().setThemePreference(next);
  },
}));

/** The base flight import request for a path, with sensible defaults. */
function baseFlightRequest(path: string): import("./types").FlightImportRequest {
  return {
    path,
    delimiter: null,
    has_header: null,
    timestamp_column: null,
    timestamp_unit: "seconds",
    tick_rate: null,
    comment_prefix: null,
    max_rows: null,
    decimal_comma: null,
    mappings: [],
    expected_rate_hz: null,
    gap_factor: 3,
    rollover_bits: null,
    accel_range: null,
    gyro_range: null,
    label: null,
    create_session: false,
  };
}

/** Subscribe to the backend event channels and route them into the store. */
export function wireEvents(): () => void {
  const unlisteners: (() => void)[] = [];
  const subscribe = async () => {
    unlisteners.push(
      await api.on<SimulationProgressEvent>(api.channels.simulationProgress, (payload) => {
        useStore.getState().setProgress(payload);
      }),
    );
    unlisteners.push(
      await api.on<NoticeEvent>(api.channels.notice, (payload) => {
        useStore.getState().pushToast(payload);
      }),
    );
    unlisteners.push(
      await api.on<TelemetryHealthEvent>(api.channels.telemetryHealth, (payload) => {
        useStore.setState({ telemetryHealth: payload });
      }),
    );
    unlisteners.push(
      await api.on<NoticeEvent>(api.channels.simulationWarning, (payload) => {
        useStore.getState().pushToast({ ...payload, level: "warning" });
      }),
    );
    unlisteners.push(
      await api.on<NoticeEvent>(api.channels.simulationFailed, (payload) => {
        useStore.getState().pushToast({ ...payload, level: "error" });
      }),
    );
    unlisteners.push(
      await api.on<{ run_name: string | null; status: string; samples: number; warnings: number; events: number }>(
        api.channels.simulationCompleted,
        (payload) => {
          useStore.getState().pushToast({
            level: "info",
            title: `Run ${payload.status.toLowerCase()}`,
            detail: `${payload.samples} samples, ${payload.events} events, ${payload.warnings} warnings.`,
          });
        },
      ),
    );
    unlisteners.push(
      await api.on<{ port: string; state: string; detail: string }>(
        api.channels.serialDisconnected,
        (payload) => {
          useStore.getState().pushToast({
            level: "warning",
            title: `Port ${payload.port} closed`,
            detail: payload.detail,
          });
        },
      ),
    );
  };
  void subscribe();
  return () => {
    for (const off of unlisteners) off();
  };
}
