/**
 * Types shared with the Rust backend.
 *
 * The backend serialises Rust structs directly, so the field names here are the
 * backend's snake_case names. One naming convention across the boundary is worth
 * more than idiomatic casing on one side, because it removes a translation layer
 * where a rename could silently break a field.
 */

export type Severity = "info" | "warning" | "error";

export interface ValidationIssue {
  code: string;
  title: string;
  detail: string;
  severity: Severity;
  context?: string;
  suggestion?: string;
  technical?: string;
}

export type ValidationStatus = "valid" | "valid_with_warnings" | "invalid";

export interface ValidationReport {
  subject: string;
  status: ValidationStatus;
  issues: ValidationIssue[];
}

/* ------------------------------------------------------------------- errors */

export type ErrorSeverity = "warning" | "error";

export interface CommandError {
  code: string;
  title: string;
  detail: string;
  severity: ErrorSeverity;
  suggestion: string;
  technical?: string;
}

/** Any thrown value, normalised into a shape the interface can always render. */
export function asError(value: unknown): CommandError {
  if (value && typeof value === "object") {
    const candidate = value as Partial<CommandError>;
    if (typeof candidate.code === "string" && typeof candidate.title === "string") {
      return {
        code: candidate.code,
        title: candidate.title,
        detail: candidate.detail ?? "",
        severity: candidate.severity ?? "error",
        suggestion: candidate.suggestion ?? "",
        technical: candidate.technical,
      };
    }
  }
  const text = typeof value === "string" ? value : String(value);
  return {
    code: "unknown",
    title: "Something went wrong",
    detail: text,
    severity: "error",
    suggestion: "Try the action again, and report it if it keeps failing.",
  };
}

/* ------------------------------------------------------------------ project */

export interface ProjectInventory {
  models: number;
  simulations: number;
  flight_logs: number;
  telemetry_sessions: number;
  reports: number;
}

export interface ProjectView {
  name: string;
  description: string;
  root: string;
  project_id: string;
  schema_version: string;
  created_at: string;
  updated_at: string;
  model_reference: string | null;
  unit_system: string;
  world_frame: string;
  body_frame: string;
  dirty: boolean;
  warnings: string[];
  inventory: ProjectInventory;
  layout_complete: boolean;
}

export interface DiscoveredProject {
  root: string;
  name: string;
  updated_at: string;
  readable: boolean;
  problem?: string;
}

/* -------------------------------------------------------------------- model */

export interface ModelView {
  model_id: string;
  model_version: string;
  name: string;
  description: string | null;
  unit_system: string;
  world_frame: string;
  body_frame: string;
  mass: number;
  center_of_gravity: [number, number, number];
  inertia: [number, number, number, number, number, number];
  reference_area: number;
  reference_length: number;
  mesh_reference: string | null;
  has_thrust: boolean;
  has_mass_curve: boolean;
  has_aerodynamics: boolean;
  aerodynamics_summary: string | null;
  drag_table_entries: number;
  has_lift_or_moments: boolean;
  source_hash: string;
  valid: boolean;
  validation_summary: string;
  error_count: number;
  warning_count: number;
  validation: ValidationReport;
}

export interface ModelImportResult {
  model: ModelView;
  usable: boolean;
  document_json?: string;
}

export interface ModelImportRequest {
  path?: string | null;
  text?: string | null;
  unit_system?: string | null;
  mesh_scale?: number | null;
}

/* --------------------------------------------------------------- simulation */

export type SimulationMode = "six_dof" | "three_dof";

export interface ScenarioRequest {
  name?: string | null;
  description?: string | null;
  mode?: string | null;
  start_time?: number | null;
  end_time?: number | null;
  output_interval?: number | null;
  step?: number | null;
  relative_tolerance?: number | null;
  absolute_tolerance?: number | null;
  position?: [number, number, number] | null;
  velocity?: [number, number, number] | null;
  euler?: [number, number, number] | null;
  angular_velocity?: [number, number, number] | null;
  mass?: number | null;
  inertia?: [number, number, number, number, number, number] | null;
  gravity?: number | null;
  standard_atmosphere?: boolean | null;
  air_density?: number | null;
  wind_speed?: number | null;
  wind_direction?: [number, number, number] | null;
  rail_length?: number | null;
  gravity_enabled?: boolean | null;
  thrust?: number | null;
  burn_time?: number | null;
  thrust_application_point?: [number, number, number] | null;
  drag_coefficient?: number | null;
  /**
   * Which aerodynamic model the run uses.
   *
   * `auto` takes the imported model's coefficients when it declares them and the
   * built-in drag estimate otherwise. `model` requires them, `estimate` and
   * `estimate_with_lift` select the built-in estimates, and `none` disables
   * aerodynamics.
   */
  aero_source?: string | null;
  lift_slope?: number | null;
  pitch_damping?: number | null;
  control_moment?: [number, number, number] | null;
  control_duration?: number | null;
  /**
   * A landing burn that ignites on the stopping-distance condition.
   *
   * The engine lights when the vehicle is descending, low, and close enough to
   * the ground to stop at its own thrust-to-weight. It is a trigger, not a
   * guidance law: nothing steers.
   */
  landing_burn?: LandingBurnRequest | null;
  ground_elevation?: number | null;
  save?: boolean | null;
}

