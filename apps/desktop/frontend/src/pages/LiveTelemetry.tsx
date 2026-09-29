/**
 * The Live Telemetry workspace.
 *
 * An instrumentation console: connection on the left, the live rocket in the
 * middle, health on the right, raw and filtered channels below, and live plots at
 * the bottom.
 *
 * The display is throttled and the recording is not, so this screen always says
 * which of the two it is showing.
 */

import { useEffect, useMemo, useRef, useState } from "react";
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
  Notice,
  Panel,
  Select,
  Tabs,
  TextInput,
} from "../components/ui";
import {
  IconPlug,
  IconRecord,
  IconStop,
  IconWarning,
} from "../components/Icons";
import { Chart, allTimes, type ChartSeries, type ChartView } from "../components/Chart";
import { Viewport, defaultViewportOptions, type CameraPreset } from "../components/Viewport";
import { useStore, useAdvancedMode } from "../lib/store";
import {
  formatBytes,
  formatNumber,
  formatRate,
  formatSeconds,
} from "../lib/format";
import type { DeviceProfile } from "../lib/types";

type ChannelTab = "raw" | "filtered" | "packets" | "mapping" | "validation";

/** The baud rates the picker offers. */
const BAUD_RATES = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

export function TelemetryPage() {
  const ports = useStore((s) => s.ports);
  const refreshPorts = useStore((s) => s.refreshPorts);
  const status = useStore((s) => s.telemetryStatus);
  const health = useStore((s) => s.telemetryHealth);
  const frames = useStore((s) => s.liveFrames);
  const pollTelemetry = useStore((s) => s.pollTelemetry);
  const connectPort = useStore((s) => s.connectPort);
  const connectScripted = useStore((s) => s.connectScripted);
  const disconnectPort = useStore((s) => s.disconnectPort);
  const startRecording = useStore((s) => s.startRecording);
  const stopRecording = useStore((s) => s.stopRecording);
  const sensorValidation = useStore((s) => s.sensorValidation);
  const runSensorValidation = useStore((s) => s.runSensorValidation);
  const deviceProfiles = useStore((s) => s.deviceProfiles);
  const loadProfiles = useStore((s) => s.loadProfiles);
  const scripted = useStore((s) => s.scriptedStream);
  const sessions = useStore((s) => s.telemetrySessions);
  const loadTelemetrySessions = useStore((s) => s.loadTelemetrySessions);
  const deleteSession = useStore((s) => s.deleteTelemetrySession);
  const settings = useStore((s) => s.settings);
  const advanced = useAdvancedMode();
  const setAdvancedMode = useStore((s) => s.setAdvancedMode);

  const [portName, setPortName] = useState("");
  const [baudRate, setBaudRate] = useState(115200);
  const [profileId, setProfileId] = useState("");
  const [tab, setTab] = useState<ChannelTab>("raw");
  const [orientation, setOrientation] = useState("stationary");
  const [sessionLabel, setSessionLabel] = useState("Telemetry session");
  const [camera, setCamera] = useState<CameraPreset>("follow");
  const [cursor, setCursor] = useState<number | null>(null);
  const [view, setView] = useState<ChartView>({ start: 0, end: 1 });
  const [hidden, setHidden] = useState<string[]>([]);
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    void refreshPorts();
    void loadProfiles();
    void loadTelemetrySessions();
  }, [refreshPorts, loadProfiles, loadTelemetrySessions]);

  useEffect(() => {
    if (settings) setBaudRate(settings.telemetry.default_baud_rate);
  }, [settings]);

  // Poll while connected. The backend throttles the display stream, so this is
  // the display rate, not the packet rate.
  useEffect(() => {
    if (!status?.connected) return;
    const interval = Math.max(50, 1000 / (settings?.telemetry.display_rate_hz ?? 30));
    timerRef.current = window.setInterval(() => void pollTelemetry(), interval);
    return () => {
      if (timerRef.current !== null) window.clearInterval(timerRef.current);
    };
  }, [status?.connected, pollTelemetry, settings?.telemetry.display_rate_hz]);

  const latest = frames.length > 0 ? frames[frames.length - 1] : null;

  /** Build replay-shaped data from the live frames for the viewport. */
  const liveReplay = useMemo(() => {
    if (frames.length === 0) return null;
    const times = frames.map((f) => f.host_time - frames[0].host_time);
    return {
      times,
      positions: frames.map(() => [0, 0, 0] as [number, number, number]),
      quaternions: frames.map((f) => f.attitude ?? ([1, 0, 0, 0] as [number, number, number, number])),
      velocities: frames.map(() => [0, 0, 0] as [number, number, number]),
      angular_rates: frames.map((f) => f.angular_rate ?? ([0, 0, 0] as [number, number, number])),
      events: [],
      source: "live telemetry",
      has_position: false,
    };
  }, [frames]);

  const series = useMemo<ChartSeries[]>(() => {
    if (frames.length === 0) return [];
    const times = frames.map((f) => f.host_time - frames[0].host_time);
    const build = (name: string, unit: string, pick: (i: number) => number): ChartSeries => ({
      name,
      unit,
      times,
      values: frames.map((_, i) => pick(i)),
      visible: !hidden.includes(name),
    });
    return [
      build("acceleration X", "m/s²", (i) => frames[i].acceleration?.[0] ?? Number.NaN),
      build("acceleration Y", "m/s²", (i) => frames[i].acceleration?.[1] ?? Number.NaN),
      build("acceleration Z", "m/s²", (i) => frames[i].acceleration?.[2] ?? Number.NaN),
      build("angular rate X", "rad/s", (i) => frames[i].angular_rate?.[0] ?? Number.NaN),
      build("angular rate Y", "rad/s", (i) => frames[i].angular_rate?.[1] ?? Number.NaN),
      build("angular rate Z", "rad/s", (i) => frames[i].angular_rate?.[2] ?? Number.NaN),
      build("pressure", "Pa", (i) => frames[i].pressure ?? Number.NaN),
    ];
  }, [frames, hidden]);

  useEffect(() => {
    const all = allTimes(series);
    if (all) setView(all);
  }, [series.length]);

  const latestProfile: DeviceProfile | undefined = deviceProfiles.find((p) => p.id === profileId);

  const derivedRate = useMemo(() => {
    if (!status || status.bytes_received === 0) return 0;
    const span = latest ? latest.host_time : 0;
    if (span <= 0) return 0;
    return status.packets_received / span;
  }, [status, latest]);

  return (
    <div className="page" style={{ padding: "var(--space-3)", gap: "var(--space-3)" }}>
      <div className="row wrap">
        <h1 className="page-title" style={{ fontSize: "var(--text-section)" }}>
          Live Telemetry
        </h1>
        {/* Simple mode hides this console from the tab bar. It stays reachable
            through the command palette, so the page says why it is quiet rather
            than showing half a working console. */}
        {!advanced && (
          <>
            <Badge tone="neutral">Hidden in Simple mode</Badge>
            <span className="spacer" />
            <Button size="small" variant="primary" onClick={() => void setAdvancedMode(true)}>
              Switch to Advanced
            </Button>
          </>
        )}
        {advanced && (
          <Badge tone={status?.connected ? (status.recording ? "error" : "success") : "neutral"}>
            {status?.connected ? status.state : "Disconnected"}
          </Badge>
        )}
        {advanced && scripted && (
          <Badge tone="warning">Synthetic stream, not recorded flight data</Badge>
        )}
        {advanced && <span className="spacer" />}
        {advanced && status?.recording && (
          <Button size="small" variant="danger" icon={<IconStop />} onClick={() => void stopRecording(sessionLabel)}>
            Stop and save
          </Button>
        )}
        {advanced && status?.connected && !status.recording && (
          <Button size="small" variant="primary" icon={<IconRecord />} onClick={() => void startRecording()}>
            Start recording
          </Button>
        )}
      </div>

      {!advanced && (
        <Panel title="The hardware console is an Advanced feature">
          <Hint>
            Simple mode keeps the simulate, replay, and compare workflow and leaves out the parts that
            need a device and sensor tuning: the serial connection, device profiles, packet formats,
            channel mapping, calibration, and the estimator settings.
          </Hint>
          <Hint>
            Recorded sessions in the open project are still listed on the Overview screen, and a saved
            session can be opened on the Flight Analysis screen like any other log.
          </Hint>
          <div className="row">
            <Button variant="primary" onClick={() => void setAdvancedMode(true)}>
              Switch to Advanced
            </Button>
          </div>
        </Panel>
      )}

      {advanced && (
      <>
      <div className="grid-12">
        <section className="col-4 stack">
          <Panel
            title="Connection"
            subtitle={status?.connected ? `${status.port} at ${status.baud_rate} baud` : "No device"}
            actions={
              <Button size="small" variant="ghost" onClick={() => void refreshPorts()}>
                Refresh
              </Button>
            }
          >
            <Select
              label="Serial port"
              value={portName}
              onChange={setPortName}
              options={[
                { value: "", label: ports.length === 0 ? "No ports found" : "Choose a port" },
                ...ports.map((p) => ({ value: p.port_name, label: p.label })),
              ]}
              hint={ports.length === 0 ? "No serial ports were reported by the operating system." : undefined}
            />
            <Select
              label="Baud rate"
              value={String(baudRate)}
              onChange={(value) => setBaudRate(Number(value))}
              options={BAUD_RATES.map((rate) => ({ value: String(rate), label: `${rate}` }))}
            />
            <Select
              label="Device profile"
              value={profileId}
              onChange={setProfileId}
              options={[
                { value: "", label: "Default (CSV, 100 Hz)" },
                ...deviceProfiles.map((p) => ({ value: p.id, label: p.name })),
              ]}
            />
            <div className="row">
              <Button
                icon={<IconPlug />}
                variant="primary"
                disabled={portName === "" || status?.connected}
                onClick={() => void connectPort(portName, baudRate)}
              >
                Connect
              </Button>
              <Button disabled={!status?.connected} onClick={() => void disconnectPort()}>
                Disconnect
              </Button>
            </div>
            <div className="divider" />
            <Button variant="ghost" onClick={() => void connectScripted()} disabled={status?.connected}>
              Attach the synthetic stream (no hardware)
            </Button>
            <Hint>
              HexaDOF never sends commands to a device. Telemetry is read-only in this version.
            </Hint>
          </Panel>

          <Panel title="Recording">
            <TextInput
              label="Session label"
              value={sessionLabel}
              onChange={(e) => setSessionLabel(e.target.value)}
              hint="Used for the artifact name inside the project."
            />
            <KeyValue
              pairs={[
                ["Samples", status ? `${status.recorded_samples}` : "0"],
                ["Capacity", status ? `${status.capacity}` : "0"],
                [
                  "Fill",
                  status && status.capacity > 0
                    ? `${((status.recorded_samples / status.capacity) * 100).toFixed(2)} %`
                    : "--",
                ],
                ["State", status?.recording ? "Recording" : "Idle"],
              ]}
            />
            {status && status.recorded_samples > (status.capacity || 0) * 0.9 && (
              <Notice
                level="warning"
                title="The recorder is nearly full"
                detail="Increase the recording buffer in Settings, or stop and save the session."
              />
            )}
            <Hint>
              The recorder runs at full device rate. The display below is throttled, so a slow screen
              never costs a recorded sample.
            </Hint>
          </Panel>

          <Panel title="Saved sessions" subtitle={`${sessions.length} in the project`}>
            {sessions.length === 0 ? (
              <Hint>No telemetry session has been saved in this project yet.</Hint>
            ) : (
              <DataTable
                headers={[
                  { label: "Session" },
                  { label: "Samples", numeric: true },
                  { label: "" },
                ]}
              >
                {sessions.map((session) => (
                  <tr key={session.name}>
                    <td>
                      <div className="col" style={{ gap: 0 }}>
                        <span>{session.label}</span>
                        <span className="meta mono">{session.name}</span>
                      </div>
                    </td>
                    <td className="num">{session.samples}</td>
                    <td>
                      <Button
                        size="small"
                        variant="ghost"
                        onClick={() => {
                          if (window.confirm(`Delete ${session.name}? This cannot be undone.`)) {
                            void deleteSession(session.name);
                          }
                        }}
                      >
                        Delete
                      </Button>
                    </td>
                  </tr>
                ))}
              </DataTable>
            )}
          </Panel>
        </section>

        <section className="col-5 stack">
          <div className="card-slot">
            <Viewport
              data={liveReplay}
              index={liveReplay ? liveReplay.times.length - 1 : 0}
              options={{
                ...defaultViewportOptions,
                showTrajectory: false,
                showVelocity: false,
                showGround: true,
              }}
              camera={camera}
              onCameraChange={setCamera}
              attitudeLabel={latest?.attitude_source ?? "NO ATTITUDE AVAILABLE"}
              attitudeNote={latest?.estimator_note ?? undefined}
              attitudePoor={latest?.estimator_indicative ?? true}
            />
          </div>

          <Panel title="Live channels" subtitle={latest ? `${latest.host_time.toFixed(3)} s` : "no data"}>
            <Tabs<ChannelTab>
              active={tab}
              onChange={setTab}
              tabs={[
                { id: "raw", label: "Raw" },
                { id: "filtered", label: "Filtered" },
                { id: "packets", label: "Packet inspector" },
                { id: "mapping", label: "Mapping" },
                { id: "validation", label: "Validation" },
              ]}
            />

            {tab === "raw" && (
              <>
                {latest?.mapped && latest.mapped.length > 0 ? (
                  <DataTable
                    headers={[
                      { label: "Destination" },
                      { label: "Value", numeric: true },
                    ]}
                  >
                    {latest.mapped.map(([name, value]) => (
                      <tr key={name}>
                        <td>{name}</td>
                        <td className="num">{formatNumber(value, 6)}</td>
                      </tr>
                    ))}
                  </DataTable>
                ) : (
                  <EmptyState
                    title="No samples yet"
                    detail="Connect a device, or attach the synthetic stream, to see the converted channel values."
                  />
                )}
              </>
            )}

            {tab === "filtered" && (
              <>
                <DataTable
                  headers={[
                    { label: "Quantity" },
                    { label: "Value", numeric: true },
                  ]}
                >
                  <tr>
                    <td>Acceleration</td>
                    <td className="num">
                      {latest?.acceleration
                        ? latest.acceleration.map((v) => formatNumber(v, 4)).join(", ") + " m/s²"
                        : "--"}
                    </td>
                  </tr>
                  <tr>
                    <td>Angular rate</td>
                    <td className="num">
                      {latest?.angular_rate
                        ? latest.angular_rate.map((v) => formatNumber(v, 5)).join(", ") + " rad/s"
                        : "--"}
                    </td>
                  </tr>
                  <tr>
                    <td>Magnetic field</td>
                    <td className="num">
                      {latest?.magnetic_field
                        ? latest.magnetic_field.map((v) => formatNumber(v, 6)).join(", ") + " T"
                        : "--"}
                    </td>
                  </tr>
                  <tr>
                    <td>Pressure</td>
                    <td className="num">
                      {latest?.pressure === null || latest?.pressure === undefined
                        ? "--"
                        : `${formatNumber(latest.pressure, 2)} Pa`}
                    </td>
                  </tr>
                  <tr>
                    <td>Estimated attitude</td>
                    <td className="num">
                      {latest?.attitude
                        ? latest.attitude.map((v) => formatNumber(v, 5)).join(", ")
                        : "--"}
                    </td>
                  </tr>
                </DataTable>
                <Hint>
                  The estimator is a gyro integration or a gated complementary filter. Neither is an
                  absolute orientation reference, and the label above the rocket says which one is
                  running.
                </Hint>
              </>
            )}

            {tab === "packets" && (
              <>
                <KeyValue
                  pairs={[
                    ["Packets decoded", `${health?.packets_received ?? status?.packets_received ?? 0}`],
                    ["Packets rejected", `${health?.packets_rejected ?? 0}`],
                    ["Checksum failures", `${health?.crc_failures ?? 0}`],
                    ["Bytes discarded", formatBytes(health?.discarded_bytes ?? 0)],
                    ["Display downsampled", health?.display_downsampled ? "yes" : "no"],
                    ["Pending display frames", `${health?.pending_display_frames ?? 0}`],
                  ]}
                />
                {health?.last_rejection && (
                  <Notice level="warning" title="Last rejection" detail={health.last_rejection} />
                )}
                <span className="meta">Most recent frame</span>
                <pre className="issue-technical">{JSON.stringify(latest, null, 2)}</pre>
              </>
            )}

            {tab === "mapping" && (
              <>
                {latestProfile ? (
                  <>
                    <KeyValue
                      pairs={[
                        ["Profile", latestProfile.name],
                        ["Identifier", latestProfile.id],
                        ["Expected rate", formatRate(latestProfile.expected_rate_hz ?? null)],
                        ["Timestamp source", latestProfile.timestamp_source],
                      ]}
                    />
                    <Hint>
                      Unit conversion, sign, axis mapping, and calibration are configured in the device
                      profile. HexaDOF refuses to guess an unknown unit rather than silently corrupting
                      every downstream result.
                    </Hint>
                  </>
                ) : (
                  <Hint>
                    Choose a stored device profile above to see its mapping. The default profile maps a
                    CSV line of accelerometer, gyroscope, and barometer fields in SI units.
                  </Hint>
                )}
              </>
            )}

            {tab === "validation" && (
              <>
                <div className="field-row">
                  <Select
                    label="Declared pose"
                    value={orientation}
                    onChange={setOrientation}
                    options={[
                      { value: "stationary", label: "Device stationary" },
                      { value: "nose_up", label: "Rocket nose upward" },
                      { value: "nose_down", label: "Rocket nose downward" },
                      { value: "on_side", label: "Rocket lying on its side" },
                    ]}
                  />
                  <Field label="Expected rate">
                    <TextInput
                      value={settings ? String(settings.telemetry.default_baud_rate) : "100"}
                      readOnly
                    />
                  </Field>
                </div>
                <Button onClick={() => void runSensorValidation(orientation, 100)}>
                  Run the checklist
                </Button>
                {sensorValidation ? (
                  <>
                    <div className="row wrap">
                      <Badge tone={sensorValidation.ready ? "success" : "error"}>
                        {sensorValidation.ready ? "Ready to record" : "Not ready"}
                      </Badge>
                      <Badge tone={sensorValidation.has_warning ? "warning" : "neutral"}>
                        {sensorValidation.has_warning ? "Warnings present" : "No warnings"}
                      </Badge>
                    </div>
                    <DataTable
                      headers={[
                        { label: "Check" },
                        { label: "Status" },
                        { label: "Detail" },
                      ]}
                    >
                      {sensorValidation.steps.map((step) => (
                        <tr key={step.id}>
                          <td>{step.title}</td>
                          <td>
                            <Badge
                              tone={
                                step.status === "pass"
                                  ? "success"
                                  : step.status === "warning"
                                    ? "warning"
                                    : step.status === "fail"
                                      ? "error"
                                      : "neutral"
                              }
                            >
                              {step.status}
                            </Badge>
                          </td>
                          <td className="meta">
                            {step.detail ?? step.instruction ?? step.expected ?? ""}
                          </td>
                        </tr>
                      ))}
                    </DataTable>
                  </>
                ) : (
                  <Hint>
                    The checklist compares the observed behaviour against what each physical pose
                    predicts. A swapped or inverted axis is the most common cause of an attitude that
                    looks wrong.
                  </Hint>
                )}
              </>
            )}
          </Panel>
        </section>

        <section className="col-3 stack">
          <Panel title="Connection health">
            <KeyValue
              pairs={[
                ["State", status?.state ?? "Disconnected"],
                ["Packets", `${status?.packets_received ?? 0}`],
                ["Rejected", `${status?.packets_rejected ?? 0}`],
                ["Checksum failures", `${status?.crc_failures ?? 0}`],
                ["Bytes received", formatBytes(status?.bytes_received ?? 0)],
                ["Bytes discarded", formatBytes(status?.discarded_bytes ?? 0)],
                ["Observed rate", formatRate(derivedRate)],
                ["Recorded samples", `${status?.recorded_samples ?? 0}`],
                ["Display frames dropped", `${status?.display_dropped ?? 0}`],
                ["Display downsampled", status?.display_downsampled ? "yes" : "no"],
                ["Source finished", status?.finished ? "yes" : "no"],
              ]}
            />
            {status?.display_downsampled && (
              <Notice
                level="info"
                title="The display is downsampled"
                detail="The recorder holds every sample. Only this screen is showing fewer frames."
              />
            )}
            {status?.last_error && (
              <Notice level="error" title="Read error" detail={status.last_error} />
            )}
            {status && (status.packets_rejected > 0 || status.crc_failures > 0) && (
              <Notice
                level="warning"
                title="Frames are being discarded"
                detail="Check the baud rate and the packet format in the device profile. A mismatched format produces checksum failures rather than data."
              />
            )}
          </Panel>

          <Panel title="Estimator health">
            {latest ? (
              <>
                <KeyValue
                  pairs={[
                    ["Mode", latest.attitude_source ?? "unavailable"],
                    ["Health", latest.estimator_health ?? "unknown"],
                    ["Indicative only", latest.estimator_indicative ? "yes" : "no"],
                  ]}
                />
                {/* The estimator's own sentence, stated once. The warning badge
                    on the viewport is where the caveat belongs. */}
                {latest.estimator_note && <Hint>{latest.estimator_note}</Hint>}
              </>
            ) : (
              <Hint>No estimator report yet.</Hint>
            )}
          </Panel>

          <Panel title="Plot series" subtitle="SI units, converted only for display">
            <div className="row wrap">
              {["acceleration X", "acceleration Y", "acceleration Z", "angular rate X", "angular rate Y", "angular rate Z", "pressure"].map(
                (name) => (
                  <Checkbox
                    key={name}
                    label={name}
                    checked={!hidden.includes(name)}
                    onChange={(v) =>
                      setHidden((current) =>
                        v ? current.filter((n) => n !== name) : [...current, name],
                      )
                    }
                  />
                ),
              )}
            </div>
          </Panel>
        </section>
      </div>

      <Panel title="Live plots" subtitle={`${frames.length} display frame(s) held`}>
        {series.length === 0 ? (
          <EmptyState
            icon={<IconWarning />}
            title="No live data"
            detail="Once frames arrive, the accelerometer, gyroscope, and pressure channels are plotted here at the display rate."
          />
        ) : (
          <div className="grid-12">
            <div className="col-6 card-slot short">
              <Chart
                title="Accelerometer"
                series={series.slice(0, 3)}
                view={view}
                onViewChange={setView}
                cursor={cursor}
                onCursorChange={setCursor}
                grid={settings?.appearance.chart_grid ?? true}
                onToggleSeries={(name) =>
                  setHidden((current) =>
                    current.includes(name) ? current.filter((n) => n !== name) : [...current, name],
                  )
                }
              />
            </div>
            <div className="col-6 card-slot short">
              <Chart
                title="Gyroscope and pressure"
                series={series.slice(3)}
                view={view}
                onViewChange={setView}
                cursor={cursor}
                onCursorChange={setCursor}
                grid={settings?.appearance.chart_grid ?? true}
                onToggleSeries={(name) =>
                  setHidden((current) =>
                    current.includes(name) ? current.filter((n) => n !== name) : [...current, name],
                  )
                }
              />
            </div>
          </div>
        )}
        <Hint>
          The shared cursor reads the same instant on every trace. Each chart lists the minimum, maximum,
          mean, and last value of the visible window.
        </Hint>
      </Panel>

      <Collapsible title="Recorder notes" meta={<span className="meta">what the recorder knows</span>}>
        <KeyValue
          pairs={[
            ["Samples held for display", `${frames.length}`],
            ["First display frame", frames.length > 0 ? formatSeconds(frames[0].host_time) : "--"],
            ["Last display frame", latest ? formatSeconds(latest.host_time) : "--"],
            ["Sample time axis", latest?.device_time === null ? "host clock" : "device clock"],
            ["Sequence", latest?.sequence === null || latest?.sequence === undefined ? "--" : `${latest.sequence}`],
          ]}
        />
        <Hint>
          A device clock and the host clock are different clocks. The recording is stored against the
          device clock, which is the one that survives a burst read.
        </Hint>
      </Collapsible>
      </>
      )}
    </div>
  );
}
