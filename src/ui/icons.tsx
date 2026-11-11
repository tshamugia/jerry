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
  size?: 10 | 11 | 12 | 14 | 16 | 20;
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

/** A toast's 20 px "i" in a circle: a note, not an error. */
export function InfoIcon() {
  return (
    <svg
      width={20}
      height={20}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.2}
      strokeLinecap="round"
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="6.5" />
      <path d="M8 7.2v4" />
      <circle cx="8" cy="4.9" r="0.4" fill="currentColor" />
    </svg>
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

/** One pane: a window with a single list. */
export function SinglePaneIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
    </Icon>
  );
}

/** Two panes: a window split down the middle. */
export function DualPaneIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
      <path d="M8 2.5v11" />
    </Icon>
  );
}

/** An address bar segment's arrow: it drops the folder's subfolders. */
export function ChevronIcon() {
  return (
    <Icon size={10} stroke={1.6}>
      <path d="M6 4l4 4-4 4" />
    </Icon>
  );
}

/** A button that drops a menu: an arrow down. */
export function DropIcon() {
  return (
    <Icon size={10} stroke={1.6}>
      <path d="M4 6l4 4 4-4" />
    </Icon>
  );
}

/** Segments left out of a narrow address bar. */
export function MoreIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M3.5 8h.01M8 8h.01M12.5 8h.01" strokeWidth={2.2} />
    </Icon>
  );
}

/** Show or hide the sidebar: a window with its left column marked off. */
export function SidebarIcon() {
  return (
    <Icon>
      <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
      <path d="M6 2.5v11" />
    </Icon>
  );
}

/** Settings: the design's gear. */
export function SettingsIcon() {
  return (
    <Icon>
      <circle cx="8" cy="8" r="2.3" />
      <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" />
    </Icon>
  );
}

/** A sidebar section's caption arrow: down while the section is open, right while folded. */
export function SectionIcon({ folded }: { folded: boolean }) {
  return (
    <Icon size={12} stroke={1.6}>
      <path d={folded ? "M6 3.5L10.5 8 6 12.5" : "M3.5 6L8 10.5 12.5 6"} />
    </Icon>
  );
}

/** Search: a magnifier. */
export function SearchIcon() {
  return (
    <Icon>
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.3 10.3L14 14" />
    </Icon>
  );
}

/** Recent folders: a clock. */
export function RecentIcon() {
  return (
    <Icon>
      <circle cx="8" cy="8" r="6.2" />
      <path d="M8 4.5V8l2.3 1.5" />
    </Icon>
  );
}

/** Eject: a triangle over a bar. */
export function EjectIcon() {
  return (
    <Icon size={14} stroke={1.3}>
      <path d="M8 3.5l4.5 5h-9zM3.5 12h9" />
    </Icon>
  );
}

/** A copy, as the transfer panel shows it: two sheets (design: 20 px, 1.1 stroke). */
export function CopyIcon({ size = 20 }: { size?: 16 | 20 }) {
  return (
    <Icon size={size} stroke={1.1}>
      <rect x="5.5" y="5.5" width="8.5" height="8.5" rx="1.2" />
      <path d="M3.5 10.5H3A1 1 0 0 1 2 9.5V3a1 1 0 0 1 1-1h6.5a1 1 0 0 1 1 1v.5" />
    </Icon>
  );
}

/** A move: an arrow into a sheet. */
export function MoveIcon({ size = 20 }: { size?: 16 | 20 }) {
  return (
    <Icon size={size} stroke={1.1}>
      <path d="M8.5 2.5h4a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-4" />
      <path d="M2 8h8.5M7.5 5l3 3-3 3" />
    </Icon>
  );
}

/** Pause: two bars (design: 12 px, 1.8 stroke). */
export function PauseIcon() {
  return (
    <Icon size={12} stroke={1.8}>
      <path d="M5.5 3.5v9M10.5 3.5v9" />
    </Icon>
  );
}

/** Resume: a triangle pointing on. */
export function ResumeIcon() {
  return (
    <Icon size={12} stroke={1.5}>
      <path d="M5 3.2v9.6L12.5 8z" />
    </Icon>
  );
}

/** Cancel: a cross (design: 11 px, 1.8 stroke). */
export function CancelIcon() {
  return (
    <Icon size={11} stroke={1.8}>
      <path d="M3 3l10 10M13 3L3 13" />
    </Icon>
  );
}

/** The queue's arrow: up while it can open above the panel, down while it is open. */
export function QueueIcon({ open }: { open: boolean }) {
  return (
    <Icon size={10} stroke={1.6}>
      <path d={open ? "M3.5 6L8 10.5 12.5 6" : "M3.5 10L8 5.5l4.5 4.5"} />
    </Icon>
  );
}

/** The grip a row is dragged by: six dots. */
export function GripIcon() {
  return (
    <svg width={16} height={16} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <circle cx="6" cy="4" r="1" />
      <circle cx="10" cy="4" r="1" />
      <circle cx="6" cy="8" r="1" />
      <circle cx="10" cy="8" r="1" />
      <circle cx="6" cy="12" r="1" />
      <circle cx="10" cy="12" r="1" />
    </svg>
  );
}

/** Something went wrong: an "!" in a circle, 20 px, for a summary. It takes the text color. */
export function ErrorIcon({ size = 20 }: { size?: 16 | 20 }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.2}
      strokeLinecap="round"
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="6.5" />
      <path d="M8 4.6v4.2" />
      <circle cx="8" cy="11.2" r="0.4" fill="currentColor" />
    </svg>
  );
}
