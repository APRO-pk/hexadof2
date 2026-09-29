/**
 * Typed wrappers over the Tauri command surface.
 *
 * Every command goes through {@link call}, which normalises a rejection into a
 * `CommandError`. That means a component never has to inspect the shape of a
 * thrown value, and the error panel always has a title, an explanation, and a
 * suggested fix to render.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  asError,
  type CalibrationSet,
  type ChannelRequest,
  type CommandError,
  type ComparableChannel,
  type Comparison,
  type ComparisonRequest,
  type DestinationView,
  type DetectedEvent,
  type DeviceProfile,
  type DiscoveredProject,
  type EventTimelineView,
  type FlightChannelSummary,
  type FlightImportRequest,
  type FlightImportResult,
  type FlightPreview,
  type FlightSeries,
  type LiveFrame,
  type MappingProblem,
  type ModelImportRequest,
  type ModelImportResult,
  type ModelView,
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
  type TelemetryStatus,
  type TimestampReport,
  type ValidationReport,
} from "./types";

/** Call a backend command, normalising any failure into a `CommandError`. */
export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (value) {
    throw asError(value);
  }
}

/** Await a command and return either the value or the normalised error. */
export async function attempt<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<{ ok: true; value: T } | { ok: false; error: CommandError }> {
  try {
    return { ok: true, value: await call<T>(command, args) };
  } catch (value) {
    return { ok: false, error: asError(value) };
  }
}

export const project = {
  create: (directory: string, name: string) =>
    call<ProjectView>("project_create", { directory, name }),
  open: (directory: string) => call<ProjectView>("project_open", { directory }),
  save: () => call<ProjectView>("project_save"),
  status: () => call<ProjectView | null>("project_status"),
  update: (name?: string, description?: string, save = true) =>
    call<ProjectView>("project_update", {
      name: name ?? null,
      description: description ?? null,
      save,
    }),
  discover: (directory: string, maxDepth?: number) =>
    call<DiscoveredProject[]>("project_discover", {
      directory,
      maxDepth: maxDepth ?? null,
    }),
  recent: () => call<string[]>("project_recent"),
  forgetRecent: (directory: string) => call<string[]>("project_forget_recent", { directory }),
  schemaVersion: () => call<string>("project_schema_version"),
  deleteRun: (name: string) => call<number>("run_delete", { name }),
  deleteTelemetrySession: (name: string) =>
    call<void>("telemetry_session_delete", { name }),
  deleteFlightSession: (name: string) => call<void>("flight_session_delete", { name }),
};

export const settings = {
  get: () => call<Settings>("settings_get"),
  set: (value: Settings) => call<string[]>("settings_set", { settings: value }),
  reset: () => call<Settings>("settings_reset"),
  warnings: () => call<string[]>("settings_warnings"),
  path: () => call<string>("settings_path"),
};

export const diagnostics = {
  lines: () => call<string[]>("app_diagnostics"),
};

export const model = {
  import: (request: ModelImportRequest) =>
    call<ModelImportResult>("model_import", { request }),
  validate: (request: ModelImportRequest) =>
    call<ValidationReport>("model_validate", { request }),
  list: () => call<ModelView[]>("model_list"),
  current: () => call<ModelView | null>("model_current"),
  clear: () => call<void>("model_clear"),
  example: () => call<string>("model_example"),
  importExample: () => call<ModelImportResult>("model_import_example"),
  saveToProject: () => call<string>("model_save_to_project"),
};

export const simulation = {
  validate: (scenario: ScenarioRequest) =>
    call<ScenarioValidation>("simulation_validate", { scenario }),
  run: (scenario: ScenarioRequest, channels?: ChannelRequest) =>
    call<RunResultView>("simulation_run", { scenario, channels: channels ?? null }),
  cancel: () => call<boolean>("simulation_cancel"),
  progress: () => call<SimulationProgressEvent | null>("simulation_progress"),
  isRunning: () => call<boolean>("simulation_is_running"),
  loadResults: (channels?: ChannelRequest) =>
    call<RunResultView>("simulation_load_results", { channels: channels ?? null }),
  listSaved: () => call<SavedRunSummary[]>("simulation_list_saved"),
  openSaved: (name: string, channels?: ChannelRequest) =>
    call<RunResultView>("simulation_open_saved", { name, channels: channels ?? null }),
  addMarker: (time: number, label: string) =>
    call<import("./types").FlightEvent[]>("simulation_add_marker", { time, label }),
  exportCsv: (name: string) => call<string>("simulation_export_csv", { name }),
  replayData: (maximumSamples?: number) =>
    call<ReplayData>("simulation_replay_data", { maximumSamples: maximumSamples ?? null }),
  events: () => call<import("./types").FlightEvent[]>("simulation_events"),
};

