/**
 * A layout probe for the three workspace pages.
 *
 * The complaint this exists to answer is structural: a 3D view or a chart that
 * collapses to zero height because the box around it has no resolvable height.
 * That is invisible in a unit test and obvious in a measurement, so this script
 * loads the built frontend in a headless browser with a stubbed Tauri bridge,
 * measures the boxes, and writes a screenshot of each page.
 *
 * Usage, from the repository root, after `npm --prefix apps/desktop/frontend run build`:
 *
 *   node tools/layout-check/run.mjs
 *
 * Output: measurements on stdout and PNGs in tools/layout-check/out/.
 */

import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { readFile, mkdir, writeFile, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { tmpdir } from "node:os";

const ROOT = resolve(import.meta.dirname, "../..");
const DIST = join(ROOT, "apps/desktop/frontend/dist");
const OUT = join(import.meta.dirname, "out");
const PORT = 5199;
const DEBUG_PORT = 9333;

/** Simple mode is checked by running the same probes with the reduced surface. */
const SIMPLE = process.argv.includes("--simple");
/** The narrow pass checks the smallest window the application allows. */
const NARROW = process.argv.includes("--narrow");
/** An explicit width, for checking a breakpoint boundary. */
const WIDTH_ARG = process.argv.find((arg) => arg.startsWith("--width="));
const VIEWPORT = WIDTH_ARG
  ? { width: Number(WIDTH_ARG.split("=")[1]), height: 900 }
  : NARROW
    ? { width: 1180, height: 800 }
    : { width: 1600, height: 1000 };

const EDGE_CANDIDATES = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
  "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
];

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".json": "application/json",
};

/** A model that declares aerodynamic coefficients, so the aero controls appear. */
function modelView() {
  return {
    model_id: "harness.aero.rocket",
    model_version: "1.0",
    name: "Harness rocket with aerodynamics",
    description: null,
    unit_system: "SI (m, kg, s, rad)",
    world_frame: "ENU (east, north, up)",
    body_frame: "Forward, Right, Down",
    mass: 4.5,
    center_of_gravity: [0, 0, 0],
    inertia: [0.09, 0.09, 0.012, 0, 0, 0],
    reference_area: 0.0123,
    reference_length: 0.61,
    mesh_reference: null,
    has_thrust: true,
    has_mass_curve: false,
    has_aerodynamics: true,
    aerodynamics_summary: "drag table with 3 Mach entries, lift and moment derivatives, drag only: false",
    drag_table_entries: 3,
    has_lift_or_moments: true,
    source_hash: "00000000",
    valid: true,
    validation_summary: "0 error(s), 0 warning(s), 2 note(s)",
    error_count: 0,
    warning_count: 0,
    validation: { subject: "harness.aero.rocket", status: "valid_with_notes", issues: [] },
  };
}

/** A settings object matching the TypeScript type, so pages render normally. */
const SETTINGS = {
  schema_version: "1.0",
  general: {
    default_project_directory: null,
    autosave: true,
    autosave_interval_seconds: 120,
    recent_projects: [],
    confirm_destructive_actions: true,
    unit_system: "si",
    language: "en",
    advanced_mode: !SIMPLE,
  },
  appearance: {
    theme: "dark",
    density: "comfortable",
    reduced_motion: false,
    chart_grid: true,
    monospace_numbers_only: false,
  },
  dynamics: {
    solver: { kind: "rk4", step: 0.0005 },
    normalization: "every_step",
    world_frame: "enu",
    gravity: { model: "uniform", magnitude: 9.80665 },
    quaternion_norm_tolerance: 1e-9,
    angular_rate_warning: 20,
    acceleration_warning: 1000,
  },
  telemetry: {
    serial_scan_interval_seconds: 2,
    default_baud_rate: 115200,
    packet_timeout_seconds: 1,
    maximum_packet_bytes: 4096,
    recording_buffer_samples: 200000,
    display_rate_hz: 30,
    auto_reconnect: false,
    estimator_mode: "complementary",
  },
  storage: {
    log_directory: null,
    cache_directory: null,
    retention_days: 30,
    retention_enabled: false,
    export_format: "csv",
    compress_exports: false,
    backup_project_file: true,
  },
  diagnostics: { log_level: "info", developer_tools: false, performance_metrics: false },
};

