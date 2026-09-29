# HexaDOF

## End-to-End Product, UI, Software Architecture, and Dynamics Specification

**Product:** HexaDOF\
**Brand:** APRO Works\
**Platform:** Windows desktop application\
**Desktop stack:** Tauri + Rust\
**Frontend:** Any modern web UI framework compatible with Tauri\
**Primary purpose:** Simulate, observe, record, replay, and compare the
six-degree-of-freedom motion of a rocket or other rigid aerospace body.

------------------------------------------------------------------------

# 1. Product Definition

HexaDOF is not a CAD system, rocket builder, motor-design application,
or complete mission-design suite.

It is a focused dynamics and telemetry workspace that connects existing
APRO Works tools with real-world hardware.

The core workflow is:

> **Import a dynamics model → configure and run a 6-DOF simulation →
> connect an Arduino or flight computer → visualize live sensor data →
> import an SD-card flight log → replay the flight → compare measured
> and simulated behavior.**

HexaDOF should be easy enough for a hobby rocket user to operate, while
retaining a technically correct architecture for researchers and
advanced users.

## 1.1 MVP scope

The first production version should include:

-   Rigid-body 6-DOF simulation
-   3-DOF point-mass simulation as a simplified mode
-   Imported dynamic model files
-   Position, velocity, orientation, and angular-rate states
-   Quaternion-based attitude representation
-   Gravity
-   Configurable forces and moments
-   Basic atmosphere, wind, drag, and thrust inputs
-   Numerical integration with fixed-step RK4 and optional adaptive
    solver support
-   USB serial telemetry
-   Configurable packet decoding
-   Sensor calibration and coordinate-frame mapping
-   Live 3D rocket visualization
-   Flight-log import from CSV and a documented binary format
-   Time-series plots
-   Event markers
-   Flight replay
-   Simulation-versus-flight comparison
-   Dark and light themes
-   Reproducible project files and run metadata

## 1.2 Explicitly out of scope for the MVP

Do not initially build:

-   A full CAD editor
-   Solid-motor grain design
-   Liquid-engine design
-   Full aerodynamic CFD
-   FEA or structural flexibility
-   Fluid slosh simulation
-   Arbitrary user-authored multibody mechanisms
-   Advanced guidance and control design
-   Full mission planning
-   Cloud collaboration
-   Automatic certification or flight safety approval

The architecture may be extensible toward these areas, but they should
not complicate the first user experience.

------------------------------------------------------------------------

# 2. Design Principles

## 2.1 Correctness before visual smoothness

The 3D rocket animation must never be treated as the source of truth.
The numerical state and recorded telemetry are authoritative.

The application must distinguish:

-   Simulated state
-   Raw sensor measurements
-   Filtered sensor estimates
-   Reconstructed flight state
-   Display interpolation

## 2.2 Explicit coordinate frames

Every vector must have a documented frame.

Recommended conventions:

-   **World/inertial frame:** North-East-Down (NED) or East-North-Up
    (ENU), selected per project
-   **Body frame:** X forward, Y right, Z down, or another explicit
    convention
-   **Sensor frame:** Native sensor axes
-   **Display frame:** Coordinate system required by the 3D renderer

Do not silently assume that the Arduino's X/Y/Z axes match the rocket
body axes.

## 2.3 SI units internally

Use SI units internally:

-   Distance: metres
-   Time: seconds
-   Velocity: metres per second
-   Acceleration: metres per second squared
-   Angular velocity: radians per second
-   Angular acceleration: radians per second squared
-   Force: newtons
-   Moment: newton-metres
-   Mass: kilograms
-   Inertia: kilogram-metres squared
-   Pressure: pascals
-   Temperature: kelvin

Users may display degrees, kilometres per hour, grams, bar, and other
units, but conversions should occur at the UI boundary.

## 2.4 Reproducibility

Every simulation run must store:

-   Model version
-   Environment settings
-   Force and moment models
-   Solver
-   Step size or tolerance
-   Initial conditions
-   Random seed, if noise is enabled
-   Application version
-   Timestamp
-   Input files and hashes
-   Warnings and validation results

A user should be able to reopen a run and reproduce it as closely as the
selected numerical configuration permits.

------------------------------------------------------------------------

# 3. End-to-End User Workflow

## 3.1 Workflow A --- Simulation

1.  Create or open a HexaDOF project.
2.  Import a dynamics model exported by another APRO Works application.
3.  Validate mass, centre of gravity, inertia, reference frame, and
    units.
4.  Select 3-DOF or rigid-body 6-DOF mode.
5.  Configure initial position, velocity, attitude, and angular
    velocity.
6.  Configure environment:
    -   Gravity
    -   Atmosphere
    -   Wind
    -   Launch rail or constraint, if applicable
7.  Configure force and moment providers:
    -   Thrust
    -   Aerodynamic forces
    -   Drag
    -   Control or TVC moment
    -   User-defined external loads
8.  Select numerical solver and output rate.
9.  Run validation checks.
10. Run the simulation.
11. Inspect the 3D motion, plots, events, and warnings.
12. Save the run as an immutable result artifact.

## 3.2 Workflow B --- Live telemetry

1.  Open Live Telemetry.
2.  Connect the microcontroller through USB.
3.  Select or create a device profile.
4.  Select the serial port and baud rate.
5.  Select the packet format.
6.  Map packet fields to physical channels.
7.  Configure units, sensor axes, signs, and mounting offsets.
8.  Run a sensor validation procedure.
9.  Confirm packet rate, timestamps, ranges, and missing packets.
10. Start the live session.
11. View:
    -   Raw sensor values
    -   Filtered values
    -   Estimated orientation
    -   Digital rocket
    -   Connection health
