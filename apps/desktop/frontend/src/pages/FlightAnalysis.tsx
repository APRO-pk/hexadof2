/**
 * The Flight Analysis workspace.
 *
 * Import a log, validate its timestamps, inspect data quality, replay the flight,
 * add event markers, then align it against a simulation and read the error
 * metrics. The import pipeline reports every stage, so a failure says exactly
 * where it stopped.
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
  Select,
  SeverityBadge,
  Tabs,
  TextInput,
} from "../components/ui";
import {
  IconAnalysis,
  IconCompare,
  IconExport,
  IconImport,
} from "../components/Icons";
import { Chart, allTimes, downloadText, seriesToCsv, type ChartSeries, type ChartView } from "../components/Chart";
import { Transport, eventsFromRun } from "../components/Timeline";
import { Viewport, defaultViewportOptions, type CameraPreset } from "../components/Viewport";
import { useAdvancedMode, useStore } from "../lib/store";
import { pickFile } from "../lib/api";
import { formatBytes, formatNumber, formatRate, formatSeconds, formatSigned } from "../lib/format";
import type { AlignmentMethodCode } from "../lib/types";

type ImportTab = "import" | "timestamps" | "quality" | "channels" | "comparison";

export function FlightAnalysisPage() {
  const preview = useStore((s) => s.flightPreview);
  const previewPath = useStore((s) => s.flightPreviewPath);
  const previewFlight = useStore((s) => s.previewFlight);
  const validateFlight = useStore((s) => s.validateFlight);
  const timestampReport = useStore((s) => s.timestampReport);
  const flightSession = useStore((s) => s.flightSession);
  const flightChannels = useStore((s) => s.flightChannels);
  const importFlight = useStore((s) => s.importFlight);
  const flightSeries = useStore((s) => s.flightSeries);
  const loadFlightSeries = useStore((s) => s.loadFlightSeries);
  const flightTimeline = useStore((s) => s.flightTimeline);
  const detectFlightEvents = useStore((s) => s.detectFlightEvents);
  const addFlightMarker = useStore((s) => s.addFlightMarker);
  const flightReplay = useStore((s) => s.flightReplay);
  const loadFlightReplay = useStore((s) => s.loadFlightReplay);
  const flightSessions = useStore((s) => s.flightSessions);
  const loadFlightSessions = useStore((s) => s.loadFlightSessions);
  const deleteFlightSession = useStore((s) => s.deleteFlightSession);
  const runResult = useStore((s) => s.runResult);
  const comparison = useStore((s) => s.comparison);
  const comparisonRequest = useStore((s) => s.comparisonRequest);
  const setComparisonRequest = useStore((s) => s.setComparisonRequest);
  const comparisonMethods = useStore((s) => s.comparisonMethods);
  const loadComparisonMethods = useStore((s) => s.loadComparisonMethods);
  const comparableChannels = useStore((s) => s.comparableChannels);
  const loadComparableChannels = useStore((s) => s.loadComparableChannels);
  const buildComparison = useStore((s) => s.buildComparison);
  const exportReport = useStore((s) => s.exportReport);
  const settings = useStore((s) => s.settings);
  const advanced = useAdvancedMode();

  const [tab, setTab] = useState<ImportTab>("import");
  const [index, setIndex] = useState(0);
  const [camera, setCamera] = useState<CameraPreset>("follow");
  const [cursor, setCursor] = useState<number | null>(null);
  const [view, setView] = useState<ChartView>({ start: 0, end: 1 });
  const [hidden, setHidden] = useState<string[]>([]);
  const [label, setLabel] = useState("Flight 001");
  const [timestampUnit, setTimestampUnit] = useState("seconds");
  const [markerLabel, setMarkerLabel] = useState("");
  const [markerTime, setMarkerTime] = useState("0");
  const [showMarker, setShowMarker] = useState(false);
  const [reportTitle, setReportTitle] = useState("Flight analysis report");
  const [showReport, setShowReport] = useState(false);

  useEffect(() => {
    void loadFlightSessions();
    void loadComparisonMethods();
    void loadComparableChannels();
  }, [loadFlightSessions, loadComparisonMethods, loadComparableChannels]);

  useEffect(() => {
    if (flightSeries) {
      const series = seriesFromFlight(flightSeries, hidden);
      const all = allTimes(series);
      if (all) setView(all);
    }
  }, [flightSeries, hidden]);

  const chooseFile = async () => {
    const path = await pickFile({
      title: "Choose a flight log",
      filters: [
        { name: "Flight log", extensions: ["csv", "hlog", "txt"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (!path) return;
    await previewFlight(path);
    await validateFlight(path);
  };

  const flightChartSeries = useMemo(() => {
    if (!flightSession || !flightSeries) return [];
    return seriesFromFlight(flightSeries, hidden);
  }, [flightSession, flightSeries, hidden]);

  const alignmentOptions = comparisonMethods.length > 0
    ? comparisonMethods.map(([code, labelText, recommended]) => ({
        value: code,
        label: recommended ? `${labelText} (recommended)` : labelText,
      }))
    : [
        { value: "first_sample", label: "First valid sample" },
        { value: "absolute", label: "Absolute timestamps" },
        { value: "manual", label: "Manual offset" },
      ];

  const replayData = useMemo(() => {
    if (flightReplay) return flightReplay;
    if (!flightSession) return null;
    return {
      times: flightSeries?.times ?? [],
      positions: (flightSeries?.times ?? []).map(() => [0, 0, 0] as [number, number, number]),
      quaternions: (flightSeries?.times ?? []).map(() => [1, 0, 0, 0] as [number, number, number, number]),
      velocities: (flightSeries?.times ?? []).map(() => [0, 0, 0] as [number, number, number]),
      angular_rates: (flightSeries?.times ?? []).map(() => [0, 0, 0] as [number, number, number]),
      events: [],
      source: "orientation only",
      has_position: false,
    };
  }, [flightReplay, flightSession, flightSeries]);

  return (
    <div className="page" style={{ padding: "var(--space-3)", gap: "var(--space-3)" }}>
      <div className="row wrap">
        <h1 className="page-title" style={{ fontSize: "var(--text-section)" }}>
          Flight Analysis
        </h1>
        {flightSession && (
          <>
            <Badge tone="accent">{flightSession.source_file}</Badge>
            <span className="meta">
              {flightSession.sample_count} samples at {formatRate(flightSession.sample_rate_hz)}
            </span>
          </>
        )}
        <span className="spacer" />
        <Button icon={<IconImport />} variant="primary" onClick={() => void chooseFile()}>
          Import a flight log
        </Button>
        <Button
          icon={<IconCompare />}
          disabled={!runResult || !flightSession}
          onClick={() => void buildComparison()}
        >
          Compare with the run
        </Button>
        <Button icon={<IconExport />} disabled={!comparison} onClick={() => setShowReport(true)}>
          Export report
        </Button>
      </div>

      <div className="grid-12">
        <section className="col-4 stack">
          <Panel
            title="Import"
            subtitle={previewPath ? previewPath.split(/[\\/]/).pop() : "No file selected"}
          >
            <Tabs<ImportTab>
              active={tab}
              onChange={setTab}
              tabs={[
                { id: "import", label: "File" },
                // Simple mode imports a log and analyses it. The timestamp
                // audit, quality table, channel mapping, and comparison options
                // are the diagnostic view of the same import.
                ...(advanced
                  ? [
                      { id: "timestamps" as const, label: "Timestamps" },
                      { id: "quality" as const, label: "Quality" },
                      { id: "channels" as const, label: "Channels" },
                      { id: "comparison" as const, label: "Compare" },
                    ]
                  : []),
              ]}
            />

            {tab === "import" && (
              <>
                <TextInput
                  label="Session label"
                  value={label}
                  onChange={(e) => setLabel(e.target.value)}
                  hint="Used for the folder name inside flights/."
                />
                <Select
                  label="Timestamp unit"
                  value={timestampUnit}
                  onChange={setTimestampUnit}
                  options={[
                    { value: "seconds", label: "Seconds" },
                    { value: "milliseconds", label: "Milliseconds" },
                    { value: "microseconds", label: "Microseconds" },
                    { value: "ticks", label: "Device ticks" },
                  ]}
                />
                <div className="row">
                  <Button onClick={() => void chooseFile()}>Choose a file</Button>
                  <Button
                    variant="ghost"
                    disabled={!previewPath}
                    onClick={() => previewPath && void validateFlight(previewPath)}
                  >
                    Re-validate
                  </Button>
                </div>
                <Button
                  variant="primary"
                  disabled={!previewPath}
                  onClick={async () => {
                    if (previewPath) {
                      await importFlight(previewPath, label, true);
                      await loadFlightReplay();
                    }
                  }}
                >
                  Import and create a session
                </Button>
                {preview ? (
                  <>
                    <KeyValue
                      pairs={[
                        ["Delimiter", preview.delimiter === "\t" ? "tab" : preview.delimiter],
                        ["Header row", preview.has_header ? "detected" : "none"],
                        ["Columns", `${preview.headers.length}`],
                        ["Rows previewed", `${preview.row_count}`],
                      ]}
                    />
                    {preview.warnings.length > 0 && (
                      <Notice
                        level="warning"
                        title="Preview notes"
                        detail={preview.warnings.join(" ")}
                      />
                    )}
                    <span className="meta">Column preview and inferred roles</span>
                    <DataTable
                      headers={[
                        { label: "Column" },
                        { label: "Inferred role" },
                        { label: "First value" },
                      ]}
                    >
                      {preview.headers.map((header, i) => (
                        <tr key={`${header}-${i}`}>
                          <td>{header}</td>
                          <td>
                            {preview.inferred_physical[i] ? (
                              <Badge tone="accent">{preview.inferred_roles[i]}</Badge>
                            ) : (
                              <span className="muted">{preview.inferred_roles[i]}</span>
                            )}
                          </td>
                          <td className="num">
                            {preview.sample_rows[0]?.[i] ?? "--"}
                          </td>
                        </tr>
                      ))}
                    </DataTable>
                    <Hint>
                      A role marked as a channel is a physical quantity HexaDOF will use. Anything not
                      recognised stays raw and is never fed to an estimator.
                    </Hint>
                  </>
                ) : (
                  <EmptyState
                    icon={<IconAnalysis />}
                    title="No file chosen"
                    detail="HexaDOF reads CSV and its own binary log. The original file is copied into the project untouched, and its hash is recorded."
                  />
                )}
              </>
            )}

            {tab === "timestamps" && (
              <>
                {timestampReport ? (
                  <>
                    <KeyValue
                      pairs={[
                        ["Samples", `${timestampReport.sample_count}`],
                        ["Median interval", `${formatNumber(timestampReport.median_dt_seconds * 1000, 4)} ms`],
                        ["Effective rate", formatRate(timestampReport.effective_rate_hz)],
                        ["Strictly increasing", timestampReport.strictly_increasing ? "yes" : "no"],
                        ["Duplicate timestamps", `${timestampReport.duplicate_count}`],
                        ["Time reversals", `${timestampReport.reversal_count}`],
                        ["Gaps", `${timestampReport.gaps.length}`],
                        ["Rollover corrected", timestampReport.rollover_corrected ? "yes" : "no"],
                      ]}
                    />
                    {timestampReport.gaps.length > 0 && (
                      <>
                        <span className="meta">Gaps</span>
                        <DataTable
                          headers={[
                            { label: "Starts at", numeric: true },
                            { label: "Duration", numeric: true },
                            { label: "Estimated missing", numeric: true },
                          ]}
                        >
                          {timestampReport.gaps.slice(0, 20).map((gap, i) => (
                            <tr key={`${gap[0]}-${i}`}>
                              <td className="num">{gap[0].toFixed(4)} s</td>
                              <td className="num">{gap[1].toFixed(4)} s</td>
                              <td className="num">{gap[2]}</td>
                            </tr>
                          ))}
                        </DataTable>
                      </>
                    )}
                    {timestampReport.warnings.length > 0 && (
                      <Notice
                        level="warning"
                        title="Timestamp findings"
                        detail={timestampReport.warnings.join(" ")}
                      />
                    )}
                    {timestampReport.strictly_increasing && timestampReport.gaps.length === 0 && (
                      <Notice
                        level="success"
                        title="Timestamps are clean"
                        detail="Time increases at a consistent rate with no duplicates, reversals, or gaps."
                      />
                    )}
                  </>
                ) : (
                  <Hint>Choose a file to validate its timestamps.</Hint>
                )}
              </>
            )}

            {tab === "quality" && (
              <>
                {flightSession ? (
                  <>
                    <div className="row wrap">
                      <Badge
                        tone={
                          flightSession.quality_severity.toLowerCase().includes("error")
                            ? "error"
                            : flightSession.quality_flags.length > 0
                              ? "warning"
                              : "success"
                        }
                      >
                        {flightSession.quality_flags.length === 0
                          ? "No findings"
                          : `${flightSession.quality_flags.length} finding(s)`}
                      </Badge>
                      <span className="meta">Overall: {flightSession.quality_severity}</span>
                    </div>
                    {flightSession.quality_flags.length === 0 ? (
                      <Notice
                        level="success"
                        title="No data quality problems were found"
                        detail="Every channel is within its plausible range, no channel is stuck, and no pressure jumps or time reversals were detected."
                      />
                    ) : (
                      <div className="issue-list">
                        {flightSession.quality_flags.map((flag) => (
                          <article className="issue issue-warning" key={flag}>
                            <header className="issue-head">
                              <SeverityBadge severity="warning" />
                              <span className="issue-title">{flag.split(":")[0]}</span>
                            </header>
                            <p className="issue-detail">{flag}</p>
                          </article>
                        ))}
                      </div>
                    )}
                  </>
                ) : (
                  <Hint>Import a flight log to assess its data quality.</Hint>
                )}
              </>
            )}

            {tab === "channels" && (
              <>
                {flightChannels.length > 0 ? (
                  <DataTable
                    headers={[
                      { label: "Channel" },
                      { label: "Role" },
                      { label: "Unit" },
                      { label: "Min", numeric: true },
                      { label: "Max", numeric: true },
                    ]}
                  >
                    {flightChannels.map((channel) => (
                      <tr key={channel.name}>
                        <td>{channel.name}</td>
                        <td>
                          <Badge tone={channel.usable ? "neutral" : "warning"}>
                            {channel.role_label}
                          </Badge>
                        </td>
                        <td>
                          {channel.unit}
                          {!channel.unit_known && <span className="muted"> (unknown)</span>}
                        </td>
                        <td className="num">{formatNumber(channel.min, 4)}</td>
                        <td className="num">{formatNumber(channel.max, 4)}</td>
                      </tr>
                    ))}
                  </DataTable>
                ) : (
                  <Hint>Import a flight log to list its channels.</Hint>
                )}
                {flightSession && (
                  <>
                    <div className="divider" />
                    <KeyValue
                      pairs={[
                        ["Format", flightSession.source_format],
                        ["Timebase", flightSession.timebase_note],
                        ["Position available", flightSession.has_position ? "yes" : "no"],
                        ["Derived channels", flightSession.derived_channels.join(", ") || "none"],
                      ]}
                    />
                    <Hint>
                      Clicking a channel name plots it below. A series can be hidden from its legend.
                    </Hint>
                    <div className="pill-row">
                      {flightChannels.slice(0, 24).map((channel) => (
                        <Button
                          key={channel.name}
                          size="small"
                          variant="ghost"
                          onClick={() => void loadFlightSeries(channel.name)}
                        >
                          {channel.name}
                        </Button>
                      ))}
                    </div>
                  </>
                )}
              </>
            )}

            {tab === "comparison" && (
              <>
                <Select
                  label="Alignment method"
                  value={(comparisonRequest.method ?? "first_sample") as string}
                  onChange={(value) => setComparisonRequest({ method: value as AlignmentMethodCode })}
                  options={alignmentOptions}
                  hint="The method that produced a comparison is recorded with it, so a saved result is reproducible."
                />
                {comparisonRequest.method === "event" && (
                  <Select
                    label="Event"
                    value={comparisonRequest.event_code ?? "apogee"}
                    onChange={(value) => setComparisonRequest({ event_code: value })}
                    options={[
                      { value: "lift_off", label: "Lift-off" },
                      { value: "burnout", label: "Motor burnout" },
                      { value: "max_q", label: "Maximum dynamic pressure" },
                      { value: "apogee", label: "Apogee" },
                      { value: "ground_impact", label: "Ground impact" },
                    ]}
                  />
                )}
                {comparisonRequest.method === "manual" && (
                  <NumberInput
                    label="Manual offset"
                    unit="s"
                    value={comparisonRequest.offset_seconds ?? 0}
                    onChange={(v) => setComparisonRequest({ offset_seconds: v ?? 0 })}
                  />
                )}
                {comparisonRequest.method === "cross_correlation" && (
                  <div className="field-row">
                    <NumberInput
                      label="Search half width"
                      unit="s"
                      value={comparisonRequest.correlation_half_width ?? 1}
                      onChange={(v) => setComparisonRequest({ correlation_half_width: v ?? 1 })}
                    />
                    <NumberInput
                      label="Step"
                      unit="s"
                      value={comparisonRequest.correlation_step ?? 0.01}
                      onChange={(v) => setComparisonRequest({ correlation_step: v ?? 0.01 })}
                    />
                  </div>
                )}
                <NumberInput
                  label="Attitude warning threshold"
                  unit="deg"
                  value={comparisonRequest.attitude_warning_degrees ?? 10}
                  onChange={(v) => setComparisonRequest({ attitude_warning_degrees: v ?? 10 })}
                />
                <span className="meta">Channels to compare</span>
                <div className="pill-row">
                  {comparableChannels.map((channel) => {
                    const available = channel.in_simulation && channel.in_flight;
                    const selected =
                      comparisonRequest.channels.length === 0 || comparisonRequest.channels.includes(channel.code);
                    return (
                      <Checkbox
                        key={channel.code}
                        disabled={!available}
                        label={
                          <span>
                            {channel.label}
                            {!available && <span className="muted"> (not on both sides)</span>}
                          </span>
                        }
                        checked={selected && available}
                        onChange={(v) => {
                          const current =
                            comparisonRequest.channels.length === 0
                              ? comparableChannels.filter((c) => c.in_simulation && c.in_flight).map((c) => c.code)
                              : comparisonRequest.channels;
                          setComparisonRequest({
                            channels: v
                              ? [...current, channel.code]
                              : current.filter((c) => c !== channel.code),
                          });
                        }}
                      />
                    );
                  })}
                </div>
                <Button
                  variant="primary"
                  disabled={!runResult || !flightSession}
                  onClick={() => void buildComparison()}
                >
                  Build the comparison
                </Button>
                {!runResult && <Hint>Run a simulation first; a comparison needs both sides.</Hint>}
                {runResult && !flightSession && <Hint>Import a flight log first.</Hint>}
              </>
            )}
          </Panel>

          <Panel title="Saved flights" subtitle={`${flightSessions.length} in the project`}>
            {flightSessions.length === 0 ? (
              <Hint>No flight session has been saved in this project yet.</Hint>
            ) : (
              <DataTable
                headers={[
                  { label: "Flight" },
                  { label: "Samples", numeric: true },
                  { label: "" },
                ]}
              >
                {flightSessions.map((session) => (
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
                            void deleteFlightSession(session.name);
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

        <section className="col-8 stack">
          <div className="grid-12">
            <div className="col-7 card-slot tall">
              <Viewport
                data={replayData}
                index={index}
                options={{ ...defaultViewportOptions, showTrajectory: flightSession?.has_position ?? false }}
                camera={camera}
                onCameraChange={setCamera}
                attitudeLabel="GYRO INTEGRATION (DRIFTS)"
                attitudeNote={
                  flightSession?.has_position
                    ? "Position comes from the recorded GNSS channel."
                    : "This log has no measured position, so no trajectory is shown. Orientation only."
                }
                attitudePoor={!flightSession?.has_position}
                onAddMarker={() => {
                  const times = replayData?.times ?? [];
                  const current = times[Math.min(index, Math.max(times.length - 1, 0))];
                  setMarkerTime((current ?? 0).toFixed(3));
                  setShowMarker(true);
                }}
              />
            </div>
            <div className="col-5 stack">
              <Panel title="Events" subtitle="detected, and marked by hand">
                <Button size="small" onClick={() => void detectFlightEvents()} disabled={!flightSession}>
                  Detect events
                </Button>
                {flightTimeline && flightTimeline.events.length > 0 ? (
                  <DataTable
                    headers={[
                      { label: "Time", numeric: true },
                      { label: "Event" },
                      { label: "Basis" },
                    ]}
                  >
                    {flightTimeline.events.map((event) => (
                      <tr key={`${event.code}-${event.time}`}>
                        <td className="num">{event.time.toFixed(3)} s</td>
                        <td>
                          {event.label}
                          {event.user_added && (
                            <>
                              {" "}
                              <Badge tone="neutral">added by hand</Badge>
                            </>
                          )}
                        </td>
                        <td>
                          <Badge
                            tone={
                              event.evidence === "direct" || event.evidence === "manual"
                                ? "success"
                                : event.evidence === "integrated"
                                  ? "warning"
                                  : "neutral"
                            }
                          >
                            {event.user_added ? "your marker" : event.evidence.replace(/_/g, " ")}
                          </Badge>
                        </td>
                      </tr>
                    ))}
                  </DataTable>
                ) : (
                  <Hint>
                    An event is inferred from one channel, so it carries that channel's confidence. A
                    marker you place is your own observation and is labelled as such.
                  </Hint>
                )}
              </Panel>

              <Panel title="Data quality summary">
                {flightSession ? (
                  <KeyValue
                    pairs={[
                      ["Findings", `${flightSession.quality_flags.length}`],
                      ["Severity", flightSession.quality_severity],
                      ["Duration", formatSeconds(flightSession.duration_seconds)],
                      ["Rate", formatRate(flightSession.sample_rate_hz)],
                      ["Channels", `${flightSession.channels.length}`],
                    ]}
                  />
                ) : (
                  <Hint>Import a flight log to see its quality summary.</Hint>
                )}
              </Panel>
            </div>
          </div>

          <Panel title="Replay" subtitle={replayData?.source ?? "nothing loaded"}>
            {replayData && replayData.times.length > 0 ? (
              <>
                <Transport
                  times={replayData.times}
                  index={index}
                  onIndexChange={setIndex}
                  events={eventsFromRun(replayData.events)}
                />
                {!replayData.has_position && (
                  <Hint>
                    No trajectory is drawn: this log recorded no position, and the view will not fabricate
                    one.
                  </Hint>
                )}
              </>
            ) : (
              <EmptyState
                title="Nothing to replay"
                detail="Import a flight log, then load its replay data."
              />
            )}
          </Panel>

          <Panel title="Flight channels">
            {flightSession && flightChartSeries.length > 0 ? (
              <>
                <div className="card-slot short">
                  <Chart
                    title={flightSeries?.name ?? "Channel"}
                    series={flightChartSeries}
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
                <div className="row">
                  <Button
                    size="small"
                    variant="ghost"
                    onClick={() => {
                      const csv = seriesToCsv(flightChartSeries, view);
                      if (csv) downloadText(csv, "hexadof-flight-channels.csv");
                    }}
                  >
                    Export as CSV
                  </Button>
                  <span className="meta">
                    Use the channel list on the left to plot a different column.
                  </span>
                </div>
              </>
            ) : (
              <Hint>Import a flight log and pick a channel to plot it here.</Hint>
            )}
          </Panel>

          <Panel
            title="Comparison"
            subtitle={
              comparison
                ? `${comparison.simulation_name} against ${comparison.flight_name}`
                : "not built yet"
            }
          >
            {comparison ? (
              <>
                <div className="row wrap">
                  <Badge tone="neutral">{describeAlignment(comparison)}</Badge>
                  <Badge tone={comparison.warnings.some((w) => w.severe) ? "warning" : "success"}>
                    {comparison.warnings.length} warning(s)
                  </Badge>
                  <span className="spacer" />
                  <Button size="small" variant="ghost" onClick={() => setShowReport(true)}>
                    Export a report
                  </Button>
                </div>

                {comparison.warnings.length > 0 && (
                  <div className="issue-list">
                    {comparison.warnings.map((warning) => (
                      <article
                        className={`issue issue-${warning.severe ? "error" : "info"}`}
                        key={warning.code + warning.title}
                      >
                        <header className="issue-head">
                          <SeverityBadge severity={warning.severe ? "error" : "info"} />
                          <span className="issue-title">{warning.title}</span>
                        </header>
                        <p className="issue-detail">{warning.detail}</p>
                      </article>
                    ))}
                  </div>
                )}

                <DataTable
                  headers={[
                    { label: "Channel" },
                    { label: "Samples", numeric: true },
                    { label: "MAE", numeric: true },
                    { label: "RMSE", numeric: true },
                    { label: "Max", numeric: true },
                    { label: "Final", numeric: true },
                    { label: "Normalised RMSE", numeric: true },
                  ]}
                >
                  {comparison.metrics.rows
                    .filter((row) => row.scalar)
                    .map((row) => (
                      <tr key={row.channel}>
                        <td>{humanise(row.channel)}</td>
                        <td className="num">{row.scalar?.samples ?? 0}</td>
                        <td className="num">{formatNumber(row.scalar?.mean_absolute_error ?? null, 4)}</td>
                        <td className="num">{formatNumber(row.scalar?.root_mean_square_error ?? null, 4)}</td>
                        <td className="num">{formatNumber(row.scalar?.maximum_absolute_error ?? null, 4)}</td>
                        <td className="num">{formatNumber(row.scalar?.final_error ?? null, 4)}</td>
                        <td className="num">
                          {row.scalar?.normalized_rmse === null || row.scalar?.normalized_rmse === undefined
                            ? "--"
                            : `${(row.scalar.normalized_rmse * 100).toFixed(2)} %`}
                        </td>
                      </tr>
                    ))}
                  {comparison.metrics.rows
                    .filter((row) => row.attitude)
                    .map((row) => (
                      <tr key={`${row.channel}-attitude`}>
                        <td>{humanise(row.channel)}</td>
                        <td className="num">{row.attitude?.samples ?? 0}</td>
                        <td className="num" colSpan={3}>
                          Mean {row.attitude ? row.attitude.mean_angle_error.toFixed(4) : "--"} rad, max{" "}
                          {row.attitude ? row.attitude.maximum_angle_error.toFixed(4) : "--"} rad
                        </td>
                        <td className="num">
                          {row.attitude ? row.attitude.final_angle_error.toFixed(4) : "--"}
                        </td>
                        <td className="num">
                          {row.attitude
                            ? `${((row.attitude.root_mean_square_angle_error * 180) / Math.PI).toFixed(2)} deg RMS`
                            : "--"}
                        </td>
                      </tr>
                    ))}
                </DataTable>

                {comparison.event_differences.length > 0 && (
                  <>
                    <span className="meta">Event timing</span>
                    <DataTable
                      headers={[
                        { label: "Event" },
                        { label: "Simulation", numeric: true },
                        { label: "Flight", numeric: true },
                        { label: "Difference", numeric: true },
                        { label: "Evidence" },
                      ]}
                    >
                      {comparison.event_differences.map((difference) => (
                        <tr key={difference.code}>
                          <td>{difference.label}</td>
                          <td className="num">{difference.simulation_time.toFixed(3)} s</td>
                          <td className="num">{difference.flight_time.toFixed(3)} s</td>
                          <td className="num">{formatSigned(difference.difference, 4)} s</td>
                          <td>
                            <Badge
                              tone={difference.evidence === "direct" ? "success" : "warning"}
                            >
                              {difference.evidence.replace(/_/g, " ")}
                            </Badge>
                          </td>
                        </tr>
                      ))}
                    </DataTable>
                  </>
                )}

                {comparison.position_metrics && (
                  <KeyValue
                    pairs={[
                      ["Position RMSE", `${formatNumber(comparison.position_metrics.root_mean_square_error, 3)} m`],
                      ["Position max error", `${formatNumber(comparison.position_metrics.maximum_absolute_error, 3)} m`],
                      ["Position samples", `${comparison.position_metrics.samples}`],
                    ]}
                  />
                )}

                {comparison.metrics.unavailable.length > 0 && (
                  <Collapsible title="Channels that could not be compared">
                    <DataTable headers={[{ label: "Channel" }, { label: "Reason" }]}>
                      {comparison.metrics.unavailable.map(([channel, reason]) => (
                        <tr key={channel}>
                          <td>{channel}</td>
                          <td className="meta">{reason}</td>
                        </tr>
                      ))}
                    </DataTable>
                  </Collapsible>
                )}

                <Notice
                  level="info"
                  title="A mismatch does not identify a cause"
                  detail="Treat these numbers as places to investigate. HexaDOF never implies that a single difference is the root cause of a divergence."
                />
              </>
            ) : (
              <EmptyState
                icon={<IconCompare />}
                title="No comparison yet"
                detail="Load a simulation run and import a flight log, then choose an alignment method and build the comparison."
              />
            )}
          </Panel>
        </section>
      </div>

      {showReport && (
        <Modal
          title="Export a report"
          onClose={() => setShowReport(false)}
          footer={
            <>
              <Button
                variant="primary"
                disabled={reportTitle.trim().length === 0}
                onClick={() => {
                  void exportReport(reportTitle.trim());
                  setShowReport(false);
                }}
              >
                Export
              </Button>
              <Button variant="ghost" onClick={() => setShowReport(false)}>
                Cancel
              </Button>
            </>
          }
        >
          <Field label="Report title" hint="Used as the report heading and as its file name.">
            <TextInput value={reportTitle} onChange={(e) => setReportTitle(e.target.value)} />
          </Field>
        </Modal>
      )}

      {showMarker && (
        <Modal
          title="Add an event marker"
          onClose={() => setShowMarker(false)}
          footer={
            <>
              <Button
                variant="primary"
                disabled={markerLabel.trim().length === 0 || !Number.isFinite(Number(markerTime))}
                onClick={() => {
                  void addFlightMarker(Number(markerTime), markerLabel.trim());
                  setShowMarker(false);
                  setMarkerLabel("");
                }}
              >
                Add marker
              </Button>
              <Button variant="ghost" onClick={() => setShowMarker(false)}>
                Cancel
              </Button>
            </>
          }
        >
          <Field label="Marker label">
            <TextInput
              value={markerLabel}
              placeholder="Burnout"
              onChange={(e) => setMarkerLabel(e.target.value)}
            />
          </Field>
          <Field
            label="Time"
            hint={
              flightSession
                ? `Seconds on the corrected session axis, from 0 to ${flightSession.duration_seconds.toFixed(3)}.`
                : "Seconds on the corrected session axis."
            }
          >
            <TextInput
              type="number"
              step="0.001"
              value={markerTime}
              onChange={(e) => setMarkerTime(e.target.value)}
            />
          </Field>
          <Hint>
            A marker is your observation, not a measurement, so it is labelled as added by hand wherever
            it is shown. It is written beside the session when the log was imported into a project, and it
            never modifies the original file.
          </Hint>
        </Modal>
      )}

      <Collapsible title="Import pipeline" meta={<span className="meta">stage by stage</span>}>
        {flightSession ? (
          <KeyValue
            pairs={[
              ["Source", flightSession.source_file],
              ["Format", flightSession.source_format],
              ["Samples", `${flightSession.sample_count}`],
              ["Duration", formatSeconds(flightSession.duration_seconds)],
              ["Size on disk", formatBytes(0)],
            ]}
          />
        ) : (
          <Hint>
            Every import stage reports its own status: selected, parsed, channels discovered, mapped,
            units converted, frames converted, timestamps validated, calibrated, derived, session
            created. A failure names the stage that stopped.
          </Hint>
        )}
        {preview && (
          <Notice
            level="info"
            title="Inferred column roles"
            detail={`${preview.inferred_physical.filter(Boolean).length} of ${preview.headers.length} columns carry a physical quantity.`}
          />
        )}
      </Collapsible>
    </div>
  );
}

/** Build chart series from a single flight channel. */
function seriesFromFlight(
  channel: { name: string; unit: string; times: number[]; values: number[] },
  hidden: string[],
): ChartSeries[] {
  return [
    {
      name: channel.name,
      unit: channel.unit,
      times: channel.times,
      values: channel.values,
      visible: !hidden.includes(channel.name),
    },
  ];
}

/** A readable channel name. */
function humanise(code: string): string {
  return code.replace(/_/g, " ").replace(/^\w/, (c) => c.toUpperCase());
}

/** A readable description of the alignment a comparison used. */
function describeAlignment(comparison: { alignment: { offset_seconds: number; overlap_seconds: number; method: unknown } }): string {
  const method = comparison.alignment.method;
  const name =
    typeof method === "string"
      ? method
      : method && typeof method === "object" && "method" in method
        ? String((method as { method: string }).method)
        : "unknown";
  return `${humanise(name)} alignment, offset ${formatSigned(comparison.alignment.offset_seconds, 4)} s, overlap ${comparison.alignment.overlap_seconds.toFixed(3)} s`;
}