// The names a result view returns are the names that were requested, so the mock
// uses the same codes the interface asks for. Otherwise the plot grouping has
// nothing to match and every chart comes back empty.
const CHANNELS = [
  "time",
  "altitude",
  "vertical_velocity",
  "speed",
  "acceleration",
  "dynamic_pressure",
  "roll",
  "pitch",
  "yaw",
  "angular_rate_x",
  "mass",
  "force",
  "moment",
  "angle_of_attack",
  "sideslip",
];

const UNITS = ["s", "m", "m/s", "m/s", "m/s^2", "Pa", "deg", "deg", "deg", "rad/s", "kg", "N", "N*m", "deg", "deg"];

function series(count, fn) {
  return Array.from({ length: count }, (_, i) => fn(i));
}

/** A plausible vertical flight, so the plots and the state panel have content. */
function runResult() {
  const n = 240;
  const times = series(n, (i) => i * 0.02);
  const altitude = series(n, (i) => Math.max(0, 60 * Math.sin((Math.PI * i) / n)));
  const vertical = series(n, (i) => 30 * Math.cos((Math.PI * i) / n));
  const speed = series(n, (i) => Math.abs(30 * Math.cos((Math.PI * i) / n)));
  const acceleration = series(n, (i) => 20 + 12 * Math.cos((Math.PI * i) / 12));
  const pressure = series(n, (i) => 0.5 * 1.2 * Math.pow(30 * Math.cos((Math.PI * i) / n), 2));
  const roll = series(n, (i) => 2 * Math.sin(i / 20));
  const pitch = series(n, (i) => -90 + 4 * Math.sin(i / 30));
  const yaw = series(n, (i) => 1.5 * Math.cos(i / 25));
  const rate = series(n, (i) => 0.1 + 0.05 * Math.sin(i / 10));
  const mass = series(n, (i) => 4.5 - 0.001 * i);
  const force = series(n, (i) => 120 * Math.exp(-i / 60));
  const moment = series(n, (i) => 0.4 * Math.sin(i / 15));
  const angleOfAttack = series(n, (i) => 3.2 * Math.sin(i / 40));
  const sideslip = series(n, (i) => 1.4 * Math.cos(i / 55));
  const columns = [
    times,
    altitude,
    vertical,
    speed,
    acceleration,
    pressure,
    roll,
    pitch,
    yaw,
    rate,
    mass,
    force,
    moment,
    angleOfAttack,
    sideslip,
  ];

  return {
    artifact_name: "vertical-flight",
    summary: {
      name: "Vertical flight",
      description: "",
      status: "Completed",
      mode: "six_dof",
      duration: 4.78,
      real_time_factor: 320.5,
      wall_clock_seconds: 0.015,
      solver: "RK4 fixed step",
      integration_step: 0.0005,
      relative_tolerance: null,
      absolute_tolerance: null,
      output_interval: 0.02,
      output_rate_hz: 50,
      sample_count: n,
      solver_statistics: {
        accepted_steps: 9560,
        rejected_steps: 0,
        function_evaluations: 38240,
        minimum_step_used: 0.0005,
        maximum_step_used: 0.0005,
      },
      normalization: "Normalize every step",
      mass_model: "Constant mass",
      environment: ["Uniform gravity 9.80665 m/s^2", "Standard atmosphere"],
      force_models: ["Gravity", "Thrust", "Drag"],
      active_providers: ["gravity", "thrust", "aero"],
      initial: {
        position: [0, 0, 0],
        velocity: [0, 0, 0],
        attitude: [0.7071, 0, -0.7071, 0],
        angular_velocity: [0, 0, 0],
        mass: 4.5,
        time: 0,
      },
      event_count: 5,
      maximum_quaternion_norm_error: 2.2e-16,
      maximum_angular_rate: 0.14,
      maximum_acceleration: 32.4,
      maximum_dynamic_pressure: 540.2,
      maximum_altitude: 60.1,
      maximum_speed: 30.0,
      maximum_mach: 0.088,
      initial_energy: 0,
      final_energy: 12.4,
      energy_drift_ratio: null,
      application_version: "0.1.0",
      created_at: "2026-01-01T00:00:00Z",
      model_id: "builtin.example.rocket",
      model_version: "1.0",
      warnings: [],
    },
    times,
    columns,
    channel_names: CHANNELS,
    channel_units: UNITS,
    events: [
      { id: "ignition-0", kind: "ignition", time: 0, label: "Ignition", description: "", trigger_value: 120, trigger_unit: "N", user_added: false },
      { id: "liftoff-1", kind: "lift_off", time: 0.44, label: "Lift-off", description: "", trigger_value: 7.4, trigger_unit: "m/s", user_added: false },
      { id: "burnout-2", kind: "burnout", time: 1.52, label: "Motor burnout", description: "", trigger_value: 4.5, trigger_unit: "kg", user_added: false },
      { id: "apogee-3", kind: "apogee", time: 4.03, label: "Apogee", description: "", trigger_value: 60.1, trigger_unit: "m", user_added: false },
      { id: "impact-4", kind: "ground_impact", time: 4.78, label: "Ground impact", description: "", trigger_value: 0, trigger_unit: "m", user_added: false },
    ],
    warnings: [
      {
        code: "flight.ground_impact",
        title: "The run ended at ground impact",
        detail: "The vehicle reached the ground at t = 4.7800 s. The trajectory after impact is not modelled.",
        blocking: false,
      },
    ],
    available_channels: CHANNELS,
  };
}

