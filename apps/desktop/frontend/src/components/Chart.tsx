/**
 * The time-series chart.
 *
 * One canvas per chart, drawn imperatively. The plot is decimated to the pixel
 * width using a min/max envelope per column, which is what keeps a long recording
 * readable: drawing every sample of a 200 Hz log into 800 pixels would alias into
 * noise, and the envelope shows the true extremes instead.
 *
 * Charts share a time cursor and a visible time window through props, so several
 * charts stay aligned without any of them owning the others.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "./ui";
import { IconReset, IconZoomIn, IconZoomOut } from "./Icons";
import { cssVar, formatNumber, seriesColour } from "../lib/format";

export interface ChartSeries {
  /** Series name, shown in the legend and the readout. */
  name: string;
  /** Unit of the values, already converted for display. */
  unit: string;
  /** Times in seconds, increasing. */
  times: number[];
  /** Values aligned with `times`. */
  values: number[];
  /** Whether the series is drawn. */
  visible?: boolean;
  /** Override the palette colour. */
  colour?: string;
  /** Whether to draw this series on the right-hand axis. */
  onRightAxis?: boolean;
}

export interface ChartView {
  /** Visible time window. */
  start: number;
  end: number;
}

interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

const AXIS_HEIGHT = 22;
const AXIS_WIDTH = 62;
const RIGHT_AXIS_WIDTH = 52;
const TOP_PAD = 10;

/** Statistics for one series, over the visible window. */
export interface SeriesStats {
  name: string;
  unit: string;
  min: number;
  max: number;
  mean: number;
  last: number;
  count: number;
}

/** Compute statistics over a time window. */
export function computeStats(series: ChartSeries, view: ChartView): SeriesStats | null {
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  let sum = 0;
  let count = 0;
  let last = Number.NaN;
  for (let i = 0; i < series.times.length; i += 1) {
    const t = series.times[i];
    if (t < view.start || t > view.end) continue;
    const v = series.values[i];
    if (!Number.isFinite(v)) continue;
    if (v < min) min = v;
    if (v > max) max = v;
    sum += v;
    count += 1;
    last = v;
  }
  if (count === 0) return null;
  return { name: series.name, unit: series.unit, min, max, mean: sum / count, last, count };
}

/** Interpolated value of a series at a time, or null outside its range. */
export function valueAt(series: ChartSeries, time: number): number | null {
  const n = series.times.length;
  if (n === 0 || time < series.times[0] || time > series.times[n - 1]) return null;
  let lo = 0;
  let hi = n - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (series.times[mid] <= time) lo = mid;
    else hi = mid;
  }
  const t0 = series.times[lo];
  const t1 = series.times[hi];
  if (t1 === t0) return series.values[lo];
  const f = (time - t0) / (t1 - t0);
  const a = series.values[lo];
  const b = series.values[hi];
  if (!Number.isFinite(a) || !Number.isFinite(b)) return null;
  return a + (b - a) * f;
}

/** A "nice" axis step for a value range. */
function niceStep(range: number, targetTicks: number): number {
  if (!(range > 0) || !Number.isFinite(range)) return 1;
  const rough = range / Math.max(1, targetTicks);
  const magnitude = Math.pow(10, Math.floor(Math.log10(rough)));
  const normalised = rough / magnitude;
  const step = normalised >= 5 ? 5 : normalised >= 2 ? 2 : 1;
  return step * magnitude;
}

/** Format a time for the x axis. */
function formatTime(value: number): string {
  if (Math.abs(value) >= 60) {
    const minutes = Math.floor(value / 60);
    const seconds = value % 60;
    return `${minutes}:${seconds.toFixed(1).padStart(4, "0")}`;
  }
  if (Math.abs(value) < 1) return value.toFixed(2);
  return value.toFixed(1);
}