12. Record the session.
13. Stop recording and save the session.

## 3.3 Workflow C --- Flight-log analysis

1.  Open Flight Analysis.
2.  Import a CSV or HexaDOF binary log.
3.  Select the time column.
4.  Map channels:
    -   Accelerometer
    -   Gyroscope
    -   Magnetometer
    -   Barometer
    -   GNSS
    -   Orientation
    -   Motor or control channels
5.  Configure sensor frames and units.
6.  Validate timestamp monotonicity and sampling rate.
7.  Detect gaps, saturation, clipping, and invalid values.
8.  Create a flight session.
9.  Run orientation and state estimation if required.
10. Replay the flight in 3D.
11. Add event markers.
12. Plot channels and derived quantities.
13. Load a simulation run.
14. Align simulation and measured data.
15. Compare trajectories, attitude, angular rates, and events.
16. Export plots, tables, and a report.

------------------------------------------------------------------------

# 4. Information Architecture

The main navigation should be a compact left sidebar.

## 4.1 Navigation

-   **Overview**
-   **Dynamics**
-   **Live Telemetry**
-   **Flight Analysis**
-   **Projects**
-   **Settings**

The active section should be obvious through a filled background, a thin
accent indicator, and a clear icon.

## 4.2 Global top bar

The top bar contains:

-   Current project name
-   Current workspace
-   Unsaved changes indicator
-   Simulation status
-   Device connection status
-   Theme switch
-   Notifications
-   Help
-   User/project menu

The top bar must not become overloaded with technical controls.
Technical settings belong in contextual panels.

## 4.3 Global command palette

Support a command palette with:

-   Open project
-   Import model
-   Run validation
-   Start simulation
-   Connect device
-   Start recording
-   Import flight log
-   Add event marker
-   Export report
-   Toggle theme
-   Open settings

Keyboard shortcut: `Ctrl + K`.

------------------------------------------------------------------------

# 5. Visual Design System

HexaDOF should feel like a modern engineering instrument rather than a
generic dashboard.

## 5.1 Visual language

Use:

-   Clean geometric layout
-   Moderate corner radii
-   Fine borders
-   Soft depth instead of excessive shadows
-   High information density with generous spacing
-   Clear hierarchy
-   Monospaced typography for numeric telemetry
-   Smooth but restrained transitions
-   Strong focus states
-   Consistent chart styling

Avoid:

-   Excessive glassmorphism
-   Overly bright gradients
-   Huge decorative cards
-   Excessive neon
-   Hidden technical state
-   Low-contrast text
-   Animations that interfere with analysis

## 5.2 Theme system

Provide two complete themes:

### Dark theme

-   Background: near-black charcoal
-   Main panels: dark graphite
-   Secondary panels: slightly lighter graphite
-   Borders: low-contrast cool grey
-   Text: off-white
-   Secondary text: muted grey
-   Accent: electric blue or cyan
-   Warning: amber
-   Error: red
-   Success: green

### Light theme

-   Background: warm white or cool off-white
-   Main panels: white
-   Secondary panels: light grey
-   Borders: neutral grey
-   Text: dark charcoal
-   Secondary text: slate grey
-   Accent: deep blue
-   Warning: amber
-   Error: red
-   Success: green

The same semantic color must preserve meaning across both themes. Do not
use color as the only indicator; pair it with labels or icons.

## 5.3 Typography

Recommended hierarchy:

-   Page title: 24--30 px
-   Section title: 16--20 px
-   Body: 13--15 px
-   Metadata: 11--12 px
-   Numeric telemetry: monospaced 12--15 px
-   Chart labels: 11--12 px

Use a modern sans-serif font for the interface and a monospaced font for
values, timestamps, packet fields, and logs.

## 5.4 Layout grid

Use a 12-column responsive grid for desktop panels.

Typical layout:

-   Sidebar: 224--260 px
-   Top bar: 56--64 px
-   Main content padding: 24--32 px
-   Panel radius: 10--16 px
-   Standard spacing: 8, 12, 16, 24, 32 px

Allow panel resizing in analysis-heavy workspaces.

------------------------------------------------------------------------

# 6. Overview Screen

The Overview screen should answer:

-   What project is open?
-   What was the last simulation?
-   Is a device connected?
-   What flight logs are available?
-   Are there validation problems?
-   What should the user do next?

## 6.1 Layout

### Header

-   Project title
-   Project description
-   Last modified time
-   Primary action: `Open Workspace`
-   Secondary action: `Import Model`

### Status strip

Four compact status cards:

1.  Model status
2.  Last simulation
3.  Device status
4.  Data status

Each card includes a status label, timestamp, and direct action.

### Recent activity

Show:

-   Recent simulation runs
-   Imported flight logs
-   Telemetry sessions
-   Validation reports

### Quick actions

-   New project
-   Import dynamics model
-   Run simulation
-   Connect device
-   Import flight log

### Warnings

A dedicated warning panel displays:

-   Missing inertia tensor
-   Unknown units
-   Invalid timestamps
-   Unmapped sensor channels
-   Non-normalized quaternion
-   Solver instability
-   Missing environment parameters

------------------------------------------------------------------------

# 7. Project Structure

A HexaDOF project should be a directory with explicit, human-readable
metadata.

Example:

