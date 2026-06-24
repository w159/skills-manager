# Modular UI Component Patterns

Full TypeScript/React code for reusable components. These are framework-agnostic patterns — adapt the imports to your project's actual utils (`cn`, `useTranslation`, etc.).

## StatRing — Circular SVG Progress

```tsx
export function StatRing({
  value, max, size = 48, strokeWidth = 4,
  color = "var(--color-accent-light)",
  trackColor = "var(--color-surface-hover)",
  className, children,
}: {
  value: number; max: number; size?: number; strokeWidth?: number;
  color?: string; trackColor?: string; className?: string;
  children?: React.ReactNode;
}) {
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const pct = max > 0 ? Math.min(value / max, 1) : 0;
  const offset = circumference * (1 - pct);

  return (
    <div className={cn("relative inline-flex items-center justify-center", className)}
      style={{ width: size, height: size }}>
      <svg width={size} height={size} className="-rotate-90">
        <circle cx={size/2} cy={size/2} r={radius} fill="none"
          stroke={trackColor} strokeWidth={strokeWidth} />
        <circle cx={size/2} cy={size/2} r={radius} fill="none"
          stroke={color} strokeWidth={strokeWidth}
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          strokeLinecap="round"
          style={{ transition: "stroke-dashoffset 0.8s var(--ease-spring)" }} />
      </svg>
      {children && (
        <div className="absolute inset-0 flex items-center justify-center">{children}</div>
      )}
    </div>
  );
}
```

## StatCard — Bento Grid Card

```tsx
export function StatCard({
  title, value, subtitle, icon: Icon, iconBg, iconColor,
  ringValue, ringMax, ringColor, delay = 0, className,
}: {
  title: string; value: React.ReactNode; subtitle?: string;
  icon: React.ElementType; iconBg?: string; iconColor?: string;
  ringValue?: number; ringMax?: number; ringColor?: string;
  delay?: number; className?: string;
}) {
  return (
    <div className={cn("bento-card p-4 animate-fade-in-up hover-lift", className)}
      style={{ animationDelay: `${delay}ms` }}>
      <div className="flex items-start justify-between">
        <div className="min-w-0 flex-1">
          <p className="app-section-title mb-1.5">{title}</p>
          <h3 className="text-2xl font-bold text-primary leading-none tracking-tight">{value}</h3>
          {subtitle && <p className="mt-1.5 text-[12px] text-muted leading-tight">{subtitle}</p>}
        </div>
        {ringValue !== undefined && ringMax !== undefined ? (
          <StatRing value={ringValue} max={ringMax} size={48} strokeWidth={4} color={ringColor}>
            <span className="text-[10px] font-semibold tabular-nums text-tertiary">
              {ringMax > 0 ? `${Math.round((ringValue / ringMax) * 100)}%` : "—"}
            </span>
          </StatRing>
        ) : (
          <div className={cn("flex h-10 w-10 items-center justify-center rounded-lg border border-border-subtle",
            iconBg ?? "bg-accent-bg")}>
            <Icon className={cn("h-4 w-4", iconColor ?? "text-accent-light")} />
          </div>
        )}
      </div>
    </div>
  );
}
```

## StatusBadge — Pill Badge with Tones

```tsx
type BadgeTone = "default" | "success" | "warning" | "danger" | "info" | "violet" | "neutral";

const BADGE_STYLES: Record<BadgeTone, string> = {
  default: "border-border bg-surface-hover text-tertiary",
  success: "border-accent-border bg-accent-bg text-accent-light",
  warning: "border-amber/30 bg-amber-bg text-amber-light",
  danger:  "border-rose/30 bg-rose-bg text-rose-light",
  info:    "border-sky/30 bg-sky-bg text-sky-light",
  violet:  "border-violet/30 bg-violet-bg text-violet-light",
  neutral: "border-border bg-bg-secondary text-muted",
};

export function StatusBadge({
  tone = "default", children, className, dot = false, pulse = false,
}: {
  tone?: BadgeTone; children: React.ReactNode; className?: string;
  dot?: boolean; pulse?: boolean;
}) {
  return (
    <span className={cn(
      "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium",
      BADGE_STYLES[tone], className
    )}>
      {dot && <span className={cn("relative h-1.5 w-1.5 rounded-full bg-current",
        pulse && "pulse-dot")} />}
      {children}
    </span>
  );
}
```

## EmptyState — Icon + Description + Action

```tsx
export function EmptyState({
  icon: Icon, title, description, actionLabel, onAction, className,
}: {
  icon: React.ElementType; title: string; description?: string;
  actionLabel?: string; onAction?: () => void; className?: string;
}) {
  return (
    <div className={cn("flex flex-col items-center justify-center py-12 px-6 text-center", className)}>
      <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl border border-border-subtle bg-bg-secondary">
        <Icon className="h-6 w-6 text-faint" />
      </div>
      <h3 className="text-[14px] font-semibold text-secondary mb-1">{title}</h3>
      {description && <p className="text-[13px] text-muted max-w-sm leading-relaxed">{description}</p>}
      {actionLabel && onAction && (
        <button onClick={onAction} className="app-button-secondary mt-4">{actionLabel}</button>
      )}
    </div>
  );
}
```

