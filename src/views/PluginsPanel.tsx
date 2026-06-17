import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, AlertCircle, ChevronRight, ChevronDown, ShieldOff } from "lucide-react";
import { cn } from "../utils";
import { listInstalledPlugins, setPluginEnabled } from "../lib/tauri";
import type { Plugin, BundledAsset } from "../lib/tauri";
import { getErrorMessage } from "../lib/error";
import { toast } from "sonner";

// ── Asset type ordering for the drill-down groups ─────────────────────────────

const ASSET_TYPE_ORDER = ["skill", "agent", "command", "hook", "mcp"] as const;

function groupByType(assets: BundledAsset[]): Record<string, BundledAsset[]> {
  const grouped: Record<string, BundledAsset[]> = {};
  for (const a of assets) {
    if (!grouped[a.asset_type]) grouped[a.asset_type] = [];
    grouped[a.asset_type].push(a);
  }
  return grouped;
}

function sortedGroupEntries(
  grouped: Record<string, BundledAsset[]>
): [string, BundledAsset[]][] {
  const known = ASSET_TYPE_ORDER.filter((t) => grouped[t]);
  const rest = Object.keys(grouped).filter(
    (k) => !ASSET_TYPE_ORDER.includes(k as (typeof ASSET_TYPE_ORDER)[number])
  );
  return [...known, ...rest].map((k) => [k, grouped[k]]);
}

// ── Minimal toggle switch component ───────────────────────────────────────────

interface ToggleSwitchProps {
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
  label: string;
}

function ToggleSwitch({ checked, disabled, onChange, label }: ToggleSwitchProps) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!checked);
      }}
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent",
        "transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        "disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "bg-accent" : "bg-surface-hover"
      )}
    >
      <span
        className={cn(
          "pointer-events-none block h-4 w-4 rounded-full bg-white shadow-sm transition-transform",
          checked ? "translate-x-4" : "translate-x-0"
        )}
      />
    </button>
  );
}

// ── Single plugin row with expandable asset drill-down ────────────────────────

interface PluginRowProps {
  plugin: Plugin;
  onToggle: (pluginId: string, enabled: boolean) => void;
}

