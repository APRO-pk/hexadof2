/**
 * The Dynamics workspace.
 *
 * A three-region layout: configuration on the left, the 3D viewport in the middle,
 * the state inspector on the right, and the plots and event timeline along the
 * bottom. The configuration and state panels collapse so the viewport can be
 * maximised during analysis.
 *
 * The run button's state is derived from validation rather than from a guess: a
 * scenario with a blocking error cannot be run, and the reason is shown beside it.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Badge,
  Button,
  Checkbox,
  Collapsible,
  DataTable,
  EmptyState,
  Field,
  Hint,
  KeyValue,
  Modal,
  Notice,
  NumberInput,
  Panel,
  ProgressBar,
  Select,
  Tabs,
  TextInput,
  Toggle,
  ValidationList,
} from "../components/ui";
import {
  IconExport,
  IconPanelLeft,
  IconPanelRight,
  IconPlay,
  IconRocket,
  IconSave,
  IconStop,
} from "../components/Icons";
import { Chart, allTimes, downloadText, seriesToCsv, type ChartSeries, type ChartView } from "../components/Chart";
import { ExampleFlightDialog } from "../components/ExampleFlights";
import { Transport, eventsFromRun } from "../components/Timeline";
import { Viewport, defaultViewportOptions, type CameraPreset, type ViewportOptions } from "../components/Viewport";
import { useAdvancedMode, useStore } from "../lib/store";
import { pickFile } from "../lib/api";
import {
  ANGLE_UNITS,
  LENGTH_UNITS,
  VELOCITY_UNITS,
  formatNumber,
  formatSeconds,
  unitByLabel,
} from "../lib/format";

type ConfigTab = "model" | "initial" | "environment" | "forces" | "moments" | "solver" | "outputs";

/** The result views, one per kind of answer a run gives. */
type ResultTab = "summary" | "plots" | "timeline" | "warnings" | "metadata";

const RESULT_TABS: { id: ResultTab; label: string }[] = [
  { id: "summary", label: "Summary" },
  { id: "plots", label: "Plots" },
  // The timeline is its own view rather than a strip above the plots, because
  // reading a flight against time is a different job from reading a trace.
  { id: "timeline", label: "Timeline" },
  { id: "warnings", label: "Warnings" },
  { id: "metadata", label: "Metadata" },
];

/** The configuration tabs that Simple mode hides, in their advanced order. */
const TABS: { id: ConfigTab; label: string }[] = [
  { id: "environment", label: "Environment" },
  { id: "forces", label: "Forces" },
  { id: "moments", label: "Moments" },
  { id: "solver", label: "Solver" },
  { id: "outputs", label: "Outputs" },
];

/**
 * The configuration tabs a mode offers.
 *
 * Simple mode keeps the shape of a flight: which model, where it starts, what
 * pushes it, and how long it runs. Solver settings, environment overrides,
 * control moments, and channel selection are what an advanced user tunes.
 */
function configTabs(advanced: boolean): { id: ConfigTab; label: string }[] {
  return [
    { id: "model" as ConfigTab, label: "Model" },
    { id: "initial" as ConfigTab, label: "Initial state" },
    ...TABS.filter((entry) => advanced || entry.id === "forces"),
  ];
}