## Skeleton Primitives

```tsx
export function SkeletonText({ lines = 1, className, lineHeight = "h-3" }:
  { lines?: number; className?: string; lineHeight?: string }) {
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      {Array.from({ length: lines }).map((_, i) => (
        <div key={i} className={cn("skeleton", lineHeight, i === lines - 1 ? "w-2/3" : "w-full")} />
      ))}
    </div>
  );
}

export function SkeletonCard({ className }: { className?: string }) {
  return (
    <div className={cn("bento-card p-4 animate-fade-in", className)}>
      <div className="flex items-center justify-between mb-3">
        <div className="skeleton h-4 w-24 rounded" />
        <div className="skeleton h-8 w-8 rounded-lg" />
      </div>
      <div className="skeleton h-7 w-16 rounded mb-2" />
      <div className="skeleton h-3 w-full rounded" />
    </div>
  );
}

export function SkeletonRow({ className }: { className?: string }) {
  return (
    <div className={cn("flex items-center gap-3 px-3.5 py-3", className)}>
      <div className="skeleton h-6 w-6 rounded-[4px] shrink-0" />
      <div className="flex-1 flex flex-col gap-1.5">
        <div className="skeleton h-3.5 w-48 rounded" />
        <div className="skeleton h-3 w-32 rounded" />
      </div>
      <div className="skeleton h-5 w-16 rounded-full shrink-0" />
    </div>
  );
}
```

## MiniBarChart — CSS Only

```tsx
export function MiniBarChart({
  data, className, height = 48, barColor = "var(--color-accent-light)",
}: {
  data: { label: string; value: number }[];
  className?: string; height?: number; barColor?: string;
}) {
  const max = Math.max(...data.map((d) => d.value), 1);
  return (
    <div className={cn("flex items-end gap-1", className)} style={{ height }}>
      {data.map((d, i) => {
        const h = max > 0 ? (d.value / max) * height : 0;
        return (
          <div key={i} className="flex-1 flex flex-col items-center gap-1 justify-end">
            <div className="w-full rounded-sm transition-all duration-500"
              style={{
                height: Math.max(h, 2),
                background: barColor,
                opacity: 0.4 + (d.value / max) * 0.6,
                transitionDelay: `${i * 50}ms`,
              }}
              title={`${d.label}: ${d.value}`} />
          </div>
        );
      })}
    </div>
  );
}
```

## AnimatedCounter — requestAnimationFrame Count-Up

```tsx
export function AnimatedCounter({
  value, duration = 600, className,
}: { value: number; duration?: number; className?: string }) {
  const [display, setDisplay] = useState(0);

  useEffect(() => {
    const start = display;
    const diff = value - start;
    if (diff === 0) return;
    const startTime = performance.now();
    let raf: number;
    const tick = (now: number) => {
      const progress = Math.min((now - startTime) / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      setDisplay(Math.round(start + diff * eased));
      if (progress < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  return <span className={cn("tabular-nums", className)}>{display}</span>;
}
```

## SyncStatusIndicator — 5-State Pulsing Dot

```tsx
export function SyncStatusIndicator({
  status, className,
}: {
  status: "idle" | "syncing" | "synced" | "error" | "pending";
  className?: string;
}) {
  const config: Record<string, { color: string; dot: string; label: string; pulse?: boolean }> = {
    idle:    { color: "text-faint",        dot: "bg-faint",          label: "Idle" },
    syncing:  { color: "text-sky-light",   dot: "bg-sky-light",     label: "Syncing", pulse: true },
    synced:   { color: "text-accent-light", dot: "bg-accent-light", label: "Synced" },
    error:    { color: "text-rose-light",  dot: "bg-rose-light",     label: "Error" },
    pending:  { color: "text-amber-light",  dot: "bg-amber-light",   label: "Pending" },
  };
  const c = config[status];
  return (
    <div className={cn("inline-flex items-center gap-1.5", className)}>
      <span className={cn("relative h-1.5 w-1.5 rounded-full", c.dot, c.pulse && "pulse-dot")} />
      <span className={cn("text-[11px] font-medium", c.color)}>{c.label}</span>
    </div>
  );
}
```

## ProgressBar — Determinate / Indeterminate

```tsx
export function ProgressBar({
  value, max = 100, className, indeterminate = false,
}: {
  value?: number; max?: number; className?: string; indeterminate?: boolean;
}) {
  const pct = max > 0 ? Math.min((value ?? 0) / max, 1) * 100 : 0;
  return (
    <div className={cn("progress-bar h-1.5 w-full", className)}>
      <div className={cn("progress-bar-fill", indeterminate && "indeterminate")}
        style={indeterminate ? undefined : { width: `${pct}%` }} />
    </div>
  );
}
```

## SectionHeader — Title + Subtitle + Action

```tsx
export function SectionHeader({
  title, subtitle, action, className,
}: {
  title: string; subtitle?: string; action?: React.ReactNode; className?: string;
}) {
  return (
    <div className={cn("flex items-end justify-between gap-3", className)}>
      <div>
        <h2 className="app-section-title">{title}</h2>
        {subtitle && <p className="mt-1 text-[12px] text-faint">{subtitle}</p>}
      </div>
      {action}
    </div>
  );
}
```