``` text
my-flight-project/
├── project.hexadof.json
├── models/
│   └── rocket.dynamic.json
├── simulations/
│   ├── run-001/
│   │   ├── config.json
│   │   ├── results.hdf5
│   │   └── summary.json
│   └── run-002/
├── telemetry/
│   ├── device-profile.json
│   └── session-001.hlog
├── flights/
│   ├── flight-001/
│   │   ├── source.csv
│   │   ├── mapping.json
│   │   ├── validation.json
│   │   └── derived.hlog
├── reports/
└── assets/
```

The exact storage format can evolve. The logical separation must remain
stable.

## 7.1 Project metadata

``` json
{
  "schema_version": "1.0",
  "project_id": "uuid",
  "name": "Example Rocket",
  "description": "6-DOF test project",
  "unit_system": "SI",
  "world_frame": "ENU",
  "created_at": "ISO-8601",
  "updated_at": "ISO-8601",
  "model_reference": "models/rocket.dynamic.json"
}
```

------------------------------------------------------------------------

# 8. Dynamics Model Import

HexaDOF should import a dynamics model from existing APRO Works software
rather than rebuilding the rocket.

## 8.1 Dynamic model responsibilities

The imported model should contain only information required to calculate
or visualize motion.

Suggested fields:

-   Model ID and version
-   Display mesh reference
-   Total mass
-   Mass variation model, if available
-   Centre of gravity
-   Inertia tensor
-   Body reference frame
-   Reference area and length
-   Thrust source reference
-   Aerodynamic coefficient source
-   Force application points
-   Moment application points
-   Optional launch rail parameters
-   Optional control surface or TVC metadata

## 8.2 Required validation

Before accepting a model, validate:

-   Mass is greater than zero
-   All units are known
-   Centre of gravity is finite
-   Inertia tensor is symmetric within tolerance
-   Principal moments are physically valid
-   Inertia tensor is positive definite
-   Reference frame is explicitly declared
-   Mesh scale is known
-   Force application points use the same frame as the CG
-   Time-dependent mass data is monotonic and physically plausible

The user must see validation errors in plain language.

Example:

> **Invalid inertia tensor:** The supplied tensor is not positive
> definite. Check the mass properties export and body reference frame.

## 8.3 Model import UI

Use a three-step import dialog:

### Step 1 --- File

-   Drag-and-drop zone
-   Browse button
-   Supported file types
-   File metadata

### Step 2 --- Interpret

-   Unit system
-   World frame
-   Body frame
-   Mesh orientation
-   Mass-property source

### Step 3 --- Validate

-   Validation summary
-   Expandable technical details
-   Fix suggestions
-   `Import Model` button disabled until blocking errors are resolved

------------------------------------------------------------------------

# 9. Dynamics Workspace

The Dynamics workspace is the main simulation environment.

## 9.1 Layout

Use a three-region layout:

``` text
┌─────────────────────────────────────────────────────────────┐
│ Run controls / scenario / solver / status                   │
├───────────────┬───────────────────────────────┬─────────────┤
│ Configuration │ 3D viewport                   │ State panel │
│ panel         │                               │             │
│               │                               │             │
├───────────────┴───────────────────────────────┴─────────────┤
│ Timeline / plots / events                                    │
└─────────────────────────────────────────────────────────────┘
```

The user should be able to collapse the configuration and state panels
to maximize the viewport.

## 9.2 Run toolbar

Include:

-   Scenario selector
-   Model selector
-   Solver selector
-   Start time
-   End time
-   Output rate
-   Validate button
-   Run button
-   Pause button
-   Stop button
-   Save run button

Run button states:

-   Disabled: blocking validation errors
-   Ready: valid configuration
-   Running: progress and stop
-   Complete: open results
-   Failed: show diagnostic panel

## 9.3 Configuration tabs

### Model

-   Imported model
-   Mass properties
-   CG
-   Inertia tensor
-   Reference frame
-   Visualization mesh

### Initial state

-   Position
-   Velocity
-   Quaternion or Euler input
-   Angular velocity
-   Initial time
-   Initial mass

Euler angles may be offered for convenience, but the internal state must
use quaternions.

### Environment

-   Gravity model
-   Atmosphere model
-   Wind
-   Air density source
-   Temperature
-   Pressure
-   Launch location
-   Ground altitude

### Forces

-   Thrust
-   Aerodynamic forces
-   Drag
-   Lift
-   Control or TVC
-   External force inputs
-   Force application points

### Moments

-   Aerodynamic moments
-   Thrust misalignment
-   TVC moment
-   External moments
-   Moment application points

### Solver

-   Integrator
-   Fixed time step
-   Relative tolerance
-   Absolute tolerance
-   Maximum step
-   Output sampling rate
-   Event detection
-   Quaternion normalization policy

### Outputs

-   State channels
-   Force channels
-   Moment channels
-   Sensor simulation
-   Export format
-   Data compression

------------------------------------------------------------------------

# 10. 6-DOF Mathematical Model

The core model should use rigid-body Newton-Euler dynamics.

The implementation must document every convention and test it
independently.

## 10.1 State representation

A recommended state is:

``` text
x = [
  r_I       position in inertial frame, 3
  v_I       velocity in inertial frame, 3
  q_BI      body-to-inertial quaternion, 4
  ω_B       angular velocity in body frame, 3
  m         mass, 1
]
```

This produces 14 stored values, although the quaternion has one
unit-norm constraint. If mass is constant, mass may be treated as a
parameter rather than a state.

Use scalar-first quaternion convention:

``` text
q = [q0, q1, q2, q3]
```

with:

``` text
q0² + q1² + q2² + q3² = 1
```

