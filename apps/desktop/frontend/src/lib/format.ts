/**
 * Number, unit, and time formatting for display.
 *
 * Everything crossing this module converts from the SI value the backend stores
 * into the unit the user selected. Conversions happen only here, at the UI
 * boundary, which is what keeps the numerical core free of display concerns.
 */

/** The units the interface can display a quantity in. */
export interface UnitChoice {
  /** Label shown beside the value. */
  label: string;
  /** Multiply the SI value by this to reach the displayed unit. */
  factor: number;
  /** Offset added after scaling, for temperature. */
  offset?: number;
}

export const LENGTH_UNITS: UnitChoice[] = [
  { label: "m", factor: 1 },
  { label: "km", factor: 1 / 1000 },
  { label: "ft", factor: 1 / 0.3048 },
  { label: "mi", factor: 1 / 1609.344 },
];

export const VELOCITY_UNITS: UnitChoice[] = [
  { label: "m/s", factor: 1 },
  { label: "km/h", factor: 3.6 },
  { label: "ft/s", factor: 1 / 0.3048 },
  { label: "kt", factor: 1 / 0.5144444444444444 },
];

export const ACCELERATION_UNITS: UnitChoice[] = [
  { label: "m/s²", factor: 1 },
  { label: "g", factor: 1 / 9.80665 },
  { label: "ft/s²", factor: 1 / 0.3048 },
];

export const ANGLE_UNITS: UnitChoice[] = [
  { label: "deg", factor: 180 / Math.PI },
  { label: "rad", factor: 1 },
];

export const PRESSURE_UNITS: UnitChoice[] = [
  { label: "Pa", factor: 1 },
  { label: "hPa", factor: 1 / 100 },
  { label: "kPa", factor: 1 / 1000 },
  { label: "bar", factor: 1 / 100000 },
  { label: "psi", factor: 1 / 6894.757293168 },
];

export const PRESSURE_ALTITUDE_UNITS: UnitChoice[] = LENGTH_UNITS;

/** Convert an SI value into the chosen display unit. */
export function convert(value: number, unit: UnitChoice): number {
  return value * unit.factor + (unit.offset ?? 0);
}

/** Find a unit by label, falling back to the first entry. */
export function unitByLabel(units: UnitChoice[], label: string): UnitChoice {
  return units.find((u) => u.label === label) ?? units[0];
}

/**
 * Format a number for a telemetry readout.
 *
 * A magnitude-aware precision is chosen so a small value keeps significant digits
 * and a large one does not fill the column with noise. Non-finite values render as
 * an explicit marker rather than as `NaN`, because a gap in data must look like a
 * gap.
 */
export function formatNumber(value: number | null | undefined, precision = 3): string {
  if (value === null || value === undefined) return "--";
  if (Number.isNaN(value)) return "no data";
  if (!Number.isFinite(value)) return value > 0 ? "+inf" : "-inf";
  const magnitude = Math.abs(value);
  if (magnitude === 0) return (0).toFixed(precision > 2 ? 2 : precision);
  if (magnitude >= 100000) return value.toExponential(3);
  if (magnitude >= 1000) return value.toFixed(Math.max(1, precision - 2));
  if (magnitude >= 1) return value.toFixed(precision);
  if (magnitude >= 0.001) return value.toFixed(precision + 2);
  return value.toExponential(3);
}

/** Format a value with its unit, converting first. */
export function formatQuantity(
  value: number,
  unit: UnitChoice,
  precision = 3,
): string {
  return `${formatNumber(convert(value, unit), precision)} ${unit.label}`;
}

/** Format a duration as seconds with an appropriate number of decimals. */
export function formatSeconds(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return "--";
  if (Math.abs(value) >= 3600) {
    const hours = Math.floor(value / 3600);
    const minutes = Math.floor((value % 3600) / 60);
    const seconds = value % 60;
    return `${hours}:${String(minutes).padStart(2, "0")}:${seconds.toFixed(2).padStart(5, "0")}`;
  }
  if (Math.abs(value) >= 60) {
    const minutes = Math.floor(value / 60);
    const seconds = value % 60;
    return `${minutes}:${seconds.toFixed(3).padStart(6, "0")}`;
  }
  return `${value.toFixed(3)} s`;
}

/** Format a byte count. */
export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "--";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

/** Format a rate in hertz. */
export function formatRate(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return "--";
  if (value >= 1000) return `${(value / 1000).toFixed(2)} kHz`;
  return `${value.toFixed(value >= 100 ? 0 : 1)} Hz`;
}

/** Format an ISO 8601 timestamp into something readable, falling back to the raw text. */
export function formatTimestamp(value: string | null | undefined): string {
  if (!value) return "--";
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return value;
  return new Date(parsed).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** A relative description of how long ago a timestamp was. */
export function formatRelative(value: string | null | undefined): string {
  if (!value) return "--";
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return value;
  const seconds = (Date.now() - parsed) / 1000;
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  const days = Math.floor(seconds / 86400);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

/** Format a signed difference, always showing the sign. */
export function formatSigned(value: number, precision = 4): string {
  if (!Number.isFinite(value)) return "--";
  const formatted = formatNumber(Math.abs(value), precision);
  return `${value < 0 ? "-" : "+"}${formatted}`;
}

/** Wrap an angle in degrees into (-180, 180]. */
export function wrapDegrees(value: number): number {
  let v = ((value + 180) % 360 + 360) % 360 - 180;
  if (v <= -180) v += 360;
  return v;
}

/** Format a quaternion as four decimals, for the state inspector. */
export function formatQuaternion(q: [number, number, number, number]): string {
  return q.map((v) => v.toFixed(4)).join(", ");
}

/** Format a vector. */
export function formatVector(v: [number, number, number], unit?: UnitChoice): string {
  if (unit) {
    return v.map((c) => formatNumber(convert(c, unit), 3)).join(", ");
  }
  return v.map((c) => formatNumber(c, 3)).join(", ");
}

/**
 * The colour for a chart series index.
 *
 * The palette is defined once in the design tokens, so a chart, a legend swatch,
 * and a table row all agree about which colour a series is.
 */
export function seriesColour(index: number): string {
  return `var(--series-${(index % 8) + 1})`;
}

/** The resolved colour value for canvas drawing, read from the document. */
export function resolvedColour(name: string): string {
  if (typeof document === "undefined") return "#38c7ff";
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || "#38c7ff";
}

/**
 * Read a CSS variable as a string, for canvas drawing.
 *
 * Canvas cannot resolve a CSS custom property, so every colour and size it needs
 * has to be looked up on the document root first. This is the single place that
 * lookup happens, which is why a theme change is picked up by every canvas.
 */
export function cssVar(name: string, fallback: string): string {
  if (typeof document === "undefined") return fallback;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return raw || fallback;
}

/** Read a numeric CSS variable, with a fallback. */
export function cssNumber(name: string, fallback: number): number {
  if (typeof document === "undefined") return fallback;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const parsed = Number.parseFloat(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

/** A short, stable identifier for a channel name. */
export function channelKey(name: string): string {
  return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_");
}