export interface LandingBurnRequest {
  /** Thrust magnitude while the engine is lit, newtons. */
  thrust: number;
  /** Deceleration held back from the stopping distance, m/s². Larger lights the engine earlier. */
  deceleration_margin?: number | null;
  /** The burn is refused above this height, metres. */
  maximum_ignition_altitude?: number | null;
  /** Descent rate below which the burn is not lit, m/s. */
  minimum_descent_rate?: number | null;
}

export interface ScenarioValidation {
  can_run: boolean;
  errors: string[];
  warnings: string[];
  expected_samples: number;
  solver: string;
  mass_model: string;
  environment: string[];
  forces: string[];
  active_providers: string[];
  report: ValidationReport;
}

export interface RunWarning {
  code: string;
  title: string;
  detail: string;
  blocking: boolean;
}

export interface RunSummary {
  name: string;
  description: string;
  status: string;
  mode: SimulationMode;
  duration: number;
  real_time_factor?: number | null;
  wall_clock_seconds?: number | null;
  solver: string;
  integration_step?: number | null;
  relative_tolerance?: number | null;
  absolute_tolerance?: number | null;
  output_interval: number;
  output_rate_hz: number;
  sample_count: number;
  solver_statistics: {
    accepted_steps: number;
    rejected_steps: number;
    function_evaluations: number;
    minimum_step_used: number;
    maximum_step_used: number;
  };
  normalization: string;
  mass_model: string;
  environment: string[];
  force_models: string[];
  active_providers: string[];
  initial: {
    position: [number, number, number];
    velocity: [number, number, number];
    attitude: [number, number, number, number];
    angular_velocity: [number, number, number];
    mass: number;
    time: number;
  };
  event_count: number;
  maximum_quaternion_norm_error: number;
  maximum_angular_rate: number;
  maximum_acceleration: number;
  maximum_dynamic_pressure: number;
  maximum_altitude: number;
  maximum_speed: number;
  maximum_mach: number;
  initial_energy: number;
  final_energy: number;
  energy_drift_ratio?: number | null;
  application_version: string;
  created_at: string;
  model_id: string;
  model_version: string;
  warnings: RunWarning[];
}

export type EventKindCode =
  | "ignition"
  | "lift_off"
  | "rail_exit"
  | "max_q"
  | "burnout"
  | "apogee"
  | "recovery_deployment"
  | "ground_impact"
  | "solver_failure"
  | "solver_warning"
  | "custom";

export interface FlightEvent {
  id: string;
  kind: EventKindCode;
  time: number;
  label: string;
  description: string;
  trigger_value?: number | null;
  trigger_unit?: string | null;
  user_added: boolean;
}

export interface ChannelRequest {
  channels: string[];
  maximum_samples?: number | null;
}

export interface RunResultView {
  artifact_name: string | null;
  summary: RunSummary;
  times: number[];
  columns: number[][];
  channel_names: string[];
  channel_units: string[];
  events: FlightEvent[];
  warnings: RunWarning[];
  available_channels: string[];
}

export interface ReplayData {
  times: number[];
  positions: [number, number, number][];
  quaternions: [number, number, number, number][];
  velocities: [number, number, number][];
  angular_rates: [number, number, number][];
  events: FlightEvent[];
  source: string;
  has_position: boolean;
}

export interface SavedRunSummary {
  name: string;
  label: string;
  status: string;
  samples: number;
  duration: number;
  events: number;
  warnings: number;
  solver: string;
}

export interface SimulationProgressEvent {
  time: number;
  fraction: number;
  samples: number;
  accepted_steps: number;
  rejected_steps: number;
  events: number;
}

/* ---------------------------------------------------------------- telemetry */

export interface PortView {
  port_name: string;
  label: string;
  identity: string;
  is_usb: boolean;
  manufacturer: string | null;
  product: string | null;
  serial_number: string | null;
}

export interface TelemetryStatus {
  connected: boolean;
  port: string;
  baud_rate: number;
  state: string;
  recording: boolean;
  recorded_samples: number;
  capacity: number;
  packets_received: number;
  packets_rejected: number;
  crc_failures: number;
  bytes_received: number;
  discarded_bytes: number;
  display_dropped: number;
  display_downsampled: boolean;
  pending_display_frames: number;
  last_rejection: string | null;
  last_error: string | null;
  finished: boolean;
  receiving: boolean;
}

