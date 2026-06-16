import { useState, useEffect, useCallback } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Layers, Bot, Terminal, Anchor, FileCode2, BookOpen, Loader2, AlertCircle } from "lucide-react";
import { cn } from "../utils";
import { MySkills } from "./MySkills";
import * as api from "../lib/tauri";
import type { AssetType, ManagedAsset } from "../lib/tauri";

// ── Tab definitions ───────────────────────────────────────────────────────────

interface TabDef {
  type: AssetType;
  label: string;
  icon: React.ElementType;
}

const TABS: TabDef[] = [
  { type: "skill",   label: "Skills",   icon: Layers     },
  { type: "agent",   label: "Agents",   icon: Bot        },
  { type: "command", label: "Commands", icon: Terminal   },
  { type: "hook",    label: "Hooks",    icon: Anchor     },
  { type: "script",  label: "Scripts",  icon: FileCode2  },
  { type: "rule",    label: "Rules",    icon: BookOpen   },
];

const VALID_TYPES = new Set<string>(TABS.map((t) => t.type));

// ── Non-skill asset row ───────────────────────────────────────────────────────

interface AssetRowProps {
  asset: ManagedAsset;
}

function AssetRow({ asset }: AssetRowProps) {
  return (
    <div className="flex items-center gap-3 rounded-[6px] border border-border-subtle bg-surface px-3.5 py-3 text-sm">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate font-medium text-primary">{asset.name}</span>
        {asset.description && (
          <span className="truncate text-[12px] text-muted">{asset.description}</span>
        )}
      </div>
      <span
        className={cn(
          "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium",
          asset.enabled
            ? "bg-emerald-500/10 text-emerald-500"
            : "bg-neutral-500/10 text-muted"
        )}
      >
        {asset.enabled ? "enabled" : "disabled"}
      </span>
      <span className="shrink-0 text-[11px] text-faint">{asset.status}</span>
    </div>
  );
}

// ── Non-skill tab panel (loading / error / empty / list) ─────────────────────

interface AssetTabPanelProps {
  assetType: AssetType;
}

function AssetTabPanel({ assetType }: AssetTabPanelProps) {
  const { t } = useTranslation();
  const [assets, setAssets] = useState<ManagedAsset[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getManagedAssets(assetType);
      setAssets(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : t("common.error"));
    } finally {
      setLoading(false);
    }
  }, [assetType, t]);

  useEffect(() => {
    void load();
  }, [load]);

  if (loading) {
    return (
      <div className="flex flex-1 items-center justify-center py-16">
        <Loader2 className="h-5 w-5 animate-spin text-muted" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 py-16 text-center">
        <AlertCircle className="h-6 w-6 text-red-400" />
        <p className="text-sm text-muted">{error}</p>
        <button
          onClick={() => void load()}
          className="app-button app-button-secondary text-sm"
        >
          {t("common.retry")}
        </button>
      </div>
    );
  }

  if (assets.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 py-16 text-center">
        <p className="text-sm font-medium text-secondary">
          {t("library.emptyTitle", { type: assetType })}
        </p>
        <p className="text-[12px] text-muted">{t("library.emptyHint")}</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      {assets.map((asset) => (
        <AssetRow key={asset.id} asset={asset} />
      ))}
    </div>
  );
}

// ── Tab bar (shared across all tabs) ─────────────────────────────────────────

interface TabBarProps {
  activeType: AssetType;
  onSelect: (type: AssetType) => void;
}

function TabBar({ activeType, onSelect }: TabBarProps) {
  return (
    <div className="app-segmented mb-4 self-start">
      {TABS.map(({ type, label, icon: Icon }) => (
        <button
          key={type}
          onClick={() => onSelect(type)}
          className={cn(
            "app-segmented-button flex items-center gap-1.5",
            activeType === type && "app-segmented-button-active"
          )}
        >
          <Icon className="h-3.5 w-3.5 shrink-0" />
          {label}
        </button>
      ))}
    </div>
  );
}

// ── Main view ─────────────────────────────────────────────────────────────────

export function AssetLibrary() {
  const { assetType } = useParams<{ assetType: string }>();
  const navigate = useNavigate();

  // Fall back to "skill" for any unknown param value.
  const activeType: AssetType = VALID_TYPES.has(assetType ?? "") ? (assetType as AssetType) : "skill";

  const handleTabClick = (type: AssetType) => {
    navigate(`/library/${type}`, { replace: true });
  };

  // For the "skill" tab, MySkills owns its own app-page wrapper and header.
  // Render the tab bar inside a lightweight container above it.
  if (activeType === "skill") {
    return (
      <>
        <div className="px-0 pb-0">
          <TabBar activeType={activeType} onSelect={handleTabClick} />
        </div>
        <MySkills />
      </>
    );
  }

  // For all other types, wrap in app-page to get consistent page padding.
  return (
    <div className="app-page">
      <TabBar activeType={activeType} onSelect={handleTabClick} />
      <AssetTabPanel assetType={activeType} />
    </div>
  );
}