function replayData() {
  const result = runResult();
  const n = result.times.length;
  return {
    times: result.times,
    positions: series(n, (i) => [0, 0, result.columns[1][i]]),
    quaternions: series(n, (i) => [0.7071, 0, -0.7071 + 0.05 * Math.sin(i / 20), 0]),
    velocities: series(n, (i) => [0, 0, result.columns[2][i]]),
    angular_rates: series(n, (i) => [0.01, 0, result.columns[9][i]]),
    events: result.events,
    source: "saved run",
    has_position: true,
  };
}

function savedRun() {
  const result = runResult();
  return {
    name: "vertical-flight",
    label: "Vertical flight",
    status: "Completed",
    samples: result.summary.sample_count,
    duration: result.summary.duration,
    events: result.events.length,
    warnings: result.warnings.length,
  };
}

function telemetryStatus() {
  return {
    connected: true,
    state: "Receiving",
    port: "COM4",
    baud_rate: 115200,
    packets_received: 400,
    packets_rejected: 0,
    packets_per_second: 99.6,
    bytes_received: 24800,
    recorded_samples: 400,
    capacity: 200000,
    recording: true,
    display_downsampled: false,
    last_rejection: null,
  };
}

function liveFrame(i) {
  const t = i * 0.01;
  return {
    host_time: t,
    device_time: t,
    sequence: i,
    acceleration: [0.02 * Math.sin(t * 6), 0.02 * Math.cos(t * 5), -9.80665 + 0.1 * Math.sin(t * 8)],
    angular_rate: [0.01, 0.0, 0.4 + 0.3 * (i / 400)],
    magnetic_field: [21.0, -3.0, 44.0],
    pressure: 101325 - 120 * t,
    temperature: 294.0,
    motor: null,
    voltage: null,
    current: null,
    attitude: [0.999, 0.02, -0.03, 0.01],
    attitude_source: "gyro integration",
    estimator_health: "nominal",
    estimator_note: "Attitude propagated from the gyro. It drifts and has no absolute reference.",
    estimator_indicative: true,
    mapped: [
      ["Accel X", 0.02],
      ["Accel Y", 0.01],
      ["Accel Z", -9.8],
    ],
  };
}

/**
 * The stub lives in the page, so the mock has to be emitted as source rather
 * than as JSON: `JSON.stringify` drops function values, which is how the first
 * version of this harness ended up with an empty table.
 */