export interface LiveFrame {
  host_time: number;
  device_time: number | null;
  sequence: number | null;
  acceleration: [number, number, number] | null;
  angular_rate: [number, number, number] | null;
  magnetic_field: [number, number, number] | null;
  pressure: number | null;
  temperature: number | null;
  motor: number | null;
  voltage: number | null;
  current: number | null;
  attitude: [number, number, number, number] | null;
  attitude_source: string | null;
  estimator_health: string | null;
  estimator_note: string | null;
  estimator_indicative: boolean | null;
  mapped: [string, number][];
}

export interface MappingProblem {
  code: string;
  title: string;
  detail: string;
  suggestion: string;
  blocking: boolean;
}

export interface DeviceProfile {
  id: string;
  name: string;
  device_identity?: string | null;
  serial: {
    baud_rate: number;
    data_bits: number;
    stop_bits: number;
    parity: string;
    flow_control: string;
    read_timeout_ms: number;
  };
  format: unknown;
  mappings: unknown[];
  frame: unknown;
  board_rotation?: unknown;
  mounting_offset: [number, number, number];
  expected_rate_hz?: number | null;
  timestamp_source: string;
  estimator: unknown;
  notes?: string | null;
}

export type CheckStatus = "pending" | "pass" | "warning" | "fail" | "skipped";

export interface ValidationStep {
  id: string;
  title: string;
  instruction?: string | null;
  status: CheckStatus;
  measured?: number | null;
  expected?: string | null;
  detail?: string | null;
}

export interface SensorValidation {
  steps: ValidationStep[];
  ready: boolean;
  has_failure: boolean;
  has_warning: boolean;
}

export interface DestinationView {
  code: string;
  label: string;
  si_unit: string;
  quantity: string;
  physical: boolean;
}

export interface CalibrationSet {
  accelerometer: unknown;
  gyroscope: unknown;
  magnetometer: unknown;
  channels: unknown[];
  reference_gravity: number;
  sensor_to_body?: unknown;
  saved: boolean;
  established_at?: string | null;
}

export interface SavedSessionSummary {
  name: string;
  label: string;
  samples: number;
  duration_seconds: number;
  has_issues: boolean;
}

/* ------------------------------------------------------------------- flight */

export interface FlightImportRequest {
  path: string;
  delimiter?: string | null;
  has_header?: boolean | null;
  timestamp_column?: string | null;
  timestamp_unit?: string | null;
  tick_rate?: number | null;
  comment_prefix?: string | null;
  max_rows?: number | null;
  decimal_comma?: boolean | null;
  mappings: FlightMappingRequest[];
  expected_rate_hz?: number | null;
  gap_factor?: number | null;
  rollover_bits?: number | null;
  accel_range?: [number, number] | null;
  gyro_range?: [number, number] | null;
  label?: string | null;
  create_session?: boolean | null;
}

export interface FlightMappingRequest {
  source_name: string;
  role: string;
  unit: string;
  sign?: number | null;
  calibration_offset?: number | null;
  calibration_scale?: number | null;
}

export interface FlightPreview {
  delimiter: string;
  has_header: boolean;
  headers: string[];
  inferred_roles: string[];
  inferred_physical: boolean[];
  sample_rows: string[][];
  warnings: string[];
  row_count: number;
  timestamp_preview: number[];
}

export interface TimestampReport {
  sample_count: number;
  median_dt_seconds: number;
  effective_rate_hz: number;
  strictly_increasing: boolean;
  non_monotonic_indices: number[];
  duplicate_count: number;
  reversal_count: number;
  gaps: [number, number, number][];
  rate_segments: [number, number][];
  rollover_corrected: boolean;
  warnings: string[];
}

export interface ImportStageView {
  stage: string;
  status: string;
  message: string;
  detail?: string | null;
  duration_ms: number;
}

export interface ImportReportView {
  succeeded: boolean;
  primary_error: string | null;
  stages: ImportStageView[];
}

export interface FlightSessionSummary {
  id: string;
  source_file: string;
  source_format: string;
  sample_count: number;
  duration_seconds: number;
  sample_rate_hz: number;
  channels: [string, string, string][];
  derived_channels: string[];
  quality_flags: string[];
  quality_severity: string;
  has_position: boolean;
  timebase_note: string;
}

export interface FlightImportResult {
  report: ImportReportView;
  session: FlightSessionSummary | null;
  artifact_name: string | null;
}

export interface FlightChannelSummary {
  name: string;
  role: string;
  role_label: string;
  unit: string;
  unit_known: boolean;
  count: number;
  min: number;
  max: number;
  mean: number;
  constant: boolean;
  usable: boolean;
}

export interface FlightSeries {
  name: string;
  unit: string;
  times: number[];
  values: number[];
}

export interface EventTimelineView {
  events: DetectedEvent[];
  duration: number;
}

