/**
 * The Settings screen.
 *
 * Every setting here maps to a field the backend stores, and the screen writes the
 * whole document in one call so a change cannot be half-applied. Values are
 * validated by the backend as well, because a rule enforced only in the interface
 * is not a rule.
 */

import { useEffect, useState } from "react";
import {
  Badge,
  Button,
  DataTable,
  Hint,
  NumberInput,
  Panel,
  Select,
  TextInput,
  Toggle,
} from "../components/ui";
import { IconReset, IconSave } from "../components/Icons";
import { useAdvancedMode, useStore } from "../lib/store";
import { diagnostics } from "../lib/api";
import { formatBytes, formatNumber } from "../lib/format";
import type { Density, ExportFormat, LogLevel, Settings, Theme } from "../lib/types";

export function SettingsPage() {
  const settings = useStore((s) => s.settings);
  const warnings = useStore((s) => s.settingsWarnings);
  const loadSettings = useStore((s) => s.loadSettings);
  const saveSettings = useStore((s) => s.saveSettings);
  const resetSettings = useStore((s) => s.resetSettings);
  const project = useStore((s) => s.project);
  const advanced = useAdvancedMode();

  const [draft, setDraft] = useState<Settings | null>(settings);
  const [lines, setLines] = useState<string[]>([]);
  const [path, setPath] = useState("");

  useEffect(() => {
    void loadSettings();
    void diagnostics.lines().then(setLines).catch(() => setLines([]));
    void import("../lib/api").then((api) => api.settings.path().then(setPath).catch(() => setPath("")));
  }, [loadSettings]);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  if (!draft) {
    return (
      <div className="page">
        <h1 className="page-title">Settings</h1>
        <p className="page-subtitle">Loading the settings file...</p>
      </div>
    );
  }

  /**
   * Apply a change to one section of the draft.
   *
   * Spread of a generic index is not expressible in TypeScript, so the merge is
   * done through a record cast and the section type is preserved by the caller's
   * `Partial<...>` argument.
   */
  const patch = <K extends keyof Settings>(section: K, value: Partial<Settings[K]>) => {
    const current = draft[section] as Record<string, unknown>;
    const merged = { ...current, ...(value as Record<string, unknown>) };
    setDraft({ ...draft, [section]: merged } as Settings);
  };

  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);

  return (
    <div className="page">
      <div className="row-between">
        <div>
          <h1 className="page-title">Settings</h1>
          <p className="page-subtitle">
            These preferences apply to every project. The project file itself stores what a specific run
            needs, so a run stays reproducible even if these change.
          </p>
        </div>
        <div className="row">
          <Button
            icon={<IconSave />}
            variant="primary"
            disabled={!dirty}
            onClick={() => void saveSettings(draft)}
          >
            {dirty ? "Save settings" : "Saved"}
          </Button>
          <Button
            icon={<IconReset />}
            onClick={() => {
              if (window.confirm("Restore every setting to its default?")) void resetSettings();
            }}
          >
            Restore defaults
          </Button>
        </div>
      </div>

      {warnings.length > 0 && (
        <div className="notice notice-warning" role="status">
          <div className="grow">
            <div className="notice-title">Settings notes</div>
            <ul style={{ margin: 0, paddingLeft: "1.1em" }}>
              {warnings.map((w) => (
                <li key={w} className="notice-detail">
                  {w}
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}

      <div className="grid-12">
        <section className="col-6 stack">
          <Panel title="General">
            <Toggle
              label="Advanced mode"
              checked={draft.general.advanced_mode}
              onChange={(v) => patch("general", { advanced_mode: v })}
              hint="Off is Simple mode: solver, environment, control moment, channel, hardware, sensor mapping, storage, and diagnostic controls are hidden. The same switch sits next to the HexaDOF name in the top bar."
            />
            <div className="divider" />
            <TextInput
              label="Default project directory"
              value={draft.general.default_project_directory ?? ""}
              placeholder="Leave empty to use the last folder"
              onChange={(e) => patch("general", { default_project_directory: e.target.value || null })}
            />
            <Toggle
              label="Autosave the project file"
              checked={draft.general.autosave}
              onChange={(v) => patch("general", { autosave: v })}
              hint="Writes the project metadata after a change, not the run results."
            />
            <NumberInput
              label="Autosave interval"
              unit="s"
              value={draft.general.autosave_interval_seconds}
              onChange={(v) => patch("general", { autosave_interval_seconds: v ?? 30 })}
            />
            <Toggle
              label="Confirm destructive actions"
              checked={draft.general.confirm_destructive_actions}
              onChange={(v) => patch("general", { confirm_destructive_actions: v })}
              hint="Deleting a saved run, session, or project file always asks first when this is on."
            />
            <Select
              label="Display units"
              value={draft.general.unit_system}
              onChange={(value) => patch("general", { unit_system: value })}
              options={[
                { value: "Si", label: "SI (m, kg, s, rad)" },
                { value: "Imperial", label: "Imperial (ft, slug, s, rad)" },
                { value: "MetricDegrees", label: "Metric with degrees" },
              ]}
            />
            <TextInput
              label="Language"
              value={draft.general.language}
              hint="This build ships one language; the value is stored for future builds."
              onChange={(e) => patch("general", { language: e.target.value })}
            />
            {draft.general.recent_projects.length > 0 && (
              <>
                <div className="divider" />
                <span className="meta">Recent projects</span>
                <DataTable headers={[{ label: "Folder" }]}>
                  {draft.general.recent_projects.map((p) => (
                    <tr key={p}>
                      <td className="truncate" title={p}>
                        {p}
                      </td>
                    </tr>
                  ))}
                </DataTable>
              </>
            )}
          </Panel>

          <Panel title="Appearance">
            <Select
              label="Theme"
              value={draft.appearance.theme}
              onChange={(value) => patch("appearance", { theme: value as Theme })}
              options={[
                { value: "system" as Theme, label: "System default" },
                { value: "dark" as Theme, label: "Dark" },
                { value: "light" as Theme, label: "Light" },
              ]}
            />
            <Select
              label="Density"
              value={draft.appearance.density}
              onChange={(value) => patch("appearance", { density: value as Density })}
              options={[
                { value: "comfortable" as Density, label: "Comfortable" },
                { value: "compact" as Density, label: "Compact" },
              ]}
            />
            <Toggle
              label="Reduce motion"
              checked={draft.appearance.reduced_motion}
              onChange={(v) => patch("appearance", { reduced_motion: v })}
              hint="Disables transitions and playback animation. The system preference is honoured as well."
            />
            <Toggle
              label="Draw chart grids"
              checked={draft.appearance.chart_grid}
              onChange={(v) => patch("appearance", { chart_grid: v })}
            />
            <Toggle
              label="Monospaced values only"
              checked={draft.appearance.monospace_numbers_only}
              onChange={(v) => patch("appearance", { monospace_numbers_only: v })}
              hint="Keeps the interface face for text and the monospaced face for every number."
            />
          </Panel>

          {advanced && (
          <Panel title="Storage">
            <TextInput
              label="Log directory"
              value={draft.storage.log_directory ?? ""}
              placeholder="Leave empty to store sessions inside the project"
              onChange={(e) => patch("storage", { log_directory: e.target.value || null })}
            />
            <TextInput
              label="Cache directory"
              value={draft.storage.cache_directory ?? ""}
              placeholder="Leave empty for the default"
              onChange={(e) => patch("storage", { cache_directory: e.target.value || null })}
            />
            <Select
              label="Default export format"
              value={draft.storage.export_format}
              onChange={(value) => patch("storage", { export_format: value as ExportFormat })}
              options={[
                { value: "csv" as ExportFormat, label: "CSV" },
                { value: "json" as ExportFormat, label: "JSON" },
                { value: "hlog" as ExportFormat, label: "HexaDOF binary log" },
                { value: "markdown" as ExportFormat, label: "Markdown report" },
              ]}
            />
            <Toggle
              label="Compress exports where the format allows it"
              checked={draft.storage.compress_exports}
              onChange={(v) => patch("storage", { compress_exports: v })}
            />
            <Toggle
              label="Keep a backup of the project file before saving"
              checked={draft.storage.backup_project_file}
              onChange={(v) => patch("storage", { backup_project_file: v })}
            />
            <Toggle
              label="Enable the retention policy"
              checked={draft.storage.retention_enabled}
              onChange={(v) => patch("storage", { retention_enabled: v })}
              hint="Off by default. Deleting recorded flight data without being asked is never acceptable."
            />
            <NumberInput
              label="Retention period"
              unit="days"
              value={draft.storage.retention_days}
              disabled={!draft.storage.retention_enabled}
              onChange={(v) => patch("storage", { retention_days: Math.max(0, Math.round(v ?? 365)) })}
            />
          </Panel>
          )}
        </section>

        {advanced && (
        <section className="col-6 stack">
          <Panel title="Dynamics defaults">
            <Select
              label="Default integrator"
              value={typeof draft.dynamics.solver === "object" && draft.dynamics.solver && "solver" in (draft.dynamics.solver as object)
                ? String((draft.dynamics.solver as { solver: string }).solver)
                : "rk4"}
              onChange={(value) =>
                patch("dynamics", {
                  solver:
                    value === "rk45"
                      ? {
                          solver: "rk45",
                          relative_tolerance: 1e-9,
                          absolute_tolerance: 1e-12,
                          maximum_step: 0.1,
                          minimum_step: 1e-9,
                          initial_step: 1e-4,
                        }
                      : { solver: "rk4", step: 0.0005 },
                })
              }
              options={[
                { value: "rk4", label: "Fixed-step RK4" },
                { value: "rk45", label: "Adaptive Dormand-Prince 5(4)" },
              ]}
              hint="RK4 is the transparent reference solver; its step is exactly what you type."
            />
            <NumberInput
              label="Default integration step"
              unit="s"
              value={extractStep(draft.dynamics.solver)}
              disabled={extractSolverName(draft.dynamics.solver) !== "rk4"}
              onChange={(v) => patch("dynamics", { solver: { solver: "rk4", step: v ?? 0.0005 } })}
            />
            <Select
              label="Default world frame"
              value={draft.dynamics.world_frame}
              onChange={(value) => patch("dynamics", { world_frame: value })}
              options={[
                { value: "Enu", label: "ENU (east, north, up)" },
                { value: "Ned", label: "NED (north, east, down)" },
              ]}
              hint="The choice changes the sign of the vertical axis everywhere, including the accelerometer reference."
            />
            <NumberInput
              label="Quaternion norm warning threshold"
              value={draft.dynamics.quaternion_norm_tolerance}
              onChange={(v) => patch("dynamics", { quaternion_norm_tolerance: v ?? 1e-6 })}
              hint="A run is flagged when the quaternion norm error exceeds this."
            />
            <NumberInput
              label="Angular rate warning"
              unit="rad/s"
              value={draft.dynamics.angular_rate_warning}
              onChange={(v) => patch("dynamics", { angular_rate_warning: v ?? 100 })}
            />
            <NumberInput
              label="Acceleration warning"
              unit="m/s²"
              value={draft.dynamics.acceleration_warning}
              onChange={(v) => patch("dynamics", { acceleration_warning: v ?? 1000 })}
            />
          </Panel>

          <Panel title="Telemetry defaults">
            <NumberInput
              label="Serial scan interval"
              unit="s"
              value={draft.telemetry.serial_scan_interval_seconds}
              onChange={(v) => patch("telemetry", { serial_scan_interval_seconds: v ?? 2 })}
            />
            <NumberInput
              label="Default baud rate"
              unit="baud"
              value={draft.telemetry.default_baud_rate}
              onChange={(v) => patch("telemetry", { default_baud_rate: Math.round(v ?? 115200) })}
            />
            <NumberInput
              label="Packet timeout"
              unit="s"
              value={draft.telemetry.packet_timeout_seconds}
              onChange={(v) => patch("telemetry", { packet_timeout_seconds: v ?? 2 })}
              hint="No packet within this window marks the connection stalled."
            />
            <NumberInput
              label="Maximum packet size"
              unit="bytes"
              value={draft.telemetry.maximum_packet_bytes}
              onChange={(v) => patch("telemetry", { maximum_packet_bytes: Math.round(v ?? 256) })}
            />
            <NumberInput
              label="Recording buffer"
              unit="samples"
              value={draft.telemetry.recording_buffer_samples}
              onChange={(v) => patch("telemetry", { recording_buffer_samples: Math.round(v ?? 5000000) })}
              hint={`About ${formatBytes(draft.telemetry.recording_buffer_samples * 8 * 16)} for a sixteen-channel device.`}
            />
            <NumberInput
              label="Display update rate"
              unit="Hz"
              value={draft.telemetry.display_rate_hz}
              onChange={(v) => patch("telemetry", { display_rate_hz: v ?? 30 })}
              hint="The display is throttled to this rate. The recording stays at full rate."
            />
            <Toggle
              label="Reconnect automatically after a drop"
              checked={draft.telemetry.auto_reconnect}
              onChange={(v) => patch("telemetry", { auto_reconnect: v })}
            />
            <Select
              label="Default orientation estimator"
              value={draft.telemetry.estimator_mode}
              onChange={(value) => patch("telemetry", { estimator_mode: value })}
              options={[
                { value: "GyroPropagation", label: "Gyro propagation (drifts)" },
                { value: "Complementary", label: "Complementary filter" },
                { value: "DeviceSupplied", label: "Device supplied" },
              ]}
              hint="No estimator here produces an absolute attitude. The label over the rocket always says which one is active."
            />
          </Panel>

          <Panel title="Diagnostics">
            <Select
              label="Log level"
              value={draft.diagnostics.log_level}
              onChange={(value) => patch("diagnostics", { log_level: value as LogLevel })}
              options={(["error", "warn", "info", "debug", "trace"] as LogLevel[]).map((level) => ({
                value: level,
                label: level.charAt(0).toUpperCase() + level.slice(1),
              }))}
            />
            <Toggle
              label="Enable developer tools"
              checked={draft.diagnostics.developer_tools}
              onChange={(v) => patch("diagnostics", { developer_tools: v })}
            />
            <Toggle
              label="Collect performance metrics"
              checked={draft.diagnostics.performance_metrics}
              onChange={(v) => patch("diagnostics", { performance_metrics: v })}
            />
            <div className="divider" />
            <span className="meta">Settings file</span>
            <span className="mono truncate" title={path}>
              {path || "unknown"}
            </span>
            <Hint>
              Delete the settings file to fall back to defaults. A damaged settings file never prevents
              the application from starting.
            </Hint>
          </Panel>

          <Panel title="Environment" subtitle="Versions, paths, and capabilities">
            <div className="col" style={{ gap: "var(--space-1)" }}>
              {lines.map((line) => (
                <span key={line} className="mono" style={{ wordBreak: "break-word" }}>
                  {line}
                </span>
              ))}
            </div>
            <div className="divider" />
            <div className="row wrap">
              <Badge tone="neutral">
                Channels: {useStore.getState().plotChannels.length}
              </Badge>
              <Badge tone="neutral">Saved runs: {project?.inventory.simulations ?? 0}</Badge>
              <Badge tone="neutral">
                Models: {project?.inventory.models ?? 0}
              </Badge>
            </div>
            <Hint>
              Speed reference: {formatNumber(340.294, 3)} m/s is the standard sea-level speed of sound,
              used for the Mach readout.
            </Hint>
          </Panel>
        </section>
        )}

        {!advanced && (
          <section className="col-6 stack">
            <Panel title="Advanced settings are hidden">
              <Hint>
                Simple mode keeps the general and appearance preferences. Solver defaults, environment
                defaults, telemetry defaults, storage, and diagnostics are what Advanced adds back.
              </Hint>
              <Toggle
                label="Advanced mode"
                checked={false}
                onChange={(v) => patch("general", { advanced_mode: v })}
              />
            </Panel>
          </section>
        )}
      </div>
    </div>
  );
}

/** Extract the fixed step from a serialised solver kind. */
function extractStep(solver: unknown): number | null {
  if (solver && typeof solver === "object" && "step" in solver) {
    const value = (solver as { step?: number }).step;
    return typeof value === "number" ? value : null;
  }
  return null;
}

/** The solver family name from a serialised solver kind. */
function extractSolverName(solver: unknown): string {
  if (solver && typeof solver === "object" && "solver" in solver) {
    const value = (solver as { solver?: string }).solver;
    return typeof value === "string" ? value : "rk4";
  }
  return "rk4";
}