const MOCK_SOURCE = Object.entries({
  settings_get: SETTINGS,
  settings_warnings: [],
  settings_set: SETTINGS,
  project_status: {
    name: "Sounding rocket LOX 2026 flight test campaign",
    description: "Vertical flight test series.",
    root: "C:/Users/engineer/Documents/HexaDOF/sounding-rocket-lox-2026",
    project_id: "harness-project",
    schema_version: "1.0",
    created_at: "2026-01-01T09:00:00Z",
    updated_at: "2026-01-02T11:30:00Z",
    model_reference: "models/example.dynamic.json",
    unit_system: "SI (m, kg, s, rad)",
    world_frame: "ENU (east, north, up)",
    body_frame: "Forward, Right, Down",
    dirty: true,
    warnings: [],
    inventory: { models: 1, simulations: 1, flight_logs: 0, telemetry_sessions: 0, reports: 0 },
    layout_complete: true,
  },
  project_recent: [],
  project_discover: [],
  model_current: null,
  model_list: [modelView()],
  model_import_example: { model: modelView(), usable: true },
  simulation_validate: {
    can_run: true,
    can_proceed: true,
    errors: [],
    warnings: [],
    notes: [],
    summary: "The scenario can run.",
    checks: [],
  },
  simulation_list_saved: [savedRun()],
  simulation_open_saved: runResult(),
  simulation_replay_data: replayData(),
  simulation_progress: null,
  simulation_is_running: false,
  simulation_events: runResult().events,
  event_lines: [],
  detected_event_lines: [],
  serial_list_ports: [],
  device_profile_list: [],
  device_profile_examples: [],
  telemetry_status: telemetryStatus(),
  telemetry_connect_scripted: telemetryStatus(),
  telemetry_poll: [],
  telemetry_session_list: [],
  telemetry_is_scripted: false,
  flight_session_list: [],
  flight_role_options: [],
  estimator_modes: [],
  comparison_methods: [],
  comparison_channels: [],
  comparison_current: null,
  app_diagnostics: [],
  project_schema_version: "1.0",
})
  .map(([command, value]) => `  ${JSON.stringify(command)}: () => (${JSON.stringify(value)}),`)
  .join("\n");

/** The bridge the frontend expects from Tauri, reduced to what these pages use. */
function bridgeScript() {
  const frame = JSON.stringify(liveFrame(0));
  return `
const callbacks = new Map();
let nextCallback = 0;
let pollCount = 0;
const MOCK = {
${MOCK_SOURCE}
};

const FRAME_SHAPE = ${frame};

function frameAt(i) {
  const t = i * 0.01;
  const base = FRAME_SHAPE;
  return {
    ...base,
    host_time: t,
    device_time: t,
    sequence: i,
    acceleration: [
      0.02 * Math.sin(t * 6),
      0.02 * Math.cos(t * 5),
      -9.80665 + 0.1 * Math.sin(t * 8),
    ],
    angular_rate: [0.01, 0.0, 0.4 + 0.3 * (i / 400)],
    pressure: 101325 - 120 * t,
  };
}

function transformCallback(cb, once) {
  const id = ++nextCallback;
  callbacks.set(id, { cb, once });
  window["_" + id] = cb;
  return id;
}

function invoke(cmd, args) {
  if (cmd === "telemetry_poll") {
    pollCount += 1;
    const frames = [];
    for (let i = 0; i < 48; i += 1) frames.push(frameAt(pollCount * 48 + i));
    return Promise.resolve(frames);
  }
  if (cmd === "plugin:event|listen") return Promise.resolve(1);
  if (cmd === "plugin:event|unlisten") return Promise.resolve(null);
  if (Object.prototype.hasOwnProperty.call(MOCK, cmd)) return Promise.resolve(MOCK[cmd]());
  if (cmd.startsWith("plugin:")) return Promise.resolve(null);
  console.warn("[harness] unmocked command:", cmd);
  return Promise.resolve(null);
}

window.__TAURI_INTERNALS__ = {
  transformCallback,
  invoke,
  metadata: { currentWebview: { label: "main" }, currentWindow: { label: "main" } },
  convertFileSrc: (path) => path,
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
`;
}