function PluginRow({ plugin, onToggle }: PluginRowProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [toggling, setToggling] = useState(false);

  const grouped = groupByType(plugin.assets);
  const groupEntries = sortedGroupEntries(grouped);
  const ChevronIcon = open ? ChevronDown : ChevronRight;

  const handleToggle = async (next: boolean) => {
    setToggling(true);
    // Optimistic update via parent callback.
    onToggle(plugin.id, next);
    try {
      await setPluginEnabled(plugin.id, next);
      toast.success(
        next
          ? t("plugins.enabledToast", { name: plugin.name })
          : t("plugins.disabledToast", { name: plugin.name })
      );
    } catch (e) {
      // Revert on failure.
      onToggle(plugin.id, !next);
      toast.error(getErrorMessage(e, t("common.error")));
    } finally {
      setToggling(false);
    }
  };

  return (
    <div
      className={cn(
        "rounded-lg border border-border bg-surface",
        !plugin.enabled && "opacity-60"
      )}
    >
      {/* Header row */}
      <div className="flex items-start gap-3 p-3">
        {/* Expand chevron -- clicking it toggles the drill-down */}
        <button
          onClick={() => setOpen((v) => !v)}
          className="mt-0.5 flex shrink-0 items-center text-muted hover:text-secondary transition-colors"
        >
          <ChevronIcon className="h-4 w-4" />
        </button>

        {/* Main content -- also clickable to expand */}
        <button
          onClick={() => setOpen((v) => !v)}
          className="min-w-0 flex-1 text-left"
        >
          {/* Name + badges */}
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-sm font-medium text-primary">{plugin.name}</span>
            <span className="rounded px-1.5 py-0.5 text-[11px] font-medium bg-surface-hover text-muted">
              {plugin.marketplace}
            </span>
            <span className="rounded px-1.5 py-0.5 text-[11px] text-muted">
              v{plugin.version}
            </span>
            {plugin.blocked && (
              <span className="flex items-center gap-0.5 rounded px-1.5 py-0.5 text-[11px] font-medium bg-red-50 text-red-500 dark:bg-red-950 dark:text-red-400">
                <ShieldOff className="h-3 w-3" />
                {t("plugins.blocked")}
              </span>
            )}
          </div>

          {/* Description */}
          {plugin.description && (
            <p className="mt-0.5 text-[12px] text-muted line-clamp-2">
              {plugin.description}
            </p>
          )}

          {/* Asset count summary */}
          {plugin.assets.length > 0 && (
            <p className="mt-1 text-[11px] text-muted">
              {groupEntries
                .map(([type, items]) => `${items.length} ${type}${items.length !== 1 ? "s" : ""}`)
                .join(", ")}
            </p>
          )}
        </button>

        {/* Enable/disable toggle -- sits at the trailing edge, stops propagation */}
        <div className="flex shrink-0 flex-col items-end gap-1 ml-2">
          <ToggleSwitch
            checked={plugin.enabled}
            disabled={toggling}
            onChange={(next) => void handleToggle(next)}
            label={
              plugin.enabled
                ? t("plugins.disableLabel", { name: plugin.name })
                : t("plugins.enableLabel", { name: plugin.name })
            }
          />
          {!plugin.enabled && (
            <span className="text-[10px] text-muted">{t("plugins.disabled")}</span>
          )}
        </div>
      </div>

      {/* Session-restart notice -- shown when plugin is disabled */}
      {!plugin.enabled && (
        <p className="border-t border-border px-3 py-1.5 text-[11px] text-muted">
          {t("plugins.sessionRestartNote")}
        </p>
      )}

      {/* Drill-down: bundled assets grouped by type */}
      {open && (
        <div className="border-t border-border px-3 pb-3 pt-2">
          {plugin.assets.length === 0 ? (
            <p className="text-[12px] text-muted py-1">{t("plugins.noAssets")}</p>
          ) : (
            <div className="flex flex-col gap-2">
              {groupEntries.map(([type, items]) => (
                <div key={type}>
                  <p className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {type} ({items.length})
                  </p>
                  <div className="flex flex-col gap-0.5">
                    {items.map((asset) => (
                      <p
                        key={asset.path}
                        className="truncate text-[12px] text-secondary pl-1"
                        title={asset.path}
                      >
                        {asset.name}
                      </p>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── Main panel ────────────────────────────────────────────────────────────────

export function PluginsPanel() {
  const { t } = useTranslation();
  const [plugins, setPlugins] = useState<Plugin[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await listInstalledPlugins();
      setPlugins(result);
    } catch (e) {
      setError(getErrorMessage(e, t("common.error")));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  // Optimistic toggle: flip the plugin in local state immediately.
  const handleToggle = useCallback((pluginId: string, enabled: boolean) => {
    setPlugins((prev) =>
      prev.map((p) => (p.id === pluginId ? { ...p, enabled } : p))
    );
  }, []);

  // Loading state
  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center py-16">
        <Loader2 className="h-5 w-5 animate-spin text-muted" />
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 py-16 text-center">
        <AlertCircle className="h-6 w-6 text-red-400" />
        <p className="text-sm text-muted">{error}</p>
        <button
          onClick={() => void load()}
          className="text-xs text-accent hover:underline"
        >
          {t("common.retry")}
        </button>
      </div>
    );
  }

  // Empty state
  if (plugins.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 py-16 text-center">
        <p className="text-sm font-medium text-secondary">{t("plugins.emptyTitle")}</p>
        <p className="text-[12px] text-muted">{t("plugins.emptyHint")}</p>
      </div>
    );
  }

  // Success state
  return (
    <div className="flex flex-col gap-1.5">
      {plugins.map((plugin) => (
        <PluginRow key={plugin.id} plugin={plugin} onToggle={handleToggle} />
      ))}
    </div>
  );
}