export function DynamicsPage() {
  const scenario = useStore((s) => s.scenario);
  const setScenario = useStore((s) => s.setScenario);
  const resetScenario = useStore((s) => s.resetScenario);
  const validation = useStore((s) => s.validation);
  const validateScenario = useStore((s) => s.validateScenario);
  const runResult = useStore((s) => s.runResult);
  const running = useStore((s) => s.running);
  const progress = useStore((s) => s.progress);
  const replay = useStore((s) => s.replay);
  const runSimulation = useStore((s) => s.runSimulation);
  const cancelSimulation = useStore((s) => s.cancelSimulation);
  const activeModel = useStore((s) => s.activeModel);
  const models = useStore((s) => s.models);
  const importModelFromPath = useStore((s) => s.importModelFromPath);
  const importExampleModel = useStore((s) => s.importExampleModel);
  const loadExampleFlight = useStore((s) => s.loadExampleFlight);
  const savedRuns = useStore((s) => s.savedRuns);
  const openSavedRun = useStore((s) => s.openSavedRun);
  const addMarker = useStore((s) => s.addMarker);
  const plotChannels = useStore((s) => s.plotChannels);
  const setPlotChannels = useStore((s) => s.setPlotChannels);
  const settings = useStore((s) => s.settings);

  const [tab, setTab] = useState<ConfigTab>("initial");
  const [resultTab, setResultTab] = useState<ResultTab>("summary");
  const [showConfig, setShowConfig] = useState(true);
  const [showState, setShowState] = useState(true);
  const [index, setIndex] = useState(0);
  const [view, setView] = useState<ChartView>({ start: 0, end: 1 });
  const [cursor, setCursor] = useState<number | null>(null);
  const [hidden, setHidden] = useState<string[]>([]);
  const [camera, setCamera] = useState<CameraPreset>("follow");
  const [viewportOptions, setViewportOptions] = useState<ViewportOptions>(defaultViewportOptions);
  const [markerLabel, setMarkerLabel] = useState("");
  const [showMarkerDialog, setShowMarkerDialog] = useState(false);
  const [showExamples, setShowExamples] = useState(false);
  const [plotTab, setPlotTab] = useState(0);
  const [saveOnRun, setSaveOnRun] = useState(true);

  const grid = settings?.appearance.chart_grid ?? true;
  const advanced = useAdvancedMode();

  /**
   * The aerodynamic models this session can run.
   *
   * The imported model's coefficients are only offered when the model declares
   * them, so the list never promises a model that is not there. The engine
   * supports a drag table indexed against Mach plus lift, side force, and moment
   * derivatives, and that is what choosing the model means.
   */
  const aeroOptions = useMemo(() => {
    const options: { value: string; label: string }[] = [];
    if (activeModel?.has_aerodynamics) {
      options.push({ value: "auto", label: "From the imported model" });
      options.push({ value: "model", label: "Imported model coefficients, required" });
    } else {
      options.push({ value: "auto", label: "Built-in estimate, drag only" });
    }
    options.push({ value: "estimate", label: "Built-in estimate, drag only" });
    options.push({ value: "estimate_with_lift", label: "Built-in estimate with lift and moments" });
    options.push({ value: "none", label: "None" });
    return options;
  }, [activeModel]);

  const aeroHint = activeModel?.has_aerodynamics
    ? activeModel.has_lift_or_moments
      ? "The imported model declares a drag table and lift and moment derivatives. A coefficient model, not a measurement."
      : "The imported model declares a drag table but no lift or moment derivatives, so the run is still drag only."
    : "No imported coefficients are available, so the built-in estimate is used. Every run states which model it used.";

  // Reset the replay index and the chart window whenever a new run arrives.
  useEffect(() => {
    setIndex(0);
    setResultTab("summary");
    const series = buildSeries(runResult, plotChannels, hidden);
    const all = allTimes(series);
    setView(all ?? { start: 0, end: 1 });
  }, [runResult, plotChannels, hidden]);

  // A tab the mode hides cannot stay selected, so switching to Simple while the
  // solver tab is open falls back to a tab that exists in both modes.
  useEffect(() => {
    if (!configTabs(advanced).some((entry) => entry.id === tab)) {
      setTab("initial");
    }
  }, [advanced, tab]);

  // The same rule for the result views: the metadata view is the advanced one.
  useEffect(() => {
    if (!advanced && resultTab === "metadata") {
      setResultTab("summary");
    }
  }, [advanced, resultTab]);

  const series = useMemo(
    () => buildSeries(runResult, plotChannels, hidden),
    [runResult, plotChannels, hidden],
  );

  const canRun = validation?.can_run ?? false;

  const state = useMemo(() => {
    if (!runResult || runResult.times.length === 0) return null;
    const clamped = Math.min(runResult.times.length - 1, Math.max(0, index));
    const lookup: Record<string, number> = {};
    runResult.channel_names.forEach((name, i) => {
      lookup[name] = runResult.columns[i]?.[clamped] ?? Number.NaN;
    });
    return { time: runResult.times[clamped], lookup };
  }, [runResult, index]);

  const warningsBySeverity = useMemo(() => {
    if (!runResult) return [];
    return runResult.warnings;
  }, [runResult]);

  /**
   * Altitude as recorded, for the profile behind the timeline.
   *
   * It is one of the run's own channels rather than a redraw of the altitude
   * column, so the shape on the timeline is the shape that was saved.
   */
  const altitudeChannel = useMemo(() => {
    if (!runResult) return undefined;
    const column = runResult.channel_names.indexOf("altitude");
    return column >= 0 ? runResult.columns[column] : undefined;
  }, [runResult]);

  const importModel = async () => {
    const path = await pickFile({
      title: "Choose a dynamics model",
      filters: [{ name: "Dynamics model", extensions: ["json"] }],
    });
    if (path) await importModelFromPath(path);
  };

  const exportPlots = () => {
    const csv = seriesToCsv(series, view);
    if (csv) downloadText(csv, "hexadof-run-channels.csv");
  };

  return (
    <div
      className={`workspace ${showConfig ? "" : "no-config"} ${showState ? "" : "no-state"} ${
        runResult && resultTab === "plots" ? "plots-open" : ""
      }`}
    >
      {showConfig && (
        <section className="ws-config panel">
          <header className="panel-header">
            <h2 className="panel-title">Configuration</h2>
            <span className="spacer" />
            {/* One press opens the example list: a complete, documented flight
                with its model, which is the shortest path from an empty screen
                to something flying. */}
            <Button
              size="small"
              variant="primary"
              icon={<IconRocket />}
              onClick={() => setShowExamples(true)}
              title="Choose an example flight: a sounding rocket, a hop with a powered landing, a crosswind launch, and more"
            >
              Example flights
            </Button>
          </header>
          <div className="panel-body scroll" style={{ gap: "var(--space-3)" }}>
            <div className="field-row">
              <TextInput
                label="Run name"
                value={scenario.name ?? ""}
                onChange={(e) => setScenario({ name: e.target.value })}
              />
              <Select
                label="Simulation mode"
                value={scenario.mode ?? "six_dof"}
                onChange={(value) => setScenario({ mode: value })}
                options={[
                  { value: "six_dof", label: "Rigid body 6-DOF" },
                  { value: "three_dof", label: "Point mass 3-DOF" },
                ]}
              />
            </div>

            <Tabs<ConfigTab>
              active={tab}
              onChange={setTab}
              tabs={configTabs(advanced)}
            />

            {tab === "model" && (
              <>
                {activeModel ? (
                  <>
                    <KeyValue
                      pairs={[
                        ["Name", activeModel.name],
                        ["Model id", activeModel.model_id],
                        ["Version", activeModel.model_version],
                        ["Units", activeModel.unit_system],
                        ["World frame", activeModel.world_frame],
                        ["Body frame", activeModel.body_frame],
                        ["Mass", `${formatNumber(activeModel.mass, 4)} kg`],
                        [
                          "Centre of gravity",
                          activeModel.center_of_gravity.map((v) => formatNumber(v, 4)).join(", "),
                        ],
                        [
                          "Inertia (diagonal)",
                          [activeModel.inertia[0], activeModel.inertia[1], activeModel.inertia[2]]
                            .map((v) => formatNumber(v, 5))
                            .join(", "),
                        ],
                        ["Reference area", `${formatNumber(activeModel.reference_area, 5)} m²`],
                        ["Reference length", `${formatNumber(activeModel.reference_length, 4)} m`],
                        ["Mesh", activeModel.mesh_reference ?? "none"],
                        ["Mass curve", activeModel.has_mass_curve ? "present" : "none"],
                        ["Thrust profile", activeModel.has_thrust ? "present" : "none"],
                        [
                          "Aerodynamics",
                          activeModel.has_aerodynamics
                            ? (activeModel.aerodynamics_summary ?? "present")
                            : "none declared",
                        ],
                      ]}
                    />
                    <Badge tone={activeModel.valid ? "success" : "error"}>
                      {activeModel.valid
                        ? "The model is valid"
                        : `${activeModel.error_count} blocking error(s)`}
                    </Badge>
                    <Collapsible title="Validation report" defaultOpen={!activeModel.valid}>
                      <ValidationList report={activeModel.validation} />
                    </Collapsible>
                  </>
                ) : (
                  <EmptyState
                    icon={<IconRocket />}
                    title="No model imported"
                    detail="A simulation needs mass, centre of gravity, and an inertia tensor."
                    action={
                      <div className="row">
                        <Button onClick={() => void importModel()}>Import a model</Button>
                        <Button variant="ghost" onClick={() => void importExampleModel()}>
                          Use the example
                        </Button>
                      </div>
                    }
                  />
                )}
                <div className="row">
                  <Button size="small" onClick={() => void importModel()}>
                    Import a different model
                  </Button>
                  {models.length > 1 && (
                    <Badge tone="neutral">{models.length} imported</Badge>
                  )}
                </div>
              </>
            )}

            {tab === "initial" && (
              <>
                <Vector3Field
                  label="Position"
                  unit={LENGTH_UNITS}
                  value={scenario.position ?? [0, 0, 0]}
                  onChange={(v) => setScenario({ position: v })}
                  hint="World frame, ENU. Altitude is the third component."
                />
                <Vector3Field
                  label="Velocity"
                  unit={VELOCITY_UNITS}
                  value={scenario.velocity ?? [0, 0, 0]}
                  onChange={(v) => setScenario({ velocity: v })}
                />
                <Vector3Field
                  label="Attitude (roll, pitch, yaw)"
                  unit={ANGLE_UNITS}
                  value={scenario.euler ?? [0, 0, 0]}
                  onChange={(v) => setScenario({ euler: v })}
                  hint="-90° pitch points the thrust axis up. The state itself is always a quaternion."
                />
                <Vector3Field
                  label="Angular velocity"
                  unit={[{ label: "rad/s", factor: 1 }, { label: "deg/s", factor: Math.PI / 180 }]}
                  value={scenario.angular_velocity ?? [0, 0, 0]}
                  onChange={(v) => setScenario({ angular_velocity: v })}
                  hint="Body frame. Ignored in 3-DOF mode."
                />
                <div className="field-row">
                  <NumberInput
                    label="Initial mass"
                    unit="kg"
                    value={scenario.mass}
                    onChange={(v) => setScenario({ mass: v })}
                  />
                  <NumberInput
                    label="Start time"
                    unit="s"
                    value={scenario.start_time}
                    onChange={(v) => setScenario({ start_time: v })}
                  />
                </div>
                <span className="meta">Inertia about the centre of gravity, diagonal terms</span>
                <div className="field-row">
                  {(["Ixx", "Iyy", "Izz"] as const).map((label, i) => (
                    <NumberInput
                      key={label}
                      label={label}
                      unit="kg·m²"
                      value={scenario.inertia?.[i] ?? 0}
                      onChange={(v) => {
                        const next: [number, number, number, number, number, number] = scenario.inertia
                          ? [...scenario.inertia]
                          : [0.1, 0.1, 0.01, 0, 0, 0];
                        next[i] = v ?? 0;
                        setScenario({ inertia: next });
                      }}
                    />
                  ))}
                </div>
                <Button size="small" variant="ghost" onClick={resetScenario}>
                  Reset the scenario to defaults
                </Button>
              </>
            )}

            {tab === "environment" && (
              <>
                <Toggle
                  label="Apply gravity"
                  checked={scenario.gravity_enabled ?? true}
                  onChange={(v) => setScenario({ gravity_enabled: v })}
                  hint="Turning gravity off is a verification aid, not a flight case."
                />
                <NumberInput
                  label="Gravity magnitude"
                  unit="m/s²"
                  value={scenario.gravity}
                  disabled={!(scenario.gravity_enabled ?? true)}
                  onChange={(v) => setScenario({ gravity: v })}
                />
                <Toggle
                  label="Use the standard atmosphere"
                  checked={scenario.standard_atmosphere ?? true}
                  onChange={(v) => setScenario({ standard_atmosphere: v })}
                  hint="US Standard Atmosphere 1976, a standard day. It is not a weather model."
                />
                <NumberInput
                  label="Air density"
                  unit="kg/m³"
                  value={scenario.air_density}
                  disabled={scenario.standard_atmosphere ?? true}
                  onChange={(v) => setScenario({ air_density: v })}
                />
                <NumberInput
                  label="Wind speed"
                  unit="m/s"
                  value={scenario.wind_speed}
                  onChange={(v) => setScenario({ wind_speed: v })}
                />
                <Vector3Field
                  label="Wind direction"
                  unit={[{ label: "-", factor: 1 }]}
                  value={scenario.wind_direction ?? [1, 0, 0]}
                  onChange={(v) => setScenario({ wind_direction: v })}
                />
                <div className="field-row">
                  <NumberInput
                    label="Launch rail length"
                    unit="m"
                    value={scenario.rail_length}
                    onChange={(v) => setScenario({ rail_length: v })}
                    hint="Zero disables the constraint."
                  />
                  <NumberInput
                    label="Ground elevation"
                    unit="m"
                    value={scenario.ground_elevation}
                    onChange={(v) => setScenario({ ground_elevation: v })}
                  />
                </div>
              </>
            )}

            {tab === "forces" && (
              <>
                <NumberInput
                  label="Thrust magnitude"
                  unit="N"
                  value={scenario.thrust}
                  onChange={(v) => setScenario({ thrust: v })}
                />
                <NumberInput
                  label="Burn duration"
                  unit="s"
                  value={scenario.burn_time}
                  onChange={(v) => setScenario({ burn_time: v })}
                />
                <Vector3Field
                  label="Thrust application point"
                  unit={LENGTH_UNITS}
                  value={scenario.thrust_application_point ?? [0, 0, 0]}
                  onChange={(v) => setScenario({ thrust_application_point: v })}
                  hint="Measured from the centre of gravity. An offset produces M = r × F."
                />
                <div className="divider" />
                {advanced ? (
                  <Select
                    label="Aerodynamic model"
                    value={scenario.aero_source ?? "auto"}
                    onChange={(value) => setScenario({ aero_source: value })}
                    options={aeroOptions}
                    hint={aeroHint}
                  />
                ) : (
                  <Hint>
                    {activeModel?.has_aerodynamics
                      ? "Aerodynamics come from the imported model. Open Advanced to choose a different model."
                      : "Aerodynamics use the built-in drag estimate. Open Advanced to choose a different model."}
                  </Hint>
                )}
                {advanced &&
                ((scenario.aero_source ?? "auto") === "estimate" ||
                  (scenario.aero_source ?? "auto") === "estimate_with_lift" ||
                  (!activeModel?.has_aerodynamics &&
                    (scenario.aero_source ?? "auto") === "auto")) ? (
                  <NumberInput
                    label="Drag coefficient"
                    value={scenario.drag_coefficient}
                    onChange={(v) => setScenario({ drag_coefficient: v })}
                    hint="One constant for the whole flight. Labelled as a simplification in every run."
                  />
                ) : null}
                {advanced && (scenario.aero_source ?? "auto") === "estimate_with_lift" && (
                  <>
                    <NumberInput
                      label="Lift slope"
                      unit="per rad"
                      value={scenario.lift_slope}
                      onChange={(v) => setScenario({ lift_slope: v })}
                    />
                    <NumberInput
                      label="Pitch damping"
                      value={scenario.pitch_damping}
                      onChange={(v) => setScenario({ pitch_damping: v })}
                    />
                  </>
                )}
                {advanced && activeModel?.has_aerodynamics && (
                  <span className="meta">
                    Imported model aerodynamics: {activeModel.aerodynamics_summary}
                  </span>
                )}

                {/* The landing burn is part of the scenario, so it is shown here
                    rather than only implied by an example. A scenario that hides
                    a force the run applies would be a scenario nobody can read. */}
                {advanced && (
                  <>
                    <div className="divider" />
                    {scenario.landing_burn ? (
                      <>
                        <div className="row">
                          <span className="meta">Landing burn</span>
                          <span className="spacer" />
                          <Button
                            size="small"
                            variant="ghost"
                            onClick={() => setScenario({ landing_burn: null })}
                            title="Remove the landing burn from this scenario"
                          >
                            Remove
                          </Button>
                        </div>
                        <NumberInput
                          label="Landing thrust"
                          unit="N"
                          value={scenario.landing_burn.thrust}
                          onChange={(v) =>
                            setScenario({
                              landing_burn: {
                                ...scenario.landing_burn!,
                                thrust: v ?? 0,
                              },
                            })
                          }
                          hint="It has to beat the vehicle's weight, or there is no stopping distance and the burn never lights."
                        />
                        <NumberInput
                          label="Deceleration margin"
                          unit="m/s²"
                          value={scenario.landing_burn.deceleration_margin ?? 1}
                          onChange={(v) =>
                            setScenario({
                              landing_burn: {
                                ...scenario.landing_burn!,
                                deceleration_margin: v ?? 0,
                              },
                            })
                          }
                          hint="Held back from the stopping distance. A larger margin lights the engine earlier and higher."
                        />
                      </>
                    ) : (
                      <Button
                        size="small"
                        variant="ghost"
                        onClick={() =>
                          setScenario({
                            landing_burn: {
                              thrust: (scenario.thrust ?? 0) * 0.6,
                              deceleration_margin: 1,
                              maximum_ignition_altitude: 500,
                              minimum_descent_rate: 0.2,
                            },
                          })
                        }
                        title="Add a landing burn that ignites on the stopping-distance condition"
                      >
                        Add a landing burn
                      </Button>
                    )}
                  </>
                )}
              </>
            )}

            {tab === "moments" && (
              <>
                <Vector3Field
                  label="Constant control moment"
                  unit={[{ label: "N·m", factor: 1 }]}
                  value={scenario.control_moment ?? [0, 0, 0]}
                  onChange={(v) =>
                    setScenario({ control_moment: v.every((c) => c === 0) ? null : v })
                  }
                  hint="Body frame. Set every component to zero to disable it."
                />
                <NumberInput
                  label="Control moment duration"
                  unit="s"
                  value={scenario.control_duration}
                  onChange={(v) => setScenario({ control_duration: v })}
                />
                <Hint>
                  Thrust misalignment and an engine moment are configured on the imported model, not
                  here. A moment about an arbitrary reference point is never applied without being
                  transformed to the centre of gravity.
                </Hint>
              </>
            )}

            {tab === "solver" && (
              <>
                <Select
                  label="Integrator"
                  value={scenario.relative_tolerance === null || scenario.relative_tolerance === undefined ? "rk4" : "rk45"}
                  onChange={(value) =>
                    setScenario(
                      value === "rk45"
                        ? { relative_tolerance: 1e-9, absolute_tolerance: 1e-12 }
                        : { relative_tolerance: null, absolute_tolerance: null },
                    )
                  }
                  options={[
                    { value: "rk4", label: "Fixed-step RK4" },
                    { value: "rk45", label: "Adaptive Dormand-Prince 5(4)" },
                  ]}
                  hint="RK4 is the transparent reference solver. Its step is exactly what you type."
                />
                <NumberInput
                  label="Integration step"
                  unit="s"
                  value={scenario.step}
                  disabled={(scenario.relative_tolerance ?? null) !== null}
                  onChange={(v) => setScenario({ step: v })}
                />
                <div className="field-row">
                  <NumberInput
                    label="Relative tolerance"
                    value={scenario.relative_tolerance}
                    disabled={(scenario.relative_tolerance ?? null) === null}
                    onChange={(v) => setScenario({ relative_tolerance: v })}
                  />
                  <NumberInput
                    label="Absolute tolerance"
                    value={scenario.absolute_tolerance}
                    disabled={(scenario.relative_tolerance ?? null) === null}
                    onChange={(v) => setScenario({ absolute_tolerance: v })}
                  />
                </div>
                <NumberInput
                  label="End time"
                  unit="s"
                  value={scenario.end_time}
                  onChange={(v) => setScenario({ end_time: v })}
                />
                <Hint>
                  Three different rates are kept apart: the integration step above, the output sampling
                  interval on the Outputs tab, and the rendering rate in the viewport. Changing the
                  display never changes the numerics.
                </Hint>
              </>
            )}

            {tab === "outputs" && (
              <>
                <NumberInput
                  label="Output sampling interval"
                  unit="s"
                  value={scenario.output_interval}
                  onChange={(v) => setScenario({ output_interval: v })}
                />
                <span className="meta">
                  About{" "}
                  {scenario.output_interval && scenario.end_time
                    ? Math.max(
                        1,
                        Math.floor(
                          ((scenario.end_time ?? 0) - (scenario.start_time ?? 0)) /
                            scenario.output_interval,
                        ) + 1,
                      )
                    : 0}{" "}
                  samples expected.
                </span>
                <div className="divider" />
                <span className="meta">Plotted channels</span>
                <div className="pill-row">
                  {(runResult?.available_channels ?? defaultChannelNames()).map((channel) => {
                    const enabled = plotChannels.includes(channel);
                    return (
                      <Checkbox
                        key={channel}
                        label={channel.replace(/_/g, " ")}
                        checked={enabled}
                        onChange={(v) =>
                          setPlotChannels(
                            v
                              ? [...plotChannels, channel]
                              : plotChannels.filter((c) => c !== channel),
                          )
                        }
                      />
                    );
                  })}
                </div>
                <Hint>
                  The state history always records every channel; this selection only chooses what is
                  plotted and sent to the interface.
                </Hint>
              </>
            )}
          </div>

          <footer className="panel-footer">
            <Button size="small" onClick={() => void validateScenario()}>
              Validate
            </Button>
            <Button
              size="small"
              variant="primary"
              icon={<IconPlay />}
              disabled={running || (validation !== null && !canRun)}
              onClick={() => void runSimulation(saveOnRun)}
              title={
                validation === null
                  ? "Validate first, or run and see the errors"
                  : canRun
                    ? "Start the simulation"
                    : "The scenario has blocking errors"
              }
            >
              {running ? "Running..." : "Run"}
            </Button>
            <Button
              size="small"
              icon={<IconStop />}
              disabled={!running}
              onClick={() => void cancelSimulation()}
            >
              Stop
            </Button>
            <span className="spacer" />
            <Checkbox label="Save when done" checked={saveOnRun} onChange={setSaveOnRun} />
          </footer>
        </section>
      )}

      <section className="ws-viewport" style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)", minHeight: 0 }}>
        <div className="row">
          <h1 className="page-title" style={{ fontSize: "var(--text-section)" }}>
            {scenario.name || "Scenario"}
          </h1>
          {validation && (
            <Badge tone={validation.can_run ? "success" : "error"}>
              {validation.can_run ? "Configuration valid" : `${validation.errors.length} error(s)`}
            </Badge>
          )}
          {running && progress && (
            <>
              <Badge tone="accent">{Math.round(progress.fraction * 100)}%</Badge>
              <span className="meta mono">
                t = {progress.time.toFixed(3)} s, {progress.samples} samples
              </span>
            </>
          )}
          <span className="spacer" />
          <Button
            size="small"
            variant="ghost"
            icon={<IconPanelLeft />}
            onClick={() => setShowConfig((v) => !v)}
            title={showConfig ? "Hide the configuration panel" : "Show the configuration panel"}
          >
            {showConfig ? "Hide config" : "Show config"}
          </Button>
          <Button
            size="small"
            variant="ghost"
            icon={<IconPanelRight />}
            onClick={() => setShowState((v) => !v)}
            title={showState ? "Hide the state panel" : "Show the state panel"}
          >
            {showState ? "Hide state" : "Show state"}
          </Button>
        </div>

        {running && (
          <Panel title="Run in progress" flush>
            <div className="panel-body">
              <ProgressBar fraction={progress?.fraction ?? 0} />
              <div className="row">
                <span className="meta">
                  {progress
                    ? `${progress.accepted_steps} accepted, ${progress.rejected_steps} rejected, ${progress.events} event(s)`
                    : "Starting..."}
                </span>
              </div>
            </div>
          </Panel>
        )}

        <div className="viewport-column">
          <Viewport
            data={replay}
            index={index}
            options={viewportOptions}
            camera={camera}
            onCameraChange={setCamera}
            attitudeLabel="SIMULATED STATE (AUTHORITATIVE)"
            attitudeNote="The state history is the source of truth. The view interpolates between recorded samples."
            onAddMarker={() => setShowMarkerDialog(true)}
          />
          <div className="row wrap">
            <Checkbox
              label="World axes"
              checked={viewportOptions.showWorldAxes}
              onChange={(v) => setViewportOptions({ ...viewportOptions, showWorldAxes: v })}
            />
            <Checkbox
              label="Body axes"
              checked={viewportOptions.showBodyAxes}
              onChange={(v) => setViewportOptions({ ...viewportOptions, showBodyAxes: v })}
            />
            <Checkbox
              label="Trajectory"
              checked={viewportOptions.showTrajectory}
              onChange={(v) => setViewportOptions({ ...viewportOptions, showTrajectory: v })}
            />
            <Checkbox
              label="Velocity"
              checked={viewportOptions.showVelocity}
              onChange={(v) => setViewportOptions({ ...viewportOptions, showVelocity: v })}
            />
            <Checkbox
              label="Angular rate"
              checked={viewportOptions.showAngularRate}
              onChange={(v) => setViewportOptions({ ...viewportOptions, showAngularRate: v })}
            />
            <Checkbox
              label="Ground"
              checked={viewportOptions.showGround}
              onChange={(v) => setViewportOptions({ ...viewportOptions, showGround: v })}
            />
          </div>
        </div>
      </section>

      {showState && (
        <section className="ws-state panel">
          <header className="panel-header">
            <h2 className="panel-title">State</h2>
            <span className="spacer" />
            {state && <span className="meta mono">t = {state.time.toFixed(3)} s</span>}
          </header>
          <div className="panel-body scroll">
            {state ? (
              <>
                <Collapsible title="Position and velocity" defaultOpen>
                  <KeyValue
                    pairs={[
                      ["Position X", `${formatNumber(state.lookup.position_x, 4)} m`],
                      ["Position Y", `${formatNumber(state.lookup.position_y, 4)} m`],
                      ["Position Z", `${formatNumber(state.lookup.position_z, 4)} m`],
                      ["Altitude", `${formatNumber(state.lookup.altitude, 4)} m`],
                      ["Velocity X", `${formatNumber(state.lookup.velocity_x, 4)} m/s`],
                      ["Velocity Y", `${formatNumber(state.lookup.velocity_y, 4)} m/s`],
                      ["Velocity Z", `${formatNumber(state.lookup.velocity_z, 4)} m/s`],
                      ["Speed", `${formatNumber(state.lookup.speed, 4)} m/s`],
                      ["Acceleration", `${formatNumber(state.lookup.acceleration, 4)} m/s²`],
                    ]}
                  />
                </Collapsible>
                <Collapsible title="Attitude and rates" defaultOpen>
                  <KeyValue
                    pairs={[
                      ["Roll", `${formatNumber(state.lookup.roll === undefined ? NaN : (state.lookup.roll * 180) / Math.PI, 3)} deg`],
                      ["Pitch", `${formatNumber(state.lookup.pitch === undefined ? NaN : (state.lookup.pitch * 180) / Math.PI, 3)} deg`],
                      ["Yaw", `${formatNumber(state.lookup.yaw === undefined ? NaN : (state.lookup.yaw * 180) / Math.PI, 3)} deg`],
                      ["Rate X", `${formatNumber(state.lookup.angular_rate_x, 5)} rad/s`],
                      ["Rate Y", `${formatNumber(state.lookup.angular_rate_y, 5)} rad/s`],
                      ["Rate Z", `${formatNumber(state.lookup.angular_rate_z, 5)} rad/s`],
                      ["Quaternion W", formatNumber(state.lookup.quaternion_w, 6)],
                      ["Quaternion X", formatNumber(state.lookup.quaternion_x, 6)],
                      ["Quaternion Y", formatNumber(state.lookup.quaternion_y, 6)],
                      ["Quaternion Z", formatNumber(state.lookup.quaternion_z, 6)],
                    ]}
                  />
                  <Hint>
                    Roll, pitch, and yaw are derived for display only. The authoritative attitude is the
                    quaternion above.
                  </Hint>
                </Collapsible>
                <Collapsible title="Forces and moments">
                  <KeyValue
                    pairs={[
                      ["Total force", `${formatNumber(state.lookup.force, 4)} N`],
                      ["Force X", `${formatNumber(state.lookup.force_x, 4)} N`],
                      ["Force Y", `${formatNumber(state.lookup.force_y, 4)} N`],
                      ["Force Z", `${formatNumber(state.lookup.force_z, 4)} N`],
                      ["Total moment", `${formatNumber(state.lookup.moment, 4)} N·m`],
                      ["Moment X", `${formatNumber(state.lookup.moment_x, 4)} N·m`],
                      ["Moment Y", `${formatNumber(state.lookup.moment_y, 4)} N·m`],
                      ["Moment Z", `${formatNumber(state.lookup.moment_z, 4)} N·m`],
                    ]}
                  />
                </Collapsible>
                <Collapsible title="Air and mass">
                  <KeyValue
                    pairs={[
                      ["Dynamic pressure", `${formatNumber(state.lookup.dynamic_pressure, 3)} Pa`],
                      ["Angle of attack", `${formatNumber(((state.lookup.angle_of_attack ?? 0) * 180) / Math.PI, 3)} deg`],
                      ["Sideslip", `${formatNumber(((state.lookup.sideslip ?? 0) * 180) / Math.PI, 3)} deg`],
                      ["Air density", `${formatNumber(state.lookup.air_density, 5)} kg/m³`],
                      ["Mach", formatNumber(state.lookup.mach, 4)],
                      ["Mass", `${formatNumber(state.lookup.mass, 4)} kg`],
                    ]}
                  />
                </Collapsible>
                <div className="row">
                  <Button
                    size="small"
                    variant="ghost"
                    onClick={() => {
                      const text = Object.entries(state.lookup)
                        .map(([k, v]) => `${k}: ${v}`)
                        .join("\n");
                      void navigator.clipboard.writeText(`t = ${state.time} s\n${text}`);
                    }}
                  >
                    Copy the state
                  </Button>
                </div>
              </>
            ) : (
              <EmptyState
                title="No state yet"
                detail="Run a simulation, or open a saved run, to inspect the state at each sample."
              />
            )}
          </div>
        </section>
      )}

      <section className="ws-bottom panel">
        <header className="panel-header">
          <h2 className="panel-title">Results</h2>
          <span className="spacer" />
          {runResult && (
            <>
              <span className="meta">
                {runResult.summary.solver} | dt {formatNumber(runResult.summary.integration_step ?? null, 6)} s |
                output {runResult.summary.output_rate_hz.toFixed(1)} Hz
              </span>
              <Button size="small" variant="ghost" icon={<IconExport />} onClick={exportPlots}>
                Export plots as CSV
              </Button>
            </>
          )}
        </header>

        {runResult && (
          <div className="result-tabs">
            <Tabs<ResultTab>
              active={resultTab}
              onChange={setResultTab}
              tabs={RESULT_TABS.filter((entry) => advanced || entry.id !== "metadata").map(
                (entry) =>
                  entry.id === "warnings" && warningsBySeverity.length > 0
                    ? { ...entry, label: `Warnings (${warningsBySeverity.length})` }
                    : entry,
              )}
            />
          </div>
        )}

        <div className="panel-body scroll" style={{ gap: "var(--space-3)" }}>
          {!runResult ? (
            <EmptyState
              icon={<IconRocket />}
              title="No results yet"
              detail="Configure a scenario, validate it, and run it. The state history, plots, and event timeline appear here."
              action={
                savedRuns.length > 0 ? (
                  <div className="row wrap">
                    {savedRuns.slice(0, 3).map((run) => (
                      <Button key={run.name} size="small" onClick={() => void openSavedRun(run.name)}>
                        Open {run.name}
                      </Button>
                    ))}
                  </div>
                ) : undefined
              }
            />
          ) : resultTab === "summary" ? (
            <>
              {/* A warning shown only in its own tab is a warning nobody reads, so
                  the first one is summarised here with the way to the rest. */}
              {warningsBySeverity.length > 0 && (
                <Notice
                  level={warningsBySeverity.some((w) => w.blocking) ? "error" : "warning"}
                  title={`${warningsBySeverity.length} finding(s) on this run`}
                  detail={`${warningsBySeverity[0].title}. ${
                    warningsBySeverity.length > 1
                      ? `The Warnings tab lists the other ${warningsBySeverity.length - 1}.`
                      : "The Warnings tab has the detail."
                  }`}
                />
              )}

              <div className="field-row">
                <ReadoutCard
                  label="Status"
                  value={runResult.summary.status}
                  detail={`${runResult.summary.sample_count} samples, ${formatSeconds(runResult.summary.duration)}`}
                />
                <ReadoutCard
                  label="Maximum altitude"
                  value={`${formatNumber(runResult.summary.maximum_altitude, 3)} m`}
                  detail={`max speed ${formatNumber(runResult.summary.maximum_speed, 3)} m/s`}
                />
                <ReadoutCard
                  label="Maximum dynamic pressure"
                  value={`${formatNumber(runResult.summary.maximum_dynamic_pressure, 1)} Pa`}
                  detail={`max Mach ${formatNumber(runResult.summary.maximum_mach, 3)}`}
                />
                <ReadoutCard
                  label="Quaternion norm error"
                  value={formatNumber(runResult.summary.maximum_quaternion_norm_error, 3)}
                  detail={`${runResult.summary.solver_statistics.rejected_steps} rejected step(s)`}
                />
                <ReadoutCard
                  label="Energy drift"
                  value={
                    runResult.summary.energy_drift_ratio === null ||
                    runResult.summary.energy_drift_ratio === undefined
                      ? "not applicable"
                      : `${(runResult.summary.energy_drift_ratio * 100).toFixed(4)} %`
                  }
                  detail="Measured only on a run with no thrust"
                />
              </div>

              <div className="grid-12">
                <div className="col-6">
                  <KeyValue
                    pairs={[
                      ["Mass model", runResult.summary.mass_model],
                      ["Environment", runResult.summary.environment.join(", ")],
                      ["Force models", runResult.summary.force_models.join(", ")],
                    ]}
                  />
                </div>
                <div className="col-6">
                  <KeyValue
                    pairs={[
                      ["Events", `${runResult.events.length}`],
                      [
                        "Peak angular rate",
                        `${formatNumber(runResult.summary.maximum_angular_rate, 4)} rad/s`,
                      ],
                      [
                        "Peak acceleration",
                        `${formatNumber(runResult.summary.maximum_acceleration, 3)} m/s²`,
                      ],
                    ]}
                  />
                </div>
              </div>
            </>
          ) : resultTab === "warnings" ? (
            warningsBySeverity.length === 0 ? (
              <EmptyState
                title="No findings on this run"
                detail="The solver, the thresholds, and the event rules all passed. Warnings appear here when they do not."
              />
            ) : (
              <div className="col" style={{ gap: "var(--space-2)" }}>
                {warningsBySeverity.map((warning) => (
                  <Notice
                    key={warning.code + warning.title}
                    level={warning.blocking ? "error" : "warning"}
                    title={warning.title}
                    detail={warning.detail}
                  />
                ))}
              </div>
            )
          ) : resultTab === "timeline" ? (
            <>
              <Transport
                times={runResult.times}
                index={index}
                onIndexChange={setIndex}
                events={eventsFromRun(runResult.events)}
                profile={
                  altitudeChannel
                    ? { values: altitudeChannel, label: "Altitude", unit: "m" }
                    : undefined
                }
                window={view}
                onWindowChange={setView}
                extra={
                  <>
                    <Badge tone="neutral">{runResult.events.length} event(s)</Badge>
                    <Button
                      size="small"
                      variant="ghost"
                      icon={<IconSave />}
                      onClick={() => setShowMarkerDialog(true)}
                    >
                      Add marker
                    </Button>
                  </>
                }
              />

              <EventList
                events={runResult.events}
                onJump={(time) => {
                  const nearest = nearestIndex(runResult.times, time);
                  setIndex(nearest);
                }}
              />
            </>
          ) : resultTab === "plots" ? (
            <PlotsView
              groups={chartGroups(series)}
              tab={plotTab}
              onTabChange={setPlotTab}
              series={series}
              view={view}
              onViewChange={setView}
              cursor={cursor}
              onCursorChange={setCursor}
              grid={grid}
              onToggleSeries={(name) =>
                setHidden((current) =>
                  current.includes(name) ? current.filter((n) => n !== name) : [...current, name],
                )
              }
            />
          ) : (
            <>
              {/* The run metadata is what an advanced user checks when a result
                  has to be reproduced. */}
              <KeyValue
                pairs={[
                  ["Run name", runResult.summary.name],
                  ["Application version", runResult.summary.application_version],
                  ["Created", runResult.summary.created_at],
                  ["Model", `${runResult.summary.model_id} v${runResult.summary.model_version}`],
                  ["Mode", runResult.summary.mode === "six_dof" ? "Rigid body 6-DOF" : "Point mass 3-DOF"],
                  ["Solver", runResult.summary.solver],
                  ["Output interval", `${runResult.summary.output_interval} s`],
                  ["Normalisation", runResult.summary.normalization],
                  ["Mass model", runResult.summary.mass_model],
                  ["Wall clock", `${formatNumber(runResult.summary.wall_clock_seconds ?? null, 3)} s`],
                  [
                    "Real-time factor",
                    `${formatNumber(runResult.summary.real_time_factor ?? null, 2)}x`,
                  ],
                  ["Active force models", runResult.summary.active_providers.join(", ")],
                  ["Initial energy", `${formatNumber(runResult.summary.initial_energy, 4)} J`],
                  ["Final energy", `${formatNumber(runResult.summary.final_energy, 4)} J`],
                ]}
              />
              <Collapsible title="Environment">
                <ul style={{ margin: 0, paddingLeft: "1.2em" }}>
                  {runResult.summary.environment.map((line) => (
                    <li key={line} className="meta">
                      {line}
                    </li>
                  ))}
                </ul>
              </Collapsible>
              <Collapsible title="Force and moment models">
                <ul style={{ margin: 0, paddingLeft: "1.2em" }}>
                  {runResult.summary.force_models.map((line) => (
                    <li key={line} className="meta">
                      {line}
                    </li>
                  ))}
                </ul>
              </Collapsible>
            </>
          )}
        </div>
      </section>


      {showExamples && (
        <ExampleFlightDialog
          open={showExamples}
          onClose={() => setShowExamples(false)}
          onLoad={async (id) => {
            const loaded = await loadExampleFlight(id);
            if (loaded) setShowExamples(false);
          }}
        />
      )}

      {showMarkerDialog && (
        <Modal
          title="Add an event marker"
          onClose={() => setShowMarkerDialog(false)}
          footer={
            <>
              <Button variant="primary" onClick={() => {
                if (state) void addMarker(state.time, markerLabel.trim() || "Marker");
                setMarkerLabel("");
                setShowMarkerDialog(false);
              }}>
                Add marker
              </Button>
              <Button variant="ghost" onClick={() => setShowMarkerDialog(false)}>
                Cancel
              </Button>
            </>
          }
        >
          <Field label="Marker label" hint={`Placed at t = ${state ? state.time.toFixed(3) : "0"} s.`}>
            <TextInput
              value={markerLabel}
              placeholder="Camera start"
              onChange={(e) => setMarkerLabel(e.target.value)}
            />
          </Field>
        </Modal>
      )}
    </div>
  );
}