function harnessHtml(indexHtml) {
  const scriptMatch = indexHtml.match(/<script[^>]*src="([^"]+)"[^>]*><\/script>/);
  const styleMatches = [...indexHtml.matchAll(/<link[^>]*href="([^"]+)"[^>]*>/g)].map((m) => m[1]);
  if (!scriptMatch) throw new Error("no module script found in dist/index.html");
  const links = styleMatches.map((href) => `<link rel="stylesheet" href="${href}" />`).join("\n");
  return `<!doctype html>
<html lang="en" data-theme="dark" data-density="comfortable" data-reduced-motion="false">
<head>
<meta charset="utf-8" />
<title>HexaDOF layout harness</title>
${links}
<style>html, body, #root { height: 100%; margin: 0; }</style>
</head>
<body>
<div id="root"></div>
<script>${bridgeScript()}</script>
<script type="module" src="${scriptMatch[1]}"></script>
</body>
</html>`;
}

async function startServer(harness) {
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    if (url.pathname === "/" || url.pathname === "/__harness.html") {
      res.writeHead(200, { "content-type": "text/html" });
      res.end(harness);
      return;
    }
    const file = join(DIST, url.pathname);
    if (!file.startsWith(DIST) || !existsSync(file)) {
      res.writeHead(404);
      res.end("not found");
      return;
    }
    const body = await readFile(file);
    res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
    res.end(body);
  });
  await new Promise((done) => server.listen(PORT, "127.0.0.1", done));
  return server;
}