export interface DetectedEvent {
  code: string;
  label: string;
  time: number;
  channel: string;
  value: number;
  evidence: string;
  user_added: boolean;
  note?: string | null;
}

/* --------------------------------------------------------------- comparison */

export type AlignmentMethodCode =
  | "absolute"
  | "first_sample"
  | "event"
  | "cross_correlation"
  | "manual";

export interface ComparisonRequest {
  method?: string | null;
  event_code?: string | null;
  offset_seconds?: number | null;
  correlation_half_width?: number | null;
  correlation_step?: number | null;
  channels: string[];
  minimum_overlap_seconds?: number | null;
  attitude_warning_degrees?: number | null;
}

export interface ErrorMetrics {
  samples: number;
  mean_error: number;
  mean_absolute_error: number;
  root_mean_square_error: number;
  maximum_absolute_error: number;
  maximum_error_time: number;
  final_error: number;
  error_standard_deviation: number;
  reference_range: number;
  normalized_rmse?: number | null;
}

export interface AttitudeMetrics {
  samples: number;
  mean_angle_error: number;
  root_mean_square_angle_error: number;
  maximum_angle_error: number;
  maximum_error_time: number;
  final_angle_error: number;
}

export interface MetricRow {
  channel: string;
  unit: string;
  scalar?: ErrorMetrics | null;
  attitude?: AttitudeMetrics | null;
  caveat?: string | null;
}

export interface VectorErrorMetrics {
  mean_error: [number, number, number];
  mean_absolute_error: number;
  root_mean_square_error: number;
  maximum_absolute_error: number;
  maximum_error_time: number;
  final_error: number;
  samples: number;
}

export interface Comparison {
  simulation_name: string;
  flight_name: string;
  alignment: {
    method: AlignmentMethodCode | { method: string };
    offset_seconds: number;
    quality?: number | null;
    overlap_seconds: number;
    overlap_start: number;
    overlap_end: number;
  };
  metrics: {
    rows: MetricRow[];
    event_time_differences: [string, number][];
    unavailable: [string, string][];
  };
  position_metrics?: VectorErrorMetrics | null;
  velocity_metrics?: VectorErrorMetrics | null;
  event_differences: {
    code: string;
    label: string;
    simulation_time: number;
    flight_time: number;
    difference: number;
    evidence: string;
  }[];
  warnings: {
    code: string;
    title: string;
    detail: string;
    severe: boolean;
  }[];
  alignment_method: unknown;
}

export interface ComparableChannel {
  code: string;
  label: string;
  unit: string;
  in_simulation: boolean;
  in_flight: boolean;
  flight_provenance: string | null;
}

/* ----------------------------------------------------------------- settings */

export type Theme = "dark" | "light" | "system";
export type Density = "compact" | "comfortable";
export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";
export type ExportFormat = "csv" | "json" | "hlog" | "markdown";

export interface Settings {
  schema_version: string;
  general: {
    default_project_directory?: string | null;
    autosave: boolean;
    autosave_interval_seconds: number;
    recent_projects: string[];
    confirm_destructive_actions: boolean;
    unit_system: string;
    language: string;
    /** Whether the interface shows every control rather than the reduced set. */
    advanced_mode: boolean;
  };
  appearance: {
    theme: Theme;
    density: Density;
    reduced_motion: boolean;
    chart_grid: boolean;
    monospace_numbers_only: boolean;
  };
  dynamics: {
    solver: unknown;
    normalization: unknown;
    world_frame: string;
    gravity: unknown;
    quaternion_norm_tolerance: number;
    angular_rate_warning: number;
    acceleration_warning: number;
  };
  telemetry: {
    serial_scan_interval_seconds: number;
    default_baud_rate: number;
    packet_timeout_seconds: number;
    maximum_packet_bytes: number;
    recording_buffer_samples: number;
    display_rate_hz: number;
    auto_reconnect: boolean;
    estimator_mode: string;
  };
  storage: {
    log_directory?: string | null;
    cache_directory?: string | null;
    retention_days: number;
    retention_enabled: boolean;
    export_format: ExportFormat;
    compress_exports: boolean;
    backup_project_file: boolean;
  };
  diagnostics: {
    log_level: LogLevel;
    developer_tools: boolean;
    performance_metrics: boolean;
  };
}

/* ------------------------------------------------------------------- events */

export interface NoticeEvent {
  level: "info" | "warning" | "error" | string;
  title: string;
  detail: string;
}

export interface TelemetryHealthEvent {
  state: string;
  packets_received: number;
  packets_rejected: number;
  packets_per_second: number;
  bytes_received: number;
  discarded_bytes: number;
  crc_failures: number;
  recorded_samples: number;
  display_downsampled: boolean;
  pending_display_frames: number;
  last_rejection?: string | null;
}