/** One readout tile above the plots. */
function ReadoutCard({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <div className="status-card">
      <span className="status-card-label">{label}</span>
      <span className="status-card-value mono">{value}</span>
      <span className="meta">{detail}</span>
    </div>
  );
}

/** A three-component field, converted from the display unit to SI. */
function Vector3Field({
  label,
  unit,
  value,
  onChange,
  hint,
}: {
  label: string;
  unit: { label: string; factor: number }[];
  value: [number, number, number];
  onChange: (value: [number, number, number]) => void;
  hint?: string;
}) {
  const [unitLabel, setUnitLabel] = useState(unit[0].label);
  const chosen = unitByLabel(unit, unitLabel);
  const inverse = 1 / chosen.factor;

  return (
    <div className="field">
      <div className="row-between">
        <span className="field-label">{label}</span>
        {unit.length > 1 && (
          <div style={{ width: 96 }}>
            <Select
              value={unitLabel}
              onChange={setUnitLabel}
              options={unit.map((u) => ({ value: u.label, label: u.label }))}
            />
          </div>
        )}
      </div>
      <div className="field-row">
        {(["X", "Y", "Z"] as const).map((axis, i) => (
          <NumberInput
            key={axis}
            label={axis}
            unit={chosen.label}
            value={Number.isFinite(value[i]) ? value[i] * chosen.factor : null}
            onChange={(v) => {
              const next: [number, number, number] = [...value];
              next[i] = (v ?? 0) * inverse;
              onChange(next);
            }}
          />
        ))}
      </div>
      {hint && <span className="field-help">{hint}</span>}
    </div>
  );
}