The software must document whether the quaternion maps body vectors to
inertial vectors or the reverse. HexaDOF should standardize on one
convention and test every transform against known rotations.

## 10.2 Translational dynamics

In the inertial frame:

``` text
dr_I/dt = v_I

m * dv_I/dt = F_total_I
```

where:

``` text
F_total_I =
    F_gravity_I
  + F_thrust_I
  + F_aero_I
  + F_external_I
```

Forces calculated in body coordinates must be rotated into the inertial
frame before being summed with inertial-frame forces.

If gravity is treated as a uniform local field:

``` text
F_gravity_I = m * g_I
```

For a more general central gravity model:

``` text
g_I(r) = -μ * r / ||r||³
```

The selected gravity model must be visible in the run configuration.

## 10.3 Rotational dynamics

Angular velocity is expressed in the body frame.

The rigid-body equation is:

``` text
I_B * dω_B/dt + ω_B × (I_B * ω_B) = M_total_B
```

Therefore:

``` text
dω_B/dt =
    I_B⁻¹ * (M_total_B - ω_B × (I_B * ω_B))
```

where:

``` text
M_total_B =
    M_aero_B
  + M_thrust_B
  + M_control_B
  + M_external_B
```

The inertia tensor must be evaluated about the centre of mass and
expressed in the same body frame as angular velocity and moments.

For a force applied at point `r_B` relative to the centre of mass:

``` text
M_B = r_B × F_B
```

Do not apply moments about an arbitrary reference point without
transforming them correctly.

## 10.4 Quaternion kinematics

For body-frame angular velocity:

``` text
ω_B = [p, q, r]
```

and scalar-first quaternion, use a single documented multiplication
convention.

One common form is:

``` text
dq/dt = 1/2 * q ⊗ [0, ω_B]
```

The exact matrix form depends on whether the quaternion represents
body-to-inertial or inertial-to-body rotation. The implementation must
include unit tests using:

-   Zero angular velocity
-   Constant rotation about X
-   Constant rotation about Y
-   Constant rotation about Z
-   Composition of two rotations
-   Inverse transform consistency

After integration, normalize the quaternion:

``` text
q ← q / ||q||
```

Normalization is a numerical safeguard, not a substitute for a correct
integration method.

## 10.5 Variable mass

If mass changes:

``` text
dm/dt = -ṁ_propellant
```

The model must define whether thrust and mass flow are already supplied
consistently by the propulsion source.

The centre of gravity and inertia may vary with time. The model
interface should support:

``` text
mass(t)
center_of_gravity(t)
inertia(t)
```

The MVP may initially support constant mass and a time-varying mass
curve, but it must not silently use a fixed inertia tensor when the
imported model declares significant variation.

## 10.6 Aerodynamic forces

The aerodynamic model should be modular.

At minimum, support:

-   Drag
-   Lift
-   Side force
-   Pitching moment
-   Yawing moment
-   Rolling moment

Dynamic pressure:

``` text
q_dyn = 1/2 * ρ * V_rel²
```

A force coefficient model may use:

``` text
F = q_dyn * S_ref * C
```

where `C` is the relevant coefficient vector.

Relative air velocity must account for wind:

``` text
V_rel_I = V_vehicle_I - V_wind_I
```

The relative velocity must then be transformed into the body frame
before calculating angle of attack, sideslip, and aerodynamic
coefficients.

The UI must clearly label simplified aerodynamic models as
approximations.

## 10.7 Thrust and application points

Thrust must include:

-   Magnitude
-   Direction in body frame
-   Application point
-   Time profile
-   Optional misalignment
-   Optional gimbal angle
-   Optional uncertainty

The resulting moment is:

``` text
M_thrust_B = r_thrust_B × F_thrust_B + M_engine_B
```

The application point must be measured from the centre of gravity, not
from the nose or another arbitrary datum.

------------------------------------------------------------------------

# 11. Numerical Integration

## 11.1 Solver strategy

The MVP should implement:

1.  Fixed-step RK4 as a transparent reference solver
2.  A second solver option later, such as adaptive RK45 or an
    appropriate stiff solver

RK4 is useful as a predictable baseline, but the application must not
imply that one solver is correct for every scenario.

## 11.2 Step-size rules

The UI must distinguish:

-   Integration step
-   Output sampling interval
-   Rendering frame rate

These are not the same.

For example:

-   Integration: 0.0005 s
-   Output: 0.01 s
-   Rendering: 60 FPS

The renderer should interpolate recorded states instead of changing the
numerical integration step.

## 11.3 Stability and accuracy checks

Each run should calculate:

-   Quaternion norm error
-   Energy trend where energy should be conserved
-   Momentum trend for force-free cases
-   Maximum angular rate
-   Maximum acceleration
-   Solver step rejection count
-   Constraint violation
-   NaN and infinity detection
-   Ground intersection or other configured events

The application should stop with a diagnostic if a state becomes
non-finite.

## 11.4 Verification test cases

The Rust dynamics engine must include automated tests for:

### Test 1 --- Free translation

No forces, no gravity:

-   Position changes linearly
-   Velocity remains constant
-   Orientation remains constant
-   Angular velocity remains constant

### Test 2 --- Uniform gravity

No aerodynamic forces:

-   Vertical acceleration matches configured gravity
-   Horizontal velocity remains constant when appropriate

### Test 3 --- Constant torque

Compare numerical angular acceleration against:

``` text
α = I⁻¹ * M
```

for a body initially at zero angular velocity.

### Test 4 --- Torque-free asymmetric body

Check qualitative conservation of angular momentum and compare against a
trusted reference implementation.

### Test 5 --- Quaternion rotation

