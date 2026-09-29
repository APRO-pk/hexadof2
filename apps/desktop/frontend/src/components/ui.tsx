/**
 * Interface primitives.
 *
 * Every control here is a plain DOM element with a class, not a widget library, so
 * keyboard behaviour, focus order, and screen-reader semantics stay the browser's
 * responsibility rather than something reimplemented badly.
 *
 * Two rules are enforced by these components rather than left to callers:
 * a status is never conveyed by colour alone (each badge and notice carries an
 * icon and text), and a numeric input always shows its unit.
 */

import {
  useEffect,
  useId,
  useState,
  type ChangeEvent,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
} from "react";
import {
  IconCheckCircle,
  IconChevronRight,
  IconCopy,
  IconError,
  IconInfo,
  IconWarning,
} from "./Icons";
import type { Severity, ValidationIssue, ValidationReport } from "../lib/types";
import { formatNumber } from "../lib/format";

/* ------------------------------------------------------------------- layout */

export function Panel({
  title,
  subtitle,
  actions,
  children,
  footer,
  collapsible = false,
  defaultOpen = true,
  flush = false,
  scroll = false,
  className = "",
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  collapsible?: boolean;
  defaultOpen?: boolean;
  flush?: boolean;
  scroll?: boolean;
  className?: string;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const bodyClass = ["panel-body", flush ? "flush" : "", scroll ? "scroll" : ""]
    .filter(Boolean)
    .join(" ");
  return (
    <section className={`panel ${open ? "" : "collapsed"} ${className}`.trim()}>
      {(title || actions) && (
        <header className="panel-header">
          {collapsible ? (
            <button
              type="button"
              className="collapsible-head"
              data-open={open}
              aria-expanded={open}
              onClick={() => setOpen((v) => !v)}
              style={{ width: "auto" }}
            >
              <IconChevronRight />
              <h2 className="panel-title">{title}</h2>
            </button>
          ) : (
            <h2 className="panel-title" style={{ flex: "0 0 auto" }}>
              {title}
            </h2>
          )}
          {subtitle && <span className="meta truncate">{subtitle}</span>}
          <span className="spacer" />
          {actions}
        </header>
      )}
      <div className={bodyClass}>{children}</div>
      {footer && <footer className="panel-footer">{footer}</footer>}
    </section>
  );
}

export function EmptyState({
  icon,
  title,
  detail,
  action,
}: {
  icon?: ReactNode;
  title: string;
  detail: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      {icon}
      <div className="empty-title">{title}</div>
      <p className="empty-detail">{detail}</p>
      {action}
    </div>
  );
}

export function Collapsible({
  title,
  defaultOpen = false,
  children,
  meta,
}: {
  title: string;
  defaultOpen?: boolean;
  children: ReactNode;
  meta?: ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const id = useId();
  return (
    <div>
      <button
        type="button"
        className="collapsible-head"
        data-open={open}
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((v) => !v)}
      >
        <IconChevronRight />
        <span>{title}</span>
        <span className="spacer" />
        {meta}
      </button>
      {open && (
        <div id={id} style={{ paddingTop: "var(--space-2)" }}>
          {children}
        </div>
      )}
    </div>
  );
}

/* ------------------------------------------------------------------ controls */

export function Button({
  variant = "default",
  size = "default",
  icon,
  children,
  ...rest
}: {
  variant?: "default" | "primary" | "danger" | "ghost";
  size?: "default" | "small" | "icon";
  icon?: ReactNode;
  children?: ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  const classes = [
    "btn",
    variant === "primary" ? "btn-primary" : "",
    variant === "danger" ? "btn-danger" : "",
    variant === "ghost" ? "btn-ghost" : "",
    size === "small" ? "btn-sm" : "",
    size === "icon" ? "btn-icon" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <button type="button" className={classes} {...rest}>
      {icon}
      {children}
    </button>
  );
}

export function Field({
  label,
  hint,
  error,
  children,
  htmlFor,
}: {
  label: string;
  hint?: string;
  error?: string;
  children: ReactNode;
  htmlFor?: string;
}) {
  return (
    <div className="field">
      <label className="field-label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {hint && !error && <span className="field-help">{hint}</span>}
      {error && <span className="field-error">{error}</span>}
    </div>
  );
}

export function TextInput({
  label,
  hint,
  error,
  ...rest
}: { label?: string; hint?: string; error?: string } & InputHTMLAttributes<HTMLInputElement>) {
  const id = useId();
  const input = <input id={id} className="input" {...rest} />;
  if (!label) return input;
  return (
    <Field label={label} hint={hint} error={error} htmlFor={id}>
      {input}
    </Field>
  );
}

/**
 * A numeric input with a unit label.
 *
 * The value is kept as a string while editing so a partially typed number such as
 * `-` or `1.` is not destroyed by an eager parse. The parsed value is only
 * reported when it is finite.
 */
export function NumberInput({
  label,
  unit,
  value,
  onChange,
  step,
  min,
  max,
  hint,
  disabled,
}: {
  label?: string;
  unit?: string;
  value: number | null | undefined;
  onChange: (value: number | null) => void;
  step?: number;
  min?: number;
  max?: number;
  hint?: string;
  disabled?: boolean;
}) {
  const id = useId();
  const [text, setText] = useState<string>(value === null || value === undefined ? "" : String(value));
  const [focused, setFocused] = useState(false);

  const display = focused ? text : value === null || value === undefined ? "" : formatInput(value);

  const handle = (event: ChangeEvent<HTMLInputElement>) => {
    const next = event.target.value;
    setText(next);
    if (next.trim() === "") {
      onChange(null);
      return;
    }
    const parsed = Number(next);
    if (Number.isFinite(parsed)) onChange(parsed);
  };

  const control = (
    <div className="input-with-unit">
      <input
        id={id}
        className="input numeric"
        type="text"
        inputMode="decimal"
        value={display}
        step={step}
        min={min}
        max={max}
        disabled={disabled}
        onChange={handle}
        onFocus={() => {
          setFocused(true);
          setText(value === null || value === undefined ? "" : String(value));
        }}
        onBlur={() => setFocused(false)}
      />
      {unit && <span className="input-unit">{unit}</span>}
    </div>
  );

  if (!label) return control;
  return (
    <Field label={label} hint={hint} htmlFor={id}>
      {control}
    </Field>
  );
}

/** Trim trailing zeros so an input does not show 0.0005000000000000001. */
function formatInput(value: number): string {
  if (!Number.isFinite(value)) return "";
  const fixed = value.toFixed(6);
  return fixed.replace(/\.?0+$/, "") || "0";
}

export function Select<T extends string>({
  label,
  hint,
  value,
  onChange,
  options,
  disabled,
}: {
  label?: string;
  hint?: string;
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string }[];
  disabled?: boolean;
}) {
  const id = useId();
  const control = (
    <select
      id={id}
      className="select"
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value as T)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
  if (!label) return control;
  return (
    <Field label={label} hint={hint} htmlFor={id}>
      {control}
    </Field>
  );
}

export function Toggle({
  label,
  checked,
  onChange,
  hint,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  hint?: string;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="field">
      <button
        type="button"
        id={id}
        className="switch"
        data-on={checked}
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        style={{ background: "none", border: "none", padding: 0, font: "inherit", color: "inherit" }}
      >
        <span className="switch-track">
          <span className="switch-thumb" />
        </span>
        <span>{label}</span>
      </button>
      {hint && <span className="field-help">{hint}</span>}
    </div>
  );
}

export function Checkbox({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: ReactNode;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="checkbox">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

export function TextArea({
  label,
  value,
  onChange,
  rows = 6,
  placeholder,
}: {
  label?: string;
  value: string;
  onChange: (value: string) => void;
  rows?: number;
  placeholder?: string;
}) {
  const id = useId();
  const control = (
    <textarea
      id={id}
      className="textarea"
      rows={rows}
      value={value}
      placeholder={placeholder}
      onChange={(e) => onChange(e.target.value)}
    />
  );
  if (!label) return control;
  return (
    <Field label={label} htmlFor={id}>
      {control}
    </Field>
  );
}

export function SelectRaw({
  value,
  onChange,
  children,
  ...rest
}: SelectHTMLAttributes<HTMLSelectElement> & { children: ReactNode }) {
  return (
    <select className="select" value={value} onChange={onChange} {...rest}>
      {children}
    </select>
  );
}

/* ------------------------------------------------------------------- status */

export function Badge({
  tone = "neutral",
  icon,
  children,
}: {
  tone?: "neutral" | "accent" | "success" | "warning" | "error" | "info";
  icon?: ReactNode;
  children: ReactNode;
}) {
  return (
    <span className={`badge badge-${tone}`}>
      {icon}
      {children}
    </span>
  );
}

/** A badge for a validation severity, with an icon so the meaning survives
 * greyscale. */
export function SeverityBadge({ severity }: { severity: Severity }) {
  if (severity === "error") {
    return (
      <Badge tone="error" icon={<IconError />}>
        Error
      </Badge>
    );
  }
  if (severity === "warning") {
    return (
      <Badge tone="warning" icon={<IconWarning />}>
        Warning
      </Badge>
    );
  }
  return (
    <Badge tone="info" icon={<IconInfo />}>
      Note
    </Badge>
  );
}

export function CheckStatusBadge({ status }: { status: string }) {
  const map: Record<string, { tone: "success" | "warning" | "error" | "neutral" | "info"; label: string }> = {
    pass: { tone: "success", label: "Pass" },
    warning: { tone: "warning", label: "Warning" },
    fail: { tone: "error", label: "Fail" },
    skipped: { tone: "neutral", label: "Skipped" },
    pending: { tone: "info", label: "Pending" },
  };
  const entry = map[status.toLowerCase()] ?? map.pending;
  return <Badge tone={entry.tone}>{entry.label}</Badge>;
}

export function Notice({
  level,
  title,
  detail,
  action,
}: {
  level: "info" | "warning" | "error" | "success";
  title: string;
  detail?: ReactNode;
  action?: ReactNode;
}) {
  const icon =
    level === "error" ? <IconError /> : level === "warning" ? <IconWarning /> : level === "success" ? <IconCheckCircle /> : <IconInfo />;
  return (
    <div className={`notice notice-${level}`} role={level === "error" ? "alert" : undefined}>
      {icon}
      <div className="grow">
        <div className="notice-title">{title}</div>
        {detail && <div className="notice-detail">{detail}</div>}
      </div>
      {action}
    </div>
  );
}

/* -------------------------------------------------------------- validation */

export function IssueCard({ issue }: { issue: ValidationIssue }) {
  const [open, setOpen] = useState(false);
  return (
    <article className={`issue issue-${issue.severity}`}>
      <header className="issue-head">
        <SeverityBadge severity={issue.severity} />
        <span className="issue-title">{issue.title}</span>
        <code className="issue-code">{issue.code}</code>
      </header>
      <p className="issue-detail">{issue.detail}</p>
      {issue.context && <p className="meta">At {issue.context}</p>}
      {issue.suggestion && <p className="issue-suggestion">{issue.suggestion}</p>}
      {issue.technical && (
        <>
          <button type="button" className="disclosure" onClick={() => setOpen((v) => !v)}>
            {open ? "Hide technical detail" : "Show technical detail"}
          </button>
          {open && <pre className="issue-technical">{issue.technical}</pre>}
        </>
      )}
    </article>
  );
}

/**
 * Render a validation report.
 *
 * Errors come first, then warnings, then notes, because a user reading a report
 * needs the blockers before the commentary.
 */
export function ValidationList({
  report,
  emptyMessage = "No issues were found.",
}: {
  report: ValidationReport;
  emptyMessage?: string;
}) {
  if (report.issues.length === 0) {
    return <Notice level="success" title="Valid" detail={emptyMessage} />;
  }
  const order: Record<Severity, number> = { error: 0, warning: 1, info: 2 };
  const sorted = [...report.issues].sort((a, b) => order[a.severity] - order[b.severity]);
  return (
    <div className="issue-list">
      {sorted.map((issue) => (
        <IssueCard key={`${issue.code}-${issue.title}-${issue.detail}`} issue={issue} />
      ))}
    </div>
  );
}

/* -------------------------------------------------------------------- table */

export function DataTable({
  headers,
  children,
  className = "",
}: {
  headers: { label: string; numeric?: boolean; center?: boolean }[];
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className="table-wrap">
      <table className={`table ${className}`.trim()}>
        <thead>
          <tr>
            {headers.map((h) => (
              <th
                key={h.label}
                className={h.numeric ? "num" : h.center ? "center" : undefined}
                scope="col"
              >
                {h.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  );
}

/* --------------------------------------------------------------------- misc */

export function KeyValue({ pairs }: { pairs: [string, ReactNode][] }) {
  return (
    <dl className="kv">
      {pairs.map(([key, value]) => (
        <div key={key} style={{ display: "contents" }}>
          <dt>{key}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function CopyButton({ value, label = "Copy" }: { value: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="copy-btn"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value);
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1400);
        } catch {
          setCopied(false);
        }
      }}
      title={`Copy ${label.toLowerCase()} to the clipboard`}
    >
      <IconCopy style={{ width: 11, height: 11, verticalAlign: "-1px" }} />{" "}
      {copied ? "Copied" : label}
    </button>
  );
}

export function Meter({ value, max, tone }: { value: number; max: number; tone?: string }) {
  const fraction = max > 0 ? Math.min(1, Math.max(0, value / max)) : 0;
  const className = tone ? `meter-fill ${tone}` : "meter-fill";
  return (
    <div className="meter" role="presentation">
      <div className={className} style={{ width: `${fraction * 100}%` }} />
    </div>
  );
}

export function ProgressBar({ fraction }: { fraction: number }) {
  const clamped = Number.isFinite(fraction) ? Math.min(1, Math.max(0, fraction)) : 0;
  return (
    <div
      className="progress-track"
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(clamped * 100)}
    >
      <div className="progress-fill" style={{ width: `${clamped * 100}%` }} />
    </div>
  );
}

export function Tabs<T extends string>({
  tabs,
  active,
  onChange,
}: {
  tabs: { id: T; label: string; badge?: ReactNode; title?: string }[];
  active: T;
  onChange: (id: T) => void;
}) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          className="tab"
          aria-selected={active === tab.id}
          title={tab.title ?? tab.label}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
          {tab.badge !== undefined && <> {tab.badge}</>}
        </button>
      ))}
    </div>
  );
}

export function Modal({
  title,
  onClose,
  children,
  footer,
  wide = false,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}) {
  // Escape closes the dialog, which is what a dialog is expected to do. The
  // click on the backdrop is handled below as well.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      className="overlay"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className={`modal ${wide ? "wide" : ""}`.trim()} role="dialog" aria-modal="true" aria-label={title}>
        <header className="modal-header">
          <h2 className="panel-title">{title}</h2>
          <span className="spacer" />
          <Button variant="ghost" size="small" onClick={onClose} aria-label="Close dialog">
            Close
          </Button>
        </header>
        <div className="modal-body">{children}</div>
        {footer && <footer className="modal-footer">{footer}</footer>}
      </div>
    </div>
  );
}

export function Hint({ children }: { children: ReactNode }) {
  return <p className="hint">{children}</p>;
}

export function WarnStrip({ children }: { children: ReactNode }) {
  return (
    <div className="warn-strip">
      <IconWarning />
      <span>{children}</span>
    </div>
  );
}

export function Readout({
  label,
  value,
  unit,
}: {
  label: string;
  value: number | null | undefined;
  unit?: string;
}) {
  return (
    <div className="hud-row">
      <span className="hud-key">{label}</span>
      <span className="hud-value">
        {formatNumber(value ?? null, 3)}
        {unit ? ` ${unit}` : ""}
      </span>
    </div>
  );
}