/** The event timeline list. */
function EventList({
  events,
  onJump,
}: {
  events: { id: string; time: number; label: string; description: string; trigger_value?: number | null; trigger_unit?: string | null; user_added: boolean }[];
  onJump: (time: number) => void;
}) {
  if (events.length === 0) {
    return <Hint>No events were detected in this run.</Hint>;
  }
  return (
    <DataTable
      headers={[
        { label: "Time", numeric: true },
        { label: "Event" },
        { label: "Value" },
        { label: "Source" },
        { label: "" },
      ]}
    >
      {events.map((event) => (
        <tr key={event.id}>
          <td className="num">{event.time.toFixed(3)} s</td>
          <td>
            <div className="col" style={{ gap: 0 }}>
              <span>{event.label}</span>
              {event.description && <span className="meta">{event.description}</span>}
            </div>
          </td>
          <td className="num">
            {event.trigger_value === null || event.trigger_value === undefined
              ? "--"
              : `${formatNumber(event.trigger_value, 4)} ${event.trigger_unit ?? ""}`}
          </td>
          <td>
            {event.user_added ? (
              <Badge tone="warning">Added by the user</Badge>
            ) : (
              <Badge tone="neutral">Detected automatically</Badge>
            )}
          </td>
          <td>
            <Button size="small" variant="ghost" onClick={() => onJump(event.time)}>
              Jump
            </Button>
          </td>
        </tr>
      ))}
    </DataTable>
  );
}

