/**
 * The replay transport and the flight timeline.
 *
 * Playback rate is decoupled from both the integration step and the output rate:
 * this scrubs recorded samples at a wall-clock speed the user picks. The rocket is
 * never the source of truth here, only a view of the recorded samples.
 *
 * The timeline carries three things beyond a ruler:
 *
 * * the flight phases, shaded between the events that delimit them, so the shape
 *   of the flight is readable at a glance,
 * * the altitude profile of the visible window, drawn from a recorded channel, so
 *   an event can be read against where the vehicle was, and
 * * a time window that the charts share, so zooming here zooms everywhere.
 *
 * Everything drawn is either a recorded sample or a time taken from a recorded
 * event. Nothing on this axis is interpolated into existence.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Select } from "./ui";
import {
  IconPause,
  IconPlay,
  IconRewind,
  IconStepBack,
  IconStepForward,
} from "./Icons";
import { formatSeconds } from "../lib/format";
import type { FlightEvent } from "../lib/types";

/** The playback rates the product specifies. */
export const PLAYBACK_RATES = [0.1, 0.25, 0.5, 1, 2, 5, 10];

export interface TimelineEvent {
  id: string;
  time: number;
  label: string;
  kind?: string;
  userAdded?: boolean;
}

/** A recorded channel drawn behind the ruler, so events sit against the state. */
export interface TimelineProfile {
  values: number[];
  label: string;
  unit: string;
}

/** The visible time window, shared with the charts when the caller supplies it. */
export interface TimelineWindow {
  start: number;
  end: number;
}

/** One shaded stretch of the flight, delimited by two recorded events. */
interface Phase {
  label: string;
  from: number;
  to: number;
  tone: string;
}

/**
 * The phases a run's own events describe.
 *
 * Only the phases whose delimiting events were actually detected are produced. A
 * run with no burnout event has no boundary to draw, and inventing one would be a
 * claim the data does not support.
 */
export function flightPhases(events: TimelineEvent[], start: number, end: number): Phase[] {
  const at = (kind: string) => events.find((event) => event.kind === kind)?.time;
  const ignition = at("ignition") ?? start;
  const burnout = at("burnout");
  const apogee = at("apogee");
  const impact = at("ground_impact") ?? end;

  const phases: Phase[] = [];
  if (burnout !== undefined && burnout > ignition) {
    phases.push({ label: "Boost", from: ignition, to: burnout, tone: "boost" });
  }
  const coastFrom = burnout ?? ignition;
  if (apogee !== undefined && apogee > coastFrom) {
    phases.push({ label: "Coast", from: coastFrom, to: apogee, tone: "coast" });
  }
  const descentFrom = apogee ?? coastFrom;
  if (impact > descentFrom) {
    phases.push({ label: "Descent", from: descentFrom, to: impact, tone: "descent" });
  }
  return phases;
}

/** A round time step that divides the visible span into a readable number of ticks. */
export function tickStep(span: number, target: number): number {
  if (!(span > 0) || target < 1) return 1;
  const rough = span / target;
  const magnitude = Math.pow(10, Math.floor(Math.log10(rough)));
  for (const multiple of [1, 2, 2.5, 5, 10]) {
    const candidate = multiple * magnitude;
    if (candidate >= rough) return candidate;
  }
  return 10 * magnitude;
}