Apply a known constant angular rate and compare the final attitude
against the analytical solution.

### Test 6 --- Force at offset

Verify that:

``` text
M = r × F
```

has the correct sign and magnitude.

### Test 7 --- Frame transform

Transform a vector body-to-world and back. The result must match the
original vector within tolerance.

### Test 8 --- Regression scenario

Keep a versioned scenario with known outputs and tolerances. Run it in
CI.

------------------------------------------------------------------------

# 12. Simulation Results Workspace

After a run, open a results workspace rather than returning to the setup
screen.

## 12.1 Header

Display:

-   Run name
-   Completion status
-   Duration
-   Solver
-   Step size
-   Model version
-   Warning count
-   Export button
-   Compare button

## 12.2 3D viewport

Features:

-   Rocket mesh
-   World axes
-   Body axes
-   Trajectory line
-   Velocity vector
-   Angular-rate indicator
-   Force vectors
-   Moment vectors
-   Ground plane
-   Camera presets
-   Follow mode
-   Free orbit
-   Top, side, front, and world views
-   Play, pause, step, and scrub

A legend must explain every visual vector.

## 12.3 State inspector

Show:

-   Position
-   Velocity
-   Speed
-   Acceleration
-   Quaternion
-   Roll, pitch, yaw as display-only derived values
-   Angular velocity
-   Angular acceleration
-   Mass
-   Total force
-   Total moment
-   Dynamic pressure
-   Angle of attack
-   Sideslip
-   Event state

Use expandable sections and copy-to-clipboard controls.

## 12.4 Plot panel

Support:

-   Multiple synchronized charts
-   Shared time cursor
-   Zoom and pan
-   Axis locking
-   Unit selection
-   Channel search
-   Show/hide series
-   Min/max/mean statistics
-   Export image
-   Export CSV

Recommended default charts:

1.  Altitude and vertical velocity
2.  Speed and acceleration
3.  Roll, pitch, yaw
4.  Angular rates
5.  Total force and total moment
6.  Angle of attack and sideslip

## 12.5 Event timeline

Events may include:

-   Ignition
-   Lift-off
-   Rail exit
-   Maximum dynamic pressure
-   Motor burnout
-   Apogee
-   Recovery deployment
-   Ground impact
-   Sensor dropout
-   Solver warning

Events can be generated automatically or added manually.

------------------------------------------------------------------------

# 13. Live Telemetry Workspace

The Live Telemetry workspace must be designed as an instrumentation
console.

## 13.1 Layout

``` text
┌─────────────────────────────────────────────────────────────┐
│ Device / port / packet rate / recording controls            │
├─────────────────────┬───────────────────────────┬───────────┤
│ Connection panel    │ Live 3D rocket            │ Health    │
│                     │                           │ panel     │
├─────────────────────┴───────────────────────────┴───────────┤
│ Raw channels / filtered channels / packet inspector         │
├─────────────────────────────────────────────────────────────┤
│ Live plots and event controls                               │
└─────────────────────────────────────────────────────────────┘
```

## 13.2 Connection panel

Display:

-   Serial port
-   Device name
-   Baud rate
-   Connection state
-   Packet format
-   Packets per second
-   Bytes per second
-   Dropped packets
-   CRC failures
-   Last packet time
-   Reconnect button

Connection states:

-   Disconnected
-   Connecting
-   Connected
-   Receiving
-   Stalled
-   Error

## 13.3 Device profiles

A device profile should store:

-   Device ID
-   Serial settings
-   Packet protocol
-   Channel mapping
-   Sensor model
-   Axis mapping
-   Sign conventions
-   Calibration parameters
-   Sampling rate
-   Timestamp source
-   Orientation estimator settings

Profiles must be reusable across sessions.

## 13.4 Packet formats

Support two initial modes:

### CSV line mode

Example:

``` text
timestamp,ax,ay,az,gx,gy,gz,baro
123.450,0.12,0.03,9.81,0.01,-0.02,0.04,101325
```

### Binary framed mode

Recommended frame fields:

``` text
SYNC
VERSION
MESSAGE_TYPE
SEQUENCE
DEVICE_TIMESTAMP
PAYLOAD_LENGTH
PAYLOAD
CRC
```

The binary format must define:

-   Endianness
-   Integer widths
-   Floating-point representation
-   Timestamp units
-   CRC algorithm
-   Maximum payload size
-   Version compatibility
-   Error handling

Never parse arbitrary binary data without a frame boundary and integrity
check.

## 13.5 Channel mapping UI

Use a table:

  Packet field   Meaning          Unit    Frame    Sign   Destination
  -------------- ---------------- ------- -------- ------ -------------
  ax             Acceleration X   m/s²    Sensor   \+     Accel X
  gy             Angular rate Y   rad/s   Sensor   \+     Gyro Y
  baro           Pressure         Pa      N/A      \+     Barometer

Provide:

-   Dropdown destination
-   Unit conversion
-   Sign inversion
-   Axis swap
-   Sensor-to-body rotation
-   Preview of transformed values

## 13.6 Sensor validation

Before recording, provide a guided validation checklist:

-   Device responds
-   Packet rate is stable
-   Timestamps increase
-   Accelerometer is within expected range
-   Gyroscope is within expected range
-   No channel is permanently zero
-   No channel is saturated
-   Axis mapping is confirmed
-   Static gravity direction is plausible
-   Calibration is saved

The user should be able to mark the device orientation physically, such
as:

-   Rocket nose upward
-   Rocket nose downward
-   Rocket lying on its side
-   Device stationary

