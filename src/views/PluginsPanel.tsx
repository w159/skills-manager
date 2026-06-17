import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, AlertCircle, ChevronRight, ChevronDown, ShieldOff } from "lucide-react";
import { cn } from "../utils";
import { listInstalledPlugins } from "../lib/tauri";
import type { Plugin, BundledAsset } from "../lib/tauri";
import { getErrorMessage } from "../lib/error";

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

// ── Single plugin row with expandable asset drill-down ────────────────────────

function PluginRow({ plugin }: { plugin: Plugin }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  const grouped = groupByType(plugin.assets);
  const groupEntries = sortedGroupEntries(grouped);
  const ChevronIcon = open ? ChevronDown : ChevronRight;

  return (
    <div className="rounded-lg border border-border bg-surface">
      {/* Header row */}
      <button
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex w-full items-start gap-3 p-3 text-left transition-colors hover:bg-surface-hover",
          open && "rounded-t-lg"
        )}
      >
        <ChevronIcon className="mt-0.5 h-4 w-4 shrink-0 text-muted" />

        <div className="min-w-0 flex-1">
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
        </div>
      </button>

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
        <PluginRow key={plugin.id} plugin={plugin} />
      ))}
    </div>
  );
}
