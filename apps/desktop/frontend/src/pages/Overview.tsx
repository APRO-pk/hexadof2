/**
 * The Overview screen.
 *
 * It answers the questions a user has when they open the application: what project
 * is open, what the last run did, whether a device is connected, which flight logs
 * exist, whether anything is wrong, and what to do next.
 */

import { useEffect, useMemo } from "react";
import {
  Badge,
  Button,
  DataTable,
  EmptyState,
  Hint,
  Notice,
  Panel,
  ProgressBar,
  SeverityBadge,
} from "../components/ui";
import {
  IconAnalysis,
  IconCheckCircle,
  IconDynamics,
  IconImport,
  IconPlug,
  IconPlus,
  IconProjects,
  IconRocket,
  IconTelemetry,
  IconWarning,
} from "../components/Icons";
import { useStore } from "../lib/store";
import { formatRelative, formatSeconds, formatTimestamp } from "../lib/format";
import { pickDirectory, pickFile } from "../lib/api";
import type { ValidationIssue } from "../lib/types";

export function OverviewPage() {
  const project = useStore((s) => s.project);
  const models = useStore((s) => s.models);
  const activeModel = useStore((s) => s.activeModel);
  const runResult = useStore((s) => s.runResult);
  const telemetryStatus = useStore((s) => s.telemetryStatus);
  const flightSession = useStore((s) => s.flightSession);
  const flightSessions = useStore((s) => s.flightSessions);
  const savedRuns = useStore((s) => s.savedRuns);
  const telemetrySessions = useStore((s) => s.telemetrySessions);
  const comparison = useStore((s) => s.comparison);
  const running = useStore((s) => s.running);
  const progress = useStore((s) => s.progress);
  const setSection = useStore((s) => s.setSection);
  const runSimulation = useStore((s) => s.runSimulation);
  const importModelFromPath = useStore((s) => s.importModelFromPath);
  const importExampleModel = useStore((s) => s.importExampleModel);
  const openProject = useStore((s) => s.openProject);
  const importFlight = useStore((s) => s.importFlight);
  const loadSavedRuns = useStore((s) => s.loadSavedRuns);
  const loadFlightSessions = useStore((s) => s.loadFlightSessions);
  const loadTelemetrySessions = useStore((s) => s.loadTelemetrySessions);
  const loadRecentProjects = useStore((s) => s.loadRecentProjects);
  const refreshProject = useStore((s) => s.refreshProject);
  const loadModels = useStore((s) => s.loadModels);
  const recentProjects = useStore((s) => s.recentProjects);

  useEffect(() => {
    void refreshProject();
    void loadSavedRuns();
    void loadFlightSessions();
    void loadTelemetrySessions();
    void loadModels();
    void loadRecentProjects();
  }, [
    refreshProject,
    loadSavedRuns,
    loadFlightSessions,
    loadTelemetrySessions,
    loadModels,
    loadRecentProjects,
  ]);

  /** Everything that needs attention, gathered from every workspace. */
  const warnings = useMemo(() => {
    const issues: ValidationIssue[] = [];
    if (activeModel) {
      for (const issue of activeModel.validation.issues) {
        if (issue.severity !== "info") issues.push(issue);
      }
    } else if (project) {
      issues.push({
        code: "model.none_imported",
        title: "No dynamics model is imported",
        detail:
          "A simulation needs mass, centre of gravity, and an inertia tensor. Import a model, or use the built-in example to explore the workspace.",
        severity: "warning",
        suggestion: "Import a model on the Dynamics screen, or import the built-in example.",
      });
    }
    if (telemetryStatus && telemetryStatus.crc_failures > 0) {
      issues.push({
        code: "telemetry.crc_failures",
        title: `${telemetryStatus.crc_failures} checksum failure(s) on the serial stream`,
        detail:
          "Frames were discarded because their checksum did not match. The baud rate or the packet format is probably wrong.",
        severity: "warning",
        suggestion: "Check the baud rate and the packet format in the device profile.",
      });
    }
    if (telemetryStatus && telemetryStatus.packets_rejected > 0) {
      issues.push({
        code: "telemetry.rejections",
        title: `${telemetryStatus.packets_rejected} packet(s) were rejected`,
        detail: telemetryStatus.last_rejection ?? "A frame did not match the configured layout.",
        severity: "warning",
        suggestion: "Open the packet inspector on the Live Telemetry screen for the reason.",
      });
    }
    if (runResult) {
      for (const warning of runResult.warnings) {
        issues.push({
          code: warning.code,
          title: warning.title,
          detail: warning.detail,
          severity: warning.blocking ? "error" : "warning",
        });
      }
    }
    if (flightSession && flightSession.quality_flags.length > 0) {
      issues.push({
        code: "flight.quality",
        title: `${flightSession.quality_flags.length} data quality finding(s) in the flight log`,
        detail: flightSession.quality_flags[0],
        severity: "warning",
        suggestion: "Open the data quality panel on the Flight Analysis screen.",
      });
    }
    if (comparison) {
      for (const warning of comparison.warnings) {
        issues.push({
          code: warning.code,
          title: warning.title,
          detail: warning.detail,
          severity: warning.severe ? "warning" : "info",
        });
      }
    }
    return issues;
  }, [activeModel, project, telemetryStatus, runResult, flightSession, comparison]);

  const blocking = warnings.filter((w) => w.severity === "error");

  const quickImportModel = async () => {
    const path = await pickFile({
      title: "Choose a dynamics model",
      filters: [{ name: "Dynamics model", extensions: ["json"] }],
    });
    if (path) await importModelFromPath(path);
  };

  const quickOpenProject = async () => {
    const directory = await pickDirectory("Choose a HexaDOF project folder");
    if (directory) await openProject(directory);
  };

  const quickImportFlight = async () => {
    const path = await pickFile({
      title: "Choose a flight log",
      filters: [
        { name: "Flight log", extensions: ["csv", "hlog", "txt"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (path) await importFlight(path);
  };

  return (
    <div className="page">
      <div className="row-between">
        <div>
          <h1 className="page-title">{project ? project.name : "HexaDOF"}</h1>
          <p className="page-subtitle">
            {project
              ? project.description ||
                "A focused aerospace dynamics and flight-data laboratory for simulating, observing, replaying, and understanding six-degree-of-freedom motion."
              : "Import a dynamics model, configure and run a 6-DOF simulation, connect an Arduino or flight computer, replay the flight, and compare measured and simulated behaviour."}
          </p>
          {project && (
            <span className="meta">
              Last modified {formatRelative(project.updated_at)} ({formatTimestamp(project.updated_at)})
            </span>
          )}
        </div>
        <div className="row">
          <Button icon={<IconProjects />} onClick={() => (project ? setSection("dynamics") : void quickOpenProject())}>
            {project ? "Open workspace" : "Open a project"}
          </Button>
          <Button icon={<IconImport />} variant="primary" onClick={() => void quickImportModel()}>
            Import model
          </Button>
        </div>
      </div>

      {!project && (
        <Notice
          level="info"
          title="No project is open"
          detail="Open or create a project to store models, runs, sessions, and reports together."
          action={
            <Button size="small" onClick={() => setSection("projects")}>
              Go to Projects
            </Button>
          }
        />
      )}

      <div className="grid-12">
        <section className="status-card col-3">
          <span className="status-card-label">Model status</span>
          <span className="status-card-value truncate">
            {activeModel ? (
              <>
                <IconCheckCircle
                  style={{
                    width: 15,
                    height: 15,
                    color: activeModel.valid ? "var(--success)" : "var(--error)",
                  }}
                />
                {activeModel.name}
              </>
            ) : (
              <>
                <IconWarning style={{ width: 15, height: 15, color: "var(--warning)" }} />
                No model
              </>
            )}
          </span>
          <span className="meta">
            {activeModel
              ? `${activeModel.validation_summary} - ${activeModel.mass.toFixed(3)} kg`
              : "Import or use the built-in example"}
          </span>
          <Button size="small" variant="ghost" onClick={() => void importExampleModel()}>
            Import the example model
          </Button>
        </section>

        <section className="status-card col-3">
          <span className="status-card-label">Last simulation</span>
          <span className="status-card-value truncate">
            {runResult ? (
              <>
                <IconRocket style={{ width: 15, height: 15, color: "var(--accent)" }} />
                {runResult.summary.name}
              </>
            ) : (
              "No run in this session"
            )}
          </span>
          <span className="meta">
            {runResult
              ? `${runResult.summary.status}, ${runResult.summary.sample_count} samples, ${formatSeconds(
                  runResult.summary.duration,
                )}`
              : `${savedRuns.length} saved run(s) in the project`}
          </span>
          {running && progress && <ProgressBar fraction={progress.fraction} />}
          <Button
            size="small"
            variant="ghost"
            onClick={() => void runSimulation(false)}
            disabled={running}
          >
            {running ? "Running..." : "Run a simulation"}
          </Button>
        </section>

        <section className="status-card col-3">
          <span className="status-card-label">Device status</span>
          <span className="status-card-value truncate">
            <IconPlug
              style={{
                width: 15,
                height: 15,
                color: telemetryStatus?.connected ? "var(--success)" : "var(--text-muted)",
              }}
            />
            {telemetryStatus?.connected ? telemetryStatus.port : "No device connected"}
          </span>
          <span className="meta">
            {telemetryStatus?.connected
              ? `${telemetryStatus.baud_rate} baud, ${telemetryStatus.state}, ${telemetryStatus.packets_received} packets`
              : "Read-only telemetry. HexaDOF never sends commands to a device."}
          </span>
          {telemetryStatus?.connected && (
            <Badge tone={telemetryStatus.recording ? "error" : "neutral"}>
              {telemetryStatus.recording ? "Recording" : "Not recording"}
            </Badge>
          )}
          <Button size="small" variant="ghost" onClick={() => setSection("telemetry")}>
            Open Live Telemetry
          </Button>
        </section>

        <section className="status-card col-3">
          <span className="status-card-label">Data status</span>
          <span className="status-card-value truncate">
            <IconAnalysis style={{ width: 15, height: 15, color: "var(--accent)" }} />
            {flightSession
              ? flightSession.source_file
              : `${flightSessions.length} flight log(s), ${telemetrySessions.length} session(s)`}
          </span>
          <span className="meta">
            {flightSession
              ? `${flightSession.sample_count} samples at ${flightSession.sample_rate_hz.toFixed(2)} Hz`
              : "Import a CSV or HexaDOF binary log"}
          </span>
          <Button size="small" variant="ghost" onClick={() => void quickImportFlight()}>
            Import a flight log
          </Button>
        </section>

        <section className="col-8">
          <Panel
            title="Recent activity"
            subtitle={`${savedRuns.length} run(s), ${flightSessions.length} flight(s), ${telemetrySessions.length} telemetry session(s)`}
          >
            {savedRuns.length === 0 && flightSessions.length === 0 && telemetrySessions.length === 0 ? (
              <EmptyState
                title="Nothing recorded yet"
                detail="A run, a recorded session, or an imported flight log appears here with its status."
              />
            ) : (
              <DataTable
                headers={[
                  { label: "Artifact" },
                  { label: "Kind" },
                  { label: "Detail" },
                  { label: "Status" },
                ]}
              >
                {savedRuns.map((run) => (
                  <tr key={`run-${run.name}`}>
                    <td>{run.label}</td>
                    <td>Simulation run</td>
                    <td className="mono">
                      {run.samples} samples, {run.duration.toFixed(3)} s
                    </td>
                    <td>
                      <Badge tone={run.warnings > 0 ? "warning" : "success"}>{run.status}</Badge>
                    </td>
                  </tr>
                ))}
                {flightSessions.map((session) => (
                  <tr key={`flight-${session.name}`}>
                    <td>{session.label}</td>
                    <td>Flight log</td>
                    <td className="mono">
                      {session.samples} samples, {session.duration_seconds.toFixed(3)} s
                    </td>
                    <td>
                      <Badge tone={session.has_issues ? "warning" : "success"}>
                        {session.has_issues ? "Quality findings" : "Clean"}
                      </Badge>
                    </td>
                  </tr>
                ))}
                {telemetrySessions.map((session) => (
                  <tr key={`telemetry-${session.name}`}>
                    <td>{session.label}</td>
                    <td>Telemetry session</td>
                    <td className="mono">
                      {session.samples} samples, {session.duration_seconds.toFixed(3)} s
                    </td>
                    <td>
                      <Badge tone={session.has_issues ? "warning" : "success"}>
                        {session.has_issues ? "Integrity notes" : "Clean"}
                      </Badge>
                    </td>
                  </tr>
                ))}
              </DataTable>
            )}
          </Panel>

          <Panel title="Warnings and validation">
            {warnings.length === 0 ? (
              <Notice
                level="success"
                title="Nothing needs attention"
                detail="No validation problems were found across the model, the last run, the device, or the imported data."
              />
            ) : (
              <>
                <div className="row">
                  <Badge tone={blocking.length > 0 ? "error" : "warning"}>
                    {warnings.length} finding(s)
                  </Badge>
                  {blocking.length > 0 && <Badge tone="error">{blocking.length} blocking</Badge>}
                </div>
                <div className="issue-list">
                  {warnings.slice(0, 8).map((issue) => (
                    <article
                      key={`${issue.code}-${issue.title}`}
                      className={`issue issue-${issue.severity}`}
                    >
                      <header className="issue-head">
                        <SeverityBadge severity={issue.severity} />
                        <span className="issue-title">{issue.title}</span>
                        <code className="issue-code">{issue.code}</code>
                      </header>
                      <p className="issue-detail">{issue.detail}</p>
                      {issue.suggestion && <p className="issue-suggestion">{issue.suggestion}</p>}
                    </article>
                  ))}
                  {warnings.length > 8 && (
                    <span className="meta">{warnings.length - 8} more finding(s) not shown.</span>
                  )}
                </div>
              </>
            )}
            <Hint>
              A simulation result never certifies flight safety. HexaDOF is an analysis and
              instrumentation tool, and every warning here is meant to be read before a conclusion is
              drawn.
            </Hint>
          </Panel>
        </section>

        <section className="col-4">
          <Panel title="Quick actions">
            <Button icon={<IconPlus />} onClick={() => setSection("projects")}>
              New project
            </Button>
            <Button icon={<IconImport />} onClick={() => void quickImportModel()}>
              Import dynamics model
            </Button>
            <Button
              icon={<IconDynamics />}
              onClick={() => void runSimulation(false)}
              disabled={running}
            >
              {running ? "Running..." : "Run simulation"}
            </Button>
            <Button icon={<IconTelemetry />} onClick={() => setSection("telemetry")}>
              Connect device
            </Button>
            <Button icon={<IconAnalysis />} onClick={() => void quickImportFlight()}>
              Import flight log
            </Button>
            {models.length > 0 && (
              <>
                <div className="divider" />
                <span className="meta">Imported models</span>
                {models.map((m) => (
                  <div className="row-between" key={m.model_id + m.source_hash}>
                    <span className="truncate">{m.name}</span>
                    <Badge tone={m.valid ? "success" : "error"}>
                      {m.valid ? "valid" : `${m.error_count} error(s)`}
                    </Badge>
                  </div>
                ))}
              </>
            )}
          </Panel>

          {recentProjects.length > 0 && (
            <Panel title="Recent projects">
              <div className="col" style={{ gap: "var(--space-1)" }}>
                {recentProjects.slice(0, 6).map((path) => (
                  <button
                    key={path}
                    type="button"
                    className="palette-item"
                    onClick={() => void openProject(path)}
                    title={path}
                  >
                    <IconProjects />
                    <span className="truncate">{path}</span>
                  </button>
                ))}
              </div>
            </Panel>
          )}

          <Panel title="Where to start">
            <ol className="col" style={{ gap: "var(--space-2)", paddingLeft: "1.2em", margin: 0 }}>
              <li>Create a project and import a dynamics model.</li>
              <li>Read the validation report before running anything.</li>
              <li>Configure the scenario, validate it, then run it.</li>
              <li>Connect the device and validate the sensors before recording.</li>
              <li>Import the flight log and replay it.</li>
              <li>Align the run and the flight, then compare the channels.</li>
            </ol>
          </Panel>
        </section>
      </div>
    </div>
  );
}
