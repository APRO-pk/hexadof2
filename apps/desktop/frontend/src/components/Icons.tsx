/**
 * Inline SVG icons.
 *
 * Icons are drawn here rather than pulled from an icon font so they inherit
 * `currentColor`, scale with the font, and add no network request. Every icon is
 * paired with a text label in the interface, because an icon alone is not an
 * accessible indicator.
 */

import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function Base({ children, ...props }: IconProps & { children: React.ReactNode }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {children}
    </svg>
  );
}

export const IconDashboard = (p: IconProps) => (
  <Base {...p}>
    <rect x="3" y="3" width="7.5" height="7.5" rx="1.4" />
    <rect x="13.5" y="3" width="7.5" height="7.5" rx="1.4" />
    <rect x="3" y="13.5" width="7.5" height="7.5" rx="1.4" />
    <rect x="13.5" y="13.5" width="7.5" height="7.5" rx="1.4" />
  </Base>
);

export const IconDynamics = (p: IconProps) => (
  <Base {...p}>
    <path d="M3 20c4-1 6-4 8-9" />
    <path d="M11 11l3-6 3 3 4-1" />
    <path d="M12 21c3 0 5-2 6-5" />
  </Base>
);

export const IconTelemetry = (p: IconProps) => (
  <Base {...p}>
    <path d="M2 12h4l2.5-6 3 12 3-8 2.5 2H22" />
  </Base>
);

export const IconAnalysis = (p: IconProps) => (
  <Base {...p}>
    <path d="M4 20V4" />
    <path d="M4 20h16" />
    <path d="M8 16v-4" />
    <path d="M12 16V8" />
    <path d="M16 16v-6" />
    <path d="M20 16v-9" />
  </Base>
);

export const IconProjects = (p: IconProps) => (
  <Base {...p}>
    <path d="M3 7.5A1.5 1.5 0 014.5 6h4l2 2.5h7A1.5 1.5 0 0119 10v7.5A1.5 1.5 0 0117.5 19h-13A1.5 1.5 0 013 17.5z" />
  </Base>
);

export const IconSettings = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2.8v2.4M12 18.8v2.4M4.6 7.4l2 1.2M17.4 15.4l2 1.2M4.6 16.6l2-1.2M17.4 8.6l2-1.2" />
  </Base>
);

export const IconPlay = (p: IconProps) => (
  <Base {...p}>
    <path d="M7 4.8l12 7.2-12 7.2z" fill="currentColor" />
  </Base>
);

export const IconPause = (p: IconProps) => (
  <Base {...p}>
    <path d="M8 4.5v15M16 4.5v15" strokeWidth={2.4} />
  </Base>
);

export const IconStop = (p: IconProps) => (
  <Base {...p}>
    <rect x="6" y="6" width="12" height="12" rx="1.4" fill="currentColor" />
  </Base>
);

export const IconStepBack = (p: IconProps) => (
  <Base {...p}>
    <path d="M17 5.5v13L8 12z" fill="currentColor" />
    <path d="M6.5 5v14" strokeWidth={2} />
  </Base>
);

export const IconStepForward = (p: IconProps) => (
  <Base {...p}>
    <path d="M7 5.5v13L16 12z" fill="currentColor" />
    <path d="M17.5 5v14" strokeWidth={2} />
  </Base>
);

export const IconRewind = (p: IconProps) => (
  <Base {...p}>
    <path d="M11 5.5v13L2 12z" fill="currentColor" />
    <path d="M21 5.5v13L12 12z" fill="currentColor" />
  </Base>
);

export const IconRecord = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="6" fill="currentColor" />
  </Base>
);

export const IconSave = (p: IconProps) => (
  <Base {...p}>
    <path d="M5 4h11l3 3v13H5z" />
    <path d="M9 4v5h6V4" />
    <path d="M9 13h6v7H9z" />
  </Base>
);

export const IconOpen = (p: IconProps) => (
  <Base {...p}>
    <path d="M3 7.5A1.5 1.5 0 014.5 6h4l2 2.5h7A1.5 1.5 0 0119 10v1" />
    <path d="M3 8.5l2.2 9h13.6l2.2-9z" />
  </Base>
);

export const IconImport = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3v12" />
    <path d="M7.5 10.5L12 15l4.5-4.5" />
    <path d="M4 19h16" />
  </Base>
);

export const IconExport = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 15V3" />
    <path d="M7.5 7.5L12 3l4.5 4.5" />
    <path d="M4 19h16" />
  </Base>
);

export const IconCheck = (p: IconProps) => (
  <Base {...p}>
    <path d="M4.5 12.5l5 5 10-11" strokeWidth={2.2} />
  </Base>
);

export const IconCheckCircle = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="9" />
    <path d="M8 12.5l2.5 2.5L16 9.5" />
  </Base>
);

export const IconWarning = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3.5L21 20H3z" />
    <path d="M12 9.5v5" />
    <path d="M12 17.2h.01" strokeWidth={2.4} />
  </Base>
);

export const IconError = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="9" />
    <path d="M9 9l6 6M15 9l-6 6" />
  </Base>
);

export const IconInfo = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 11v5.5" />
    <path d="M12 7.8h.01" strokeWidth={2.4} />
  </Base>
);

export const IconChevronRight = (p: IconProps) => (
  <Base {...p}>
    <path d="M9 5l7 7-7 7" />
  </Base>
);

export const IconChevronDown = (p: IconProps) => (
  <Base {...p}>
    <path d="M5 9l7 7 7-7" />
  </Base>
);