The software should compare expected and observed sensor behavior.

------------------------------------------------------------------------

# 14. Orientation and State Estimation

## 14.1 Raw sensors versus estimated state

The application must clearly distinguish:

-   Raw accelerometer
-   Raw gyroscope
-   Calibrated accelerometer
-   Calibrated gyroscope
-   Estimated orientation
-   Estimated position
-   Estimated velocity

A gyroscope provides angular-rate measurements, not absolute
orientation. Integrating gyro data accumulates drift. Accelerometers can
provide a gravity reference when non-gravitational acceleration is
small, but rocket flight acceleration makes this assumption unreliable.
Magnetometers, GNSS, barometers, external references, and known launch
conditions may improve estimation.

## 14.2 MVP estimator

The first estimator may provide:

-   Gyro bias calibration
-   Quaternion propagation using gyro data
-   Optional accelerometer correction when conditions are suitable
-   Optional complementary filter
-   Clear estimator health state

Do not label gyro-only orientation as absolute or drift-free.

## 14.3 Estimator health

Display:

-   Estimated attitude
-   Gyro bias estimate
-   Correction strength
-   Innovation or residual, if applicable
-   Sensor saturation
-   Data age
-   Confidence or health indicator
-   Current estimator mode

If an estimator becomes unreliable, show a visible warning and
optionally freeze the last valid orientation.

## 14.4 Digital rocket display

The 3D model should support:

-   Raw integrated attitude
-   Filtered attitude
-   External orientation supplied by the device
-   Replayed orientation from a flight log
-   Simulated orientation

The display must include a small label indicating the source:

> `ATTITUDE SOURCE: EKF ESTIMATE`

Do not present a filtered estimate as measured truth.

------------------------------------------------------------------------

# 15. Flight Analysis Workspace

The Flight Analysis workspace is designed for post-flight investigation.

## 15.1 Import dialog

Support:

-   CSV
-   HexaDOF binary logs
-   Supported exported telemetry formats
-   Drag-and-drop import
-   Multiple file import
-   File preview
-   Encoding selection
-   Delimiter selection
-   Header row selection
-   Timestamp column selection

## 15.2 Import pipeline

``` text
File selection
→ Parsing
→ Channel discovery
→ Channel mapping
→ Unit conversion
→ Frame conversion
→ Timestamp validation
→ Sensor calibration
→ Derived-state generation
→ Flight session creation
```

Each stage must produce a status and diagnostic output.

## 15.3 Timestamp validation

Check:

-   Monotonicity
-   Duplicate timestamps
-   Negative time differences
-   Large gaps
-   Sampling-rate changes
-   Clock rollover
-   Timestamp units
-   Device versus host time
-   Start-time offset

Show a sampling-rate chart and a gap list.

## 15.4 Data quality panel

Flag:

-   Missing samples
-   NaN values
-   Infinite values
-   Clipped accelerometer channels
-   Clipped gyro channels
-   Pressure jumps
-   Unrealistic angular rates
-   Constant channels
-   Time reversals
-   Packet sequence gaps

Every flag should have:

-   Severity
-   Time range
-   Affected channel
-   Suggested action
-   Ignore/resolve status

## 15.5 Flight session

A flight session is a normalized data product containing:

-   Source file metadata
-   Mapping configuration
-   Calibration configuration
-   Coordinate-frame definition
-   Timebase
-   Raw channels
-   Corrected channels
-   Derived channels
-   Events
-   Quality flags
-   Estimator configuration

The original file must remain unchanged.

------------------------------------------------------------------------

# 16. Flight Replay

## 16.1 Replay controls

Include:

-   Play/pause
-   Playback speed
-   Step forward/backward
-   Jump to event
-   Timeline scrubber
-   Follow rocket
-   Camera mode
-   Show vectors
-   Show sensor axes
-   Show trajectory
-   Show uncertainty, if available

Playback speed options:

-   0.1×
-   0.25×
-   0.5×
-   1×
-   2×
-   5×
-   10×

## 16.2 Replay modes

-   Orientation-only replay
-   Position and orientation replay
-   Estimated-state replay
-   Sensor-derived replay
-   Simulation replay
-   Side-by-side replay

The application must indicate which channels are available. If position
is unavailable, do not fabricate a trajectory.

## 16.3 Derived position

If position is reconstructed by integrating acceleration, display a
warning:

> Position is estimated by integration and may drift substantially
> without external position references.

------------------------------------------------------------------------

# 17. Simulation and Flight Comparison

Comparison is a first-class feature, not an afterthought.

## 17.1 Alignment options

Allow alignment by:

-   Absolute timestamp
-   First valid sample
-   Lift-off event
-   User-selected event
-   Cross-correlation of a selected channel
-   Manual time offset

The selected alignment method must be saved with the comparison.

## 17.2 Comparison views

### 3D overlay

-   Simulated trajectory
-   Measured or reconstructed trajectory
-   Simulated attitude
-   Measured attitude
-   Difference vector

### Synchronized charts

Compare:

-   Altitude
-   Velocity
-   Acceleration
-   Angular velocity
-   Attitude
-   Pressure altitude
-   Dynamic pressure
-   Force and moment estimates

### Error metrics

Use clearly defined metrics:

-   Mean absolute error
-   Root mean square error
-   Maximum absolute error
-   Final error
-   Time-to-event difference
-   Angular distance between orientations

For quaternion comparison, use a rotation-distance measure rather than
subtracting Euler angles directly.

One suitable angular error is:

``` text
q_error = inverse(q_reference) ⊗ q_estimate

angle_error = 2 * acos(clamp(abs(q_error.w), -1, 1))
```