async function waitForDebugger() {
  for (let i = 0; i < 60; i += 1) {
    try {
      const response = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json/list`);
      const targets = await response.json();
      const page = targets.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // The browser is still starting.
    }
    await new Promise((done) => setTimeout(done, 250));
  }
  throw new Error("the browser never exposed a page target");
}

function connect(url) {
  const socket = new WebSocket(url);
  const pending = new Map();
  const consoleErrors = [];
  let nextId = 0;

  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve: resolvePromise, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolvePromise(message.result);
      return;
    }
    if (message.method === "Runtime.consoleAPICalled" && message.params.type !== "log") {
      consoleErrors.push(message.params.args.map((a) => a.value ?? a.description ?? "").join(" "));
    }
    if (message.method === "Runtime.exceptionThrown") {
      consoleErrors.push(
        message.params.exceptionDetails.exception?.description ??
          message.params.exceptionDetails.text,
      );
    }
  });

  const send = (method, params = {}) =>
    new Promise((resolvePromise, reject) => {
      const id = ++nextId;
      pending.set(id, { resolve: resolvePromise, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });

  const ready = new Promise((done, fail) => {
    socket.addEventListener("open", done);
    socket.addEventListener("error", fail);
  });

  return { send, ready, consoleErrors, close: () => socket.close() };
}

const PROBES = {
  topbar: `(() => {
    const bar = document.querySelector('.app-topbar');
    if (!bar) return null;
    const box = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { left: Math.round(r.left), right: Math.round(r.right), width: Math.round(r.width) };
    };
    const right = document.querySelector('.topbar-right');
    return {
      bar: { width: Math.round(bar.getBoundingClientRect().width) },
      left: box(document.querySelector('.topbar-left')),
      nav: box(document.querySelector('.topnav')),
      right: box(right),
      rightChildren: right
        ? Array.from(right.children).map((el) => ({
            cls: el.className || el.tagName,
            text: (el.textContent || '').trim().slice(0, 24),
            width: Math.round(el.getBoundingClientRect().width),
          }))
        : [],
      overflow: Math.round(bar.scrollWidth - bar.clientWidth),
    };
  })()`,
  viewport: `Array.from(document.querySelectorAll('.viewport')).map((el) => ({
    height: el.clientHeight,
    width: el.clientWidth,
    canvas: el.querySelector('canvas') ? el.querySelector('canvas').clientHeight : 0,
  }))`,
  charts: `Array.from(document.querySelectorAll('.chart-panel')).map((el) => el.clientHeight)`,
  cardSlots: `Array.from(document.querySelectorAll('.card-slot')).map((el) => ({ class: el.className, height: el.clientHeight, child: el.firstElementChild ? el.firstElementChild.clientHeight : 0 }))`,
  grids: `Array.from(document.querySelectorAll('.grid-12')).map((el) => ({
    width: Math.round(el.getBoundingClientRect().width),
    display: getComputedStyle(el).display,
    columns: getComputedStyle(el).gridTemplateColumns,
    parent: el.parentElement ? el.parentElement.className : '',
  }))`,
  panels: `Array.from(document.querySelectorAll('.panel')).map((el) => ({ title: (el.querySelector('.panel-title')||{}).textContent || '', left: Math.round(el.getBoundingClientRect().left), top: Math.round(el.getBoundingClientRect().top), width: Math.round(el.getBoundingClientRect().width), height: Math.round(el.getBoundingClientRect().height) }))`,
  columns: `Array.from(document.querySelectorAll('.grid-12 > *')).map((el) => ({ class: el.className, width: Math.round(el.getBoundingClientRect().width), height: Math.round(el.getBoundingClientRect().height) }))`,
  scrollOverflow: `({ docScroll: document.documentElement.scrollHeight, docClient: document.documentElement.clientHeight, bodyScroll: document.body.scrollHeight })`,
};

const SECTIONS = [
  { id: "overview", label: "Overview" },
  { id: "dynamics", label: "Dynamics" },
  { id: "dynamics-forces", label: "Dynamics", prepare: "forces" },
  { id: "dynamics-initial", label: "Dynamics", prepare: "initial" },
  // The one button that fills the whole screen: the example flight.
  { id: "dynamics-example", label: "Dynamics", prepare: "example" },
  // The example list, which is a dialog over the whole workspace.
  { id: "dynamics-examples", label: "Dynamics", prepare: "examples" },
  // The result views, one per tab. Opening a saved run is what puts content in
  // them, and the timeline is the one that has to be looked at.
  { id: "results-summary", label: "Dynamics", prepare: "openRun", resultTab: "Summary" },
  { id: "results-plots", label: "Dynamics", prepare: "openRun", resultTab: "Plots" },
  { id: "results-timeline", label: "Dynamics", prepare: "openRun", resultTab: "Timeline" },
  { id: "results-warnings", label: "Dynamics", prepare: "openRun", resultTab: "Warnings" },
  // The metadata view is the one result tab Simple mode does not offer.
  ...(SIMPLE
    ? []
    : [{ id: "results-metadata", label: "Dynamics", prepare: "openRun", resultTab: "Metadata" }]),
  ...(SIMPLE ? [] : [{ id: "telemetry", label: "Live Telemetry", prepare: "stream" }]),
  { id: "analysis", label: "Flight Analysis" },
  { id: "projects", label: "Projects" },
  { id: "settings", label: "Settings" },
];

/** Actions a page needs before it renders the state under test. */
const PREPARE = {
  stream: `
    (() => {
      const target = Array.from(document.querySelectorAll('button'))
        .find((el) => el.textContent.trim().toLowerCase().includes('synthetic stream'));
      if (target) target.click();
      return !!target;
    })()
  `,
  // Opening a saved run fills the result views. The button only exists while the
  // panel is empty, so a second call is a no-op rather than a second open.
  openRun: `
    (() => {
      const target = Array.from(document.querySelectorAll('button'))
        .find((el) => el.textContent.trim().toLowerCase().startsWith('open '));
      if (target) target.click();
      return !!target;
    })()
  `,
  // The example flight button installs the model and the scenario in one press.
  example: `
    (() => {
      const open = Array.from(document.querySelectorAll('button'))
        .find((el) => el.textContent.trim().toLowerCase() === 'example flights');
      if (open) open.click();
      const load = Array.from(document.querySelectorAll('button'))
        .find((el) => el.textContent.trim().toLowerCase() === 'load');
      if (load) load.click();
      return !!open && !!load;
    })()
  `,
  // The example list, left open so the dialog itself is measured.
  examples: `
    (() => {
      const open = Array.from(document.querySelectorAll('button'))
        .find((el) => el.textContent.trim().toLowerCase() === 'example flights');
      if (open) open.click();
      return !!open;
    })()
  `,
  forces: `
    (() => {
      const tab = Array.from(document.querySelectorAll('button'))
        .find((el) => el.textContent.trim().toLowerCase() === 'forces');
      if (tab) tab.click();
      // The aerodynamic controls sit below the thrust inputs, so the panel is
      // scrolled to the end before the capture.
      const body = document.querySelector('.ws-config .panel-body.scroll');
      if (body) body.scrollTop = body.scrollHeight;
      return !!tab;
    })()
  `,
  initial: `
    (() => {
      const tab = Array.from(document.querySelectorAll('button'))
        .find((el) => el.textContent.trim().toLowerCase() === 'initial state');
      if (tab) tab.click();
      return !!tab;
    })()
  `,
};

/**
 * Structural faults a measurement can catch but an eye cannot: a card squeezed
 * to nothing, a canvas with no height, a slot whose child did not fill it, or a
 * row that pushes past the window.
 */
const PROBLEM_PROBE = `
(() => {
  const problems = [];
  const round = (n) => Math.round(n);
  document.querySelectorAll('.panel').forEach((el) => {
    const title = (el.querySelector('.panel-title') || {}).textContent || '(untitled)';
    const box = el.getBoundingClientRect();
    if (box.height < 24) problems.push('collapsed panel: ' + title + ' is ' + round(box.height) + 'px tall');
  });
  document.querySelectorAll('.viewport').forEach((el) => {
    const box = el.getBoundingClientRect();
    if (box.height < 120) problems.push('viewport height ' + round(box.height));
    if (box.width < 200) problems.push('viewport width ' + round(box.width));
    const canvas = el.querySelector('canvas');
    if (!canvas) problems.push('viewport has no canvas');
  });
  document.querySelectorAll('.card-slot').forEach((el) => {
    const child = el.firstElementChild;
    if (!child) { problems.push('empty card slot: ' + el.className); return; }
    const gap = el.clientHeight - child.getBoundingClientRect().height;
    if (gap > 4) problems.push('slot not filled: ' + el.className + ' leaves ' + round(gap) + 'px');
  });
  document.querySelectorAll('.chart-panel').forEach((el) => {
    if (el.getBoundingClientRect().height < 120) problems.push('collapsed chart: ' + round(el.getBoundingClientRect().height) + 'px');
  });
  const main = document.querySelector('.app-main');
  if (main && main.scrollWidth > main.clientWidth + 2) {
    problems.push('horizontal overflow: ' + (main.scrollWidth - main.clientWidth) + 'px');
  }
  // The tabs are centred on the window, so a wide status cluster can slide under
  // them. That is invisible in a screenshot until it is unreadable.
  const nav = document.querySelector('.topnav');
  if (nav && nav.children.length > 0) {
    const navBox = nav.getBoundingClientRect();
    for (const selector of ['.topbar-left', '.topbar-right']) {
      const group = document.querySelector(selector);
      if (!group) continue;
      const box = group.getBoundingClientRect();
      if (box.width === 0) continue;
      const overlap = Math.min(box.right, navBox.right) - Math.max(box.left, navBox.left);
      if (overlap > 0) {
        problems.push('top bar overlap: ' + selector + ' and the tabs share ' + round(overlap) + 'px');
      }
    }
  }
  return problems;
})()
`;

async function main() {
  const indexHtml = await readFile(join(DIST, "index.html"), "utf8");
  const harness = harnessHtml(indexHtml);
  const server = await startServer(harness);
  await mkdir(OUT, { recursive: true });

  const browserPath = EDGE_CANDIDATES.find((path) => existsSync(path));
  if (!browserPath) throw new Error("no Chromium browser found");
  const profile = join(tmpdir(), `hexadof-layout-${Date.now()}`);
  const browser = spawn(
    browserPath,
    [
      "--headless=new",
      "--disable-gpu",
      "--no-first-run",
      "--no-default-browser-check",
      `--remote-debugging-port=${DEBUG_PORT}`,
      `--user-data-dir=${profile}`,
      `--window-size=${VIEWPORT.width},${VIEWPORT.height}`,
      `http://127.0.0.1:${PORT}/__harness.html`,
    ],
    { stdio: "ignore" },
  );

  const socketUrl = await waitForDebugger();
  const client = connect(socketUrl);
  await client.ready;
  await client.send("Runtime.enable");
  await client.send("Page.enable");
  await client.send("Emulation.setDeviceMetricsOverride", {
    width: VIEWPORT.width,
    height: VIEWPORT.height,
    deviceScaleFactor: 1,
    mobile: false,
  });

  const evaluate = async (expression) => {
    const result = await client.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (result.exceptionDetails) {
      throw new Error(`evaluate failed: ${result.exceptionDetails.text}`);
    }
    return result.result.value;
  };

  // Wait for the application to mount.
  for (let i = 0; i < 40; i += 1) {
    const mounted = await evaluate(`!!document.querySelector('.app-shell')`);
    if (mounted) break;
    await new Promise((done) => setTimeout(done, 250));
  }

  const report = {};
  for (const section of SECTIONS) {
    await evaluate(`
      (() => {
        const target = Array.from(document.querySelectorAll('.topnav-item'))
          .find((el) => el.textContent.trim().toLowerCase().includes(${JSON.stringify(section.label.toLowerCase())}));
        if (target) target.click();
        return !!target;
      })()
    `);
    await new Promise((done) => setTimeout(done, 900));
    if (section.prepare) {
      await evaluate(PREPARE[section.prepare]);
      // Let the page poll a few frames so its charts have data.
      await new Promise((done) => setTimeout(done, 2200));
      if (section.prepare === "forces") {
        await evaluate(PREPARE.forces);
      }
    }
    if (section.resultTab) {
      const picked = await evaluate(`
        (() => {
          const tab = Array.from(document.querySelectorAll('.result-tabs .tab'))
            .find((el) => el.textContent.trim().toLowerCase().startsWith(${JSON.stringify(
              section.resultTab.toLowerCase(),
            )}));
          if (tab) tab.click();
          return !!tab;
        })()
      `);
      if (!picked) console.log(`warning: the ${section.resultTab} result tab was not found`);
      await new Promise((done) => setTimeout(done, 500));
    }    // Give the pages a moment to run their own loaders.
    await new Promise((done) => setTimeout(done, 600));

    const measurements = {};
    for (const [name, expression] of Object.entries(PROBES)) {
      measurements[name] = await evaluate(expression);
    }
    measurements.problems = await evaluate(PROBLEM_PROBE);
    report[section.id] = measurements;

    const shot = await client.send("Page.captureScreenshot", { format: "png" });
    await writeFile(join(OUT, `${section.id}.png`), Buffer.from(shot.data, "base64"));
  }

  if (client.consoleErrors.length > 0) {
    report.consoleErrors = client.consoleErrors.slice(0, 20);
  }

  const summary = {};
  for (const [section, measurements] of Object.entries(report)) {
    if (section === "consoleErrors") continue;
    summary[section] = measurements.problems ?? [];
  }
  console.log(JSON.stringify(summary, null, 2));
  // The bar layout is reported for every screen, because a scrollbar or a toast
  // can change the width available to it.
  for (const [section, measurements] of Object.entries(report)) {
    if (section === "consoleErrors") continue;
    if (!measurements || !measurements.topbar) {
      console.log(`top bar ${section}: not measured`);
      continue;
    }
    const t = measurements.topbar;
    const cluster = (t.rightChildren ?? [])
      .filter((child) => child.width > 0)
      .map((child) => `${child.width}:${child.text || child.cls}`)
      .join(" | ");
    console.log(
      `top bar ${section}: nav ${t.nav?.left}-${t.nav?.right}, right ${t.right?.left}-${t.right?.right} [${cluster}]`,
    );
  }
  console.log(`screenshots: ${OUT}`);

  client.close();
  browser.kill();
  server.close();
  await rm(profile, { recursive: true, force: true }).catch(() => {});
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