export const IconPanelLeft = (p: IconProps) => (
  <Base {...p}>
    <rect x="3" y="4" width="18" height="16" rx="1.6" />
    <path d="M9.5 4v16" />
  </Base>
);

export const IconPanelRight = (p: IconProps) => (
  <Base {...p}>
    <rect x="3" y="4" width="18" height="16" rx="1.6" />
    <path d="M14.5 4v16" />
  </Base>
);

export const IconZoomIn = (p: IconProps) => (
  <Base {...p}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="M11 8.5v5M8.5 11h5" />
    <path d="M16 16l4.5 4.5" />
  </Base>
);

export const IconZoomOut = (p: IconProps) => (
  <Base {...p}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="M8.5 11h5" />
    <path d="M16 16l4.5 4.5" />
  </Base>
);

export const IconReset = (p: IconProps) => (
  <Base {...p}>
    <path d="M4 12a8 8 0 108-8" />
    <path d="M4 4v4.5h4.5" />
  </Base>
);

export const IconTarget = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="8" />
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2v3M12 19v3M2 12h3M19 12h3" />
  </Base>
);

export const IconRocket = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 2.5c3.2 2.6 4.8 6.1 4.8 10.1L12 21.5l-4.8-8.9c0-4 1.7-7.5 4.8-10.1z" />
    <circle cx="12" cy="10" r="1.9" />
    <path d="M7.2 15.5L4 18l3.2.6M16.8 15.5L20 18l-3.2.6" />
  </Base>
);

export const IconPlug = (p: IconProps) => (
  <Base {...p}>
    <path d="M9 3v6M15 3v6" />
    <path d="M6 9h12v2a6 6 0 01-6 6 6 6 0 01-6-6z" />
    <path d="M12 17v4" />
  </Base>
);

export const IconDatabase = (p: IconProps) => (
  <Base {...p}>
    <ellipse cx="12" cy="6" rx="7.5" ry="3" />
    <path d="M4.5 6v12c0 1.7 3.4 3 7.5 3s7.5-1.3 7.5-3V6" />
    <path d="M4.5 12c0 1.7 3.4 3 7.5 3s7.5-1.3 7.5-3" />
  </Base>
);

export const IconCompare = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3v18" />
    <path d="M5 8l-2 4 2 4" />
    <path d="M19 8l2 4-2 4" />
  </Base>
);

export const IconTrash = (p: IconProps) => (
  <Base {...p}>
    <path d="M4 7h16" />
    <path d="M9 7V4.5h6V7" />
    <path d="M6.5 7l1 13h9l1-13" />
    <path d="M10.5 10.5v6M13.5 10.5v6" />
  </Base>
);

export const IconPlus = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 5v14M5 12h14" />
  </Base>
);

export const IconCopy = (p: IconProps) => (
  <Base {...p}>
    <rect x="9" y="9" width="11" height="11" rx="1.6" />
    <path d="M15 9V6.5A1.5 1.5 0 0013.5 5h-8A1.5 1.5 0 004 6.5v8A1.5 1.5 0 005.5 16H9" />
  </Base>
);

export const IconSearch = (p: IconProps) => (
  <Base {...p}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="M16 16l4.5 4.5" />
  </Base>
);

export const IconMoon = (p: IconProps) => (
  <Base {...p}>
    <path d="M20 14.5A8.5 8.5 0 019.5 4a8.5 8.5 0 1010.5 10.5z" />
  </Base>
);

export const IconSun = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2.5v2M12 19.5v2M4.2 4.2l1.4 1.4M18.4 18.4l1.4 1.4M2.5 12h2M19.5 12h2M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4" />
  </Base>
);

export const IconBell = (p: IconProps) => (
  <Base {...p}>
    <path d="M6 9a6 6 0 1112 0c0 4 1.5 5.5 1.5 5.5h-15S6 13 6 9z" />
    <path d="M10 18a2 2 0 004 0" />
  </Base>
);

export const IconHelp = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="9" />
    <path d="M9.5 9.2A2.6 2.6 0 0114 10c0 1.7-2 2-2 3.5" />
    <path d="M12 17h.01" strokeWidth={2.4} />
  </Base>
);

export const IconCommand = (p: IconProps) => (
  <Base {...p}>
    <path d="M9 6a2 2 0 10-2 2h10a2 2 0 10-2-2v10a2 2 0 102-2H7a2 2 0 10-2 2V6z" />
  </Base>
);

export const IconLayers = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3l9 5-9 5-9-5z" />
    <path d="M3 13l9 5 9-5" />
  </Base>
);

export const IconCrosshair = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="7" />
    <path d="M12 2v4M12 18v4M2 12h4M18 12h4" />
  </Base>
);

export const IconTrend = (p: IconProps) => (
  <Base {...p}>
    <path d="M3 17l5-6 4 3 5-7 4 4" />
    <path d="M3 20h18" />
  </Base>
);

export const IconAlertHexagon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 2.8l8 4.6v9.2l-8 4.6-8-4.6V7.4z" />
    <path d="M12 8v5" />
    <path d="M12 15.8h.01" strokeWidth={2.4} />
  </Base>
);

/** The brand mark: a hexagon with a launch axis, drawn from the same geometry the
 * icon generator uses. */
export const BrandMark = (p: IconProps) => (
  <Base {...p} strokeWidth={1.5}>
    <path d="M12 2.6l8.1 4.7v9.4L12 21.4l-8.1-4.7V7.3z" />
    <path d="M12 6.4v11.2" strokeWidth={1.9} />
    <path d="M12 6.4l-2.6 2.6M12 6.4l2.6 2.6" strokeWidth={1.6} />
  </Base>
);