/** Build chart series from a run result. */
function buildSeries(
  runResult: ReturnType<typeof useStore.getState>["runResult"],
  channels: string[],
  hidden: string[],
): ChartSeries[] {
  if (!runResult) return [];
  const out: ChartSeries[] = [];
  runResult.channel_names.forEach((name, i) => {
    if (!channels.includes(name)) return;
    const values = runResult.columns[i];
    if (!values) return;
    out.push({
      name: name.replace(/_/g, " "),
      unit: runResult.channel_units[i] ?? "",
      times: runResult.times,
      values,
      visible: !hidden.includes(name.replace(/_/g, " ")),
    });
  });
  return out;
}

/**
 * Group the series into the plot views.
 *
 * `title` heads the chart, `short` is the tab label: seven full titles do not
 * fit across one row, and a tab that has scrolled out of sight is a tab nobody
 * knows is there.
 */
function chartGroups(
  series: ChartSeries[],
): { title: string; short: string; series: ChartSeries[] }[] {
  const groups: { title: string; short: string; names: string[] }[] = [
    {
      title: "Altitude and vertical velocity",
      short: "Altitude",
      names: ["altitude", "vertical velocity"],
    },
    {
      title: "Speed and acceleration",
      short: "Speed",
      names: ["speed", "acceleration"],
    },
    { title: "Roll, pitch, yaw", short: "Attitude", names: ["roll", "pitch", "yaw"] },
    {
      title: "Angular rates",
      short: "Rates",
      names: ["angular rate x", "angular rate y", "angular rate z"],
    },
    { title: "Total force and moment", short: "Forces", names: ["force", "moment"] },
    {
      title: "Angle of attack and sideslip",
      short: "Aero angles",
      names: ["angle of attack", "sideslip"],
    },
    {
      title: "Dynamic pressure and mass",
      short: "Pressure and mass",
      names: ["dynamic pressure", "mass"],
    },
  ];
  const used = new Set<string>();
  const result = groups.map((group) => {
    const matched = series.filter((s) => group.names.includes(s.name));
    for (const s of matched) used.add(s.name);
    return { title: group.title, short: group.short, series: matched };
  });
  const remainder = series.filter((s) => !used.has(s.name));
  if (remainder.length > 0) {
    // Anything the default groups did not claim still gets plotted, rather than
    // being silently dropped from the results workspace.
    result.push({ title: "Other selected channels", short: "Other", series: remainder });
  }
  return result.filter((g) => g.series.length > 0);
}