export const telemetry = {
  listPorts: () => call<PortView[]>("serial_list_ports"),
  connect: (request: {
    port_name: string;
    profile_id?: string | null;
    profile?: DeviceProfile | null;
    baud_rate?: number | null;
    buffer_samples?: number | null;
    display_rate_hz?: number | null;
  }) => call<TelemetryStatus>("serial_connect", { request }),
  connectScripted: (lines?: number, displayRateHz?: number) =>
    call<TelemetryStatus>("telemetry_connect_scripted", {
      lines: lines ?? null,
      profile: null,
      displayRateHz: displayRateHz ?? null,
    }),
  disconnect: () => call<TelemetryStatus>("serial_disconnect"),
  status: () => call<TelemetryStatus | null>("telemetry_status"),
  poll: (limit?: number) => call<LiveFrame[]>("telemetry_poll", { limit: limit ?? null }),
  startRecording: () => call<TelemetryStatus>("telemetry_start_recording"),
  stopRecording: (label?: string, save = true) =>
    call<TelemetryStatus>("telemetry_stop_recording", {
      label: label ?? null,
      save,
    }),
  validate: (declaredOrientation?: string, expectedRateHz?: number) =>
    call<SensorValidation>("telemetry_validate", {
      declaredOrientation: declaredOrientation ?? null,
      expectedRateHz: expectedRateHz ?? null,
    }),
  destinations: () => call<DestinationView[]>("telemetry_destinations"),
  sessions: () => call<SavedSessionSummary[]>("telemetry_session_list"),
  isScripted: () => call<boolean>("telemetry_is_scripted"),
  validationLines: (validation: SensorValidation) =>
    call<string[]>("telemetry_validation_lines", { validation }),
  profiles: () => call<DeviceProfile[]>("device_profile_list"),
  saveProfile: (profile: DeviceProfile) =>
    call<DeviceProfile[]>("device_profile_save", { profile }),
  deleteProfile: (profileId: string) =>
    call<DeviceProfile[]>("device_profile_delete", { profileId }),
  exampleProfiles: () => call<DeviceProfile[]>("device_profile_examples"),
  validateProfile: (profile: DeviceProfile) =>
    call<MappingProblem[]>("device_profile_validate", { profile }),
  profileReport: (profile: DeviceProfile) =>
    call<ValidationReport>("telemetry_profile_report", { profile }),
  estimatorModes: () =>
    call<{ code: string; label: string; note: string }[]>("estimator_modes"),
  saveCalibration: (name: string, calibration: CalibrationSet) =>
    call<string>("calibration_save", { name, calibration }),
  loadCalibration: (name: string) => call<CalibrationSet>("calibration_load", { name }),
};

export const flight = {
  preview: (request: FlightImportRequest) => call<FlightPreview>("flight_preview", { request }),
  validate: (request: FlightImportRequest) =>
    call<TimestampReport>("flight_validate", { request }),
  import: (request: FlightImportRequest) =>
    call<FlightImportResult>("flight_import", { request }),
  validateSession: () => call<TimestampReport>("flight_validate_session"),
  channels: () => call<FlightChannelSummary[]>("flight_channels"),
  series: (name?: string, role?: string, maximumSamples?: number) =>
    call<FlightSeries>("flight_series", {
      name: name ?? null,
      role: role ?? null,
      maximumSamples: maximumSamples ?? null,
    }),
  replayData: (maximumSamples?: number, initialAttitude?: [number, number, number, number]) =>
    call<ReplayData>("flight_replay_data", {
      maximumSamples: maximumSamples ?? null,
      initialAttitude: initialAttitude ?? null,
    }),
  sessions: () => call<SavedSessionSummary[]>("flight_session_list"),
  roleOptions: () => call<[string, string, string][]>("flight_role_options"),
  detectEvents: () => call<EventTimelineView>("flight_detect_events", { options: null }),
  eventLines: (events: DetectedEvent[]) =>
    call<string[]>("detected_event_lines", { events }),
};

export const analysis = {
  create: (request?: ComparisonRequest) =>
    call<Comparison>("comparison_create", { request: request ?? null }),
  build: (request?: ComparisonRequest) =>
    call<Comparison>("comparison_build", { request: request ?? null }),
  current: () => call<Comparison | null>("comparison_current"),
  channels: () => call<ComparableChannel[]>("comparison_channels"),
  methods: () => call<[string, string, boolean][]>("comparison_methods"),
  reportText: () => call<string>("comparison_report_text"),
  exportComparison: (label?: string) =>
    call<string>("comparison_export", { label: label ?? null }),
  channelCodes: () => call<[string, string, string][]>("comparison_channel_codes"),
  plotChannels: () => call<string[]>("comparison_plot_channels"),
  resultChannels: () => call<string[]>("result_channel_names"),
  eventLines: (events: import("./types").FlightEvent[]) =>
    call<string[]>("event_lines", { events }),
  exportReport: (request?: {
    title?: string | null;
    include_comparison?: boolean | null;
    include_simulation?: boolean | null;
    include_flight?: boolean | null;
    include_events?: boolean | null;
  }) => call<string>("report_export", { request: request ?? null }),
};

/* ------------------------------------------------------------------ events */

/** Channel names the backend emits. Kept in step with the Rust `events` module. */
export const channels = {
  simulationProgress: "simulation://progress",
  simulationCompleted: "simulation://completed",
  simulationWarning: "simulation://warning",
  simulationFailed: "simulation://failed",
  serialConnected: "serial://connected",
  serialDisconnected: "serial://disconnected",
  telemetryHealth: "telemetry://health",
  recordingStarted: "telemetry://recording-started",
  recordingStopped: "telemetry://recording-stopped",
  importProgress: "import://progress",
  validationResult: "validation://result",
  notice: "app://notice",
} as const;

export function on<T>(
  channel: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return listen<T>(channel, (event) => handler(event.payload));
}

/* ------------------------------------------------------------------ dialogs */

/**
 * Open a file picker through the dialog plugin.
 *
 * The dialog plugin is loaded dynamically so a browser-only preview of the
 * interface still renders; the picker is the one thing that genuinely needs the
 * desktop shell.
 */
export async function pickFile(options: {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
  multiple?: boolean;
}): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    title: options.title,
    multiple: false,
    directory: false,
    filters: options.filters,
  });
  return typeof selected === "string" ? selected : null;
}

/** Open a directory picker. */
export async function pickDirectory(title?: string): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({ title, multiple: false, directory: true });
  return typeof selected === "string" ? selected : null;
}