/** A timeline ruler with phases, an altitude profile, event markers, and a cursor. */
export function Timeline({
  times,
  events,
  index,
  onIndexChange,
  profile,
  window: timeWindow,
  onWindowChange,
}: {
  times: number[];
  events: TimelineEvent[];
  index: number;
  onIndexChange: (index: number) => void;
  profile?: TimelineProfile;
  window?: TimelineWindow;
  onWindowChange?: (window: TimelineWindow) => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [dragging, setDragging] = useState(false);
  const [brushing, setBrushing] = useState<{ from: number; to: number } | null>(null);
  const [hover, setHover] = useState<{ x: number; time: number } | null>(null);

  const first = times.length > 0 ? times[0] : 0;
  const last = times.length > 0 ? times[times.length - 1] : 1;
  const full: TimelineWindow = { start: first, end: last };

  // With no window supplied the whole history is shown, which is what every
  // caller that only wants a scrubber gets.
  const view = timeWindow && timeWindow.end > timeWindow.start ? timeWindow : full;
  const span = Math.max(1e-9, view.end - view.start);
  const zoomed = view.start > first + 1e-9 || view.end < last - 1e-9;

  const current = times.length > 0 ? times[Math.min(times.length - 1, Math.max(0, index))] : first;
  const fraction = (current - view.start) / span;

  const indexForTime = (time: number) => {
    if (times.length === 0) return 0;
    let lo = 0;
    let hi = times.length - 1;
    while (hi - lo > 1) {
      const mid = (lo + hi) >> 1;
      if (times[mid] <= time) lo = mid;
      else hi = mid;
    }
    return Math.abs(times[lo] - time) <= Math.abs(times[hi] - time) ? lo : hi;
  };

  const timeAt = (clientX: number) => {
    const element = ref.current;
    if (!element || times.length === 0) return view.start;
    const bounds = element.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (clientX - bounds.left) / bounds.width));
    return view.start + f * span;
  };

  useEffect(() => {
    if (!dragging) return;
    const move = (event: PointerEvent) => onIndexChange(indexForTime(timeAt(event.clientX)));
    const up = () => setDragging(false);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [dragging, view.start, view.end, times]);

  useEffect(() => {
    if (!brushing) return;
    const move = (event: PointerEvent) => {
      const element = ref.current;
      if (!element) return;
      const bounds = element.getBoundingClientRect();
      const f = Math.min(1, Math.max(0, (event.clientX - bounds.left) / bounds.width));
      setBrushing((current) =>
        current ? { ...current, to: view.start + f * span } : current,
      );
    };
    const up = () => {
      setBrushing((current) => {
        if (current && onWindowChange) {
          const from = Math.min(current.from, current.to);
          const to = Math.max(current.from, current.to);
          // A drag shorter than a twentieth of the window is a mis-click, not a
          // zoom, and would leave an unreadable span.
          if (to - from > span * 0.02) {
            onWindowChange({ start: from, end: to });
          }
        }
        return null;
      });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [brushing, view.start, span, onWindowChange]);

  const phases = useMemo(
    () => flightPhases(events, first, last),
    [events, first, last],
  );

  // The profile is drawn over the whole history and clipped by the window, so
  // zooming does not need the channel resampled.
  const profilePath = useMemo(() => {
    if (!profile || profile.values.length === 0 || times.length < 2) return null;
    const finite = profile.values.filter((value) => Number.isFinite(value));
    if (finite.length === 0) return null;
    const low = Math.min(...finite);
    const high = Math.max(...finite);
    const range = high - low > 1e-9 ? high - low : 1;
    const step = span / 240;
    const points: string[] = [];
    for (let i = 0; i <= 240; i += 1) {
      const time = view.start + i * step;
      const sample = indexForTime(time);
      const value = profile.values[sample];
      if (!Number.isFinite(value)) continue;
      const x = ((time - view.start) / span) * 100;
      const y = 100 - ((value - low) / range) * 88 - 6;
      points.push(`${x.toFixed(3)},${y.toFixed(3)}`);
    }
    if (points.length < 2) return null;
    return {
      line: `M ${points.join(" L ")}`,
      area: `M 0,100 L ${points.join(" L ")} L 100,100 Z`,
      low,
      high,
    };
  }, [profile, times, view.start, view.end, span, indexForTime]);

  const ticks = useMemo(() => {
    const step = tickStep(span, 8);
    const firstTick = Math.ceil(view.start / step) * step;
    const out: number[] = [];
    for (let t = firstTick; t <= view.end + 1e-9 && out.length < 40; t += step) {
      out.push(Number(t.toFixed(9)));
    }
    return out;
  }, [span, view.start, view.end]);

  const hoverValue =
    hover && profile ? profile.values[indexForTime(hover.time)] : undefined;

  return (
    <div className="timeline-block">
      <div className="timeline-tools">
        <span className="meta">
          {zoomed
            ? `Window ${formatSeconds(view.start)} to ${formatSeconds(view.end)}`
            : `Whole flight, ${formatSeconds(first)} to ${formatSeconds(last)}`}
        </span>
        {phases.length > 0 && (
          <span className="timeline-phases" aria-label="Flight phases">
            {phases.map((phase) => (
              <span key={phase.label} className={`phase-chip phase-${phase.tone}`}>
                {phase.label}
              </span>
            ))}
          </span>
        )}
        <span className="spacer" />
        {onWindowChange && zoomed && (
          <Button size="small" variant="ghost" onClick={() => onWindowChange(full)}>
            Fit
          </Button>
        )}
        {onWindowChange && (
          <Button
            size="small"
            variant="ghost"
            title="Drag across the profile to zoom into a time window. The charts follow."
            onClick={() => {
              const centre = (view.start + view.end) / 2;
              const half = Math.max(span / 4, 1e-3);
              onWindowChange({
                start: Math.max(first, centre - half),
                end: Math.min(last, centre + half),
              });
            }}
          >
            Zoom in
          </Button>
        )}
      </div>

      <div
        ref={ref}
        className="timeline"
        role="slider"
        aria-label="Playback position"
        aria-valuemin={0}
        aria-valuemax={Math.max(0, times.length - 1)}
        aria-valuenow={index}
        aria-valuetext={formatSeconds(current)}
        tabIndex={0}
        onPointerDown={(event) => {
          setDragging(true);
          onIndexChange(indexForTime(timeAt(event.clientX)));
        }}
        onPointerMove={(event) => {
          const element = ref.current;
          if (!element) return;
          const bounds = element.getBoundingClientRect();
          setHover({ x: event.clientX - bounds.left, time: timeAt(event.clientX) });
        }}
        onPointerLeave={() => setHover(null)}
        onKeyDown={(event) => {
          if (event.key === "ArrowRight") {
            event.preventDefault();
            onIndexChange(Math.min(times.length - 1, index + 1));
          } else if (event.key === "ArrowLeft") {
            event.preventDefault();
            onIndexChange(Math.max(0, index - 1));
          } else if (event.key === "Home") {
            onIndexChange(0);
          } else if (event.key === "End") {
            onIndexChange(Math.max(0, times.length - 1));
          }
        }}
      >
        <div className="timeline-track">
          {/* Phase bands first, so everything else is drawn over them. */}
          {phases.map((phase) => {
            const from = ((phase.from - view.start) / span) * 100;
            const to = ((phase.to - view.start) / span) * 100;
            const left = Math.max(0, Math.min(100, from));
            const right = Math.max(0, Math.min(100, to));
            if (right <= left) return null;
            return (
              <div
                key={phase.label}
                className={`timeline-phase phase-${phase.tone}`}
                style={{ left: `${left}%`, width: `${right - left}%` }}
                title={`${phase.label}: ${formatSeconds(phase.from)} to ${formatSeconds(phase.to)}`}
              />
            );
          })}

          {profilePath && (
            <svg
              className="timeline-profile"
              viewBox="0 0 100 100"
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              <path className="timeline-profile-area" d={profilePath.area} />
              <path className="timeline-profile-line" d={profilePath.line} />
            </svg>
          )}

          {profilePath && (
            <span className="timeline-profile-label">
              {profile?.label} {profilePath.low.toFixed(0)} to {profilePath.high.toFixed(0)}{" "}
              {profile?.unit}
            </span>
          )}

          {ticks.map((tick) => {
            const position = ((tick - view.start) / span) * 100;
            if (position < 0 || position > 100) return null;
            return (
              <span key={tick} className="timeline-tick" style={{ left: `${position}%` }}>
                <span className="timeline-tick-label">{formatSeconds(tick)}</span>
              </span>
            );
          })}

          <div className="timeline-progress" style={{ width: `${clamp(fraction) * 100}%` }} />

          {events.map((event, order) => {
            const position = ((event.time - view.start) / span) * 100;
            if (position < 0 || position > 100) return null;
            // A label at either edge would be clipped by the track, so the end
            // of the flight has its labels hung to the left of the marker.
            const labelStyle =
              position > 84
                ? { transform: "translateX(-100%)", textAlign: "right" as const }
                : position < 4
                  ? { transform: "translateX(5px)" }
                  : { transform: "translateX(3px)" };
            // Clicking a marker jumps the playback to the sample nearest it.
            return (
              <button
                key={event.id}
                type="button"
                className="timeline-event"
                style={{
                  left: `${position}%`,
                  background: event.userAdded ? "var(--warning)" : "var(--accent)",
                }}
                title={`${event.label} at ${formatSeconds(event.time)}. Jump to this event.`}
                aria-label={`Jump to ${event.label} at ${formatSeconds(event.time)}`}
                onPointerDown={(down) => down.stopPropagation()}
                onClick={(click) => {
                  click.stopPropagation();
                  onIndexChange(indexForTime(event.time));
                }}
              >
                <span
                  className={`timeline-event-label ${order % 2 === 0 ? "row-a" : "row-b"}`}
                  style={labelStyle}
                >
                  {event.label}
                </span>
              </button>
            );
          })}

          {brushing && (
            <div
              className="timeline-brush"
              style={{
                left: `${clamp(((Math.min(brushing.from, brushing.to) - view.start) / span) * 100) * 100}%`,
                width: `${
                  clamp(
                    ((Math.abs(brushing.to - brushing.from)) / span) * 100,
                  ) * 100
                }%`,
              }}
            />
          )}

          <div className="timeline-cursor" style={{ left: `${clamp(fraction) * 100}%` }} />

          {hover && (
            <div className="timeline-hover" style={{ left: `${hover.x}px` }}>
              <span className="mono">{formatSeconds(hover.time)}</span>
              {profile && Number.isFinite(hoverValue) && (
                <span className="mono">
                  {profile.label} {Number(hoverValue).toFixed(1)} {profile.unit}
                </span>
              )}
            </div>
          )}
        </div>
      </div>

      {/* The profile is also the brush surface, because dragging a time range is
          the natural gesture on a shape you can see. */}
      {onWindowChange && profilePath && (
        <div
          className="timeline-brush-surface"
          title="Drag to zoom into a time window"
          onPointerDown={(event) => {
            const element = event.currentTarget.getBoundingClientRect();
            const f = Math.min(1, Math.max(0, (event.clientX - element.left) / element.width));
            const time = view.start + f * span;
            setBrushing({ from: time, to: time });
          }}
        />
      )}

      <div className="timeline-legend">
        <span className="meta">
          Recorded samples only. {events.length} event(s) marked on this axis.
        </span>
      </div>
    </div>
  );
}

function clamp(value: number): number {
  return Math.min(1, Math.max(0, value));
}

/**
 * A transport bar driving an index into a recorded sample list.
 *
 * Playback advances by wall-clock time scaled by the chosen rate, and looks up the
 * sample nearest the resulting time. That is what keeps playback honest: no
 * sample is invented, and a rate above 1 simply skips samples.
 */
export function Transport({
  times,
  index,
  onIndexChange,
  events = [],
  extra,
  profile,
  window: timeWindow,
  onWindowChange,
}: {
  times: number[];
  index: number;
  onIndexChange: (index: number) => void;
  events?: TimelineEvent[];
  extra?: React.ReactNode;
  profile?: TimelineProfile;
  window?: TimelineWindow;
  onWindowChange?: (window: TimelineWindow) => void;
}) {
  const [playing, setPlaying] = useState(false);
  const [rate, setRate] = useState(1);
  const animationRef = useRef<number | null>(null);
  const lastRef = useRef<number | null>(null);
  const indexRef = useRef(index);
  indexRef.current = index;

  useEffect(() => {
    if (!playing || times.length < 2) return;
    const step = (timestamp: number) => {
      if (lastRef.current === null) lastRef.current = timestamp;
      const elapsed = (timestamp - lastRef.current) / 1000;
      lastRef.current = timestamp;
      const currentIndex = indexRef.current;
      const currentTime = times[currentIndex];
      const target = currentTime + elapsed * rate;
      if (target >= times[times.length - 1]) {
        onIndexChange(times.length - 1);
        setPlaying(false);
        return;
      }
      let next = currentIndex;
      while (next + 1 < times.length && times[next + 1] <= target) next += 1;
      if (next !== currentIndex) onIndexChange(next);
      animationRef.current = window.requestAnimationFrame(step);
    };
    animationRef.current = window.requestAnimationFrame(step);
    return () => {
      if (animationRef.current !== null) window.cancelAnimationFrame(animationRef.current);
      lastRef.current = null;
    };
  }, [playing, rate, times, onIndexChange]);

  const currentTime = times.length > 0 ? times[Math.min(times.length - 1, Math.max(0, index))] : 0;
  const duration = times.length > 0 ? times[times.length - 1] : 0;

  return (
    <div className="col" style={{ gap: "var(--space-2)" }}>
      <Timeline
        times={times}
        events={events}
        index={index}
        onIndexChange={onIndexChange}
        profile={profile}
        window={timeWindow}
        onWindowChange={onWindowChange}
      />
      <div className="transport">
        <Button
          size="small"
          icon={<IconRewind />}
          onClick={() => onIndexChange(0)}
          title="Jump to the start"
          aria-label="Jump to the start"
        />
        <Button
          size="small"
          icon={<IconStepBack />}
          onClick={() => onIndexChange(Math.max(0, index - 1))}
          title="Step back one sample"
          aria-label="Step back"
        />
        <Button
          size="small"
          variant="primary"
          icon={playing ? <IconPause /> : <IconPlay />}
          onClick={() => setPlaying((v) => !v)}
          disabled={times.length < 2}
          title={playing ? "Pause playback" : "Play the recording"}
        >
          {playing ? "Pause" : "Play"}
        </Button>
        <Button
          size="small"
          icon={<IconStepForward />}
          onClick={() => onIndexChange(Math.min(times.length - 1, index + 1))}
          title="Step forward one sample"
          aria-label="Step forward"
        />
        <span className="transport-time">
          {formatSeconds(currentTime)} / {formatSeconds(duration)}
        </span>
        <div style={{ width: 120 }}>
          <Select
            value={String(rate)}
            onChange={(value) => setRate(Number(value))}
            options={PLAYBACK_RATES.map((r) => ({ value: String(r), label: `${r}x` }))}
          />
        </div>
        {times.length === 0 && <Badge tone="neutral">Nothing to play</Badge>}
        <span className="spacer" />
        {extra}
      </div>
      <div className="meta">
        {times.length > 0
          ? `Sample ${index + 1} of ${times.length}. Playback reads recorded samples; it never changes the recording.`
          : "Load a run or a flight session to replay it."}
      </div>
    </div>
  );
}

/** Convenience: turn run events into timeline events. */
export function eventsFromRun(events: FlightEvent[]): TimelineEvent[] {
  return events.map((event) => ({
    id: event.id,
    time: event.time,
    label: event.label,
    kind: event.kind,
    userAdded: event.user_added,
  }));
}
