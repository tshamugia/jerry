import type { ReactNode } from "react";

/**
 * Jerry's line icons, drawn on a 16 × 16 grid as in the design canvas (design/README.md: 16 px,
 * 1.2 stroke). They take the text color, so `text-*` classes color them.
 */
function Icon({
  size = 16,
  stroke = 1.2,
  children,
}: {
  size?: 10 | 14 | 16;
  stroke?: number;
  children: ReactNode;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export function BackIcon() {
  return (
    <Icon>
      <path d="M13 8H3M7 4L3 8l4 4" />
    </Icon>
  );
}

export function ForwardIcon() {
  return (
    <Icon>
      <path d="M3 8h10M9 4l4 4-4 4" />
    </Icon>
  );
}

export function UpIcon() {
  return (
    <Icon>
      <path d="M8 13V3M4 7l4-4 4 4" />
    </Icon>
  );
}

export function RefreshIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5V5h-2.5" />
    </Icon>
  );
}

export function PaletteIcon() {
  return (
    <Icon>
      <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
      <path d="M4.5 6l2 2-2 2M8.5 10.5h3" />
    </Icon>
  );
}

export function FilterIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M2 3h12l-4.5 5.2v4.3L6.5 14V8.2z" />
    </Icon>
  );
}

export function DetailsIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M2 4h3M7 4h7M2 8h3M7 8h7M2 12h3M7 12h7" />
    </Icon>
  );
}

export function ListIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M2.5 4h.5M6 4h7.5M2.5 8h.5M6 8h7.5M2.5 12h.5M6 12h7.5" />
    </Icon>
  );
}

export function GridIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" />
      <rect x="9" y="2.5" width="4.5" height="4.5" rx="1" />
      <rect x="2.5" y="9" width="4.5" height="4.5" rx="1" />
      <rect x="9" y="9" width="4.5" height="4.5" rx="1" />
    </Icon>
  );
}

export function ComfortableIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M2 3.5h12M2 8h12M2 12.5h12" />
    </Icon>
  );
}

export function CompactIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M2 2.5h12M2 5.5h12M2 8.5h12M2 11.5h12M2 14.5h12" />
    </Icon>
  );
}

export function PlusIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M8 3v10M3 8h10" />
    </Icon>
  );
}

/** The 10 px cross of a tab's close button. */
export function CloseIcon() {
  return (
    <Icon size={10} stroke={1.6}>
      <path d="M3 3l10 10M13 3L3 13" />
    </Icon>
  );
}

/** The 9 px pin on a pinned tab, filled with the text color. */
export function PinIcon({ className }: { className?: string }) {
  return (
    <svg
      width={9}
      height={9}
      viewBox="0 0 16 16"
      fill="currentColor"
      className={className}
      aria-hidden="true"
    >
      <path d="M10 1l5 5-2.2.6-2.6 2.6.4 3.3-1.6 1.6-3.2-3.2L2 15l-1-1 4.1-3.8L1.9 7l1.6-1.6 3.3.4 2.6-2.6z" />
    </svg>
  );
}

/** The sorted column's arrow in the list header: up for ascending, down for descending. */
export function SortIcon({ descending }: { descending: boolean }) {
  return (
    <Icon size={10} stroke={1.6}>
      <path d={descending ? "M4 6l4 4 4-4" : "M4 10l4-4 4 4"} />
    </Icon>
  );
}

/** A menu item's check mark. */
export function CheckIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M3 8.5l3 3 7-7" />
    </Icon>
  );
}

/** The design's two-tone yellow folder. */
export function FolderIcon() {
  return (
    <svg width={16} height={16} viewBox="0 0 16 16" className="shrink-0" aria-hidden="true">
      <path
        d="M1.5 3.5a1 1 0 0 1 1-1h3.6l1.5 1.5h5.9a1 1 0 0 1 1 1v7.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"
        className="fill-folder-back"
      />
      <rect x="1.5" y="5.5" width="13" height="8" rx="1" className="fill-folder-front" />
    </svg>
  );
}