The exact order depends on the chosen quaternion convention and must be
tested.

## 17.3 Comparison warnings

Warn when:

-   Frames differ
-   Units differ
-   Timebases are uncertain
-   Data has large gaps
-   Position is estimated rather than measured
-   Simulation uses a different mass model
-   Simulation uses different initial conditions
-   Orientation source is unknown

Never imply that a mismatch automatically identifies a single root
cause.

------------------------------------------------------------------------

# 18. Settings

## 18.1 General

-   Default project directory
-   Autosave
-   Recent projects
-   Confirm destructive actions
-   Date and time format
-   Language
-   Units

## 18.2 Appearance

-   Dark
-   Light
-   System default
-   Compact or comfortable density
-   Reduced motion
-   Chart grid visibility

## 18.3 Dynamics

-   Default solver
-   Default step size
-   Default tolerances
-   Quaternion convention
-   World frame
-   Default gravity model
-   Numerical warning thresholds

## 18.4 Telemetry

-   Serial scan interval
-   Default baud rate
-   Packet timeout
-   Maximum packet size
-   Recording buffer size
-   Binary protocol settings
-   Auto-reconnect

## 18.5 Data and storage

-   Log directory
-   Cache directory
-   Retention policy
-   Export format
-   Compression
-   Backup settings

## 18.6 Diagnostics

-   Log level
-   Export diagnostic bundle
-   Enable developer tools
-   Performance metrics
-   Hardware information

------------------------------------------------------------------------

# 19. Rust Architecture

A modular workspace is recommended.

``` text
hexadof/
├── Cargo.toml
├── crates/
│   ├── hex-core/
│   │   ├── units/
│   │   ├── frames/
│   │   ├── vectors/
│   │   ├── quaternions/
│   │   ├── timestamps/
│   │   └── validation/
│   │
│   ├── hex-dynamics/
│   │   ├── state/
│   │   ├── equations/
│   │   ├── forces/
│   │   ├── moments/
│   │   ├── environment/
│   │   ├── integrators/
│   │   └── events/
│   │
│   ├── hex-model/
│   │   ├── schema/
│   │   ├── importer/
│   │   ├── validator/
│   │   └── versioning/
│   │
│   ├── hex-telemetry/
│   │   ├── transport/
│   │   ├── serial/
│   │   ├── protocol/
│   │   ├── mapping/
│   │   ├── calibration/
│   │   └── estimation/
│   │
│   ├── hex-flight-data/
│   │   ├── csv/
│   │   ├── binary/
│   │   ├── schema/
│   │   ├── validation/
│   │   └── derived/
│   │
│   ├── hex-analysis/
│   │   ├── alignment/
│   │   ├── comparison/
│   │   ├── metrics/
│   │   └── events/
│   │
│   └── hex-project/
│       ├── persistence/
│       ├── artifacts/
│       └── sessions/
│
└── apps/
    └── desktop/
        ├── src-tauri/
        │   ├── commands/
        │   ├── state/
        │   ├── events/
        │   └── main.rs
        └── frontend/
            ├── components/
            ├── layouts/
            ├── pages/
            ├── stores/
            ├── charts/
            └── three/
```

## 19.1 Core principle

The physics engine must not depend on the frontend.

The telemetry parser must not depend on the 3D renderer.

The frontend should consume typed commands, events, and data streams.

## 19.2 Tauri command groups

Suggested commands:

``` text
project_create
project_open
project_save
model_import
model_validate
simulation_validate
simulation_run
simulation_cancel
simulation_load_results
serial_list_ports
serial_connect
serial_disconnect
telemetry_start_recording
telemetry_stop_recording
flight_import
flight_validate
flight_create_session
comparison_create
report_export
```

Long-running tasks must run outside the UI thread and report progress
through events.

## 19.3 Event channels

Use typed events for:

-   Simulation progress
-   Simulation completed
-   Simulation warning
-   Simulation failed
-   Serial connected
-   Serial disconnected
-   Telemetry packet received
-   Telemetry health changed
-   Recording started
-   Recording stopped
-   Import progress
-   Validation result

Avoid sending every raw packet through an expensive UI rendering path.
Use a high-rate internal stream and a throttled display stream.

------------------------------------------------------------------------

# 20. Data and Threading Model

## 20.1 Separate real-time and non-real-time work

Telemetry ingestion should prioritize:

1.  Reading bytes
2.  Framing packets
3.  Validating integrity
4.  Timestamping
5.  Writing lossless records
6.  Publishing data to estimators
7.  Publishing throttled data to the UI

The UI must never be the only place where incoming packets exist.

## 20.2 Recommended pipeline

``` text
Serial reader
→ byte buffer
→ frame decoder
→ CRC/checksum validator
→ packet parser
→ channel mapper
→ calibration
→ estimator
→ recorder
→ UI stream
```

The recorder should be able to continue even if the UI becomes slow.

## 20.3 Backpressure

Define behavior for:

-   UI lag
-   Serial bursts
-   Disk write delay
-   Packet overload
-   Parser errors
-   Disconnected devices

Never silently drop recorded packets. If display data is downsampled,
distinguish it from the full-rate recording.

------------------------------------------------------------------------

# 21. Safety and Reliability

HexaDOF is an analysis and instrumentation application. It must not
imply that simulation results guarantee flight safety.

Include warnings for:

-   Unvalidated model
-   Simplified aerodynamic model
-   Unknown sensor orientation
-   Uncalibrated sensors
-   Missing timestamps
-   Estimated position
-   Solver instability
-   Extrapolated data
-   Out-of-range sensor values

