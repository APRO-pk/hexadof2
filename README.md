# HexaDOF

A six degree of freedom flight dynamics and flight data laboratory for Windows.

HexaDOF imports a vehicle dynamics model, integrates a 6-DOF trajectory, reads a
live sensor stream over USB serial, imports a flight computer log from an SD
card, replays it in 3D, and compares what the simulation predicted against what
the vehicle measured.

It is an analysis and instrumentation tool. A simulation is a model, not a test
flight, and nothing this application produces certifies flight safety.

## Contents

- [Build and run](#build-and-run)
- [The three workflows](#the-three-workflows)
- [Repository layout](#repository-layout)
- [Conventions](#conventions)
- [The project on disk](#the-project-on-disk)
- [Simple and Advanced modes](#simple-and-advanced-modes)
- [Aerodynamics](#aerodynamics)
- [The `.hlog` format](#the-hlog-format)
- [Testing](#testing)
- [What this application does not do](#what-this-application-does-not-do)

## Build and run

Prerequisites on Windows:

- Rust stable (the workspace declares `rust-version = "1.85"`)
- Node.js 18 or newer
- The MSVC build tools and a Windows SDK
- WebView2, which ships with current Windows versions

Install the frontend dependencies once:

```powershell
npm --prefix apps/desktop/frontend install
```

Run the application in development, with hot reload on the frontend:

```powershell
npm --prefix apps/desktop/frontend run dev
```

In a second terminal, from `apps/desktop/src-tauri`:

```powershell
..\frontend\node_modules\.bin\tauri.cmd dev
```

Build the release application and the Windows installers. The bundler produces
an MSI and an NSIS setup program:

```powershell
cd apps/desktop/src-tauri
..\frontend\node_modules\.bin\tauri.cmd build
```

The compiled executable lands in `target/release/hexadof-desktop.exe` and the
installers in `target/release/bundle/`.

Useful checks:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm --prefix apps/desktop/frontend run typecheck
```

The interface has a structural layout check that catches the class of fault a
unit test cannot see: a 3D view or a chart whose box has no resolvable height, so
it renders as nothing at all. It loads the built frontend in a headless browser
with a stubbed Tauri bridge, measures every panel, viewport, chart and column on
every screen, and writes a screenshot of each. Build the frontend first.

```powershell
node tools/layout-check/run.mjs
node tools/layout-check/run.mjs --simple      # the reduced interface
node tools/layout-check/run.mjs --narrow      # the smallest window allowed
node tools/layout-check/run.mjs --width=1490  # a breakpoint boundary
```

It also fails on a top bar where the centred workspace tabs and the status
cluster share a pixel, which is invisible until a project name grows long enough
to sit under a tab.

`tools/layout-check/capture-window.ps1` captures the running packaged
application instead, which also proves the assets load under the real webview and
content security policy. Add `-Attach` to capture a development session that is
already open.

## The three workflows

The whole interface is one bar and one workspace. The bar carries the product
name and the Simple or Advanced switch on the left, the workspace tabs centred,
and the project and status on the right: the running simulation, the device, the
save action, the command palette, the theme, and notifications. The workspace
under it is everything else.

### Simulation

Create a project, import a dynamics model, read the validation report, configure
a scenario, run it, and inspect the forces, moments, and state variables. A
completed run is written into the project once and then read only, so a saved
result cannot change underneath a report.

One button in the Configuration panel opens the example flight list. Each entry
is a complete, documented scenario for the built-in model, and loading one
installs it and validates it, so the screen goes from empty to ready to run in
one press:

| Example | What it shows |
|---|---|
| Sounding rocket | Boost, coast, apogee, and a ballistic descent |
| Starhopper hop | A short hard burn and a powered landing |
| Self landing hover slam | A long fall arrested by a landing burn |
| Spin stabilised | The attitude channels, with the rocket spun about its roll axis |
| Crosswind launch | Angle of attack and sideslip, and the drift the wind causes |
| Drop test | Pure dynamics: no motor, no thrust, energy drift reported |

The two landing examples depend on one piece of physics, a **landing burn** that
ignites on the stopping-distance condition: the engine lights when the vehicle is
descending, low, and close enough to the ground to stop at its own
thrust-to-weight. It is a trigger, not a guidance law. Nothing steers, and the
vehicle has to already be upright. A burn that cannot out-thrust the vehicle's
weight has no stopping distance at all, so validation refuses it rather than
running a flight that cannot land. The landing burn is its own force source, so
the pulse is visible in the force channel, and it is marked on the timeline.

The results of a run are five views, one per kind of answer:

| View | What it answers |
|---|---|
| Summary | Did it fly, how high, how fast, and was the integration sound |
| Plots | What did each channel do, one plot per tab, with the zoom shared with the timeline |
| Timeline | When did it happen, and against what altitude |
| Warnings | What should be read before drawing a conclusion |
| Metadata | What exactly produced this result, for reproducibility |

The seven plots are altitude, speed, attitude, angular rates, forces, aerodynamic
angles, and dynamic pressure with mass. Each one gets a whole tab and the height
of the panel, because a chart with a quarter of the panel to itself does not get
read.

The timeline shades the flight between the events that delimit its phases, draws
the recorded altitude behind the ruler, labels the time axis, and reports the
value under the pointer. Dragging across the profile zooms the timeline and every
chart to that window at once. Every phase boundary comes from an event the run
detected: a run with no burnout event has no boundary drawn there, because
inventing one would be a claim the data does not support.

### Live telemetry

Choose a serial port, load a device profile, map the packet fields onto
channels, calibrate the sensors, and watch the console. Recording is independent
of the display: the interface may drop display frames under load, but the
recorder accepts every decoded packet. Recorded sessions are stored with the
device's own timestamps, because a serial buffer drain delivers hundreds of
packets within one host clock tick.

### Flight log analysis

Import an SD card log as CSV or as a HexaDOF binary log, audit the timestamps
and data quality, replay the flight in 3D, mark events, align the log against a
simulation, compare the channels that both sides provide, and export a report.

## Repository layout

```
crates/
  hex-core         units, coordinate frames, vectors, mass properties,
                   quaternions, device timestamps, validation reports
  hex-dynamics     the 6-DOF state, environment, force and moment models,
                   integrators, event detection, the run loop, regression cases
  hex-model        the dynamics model schema, importer, validator, versioning
  hex-telemetry    serial transport, packet protocols, channel mapping,
                   calibration, attitude estimation, the ingestion pipeline
  hex-flight-data  the .hlog codec, CSV import, channel roles, derived channels,
                   timestamp validation, flight sessions
  hex-analysis     alignment, error metrics, event detection, the comparison
  hex-project      projects, run and session artifacts, settings, hashing
apps/desktop/
  src-tauri        the Tauri application: commands, state, events, capabilities
  frontend         React and TypeScript interface, themes, charts, 3D viewport
```

Each crate owns one subject and exposes a small surface. The numerical and
persistence logic lives in the crates, and the Tauri command layer is a thin
adapter over them, which is why the end to end workflow can be tested without a
window: `apps/desktop/src-tauri/tests/mvp_journey.rs` walks create, import,
validate, run, save, reopen, record, import, replay, compare, and export report
against the same functions the interface calls.

## Conventions

- **Units.** SI everywhere internally: metres, seconds, kilograms, newtons,
  radians, pascals, kelvin. Imperial input is converted at the import boundary
  and the original unit system is recorded with the model.
- **Quaternions.** Scalar first, `[w, x, y, z]`, always normalised, and always
  the body to world rotation, so `v_world = q * v_body`.
- **Frames.** The world frame is ENU or NED and the body frame is FRD or FLU.
  Both are declared by the model, shown in the interface, and never assumed
  silently. Angles of attack and sideslip are computed only when the frames
  needed for them are compatible, and say so when they are not.
- **State.** One flat 14 element vector: position, velocity, quaternion,
  angular velocity, mass. The quaternion norm is a constraint, not a fourteenth
  degree of freedom.
- **Interface.** The backend serialises Rust `snake_case` field names and the
  TypeScript types mirror them exactly, so there is no translation layer that
  can drift.
- **Honesty.** A derived value is labelled as derived, an estimate is labelled
  as an estimate, a relative attitude says that it drifts, and a comparison
  never names a single root cause.

## The project on disk

```
My Rocket/
  project.json       metadata, frames, unit system, model reference
  models/            imported dynamics models, copied and content hashed
  simulations/       one directory per run: results.hlog, config.json,
                     summary.json, events.json
  telemetry/         one .hlog plus one .json per recorded session
  flights/           the copied source log, metadata, mapping, validation,
                     and any derived channels
  reports/           exported reports
```

A run and a flight session are written once and never modified. The source log
is copied byte for byte and its hash is recorded, so the original on the SD card
and the copy in the project can be proven to match.

## Simple and Advanced modes

The switch next to the HexaDOF name in the top bar chooses how much of the
application is in front of you. It reads as the mode that is currently on, and
the same switch is in Settings under General. The choice is written to the
settings file immediately, so it survives a restart.

**Advanced** is the default and shows every control described in this document.

**Simple** keeps the whole workflow and hides the controls that only matter to
someone tuning a model or wiring up hardware:

| Kept | Hidden |
|---|---|
| Overview, Dynamics, Flight Analysis, Projects, Settings | Live Telemetry, and with it device profiles, packet formats, channel mapping, calibration, and the estimator settings |
| Import and validate a dynamics model | Solver family, tolerances, and step size |
| Initial state: mass, inertia, attitude, position, velocity | Environment overrides: gravity model, atmosphere, air density, wind, ground elevation |
| Forces: thrust, burn time, drag, and aerodynamics taken from the imported model | The aerodynamic model picker and its derivatives, and control or TVC moments |
| Output interval, and the run itself | Channel selection for plotting, and the run metadata view |
| The example flight list | Nothing: examples are the shortest path into the tool, so Simple mode keeps them |
| The 3D view and its display options, plotting, events, and markers | Raw packet inspection and sensor validation |
| Flight log import, replay, event detection, comparison, and report export | The timestamp audit, quality table, channel mapping, and alignment method |
| General and appearance preferences | Dynamics, telemetry, storage, and diagnostic defaults |

Two rules keep the mode honest. A command that drives a hidden control is not
offered in the command palette, so the list never promises an action the
interface cannot carry out. And the run warnings are never hidden: a simple run
still states which aerodynamic model it used and how simplified it was.

## Aerodynamics

The engine has one coefficient model, and several ways to fill it in. The choice
is on the Dynamics screen, under Forces, as **Aerodynamic model**:

| Choice | What it uses |
|---|---|
| From the imported model | The drag table indexed against Mach, plus the lift, side-force, and moment derivatives the model declares. Offered only when the model declares them. |
| Imported model coefficients, required | The same, and a blocking validation error when the model declares none, so a run can never silently fall back to an estimate. |
| Built-in estimate, drag only | One constant drag coefficient, no lift and no moments. The default when no model coefficients exist. |
| Built-in estimate with lift and moments | A constant drag coefficient plus lift and moment derivatives chosen as conventions, for seeing what those terms do. |
| None | No aerodynamics, which suits a vacuum or a verification case. |

The model form is the standard coefficient one: drag along the negative
relative wind, lift perpendicular to it, side force along body Y, and roll, pitch
and yaw moments scaled by `q_dyn * S * L` with rates non-dimensionalised by
`2 * V`. It is a coefficient model. It is not a CFD solution and not a
measurement, and every run states which coefficients it used and how simplified
the model was, in the run warnings and in the exported report.

The model schema accepts `drag_coefficient_at_mach` as a table, so drag can vary
with Mach. The table must be in ascending Mach order; an out of order or
non-finite table is a blocking import error, because interpolation over an
unordered table reads the wrong coefficient without failing.

## The `.hlog` format
Sessions, run histories, and derived channels all use one binary log format, so
one verified reader serves every path. Everything is little endian, real values
are IEEE-754 binary64, and records are independently checksummed with CRC-32. A
record that fails its checksum is skipped, the reader resynchronises on the next
record marker, and the failure is counted rather than hidden. A record holds one
timestamp and places its samples on the declared nominal interval, so a
discontinuity starts a new record and a dropped packet stays a gap instead of
being smeared into uniform samples.

The full layout is documented in `crates/hex-flight-data/src/binary.rs`.

## Testing

The workspace has 961 tests, all of which run without hardware:

- Unit tests in every crate, including the analytic verification cases the
  specification lists: free translation, uniform gravity, constant torque, a
  torque free asymmetric body, quaternion rotation, a force applied at an
  offset, and frame transforms.
- An end to end workflow test through the desktop command layer.
- An example flight test, which flies the exact scenarios the example list
  installs and fails if one stops being a plausible flight.
- Landing tests, which fly both powered landings and check the speed at
  touchdown, and check that the same vehicle without its landing burn arrives
  hard. "It landed" is a claim about a number, so it is tested as one.
- Provenance and wording tests, which fail if the tool starts claiming more
  than it knows.
- A structural layout check of every screen in a headless browser, which treats
  a collapsed 3D view or a mis-sized card as a failure rather than a matter of
  opinion. It runs in Advanced and Simple modes, at the widest and narrowest
  window, and over every result view.

The live telemetry path is exercised with a scripted byte source and with the
reference binary frame format, so packet decoding, calibration, estimation,
recording, and the codec round trip are all covered without a serial port.

## What this application does not do

Stated plainly, because a tool that overstates itself is worse than one that
does less:

- No flight certification, no safety case, and no launch decision support.
- No aerodynamic coefficient estimation beyond what an imported model declares.
  The built in drag estimate is an estimate.
- No absolute attitude from a gyroscope and accelerometer alone. Without a
  magnetometer or a GNSS course the yaw is unobservable, and the interface says
  so wherever an attitude is shown.
- No fluid structure interaction, no fin flutter, no thermal model, and no
  six degree of freedom parachute or recovery dynamics.
- No cloud service. A project is a local directory.