/**
 * The plot views, one tab per chart.
 *
 * One plot at a time rather than a grid of six, because a chart with the height
 * of the panel reads and a chart with a quarter of it does not. The tabs share
 * the same time window as the timeline, so zooming either one moves both.
 */
function PlotsView({
  groups,
  tab,
  onTabChange,
  series,
  view,
  onViewChange,
  cursor,
  onCursorChange,
  grid,
  onToggleSeries,
}: {
  groups: { title: string; short: string; series: ChartSeries[] }[];
  tab: number;
  onTabChange: (index: number) => void;
  series: ChartSeries[];
  view: ChartView;
  onViewChange: (view: ChartView) => void;
  cursor: number | null;
  onCursorChange: (index: number | null) => void;
  grid: boolean;
  onToggleSeries: (name: string) => void;
}) {
  if (series.length === 0) {
    return (
      <Notice
        level="warning"
        title="Every series is hidden"
        detail="Use the legend under a chart to show a series again, or add channels on the Outputs tab."
      />
    );
  }

  // The group list can change when channels are toggled, so the selected index
  // is clamped rather than trusted.
  const index = Math.min(Math.max(0, tab), Math.max(0, groups.length - 1));
  const group = groups[index];

  return (
    <div className="plot-pane-stack">
      <Tabs
        active={String(index)}
        onChange={(id) => onTabChange(Number(id))}
        tabs={groups.map((entry, position) => ({
          id: String(position),
          label: entry.short,
          title: entry.title,
        }))}
      />
      {group ? (
        <div className="plot-pane">
          <Chart
            title={group.title}
            series={group.series}
            view={view}
            onViewChange={onViewChange}
            cursor={cursor}
            onCursorChange={onCursorChange}
            grid={grid}
            onToggleSeries={onToggleSeries}
          />
        </div>
      ) : null}
    </div>
  );
}

/** The channel list the Outputs tab offers before a run exists. */
function defaultChannelNames(): string[] {
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

/** The sample index nearest a time. */
function nearestIndex(times: number[], time: number): number {
  if (times.length === 0) return 0;
  let best = 0;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (let i = 0; i < times.length; i += 1) {
    const distance = Math.abs(times[i] - time);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = i;
    }
  }
  return best;
}