For hardware connection:

-   Do not automatically send control commands to a device in the MVP.
-   Read-only telemetry should be the default.
-   Any future command-capable mode must require explicit user
    confirmation, device capability checks, and an independent safety
    design.

------------------------------------------------------------------------

# 22. Performance Requirements

Initial targets for a typical laptop:

-   UI remains responsive during simulation
-   3D rendering at approximately 60 FPS where hardware allows
-   Telemetry ingestion supports at least 200--1000 packets per second
    depending on packet size and device
-   Lossless recording to disk
-   Simulation runs independently of rendering
-   Large logs open incrementally rather than blocking the entire UI
-   Charts use downsampled display data while preserving original data
-   Memory usage remains bounded for long recordings through chunked
    storage

Performance benchmarks should be recorded rather than assumed.

------------------------------------------------------------------------

# 23. Error Handling

Errors must be actionable.

Bad:

> Error 0x00041

Good:

> **Unable to parse packet:** The payload length does not match the
> declared frame length at byte offset 1842. The stream may use the
> wrong baud rate or packet format.

Every error should include:

-   Human-readable title
-   Explanation
-   Severity
-   Context
-   Suggested fix
-   Technical details expandable by advanced users
-   Copy diagnostic button

------------------------------------------------------------------------

# 24. Accessibility and Usability

Support:

-   Keyboard navigation
-   Visible focus states
-   Tooltips for unfamiliar controls
-   Text labels alongside icons
-   High contrast in both themes
-   Color-independent warnings
-   Resizable panels
-   Reduced motion
-   Clear empty states
-   Undo for non-destructive mapping changes
-   Confirmation before deleting logs or projects

A beginner should be able to complete the basic workflow without
understanding quaternions or inertia tensors, while advanced users must
be able to inspect every technical value.

------------------------------------------------------------------------

# 25. MVP Delivery Plan

## Phase 1 --- Core math and model validation

-   Units
-   Frames
-   Vectors and matrices
-   Quaternion implementation
-   Rigid-body state
-   Force and moment interfaces
-   RK4 integrator
-   Basic gravity
-   Static model validation
-   Automated verification tests

Deliverable:

> A command-line or test harness that runs deterministic 6-DOF scenarios
> and produces verified state histories.

## Phase 2 --- Desktop simulation workspace

-   Tauri shell
-   Project management
-   Model import
-   Simulation setup UI
-   3D viewport
-   State inspector
-   Charts
-   Run persistence

Deliverable:

> A user can import a model, configure a scenario, run a simulation, and
> inspect results.

## Phase 3 --- USB telemetry

-   Serial transport
-   CSV line parser
-   Binary frame parser
-   Device profiles
-   Channel mapping
-   Calibration
-   Live channel display
-   Lossless recording

Deliverable:

> A user can connect an Arduino, verify incoming data, and record a
> session.

## Phase 4 --- Orientation and replay

-   Quaternion propagation
-   Basic estimator
-   Live digital rocket
-   Flight-log import
-   Timestamp validation
-   Replay controls
-   Event markers

Deliverable:

> A user can visualize live orientation and replay a recorded flight.

## Phase 5 --- Comparison and refinement

-   Simulation/flight alignment
-   Error metrics
-   Comparison plots
-   Reports
-   Performance profiling
-   Regression suite
-   Documentation
-   Export tools

Deliverable:

> A user can compare a real flight log against a simulation and
> investigate differences.

------------------------------------------------------------------------

# 26. Definition of Done

HexaDOF MVP is complete when a user can:

-   Create a project
-   Import a valid dynamics model
-   See clear validation results
-   Run a 6-DOF simulation
-   Inspect forces, moments, and state variables
-   Reopen a previous simulation
-   Connect an Arduino through USB
-   Map and calibrate sensor channels
-   View live telemetry
-   Record telemetry without relying on the UI frame rate
-   Import an SD-card flight log
-   Validate timestamps and data quality
-   Replay available flight state in 3D
-   Mark important events
-   Align a simulation and flight session
-   Compare selected channels and orientation
-   Export a report
-   Switch between dark and light themes
-   Understand warnings without reading source code

------------------------------------------------------------------------

# 27. Recommended First Technical Milestone

Before building the complete interface, implement and verify this narrow
vertical slice:

``` text
Imported dynamic model
→ 6-DOF state initialization
→ gravity + configurable force
→ RK4 integration
→ quaternion normalization
→ recorded state history
→ basic 3D replay
```

Then add:

``` text
Arduino serial stream
→ packet decoder
→ sensor mapping
→ gyro-based quaternion propagation
→ live 3D attitude
→ lossless recording
```

Finally:

``` text
Flight-log importer
→ timestamp validation
→ state reconstruction
→ replay
→ simulation comparison
```

This sequence keeps the project focused and ensures that the UI is built
around validated data structures rather than provisional assumptions.

------------------------------------------------------------------------

# 28. Final Product Positioning

HexaDOF should be presented as:

> **A focused aerospace dynamics and flight-data laboratory for
> simulating, observing, replaying, and understanding
> six-degree-of-freedom motion.**

Its value is not that it replaces every aerospace tool. Its value is
that it provides one coherent workflow between:

-   Existing APRO Works design tools
-   Numerical dynamics
-   Embedded sensors
-   USB telemetry
-   Flight logs
-   3D replay
-   Engineering comparison

The central product promise is:

> **Know how your vehicle moved, what the sensors measured, what the
> simulation predicted, and where the two diverged.**
