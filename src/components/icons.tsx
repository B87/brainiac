/** Inline 16-unit stroke icons; size and color come from props and `currentColor`. */
type IconProps = { size?: number; className?: string };

function Svg({
  size = 14,
  className,
  strokeWidth = 1.4,
  children,
}: IconProps & { strokeWidth?: number; children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export const GridIcon = (p: IconProps) => (
  <Svg {...p}>
    <rect x="2" y="2" width="5" height="5" rx="1" />
    <rect x="9" y="2" width="5" height="5" rx="1" />
    <rect x="2" y="9" width="5" height="5" rx="1" />
    <rect x="9" y="9" width="5" height="5" rx="1" />
  </Svg>
);

export const ChevronDown = (p: IconProps) => (
  <Svg strokeWidth={1.8} {...p}>
    <path d="M4 6l4 4 4-4" />
  </Svg>
);

export const ChevronRight = (p: IconProps) => (
  <Svg strokeWidth={1.8} {...p}>
    <path d="M6 4l4 4-4 4" />
  </Svg>
);

export const PlusIcon = (p: IconProps) => (
  <Svg strokeWidth={1.5} {...p}>
    <path d="M8 3v10M3 8h10" />
  </Svg>
);

export const BranchIcon = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="5" cy="3.5" r="1.5" />
    <circle cx="5" cy="12.5" r="1.5" />
    <circle cx="11" cy="5" r="1.5" />
    <path d="M5 5v6M11 6.5c0 3-6 2.5-6 4.5" />
  </Svg>
);

export const TagIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M2.5 2.5h5l6 6-5 5-6-6z" />
    <circle cx="5.5" cy="5.5" r="0.9" />
  </Svg>
);

export const RefreshIcon = (p: IconProps) => (
  <Svg strokeWidth={1.5} {...p}>
    <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v3h-3" />
  </Svg>
);

export const FolderIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M2 4.5h4l1.5 1.5H14v6.5H2z" />
  </Svg>
);

export const RescanIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M2 4.5h4l1.5 1.5H14v6.5H2z" />
    <circle cx="9" cy="9.3" r="1.8" />
  </Svg>
);

export const FolderPlusIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M2 4.5h4l1.5 1.5H14v6.5H2z" />
    <path d="M8 7.8v3M6.5 9.3h3" />
  </Svg>
);

export const ExternalIcon = (p: IconProps) => (
  <Svg strokeWidth={1.6} {...p}>
    <path d="M9 3h4v4M13 3L7.5 8.5M11 9.5V13H3V5h3.5" />
  </Svg>
);

export const MoreIcon = ({ size = 14, className }: IconProps) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 16 16"
    fill="currentColor"
    className={className}
    aria-hidden="true"
  >
    <circle cx="3" cy="8" r="1.3" />
    <circle cx="8" cy="8" r="1.3" />
    <circle cx="13" cy="8" r="1.3" />
  </svg>
);

export const SearchIcon = (p: IconProps) => (
  <Svg strokeWidth={1.5} size={13} {...p}>
    <circle cx="7" cy="7" r="4.5" />
    <path d="M10.5 10.5L14 14" />
  </Svg>
);

export const CopyIcon = (p: IconProps) => (
  <Svg size={13} {...p}>
    <rect x="5" y="5" width="8" height="8" rx="1.5" />
    <path d="M3 11V3.5a.5.5 0 0 1 .5-.5H11" />
  </Svg>
);

export const CloseIcon = (p: IconProps) => (
  <Svg strokeWidth={1.8} size={12} {...p}>
    <path d="M4 4l8 8M12 4l-8 8" />
  </Svg>
);

export const SidebarIcon = (p: IconProps) => (
  <Svg {...p}>
    <rect x="2" y="3" width="12" height="10" rx="1.5" />
    <path d="M6 3v10" />
  </Svg>
);

/** Arrow into a tray: fetch. */
export const FetchIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M8 2.5v7.5M5 7l3 3 3-3" />
    <path d="M3 11v1.5a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V11" />
  </Svg>
);

/** Arrow up out of a line: a branch moved ahead. */
export const AdvancedIcon = (p: IconProps) => (
  <Svg strokeWidth={1.6} {...p}>
    <path d="M8 13V3.5M4.5 7L8 3.5 11.5 7" />
  </Svg>
);

/** Broken arrow: history was rewritten. */
export const RewrittenIcon = (p: IconProps) => (
  <Svg strokeWidth={1.6} {...p}>
    <path d="M9.5 2L5 8.5h4L6.5 14" />
  </Svg>
);

export const BellIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M4 11V7a4 4 0 0 1 8 0v4l1 1.5H3z" />
    <path d="M6.5 14a1.5 1.5 0 0 0 3 0" />
  </Svg>
);

export const ClockIcon = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="8" cy="8" r="6" />
    <path d="M8 4.5V8l2.5 1.5" />
  </Svg>
);

/** A page with a folded corner: a note. */
export const NoteIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M4 2h5.5L12.5 5v9H4z" />
    <path d="M9.5 2v3h3M6 8h4.5M6 10.5h4.5" />
  </Svg>
);

/** A round check: a task. Tasks are round; note checkboxes are square. */
export const TaskIcon = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="8" cy="8" r="5.5" />
    <path d="M5.6 8.2l1.6 1.6 3.2-3.4" />
  </Svg>
);

/** A sun: Today. */
export const TodayIcon = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="8" cy="8" r="2.8" />
    <path d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" />
  </Svg>
);

export const TrashIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9" />
  </Svg>
);

export const LinkIcon = (p: IconProps) => (
  <Svg strokeWidth={1.5} {...p}>
    <path d="M7 9a2.5 2.5 0 0 0 3.5 0l2-2a2.5 2.5 0 0 0-3.5-3.5l-.8.8" />
    <path d="M9 7a2.5 2.5 0 0 0-3.5 0l-2 2A2.5 2.5 0 0 0 7 12.5l.8-.8" />
  </Svg>
);

export const PinIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M6 2.5h4M7 2.5v4L4.5 9h7L9 6.5v-4M8 9v4.5" />
  </Svg>
);

/** An exclamation in a triangle: overdue or a problem, never color alone. */
export const AlertIcon = (p: IconProps) => (
  <Svg strokeWidth={1.5} {...p}>
    <path d="M8 2.5l6 10.5H2z" />
    <path d="M8 6.5v3M8 11.3v.2" />
  </Svg>
);

/** A sidebar on the right: the context panel. */
export const PanelRightIcon = (p: IconProps) => (
  <Svg {...p}>
    <rect x="2" y="3" width="12" height="10" rx="1.5" />
    <path d="M10 3v10" />
  </Svg>
);