export function Chart({
  title,
  series,
  view,
  onViewChange,
  cursor,
  onCursorChange,
  height = 180,
  grid = true,
  allowRightAxis = false,
  onToggleSeries,
}: {
  title: string;
  series: ChartSeries[];
  view: ChartView;
  onViewChange: (view: ChartView) => void;
  cursor: number | null;
  onCursorChange: (time: number | null) => void;
  height?: number;
  grid?: boolean;
  allowRightAxis?: boolean;
  onToggleSeries?: (name: string) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ width: 640, height });
  const [dragging, setDragging] = useState<{ x: number; start: number; end: number } | null>(null);

  // Track the container size so the canvas matches the layout without a fixed
  // pixel size anywhere.
  useEffect(() => {
    const element = wrapRef.current;
    if (!element) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0].contentRect;
      setSize({ width: Math.max(120, rect.width), height: Math.max(80, rect.height) });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const visible = useMemo(() => series.filter((s) => s.visible !== false), [series]);

  const scale = useMemo(() => {
    let leftMin = Number.POSITIVE_INFINITY;
    let leftMax = Number.NEGATIVE_INFINITY;
    let rightMin = Number.POSITIVE_INFINITY;
    let rightMax = Number.NEGATIVE_INFINITY;
    for (const s of visible) {
      for (let i = 0; i < s.times.length; i += 1) {
        const t = s.times[i];
        if (t < view.start || t > view.end) continue;
        const v = s.values[i];
        if (!Number.isFinite(v)) continue;
        if (s.onRightAxis && allowRightAxis) {
          if (v < rightMin) rightMin = v;
          if (v > rightMax) rightMax = v;
        } else {
          if (v < leftMin) leftMin = v;
          if (v > leftMax) leftMax = v;
        }
      }
    }
    if (!Number.isFinite(leftMin)) {
      leftMin = 0;
      leftMax = 1;
    }
    if (!Number.isFinite(rightMin)) {
      rightMin = 0;
      rightMax = 1;
    }
    if (leftMin === leftMax) {
      leftMin -= 1;
      leftMax += 1;
    }
    if (rightMin === rightMax) {
      rightMin -= 1;
      rightMax += 1;
    }
    const pad = (max: number, min: number) => {
      const span = max - min;
      return span * 0.08;
    };
    leftMin -= pad(leftMax, leftMin);
    leftMax += pad(leftMax, leftMin);
    rightMin -= pad(rightMax, rightMin);
    rightMax += pad(rightMax, rightMin);
    return { leftMin, leftMax, rightMin, rightMax };
  }, [visible, view, allowRightAxis]);

  const rect = useMemo<Rect>(
    () => ({
      left: AXIS_WIDTH,
      top: TOP_PAD,
      right: size.width - (allowRightAxis ? RIGHT_AXIS_WIDTH : 14),
      bottom: size.height - AXIS_HEIGHT,
    }),
    [size, allowRightAxis],
  );

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ratio = window.devicePixelRatio || 1;
    canvas.width = Math.round(size.width * ratio);
    canvas.height = Math.round(size.height * ratio);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.clearRect(0, 0, size.width, size.height);

    const surface = cssVar("--surface-panel", "#161b22");
    const border = cssVar("--border-subtle", "#232b35");
    const textMuted = cssVar("--text-muted", "#6f7d8c");
    const textSecondary = cssVar("--text-secondary", "#9aa7b6");
    const cursorColour = cssVar("--text-primary", "#e8eef5");

    const plotWidth = Math.max(1, rect.right - rect.left);
    const plotHeight = Math.max(1, rect.bottom - rect.top);
    const span = Math.max(1e-9, view.end - view.start);

    ctx.fillStyle = surface;
    ctx.fillRect(0, 0, size.width, size.height);

    const toX = (t: number) => rect.left + ((t - view.start) / span) * plotWidth;
    const toYLeft = (v: number) =>
      rect.bottom - ((v - scale.leftMin) / (scale.leftMax - scale.leftMin)) * plotHeight;
    const toYRight = (v: number) =>
      rect.bottom - ((v - scale.rightMin) / (scale.rightMax - scale.rightMin)) * plotHeight;

    // Grid and axes.
    ctx.font = "11px var(--font-mono, monospace)";
    ctx.textBaseline = "middle";
    const xStep = niceStep(span, Math.max(2, Math.floor(plotWidth / 90)));
    const xStart = Math.ceil(view.start / xStep) * xStep;
    ctx.strokeStyle = border;
    ctx.lineWidth = 1;
    ctx.fillStyle = textMuted;
    ctx.textAlign = "center";
    for (let t = xStart; t <= view.end; t += xStep) {
      const x = Math.round(toX(t)) + 0.5;
      if (grid) {
        ctx.beginPath();
        ctx.moveTo(x, rect.top);
        ctx.lineTo(x, rect.bottom);
        ctx.stroke();
      }
      ctx.fillText(formatTime(t), x, size.height - AXIS_HEIGHT / 2 - 1);
    }

    const yRange = scale.leftMax - scale.leftMin;
    const yStep = niceStep(yRange, Math.max(2, Math.floor(plotHeight / 34)));
    const yStart = Math.ceil(scale.leftMin / yStep) * yStep;
    ctx.textAlign = "right";
    for (let v = yStart; v <= scale.leftMax; v += yStep) {
      const y = Math.round(toYLeft(v)) + 0.5;
      if (grid) {
        ctx.beginPath();
        ctx.moveTo(rect.left, y);
        ctx.lineTo(rect.right, y);
        ctx.stroke();
      }
      ctx.fillStyle = textMuted;
      ctx.fillText(formatNumber(v, 3), rect.left - 6, y);
    }

    if (allowRightAxis && (scale.rightMax !== scale.leftMax || scale.rightMin !== scale.leftMin)) {
      const rStep = niceStep(scale.rightMax - scale.rightMin, Math.max(2, Math.floor(plotHeight / 34)));
      const rStart = Math.ceil(scale.rightMin / rStep) * rStep;
      ctx.textAlign = "left";
      ctx.fillStyle = textMuted;
      for (let v = rStart; v <= scale.rightMax; v += rStep) {
        const y = Math.round(toYRight(v)) + 0.5;
        ctx.fillText(formatNumber(v, 3), rect.right + 6, y);
      }
    }

    // Axis frame.
    ctx.strokeStyle = border;
    ctx.strokeRect(rect.left + 0.5, rect.top + 0.5, plotWidth, plotHeight);

    // Series, drawn with a per-column min/max envelope so a dense log shows its
    // true extremes instead of aliasing.
    ctx.save();
    ctx.beginPath();
    ctx.rect(rect.left, rect.top, plotWidth, plotHeight);
    ctx.clip();
    visible.forEach((s, index) => {
      const resolved = s.colour ?? colourFromToken(seriesColour(index));
      const toY = s.onRightAxis && allowRightAxis ? toYRight : toYLeft;

      // Build the column envelopes.
      const columns = Math.max(1, Math.floor(plotWidth));
      const minPer = new Float64Array(columns).fill(Number.POSITIVE_INFINITY);
      const maxPer = new Float64Array(columns).fill(Number.NEGATIVE_INFINITY);
      const lastPer = new Float64Array(columns).fill(Number.NaN);
      let any = false;
      for (let i = 0; i < s.times.length; i += 1) {
        const t = s.times[i];
        if (t < view.start || t > view.end) continue;
        const v = s.values[i];
        if (!Number.isFinite(v)) continue;
        const column = Math.min(columns - 1, Math.max(0, Math.floor(((t - view.start) / span) * columns)));
        if (v < minPer[column]) minPer[column] = v;
        if (v > maxPer[column]) maxPer[column] = v;
        lastPer[column] = v;
        any = true;
      }
      if (!any) return;

      ctx.strokeStyle = resolved;
      ctx.fillStyle = resolved;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      let started = false;
      const points: [number, number][] = [];
      for (let c = 0; c < columns; c += 1) {
        if (!Number.isFinite(lastPer[c])) continue;
        const x = rect.left + c + 0.5;
        const high = toY(maxPer[c]);
        const low = toY(minPer[c]);
        points.push([x, (high + low) / 2]);
        if (!started) {
          ctx.moveTo(x, high);
          started = true;
        } else {
          ctx.lineTo(x, high);
        }
        if (Math.abs(high - low) > 0.7) {
          ctx.lineTo(x, low);
        }
      }
      ctx.stroke();

      // A marker where the series is a single or very sparse trace, so a short
      // recording is visible rather than an empty-looking line.
      if (points.length < 3) {
        for (const [x, y] of points) {
          ctx.beginPath();
          ctx.arc(x, y, 2.2, 0, Math.PI * 2);
          ctx.fill();
        }
      }
    });
    ctx.restore();

    // Cursor.
    if (cursor !== null && cursor >= view.start && cursor <= view.end) {
      const x = Math.round(toX(cursor)) + 0.5;
      ctx.strokeStyle = cursorColour;
      ctx.globalAlpha = 0.55;
      ctx.beginPath();
      ctx.moveTo(x, rect.top);
      ctx.lineTo(x, rect.bottom);
      ctx.stroke();
      ctx.globalAlpha = 1;
    }

    // Title hint for the pointer position, drawn by the parent as a readout.
    ctx.textAlign = "left";
    ctx.fillStyle = textSecondary;
    ctx.fillText(title, rect.left, 8);
  }, [size, rect, scale, view, visible, cursor, grid, allowRightAxis, title]);

  useEffect(() => {
    draw();
  }, [draw]);

  /** Convert a client x coordinate into a time. */
  const timeAt = useCallback(
    (clientX: number) => {
      const canvas = canvasRef.current;
      if (!canvas) return view.start;
      const bounds = canvas.getBoundingClientRect();
      const x = clientX - bounds.left;
      const plotWidth = Math.max(1, rect.right - rect.left);
      const fraction = Math.min(1, Math.max(0, (x - rect.left) / plotWidth));
      return view.start + fraction * (view.end - view.start);
    },
    [rect, view],
  );

  const handleWheel = (event: React.WheelEvent<HTMLCanvasElement>) => {
    event.preventDefault();
    const anchor = timeAt(event.clientX);
    const factor = event.deltaY > 0 ? 1.18 : 1 / 1.18;
    const span = (view.end - view.start) * factor;
    const spanClamped = Math.min(1e7, Math.max(1e-5, span));
    const leftFraction = (anchor - view.start) / Math.max(1e-9, view.end - view.start);
    const start = anchor - spanClamped * leftFraction;
    onViewChange({ start, end: start + spanClamped });
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const time = timeAt(event.clientX);
    if (dragging) {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const plotWidth = Math.max(1, rect.right - rect.left);
      const deltaPixels = event.clientX - dragging.x;
      const deltaTime = -(deltaPixels / plotWidth) * (dragging.end - dragging.start);
      onViewChange({ start: dragging.start + deltaTime, end: dragging.end + deltaTime });
    }
    onCursorChange(time);
  };

  const stats = useMemo(
    () => visible.map((s) => computeStats(s, view)).filter((s): s is SeriesStats => s !== null),
    [visible, view],
  );

  return (
    <div className="chart-panel" style={{ height: "100%" }}>
      <div className="chart-toolbar">
        <span className="chart-title">{title}</span>
        <span className="spacer" />
        <Button
          size="small"
          icon={<IconZoomIn />}
          onClick={() => {
            const mid = (view.start + view.end) / 2;
            const span = (view.end - view.start) * 0.6;
            onViewChange({ start: mid - span / 2, end: mid + span / 2 });
          }}
          title="Zoom in"
          aria-label="Zoom in"
        />
        <Button
          size="small"
          icon={<IconZoomOut />}
          onClick={() => {
            const mid = (view.start + view.end) / 2;
            const span = (view.end - view.start) * 1.7;
            onViewChange({ start: mid - span / 2, end: mid + span / 2 });
          }}
          title="Zoom out"
          aria-label="Zoom out"
        />
        <Button
          size="small"
          icon={<IconReset />}
          onClick={() => {
            const all = allTimes(series);
            if (all) onViewChange(all);
          }}
          title="Reset the time window to the whole recording"
          aria-label="Reset zoom"
        />
      </div>

      <div className="chart-canvas-wrap" ref={wrapRef}>
        <canvas
          ref={canvasRef}
          style={{ width: "100%", height: "100%" }}
          onWheel={handleWheel}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            setDragging({ x: event.clientX, start: view.start, end: view.end });
          }}
          onPointerUp={(event) => {
            event.currentTarget.releasePointerCapture(event.pointerId);
            setDragging(null);
          }}
          onPointerLeave={() => {
            setDragging(null);
            onCursorChange(null);
          }}
          onPointerMove={handlePointerMove}
        />
        {visible.length === 0 && <div className="chart-empty">No series selected</div>}
        {cursor !== null && visible.length > 0 && (
          <div className="chart-readout">
            <div className="hud-row">
              <span className="hud-key">t</span>
              <span className="hud-value">{cursor.toFixed(3)} s</span>
            </div>
            {visible.slice(0, 6).map((s) => {
              const value = valueAt(s, cursor);
              const resolved = seriesColourFor(series, s.name);
              return (
                <div className="hud-row" key={s.name}>
                  <span className="hud-key" style={{ color: resolved }}>
                    {s.name}
                  </span>
                  <span className="hud-value">
                    {value === null ? "--" : `${formatNumber(value, 3)} ${s.unit}`}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <div className="chart-toolbar" style={{ borderTop: "1px solid var(--border-subtle)", borderBottom: "none" }}>
        <div className="series-list">
          {series.map((s) => (
            <button
              key={s.name}
              type="button"
              className="series-toggle"
              data-hidden={s.visible === false}
              onClick={() => onToggleSeries?.(s.name)}
              title={`${s.visible === false ? "Show" : "Hide"} ${s.name}`}
            >
              <span
                className="series-dot"
                style={{
                  background: seriesColourFor(series, s.name),
                }}
              />
              {s.name} ({s.unit})
            </button>
          ))}
        </div>
      </div>

      {stats.length > 0 && (
        <div className="chart-toolbar" style={{ borderTop: "1px solid var(--border-subtle)", borderBottom: "none" }}>
          <table className="table" style={{ width: "100%" }}>
            <thead>
              <tr>
                <th scope="col">Series</th>
                <th scope="col" className="num">
                  Min
                </th>
                <th scope="col" className="num">
                  Max
                </th>
                <th scope="col" className="num">
                  Mean
                </th>
                <th scope="col" className="num">
                  Last
                </th>
                <th scope="col" className="num">
                  Samples
                </th>
              </tr>
            </thead>
            <tbody>
              {stats.map((s) => (
                <tr key={s.name}>
                  <td>{s.name}</td>
                  <td className="num">{formatNumber(s.min, 3)}</td>
                  <td className="num">{formatNumber(s.max, 3)}</td>
                  <td className="num">{formatNumber(s.mean, 3)}</td>
                  <td className="num">{formatNumber(s.last, 3)}</td>
                  <td className="num">{s.count}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** The full time range of a set of series. */
export function allTimes(series: ChartSeries[]): ChartView | null {
  let start = Number.POSITIVE_INFINITY;
  let end = Number.NEGATIVE_INFINITY;
  for (const s of series) {
    if (s.times.length === 0) continue;
    start = Math.min(start, s.times[0]);
    end = Math.max(end, s.times[s.times.length - 1]);
  }
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return null;
  return { start, end };
}

/** Resolve a `var(--series-N)` token into a concrete colour for canvas drawing. */
function colourFromToken(token: string): string {
  // Canvas cannot resolve a CSS variable, so the token is looked up on the
  // document root, which is where the theme defines it.
  const name = token.startsWith("var(") ? token.slice(4, -1) : token;
  return readColour(name, "#38c7ff");
}

function readColour(name: string, fallback: string): string {
  return cssVar(name, fallback);
}

/** The concrete colour for a series index, for canvas drawing and for swatches. */
export function seriesColourFor(series: ChartSeries[], name: string): string {
  const index = series.findIndex((s) => s.name === name);
  const entry = index < 0 ? undefined : series[index];
  if (entry?.colour) return entry.colour;
  return colourFromToken(seriesColour(index < 0 ? 0 : index));
}

/** Export a chart's series as CSV text. */
export function seriesToCsv(series: ChartSeries[], view: ChartView | null): string {
  const visible = series.filter((s) => s.visible !== false);
  if (visible.length === 0) return "";
  const times = visible[0].times.filter((t) => !view || (t >= view.start && t <= view.end));
  const header = ["time_s", ...visible.map((s) => `${s.name}_${s.unit.replace(/[^A-Za-z0-9]/g, "")}`)];
  const lines = [header.join(",")];
  for (const t of times) {
    const row = [t.toFixed(6)];
    for (const s of visible) {
      const value = valueAt(s, t);
      row.push(value === null ? "" : value.toFixed(9));
    }
    lines.push(row.join(","));
  }
  return lines.join("\n");
}

/** Download a canvas as a PNG through an anchor element. */
export function exportCanvasPng(canvas: HTMLCanvasElement, fileName: string): void {
  const url = canvas.toDataURL("image/png");
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  anchor.click();
}

/** Trigger a text download. */
export function downloadText(text: string, fileName: string, mime = "text/csv"): void {
  const blob = new Blob([text], { type: mime });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = fileName;
  anchor.click();
  URL.revokeObjectURL(url);
}
