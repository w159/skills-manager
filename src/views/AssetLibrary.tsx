import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { createPortal } from "react-dom";
import { useParams, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  Layers, Bot, Terminal, Anchor, FileCode2, BookOpen, GitBranch, Puzzle, Blocks,
  Loader2, AlertCircle, RefreshCw, Trash2, FolderOpen, X,
} from "lucide-react";
import { cn } from "../utils";
import { MySkills } from "./MySkills";
import { PluginsPanel } from "./PluginsPanel";
import { SyncDots } from "../components/SyncDots";
import { MultiSelectToolbar } from "../components/MultiSelectToolbar";
import { useApp } from "../context/AppContext";
import * as api from "../lib/tauri";
import type { AssetType, ManagedAsset, ManagedSkill, ImportCandidate } from "../lib/tauri";
import { getErrorMessage } from "../lib/error";

// ── Tab definitions ───────────────────────────────────────────────────────────

interface TabDef {
  type: AssetType;
  label: string;
  icon: React.ElementType;
}

const TABS: TabDef[] = [
  { type: "skill",    label: "Skills",    icon: Layers     },
  { type: "agent",    label: "Agents",    icon: Bot        },
  { type: "command",  label: "Commands",  icon: Terminal   },
  { type: "hook",     label: "Hooks",     icon: Anchor     },
  { type: "script",   label: "Scripts",   icon: FileCode2  },
  { type: "rule",     label: "Rules",     icon: BookOpen   },
  { type: "workflow", label: "Workflows", icon: GitBranch  },
  { type: "plugin",    label: "Plugins",    icon: Puzzle     },
  { type: "extension", label: "Extensions", icon: Blocks     },
];

const VALID_TYPES = new Set<string>(TABS.map((t) => t.type));

// ── Inline popover confirm (matches DeleteSkillButton pattern) ────────────────

interface RemovePopoverProps {
  assetName: string;
  onConfirm: () => void;
  onClose: () => void;
}

function RemovePopover({ assetName, onConfirm, onClose }: RemovePopoverProps) {
  const { t } = useTranslation();
  return (
    <div
      className="absolute right-0 top-full z-30 mt-1 w-72 rounded-lg border border-border bg-surface p-3 shadow-lg"
      onClick={(e) => e.stopPropagation()}
    >
      <p className="mb-3 text-[12px] leading-[16px] text-tertiary">
        {t("library.removeConfirm", { name: assetName })}
      </p>
      <div className="flex justify-end gap-2">
        <button
          onClick={(e) => { e.stopPropagation(); onClose(); }}
          className="rounded-[4px] px-2 py-1 text-[12px] font-medium text-tertiary transition-colors hover:bg-surface-hover hover:text-secondary outline-none"
        >
          {t("common.cancel")}
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); onConfirm(); }}
          className="rounded-[4px] border border-red-500/50 bg-red-600/90 px-2 py-1 text-[12px] font-medium text-white transition-colors hover:bg-red-500 outline-none"
        >
          {t("common.delete")}
        </button>
      </div>
    </div>
  );
}

// ── Non-skill asset row ───────────────────────────────────────────────────────

interface AssetRowProps {
  asset: ManagedAsset;
  onRemoved: () => void;
}

function AssetRow({ asset, onRemoved }: AssetRowProps) {
  const { t } = useTranslation();
  const [syncing, setSyncing] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close remove popover on outside click or Escape
  useEffect(() => {
    if (!removeOpen) return;
    const handlePointer = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setRemoveOpen(false);
      }
    };
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") setRemoveOpen(false); };
    document.addEventListener("mousedown", handlePointer);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handlePointer);
      document.removeEventListener("keydown", handleKey);
    };
  }, [removeOpen]);

  const handleSync = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setSyncing(true);
    try {
      const results = await api.deliverManagedAsset(asset.id);
      // Build per-adapter summary: "Claude: symlinked, Codex: rendered"
      const summary = results
        .map((r) => `${r.adapter_key}: ${r.outcome}`)
        .join(", ");
      toast.success(summary || t("library.syncDone"));
    } catch (e) {
      toast.error(getErrorMessage(e, t("common.error")));
    } finally {
      setSyncing(false);
    }
  };

  const handleRemoveConfirm = async () => {
    setRemoveOpen(false);
    try {
      // delete_managed_asset handles all asset types (row + central-repo copy).
      // The user's source workspace is never touched.
      await api.deleteManagedAsset(asset.id);
      toast.success(t("library.removeSuccess", { name: asset.name }));
      onRemoved();
    } catch (e) {
      toast.error(getErrorMessage(e, t("common.error")));
    }
  };

  // Derive the short display name from the plugin id: "my-plugin@market" -> "my-plugin"
  const pluginDisplayName = asset.owning_plugin?.split("@")[0] ?? null;

  return (
    <div className="flex items-center gap-3 rounded-[6px] border border-border-subtle bg-surface px-3.5 py-3 text-sm">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate font-medium text-primary">{asset.name}</span>
        {asset.description && (
          <span className="truncate text-[12px] text-muted">{asset.description}</span>
        )}
      </div>
      {pluginDisplayName && (
        <span
          className="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-medium text-primary"
          title={asset.owning_plugin ?? undefined}
        >
          {t("library.fromPlugin", { name: pluginDisplayName })}
        </span>
      )}
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

      {/* Sync / deliver button */}
      <button
        onClick={handleSync}
        disabled={syncing}
        title={t("library.syncTitle")}
        className="shrink-0 rounded text-faint transition-colors hover:text-secondary disabled:opacity-50 outline-none"
      >
        {syncing
          ? <Loader2 className="h-3.5 w-3.5 animate-spin" />
          : <RefreshCw className="h-3.5 w-3.5" />}
      </button>

      {/* Remove button with inline confirm popover */}
      <div ref={containerRef} className="relative shrink-0">
        <button
          onClick={(e) => { e.stopPropagation(); setRemoveOpen((v) => !v); }}
          title={t("library.removeTitle")}
          className={cn(
            "rounded text-faint transition-colors hover:text-red-400 outline-none",
            removeOpen && "text-red-400"
          )}
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
        {removeOpen && (
          <RemovePopover
            assetName={asset.name}
            onConfirm={() => void handleRemoveConfirm()}
            onClose={() => setRemoveOpen(false)}
          />
        )}
      </div>
    </div>
  );
}

// ── Agent asset row (extends AssetRow with per-tool sync dots) ────────────────

interface AgentAssetRowProps {
  asset: ManagedAsset;
  /** Full ManagedSkill record for this agent (carries targets[] for dot state). */
  skill: ManagedSkill | null;
  selected: boolean;
  isMultiSelect: boolean;
  pendingTool: string | null;
  onToggleTool: (tool: string, enabled: boolean) => void;
  onRemoved: () => void;
  onSelectToggle: () => void;
}

function AgentAssetRow({
  asset,
  skill,
  selected,
  isMultiSelect,
  pendingTool,
  onToggleTool,
  onRemoved,
  onSelectToggle,
}: AgentAssetRowProps) {
  const { t } = useTranslation();
  const { tools } = useApp();
  const [removeOpen, setRemoveOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!removeOpen) return;
    const handlePointer = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setRemoveOpen(false);
      }
    };
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") setRemoveOpen(false); };
    document.addEventListener("mousedown", handlePointer);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handlePointer);
      document.removeEventListener("keydown", handleKey);
    };
  }, [removeOpen]);

  const handleRemoveConfirm = async () => {
    setRemoveOpen(false);
    try {
      await api.deleteManagedAsset(asset.id);
      toast.success(t("library.removeSuccess", { name: asset.name }));
      onRemoved();
    } catch (e) {
      toast.error(getErrorMessage(e, t("common.error")));
    }
  };

  const pluginDisplayName = asset.owning_plugin?.split("@")[0] ?? null;

  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-[6px] border bg-surface px-3.5 py-3 text-sm transition-colors",
        selected ? "border-accent/50 bg-accent/5" : "border-border-subtle",
        isMultiSelect && "cursor-pointer hover:border-accent/30"
      )}
      onClick={isMultiSelect ? onSelectToggle : undefined}
    >
      {/* Multi-select checkbox */}
      {isMultiSelect && (
        <input
          type="checkbox"
          checked={selected}
          onChange={onSelectToggle}
          onClick={(e) => e.stopPropagation()}
          className="h-3.5 w-3.5 shrink-0 accent-primary"
        />
      )}

      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate font-medium text-primary">{asset.name}</span>
        {asset.description && (
          <span className="truncate text-[12px] text-muted">{asset.description}</span>
        )}
      </div>

      {pluginDisplayName && (
        <span
          className="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-medium text-primary"
          title={asset.owning_plugin ?? undefined}
        >
          {t("library.fromPlugin", { name: pluginDisplayName })}
        </span>
      )}

      {/* Per-tool sync dots - only when not in multi-select mode */}
      {skill && !isMultiSelect && (
        <SyncDots
          skill={skill}
          tools={tools}
          limit={6}
          onToggle={onToggleTool}
          pendingKey={pendingTool}
        />
      )}

      {/* Remove button with inline confirm popover */}
      {!isMultiSelect && (
        <div ref={containerRef} className="relative shrink-0">
          <button
            onClick={(e) => { e.stopPropagation(); setRemoveOpen((v) => !v); }}
            title={t("library.removeTitle")}
            className={cn(
              "rounded text-faint transition-colors hover:text-red-400 outline-none",
              removeOpen && "text-red-400"
            )}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
          {removeOpen && (
            <RemovePopover
              assetName={asset.name}
              onConfirm={() => void handleRemoveConfirm()}
              onClose={() => setRemoveOpen(false)}
            />
          )}
        </div>
      )}
    </div>
  );
}

// ── Import workspace sheet ────────────────────────────────────────────────────

interface ImportSheetProps {
  assetType: AssetType;
  open: boolean;
  onClose: () => void;
  onImported: () => void;
}

function ImportWorkspaceSheet({ assetType, open, onClose, onImported }: ImportSheetProps) {
  if (!open) return null;
  return createPortal(
    <ImportWorkspaceSheetBody
      assetType={assetType}
      onClose={onClose}
      onImported={onImported}
    />,
    document.body
  );
}

const IS_MACOS = navigator.userAgent.includes("Mac");

interface ImportSheetBodyProps {
  assetType: AssetType;
  onClose: () => void;
  onImported: () => void;
}

function ImportWorkspaceSheetBody({ assetType, onClose, onImported }: ImportSheetBodyProps) {
  const { t } = useTranslation();
  const [workspacePath, setWorkspacePath] = useState("");
  const [candidates, setCandidates] = useState<ImportCandidate[] | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);

  const handlePickFolder = async () => {
    try {
      const selected = await dialogOpen({ directory: true, multiple: false });
      if (!selected) return;
      setWorkspacePath(selected as string);
      // Auto-scan once a folder is picked
      void handleScan(selected as string);
    } catch (e) {
      toast.error(getErrorMessage(e, t("common.error")));
    }
  };

  const handleScan = async (path?: string) => {
    const target = path ?? workspacePath;
    if (!target.trim()) return;
    setScanning(true);
    setScanError(null);
    setCandidates(null);
    try {
      const result = await api.listImportCandidates(target.trim());
      // Filter to the current asset type
      const filtered = result.candidates.filter((c) => c.asset_type === assetType);
      setCandidates(filtered);
      // Default-check those already flagged in_active_set
      setChecked(new Set(filtered.filter((c) => c.in_active_set).map((c) => c.id_or_name)));
    } catch (e) {
      setScanError(getErrorMessage(e, t("common.error")));
    } finally {
      setScanning(false);
    }
  };

  const toggleChecked = (id: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) { next.delete(id); } else { next.add(id); }
      return next;
    });
  };

  const allChecked = candidates != null && candidates.length > 0 && candidates.every((c) => checked.has(c.id_or_name));
  const toggleAll = () => {
    if (!candidates) return;
    if (allChecked) {
      setChecked(new Set());
    } else {
      setChecked(new Set(candidates.map((c) => c.id_or_name)));
    }
  };

  const handleImport = async () => {
    if (!candidates) return;
    const selected = candidates.filter((c) => checked.has(c.id_or_name));
    if (selected.length === 0) return;
    setImporting(true);
    try {
      const result = await api.importSelectedAssets(selected);
      const ok = result.imported.length;
      const failed = result.errors.length;
      if (ok > 0) {
        toast.success(t("library.importSuccess", { count: ok }));
        onImported();
      }
      if (failed > 0) {
        toast.error(t("library.importErrors", { count: failed }));
      }
      if (failed === 0) onClose();
    } catch (e) {
      toast.error(getErrorMessage(e, t("common.error")));
    } finally {
      setImporting(false);
    }
  };

  const selectedCount = checked.size;

  return (
    <div className="fixed top-[28px] right-0 bottom-0 left-[220px] z-40 isolate">
      {/* Backdrop */}
      <div
        className={
          IS_MACOS
            ? "absolute inset-0 z-0 bg-black/65"
            : "absolute inset-0 z-0 bg-black/60 backdrop-blur-sm"
        }
        onClick={onClose}
      />
      {/* Panel */}
      <div className="absolute inset-0 z-10 flex min-h-0 flex-col overflow-hidden border-l border-border-subtle bg-bg-secondary">
        {/* Close */}
        <button
          onClick={onClose}
          className="absolute top-4 right-5 z-10 shrink-0 rounded-[4px] p-1.5 text-muted transition-colors outline-none hover:bg-surface-hover hover:text-secondary"
        >
          <X className="h-4 w-4" />
        </button>

        {/* Scrollable body */}
        <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-5 pb-6 scrollbar-hide">
          <h2 className="mb-1 min-w-0 pr-10 text-[28px] font-semibold leading-tight tracking-tight text-primary">
            {t("library.importTitle")}
          </h2>
          <p className="mb-5 text-[14px] text-secondary">
            {t("library.importDesc", { type: assetType })}
          </p>

          {/* Path row */}
          <div className="mb-4 flex gap-2">
            <input
              type="text"
              value={workspacePath}
              onChange={(e) => setWorkspacePath(e.target.value)}
              placeholder={t("library.importPathPlaceholder")}
              className="app-input min-w-0 flex-1"
              onKeyDown={(e) => { if (e.key === "Enter") void handleScan(); }}
            />
            <button
              type="button"
              onClick={() => void handlePickFolder()}
              className="app-button app-button-secondary shrink-0 flex items-center gap-1.5"
            >
              <FolderOpen className="h-3.5 w-3.5" />
              {t("library.importBrowse")}
            </button>
            <button
              type="button"
              onClick={() => void handleScan()}
              disabled={scanning || !workspacePath.trim()}
              className="app-button app-button-secondary shrink-0 disabled:opacity-50"
            >
              {scanning
                ? <Loader2 className="h-3.5 w-3.5 animate-spin" />
                : t("library.importScan")}
            </button>
          </div>

          {/* Scan error */}
          {scanError && (
            <div className="mb-4 flex items-center gap-2 rounded-[6px] border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-400">
              <AlertCircle className="h-4 w-4 shrink-0" />
              {scanError}
            </div>
          )}

          {/* Candidate list */}
          {scanning && (
            <div className="flex items-center justify-center py-12">
              <Loader2 className="h-5 w-5 animate-spin text-muted" />
            </div>
          )}

          {!scanning && candidates !== null && candidates.length === 0 && (
            <div className="flex flex-col items-center justify-center gap-2 py-12 text-center">
              <p className="text-sm font-medium text-secondary">{t("library.importEmpty")}</p>
              <p className="text-[12px] text-muted">{t("library.importEmptyHint")}</p>
            </div>
          )}

          {!scanning && candidates !== null && candidates.length > 0 && (
            <>
              {/* Select all toggle */}
              <div className="mb-2 flex items-center justify-between">
                <span className="text-[12px] text-muted">
                  {t("library.importFound", { count: candidates.length })}
                </span>
                <button
                  type="button"
                  onClick={toggleAll}
                  className="text-[12px] text-muted transition-colors hover:text-secondary outline-none"
                >
                  {allChecked ? t("project.deselectAll") : t("project.selectAll")}
                </button>
              </div>

              <div className="flex flex-col gap-1.5">
                {candidates.map((c) => (
                  <label
                    key={c.id_or_name}
                    className="flex cursor-pointer items-center gap-3 rounded-[6px] border border-border-subtle bg-surface px-3.5 py-2.5 text-sm transition-colors hover:bg-surface-hover"
                  >
                    <input
                      type="checkbox"
                      className="h-3.5 w-3.5 shrink-0 accent-primary"
                      checked={checked.has(c.id_or_name)}
                      onChange={() => toggleChecked(c.id_or_name)}
                    />
                    <div className="min-w-0 flex-1">
                      <span className="block truncate font-medium text-primary">
                        {c.display_name ?? c.id_or_name}
                      </span>
                      {c.description && (
                        <span className="block truncate text-[12px] text-muted">{c.description}</span>
                      )}
                    </div>
                    {c.in_active_set && (
                      <span className="shrink-0 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[11px] font-medium text-emerald-500">
                        {t("library.importActive")}
                      </span>
                    )}
                  </label>
                ))}
              </div>
            </>
          )}
        </div>

        {/* Footer */}
        {candidates !== null && candidates.length > 0 && (
          <div className="shrink-0 border-t border-border-subtle px-6 py-3 flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="app-button app-button-secondary"
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              onClick={() => void handleImport()}
              disabled={importing || selectedCount === 0}
              className="app-button app-button-primary flex items-center gap-1.5 disabled:opacity-50"
            >
              {importing && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {t("library.importConfirm", { count: selectedCount })}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Non-skill tab panel (loading / error / empty / list) ─────────────────────

interface AssetTabPanelProps {
  assetType: AssetType;
}

function AssetTabPanel({ assetType }: AssetTabPanelProps) {
  const { t } = useTranslation();
  const { managedSkills, tools, refreshManagedSkills } = useApp();
  const [assets, setAssets] = useState<ManagedAsset[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  // Filter: when false, rows with owning_plugin are hidden.
  const [showPluginOwned, setShowPluginOwned] = useState(true);

  // Agent multi-select state
  const [isMultiSelect, setIsMultiSelect] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  // Per-row pending tool key for optimistic loading dots
  const [pendingToggle, setPendingToggle] = useState<{ agentId: string; tool: string } | null>(null);

  const isAgent = assetType === "agent";

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

  // Reset multi-select when switching tabs
  useEffect(() => {
    setIsMultiSelect(false);
    setSelectedIds(new Set());
  }, [assetType]);

  const handleImported = useCallback(() => { void load(); }, [load]);

  // Whether any asset in this list carries plugin attribution.
  const hasPluginOwned = useMemo(() => assets.some((a) => a.owning_plugin != null), [assets]);

  // Apply the filter only when the toggle is off.
  const visibleAssets = useMemo(
    () => (showPluginOwned ? assets : assets.filter((a) => a.owning_plugin == null)),
    [assets, showPluginOwned]
  );

  // Build a lookup from asset id -> ManagedSkill (which carries targets[])
  const skillById = useMemo(() => {
    const map = new Map<string, ManagedSkill>();
    for (const s of managedSkills) {
      map.set(s.id, s);
    }
    return map;
  }, [managedSkills]);

  // Single-agent per-tool toggle (mirrors MySkills handleToggleSkillTarget)
  const handleToggleTool = useCallback(
    async (agentId: string, toolKey: string, enabled: boolean) => {
      if (pendingToggle) return;
      setPendingToggle({ agentId, tool: toolKey });
      try {
        if (enabled) {
          await api.syncSkillToTool(agentId, toolKey);
        } else {
          await api.unsyncSkillFromTool(agentId, toolKey);
        }
        await refreshManagedSkills();
      } catch (e) {
        toast.error(getErrorMessage(e, t("common.error")));
        await refreshManagedSkills();
      } finally {
        setPendingToggle(null);
      }
    },
    [pendingToggle, refreshManagedSkills, t]
  );

  // Batch sync/unsync for multi-selected agents (mirrors handleBatchTogglePreset pattern)
  const handleBatchSync = useCallback(
    async (enable: boolean) => {
      const ids = Array.from(selectedIds);
      if (ids.length === 0) return;
      const toolKeys = tools
        .filter((tool) => tool.installed && tool.enabled)
        .map((tool) => tool.key);
      if (toolKeys.length === 0) return;
      try {
        if (enable) {
          await api.batchSyncSkillsToTools(ids, toolKeys);
        } else {
          await api.batchUnsyncSkillsFromTools(ids, toolKeys);
        }
        await refreshManagedSkills();
      } catch (e) {
        toast.error(getErrorMessage(e, t("common.error")));
        await refreshManagedSkills();
      } finally {
        setIsMultiSelect(false);
        setSelectedIds(new Set());
      }
    },
    [selectedIds, tools, refreshManagedSkills, t]
  );

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) { next.delete(id); } else { next.add(id); }
      return next;
    });
  }, []);

  const selectAll = useCallback(() => {
    setSelectedIds(new Set(visibleAssets.map((a) => a.id)));
  }, [visibleAssets]);

  const exitMultiSelect = useCallback(() => {
    setIsMultiSelect(false);
    setSelectedIds(new Set());
  }, []);

  // Whether any selected agent has at least one synced target (used for anyDisabled toggle label)
  const anyUnsynced = useMemo(() => {
    for (const id of selectedIds) {
      const skill = skillById.get(id);
      if (!skill || skill.targets.length === 0) return true;
    }
    return false;
  }, [selectedIds, skillById]);

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

  return (
    <>
      {/* Toolbar: import button + optional plugin-owned toggle + agent multi-select toggle */}
      <div className="mb-3 flex items-center justify-between gap-2">
        {hasPluginOwned && (
          <label className="flex cursor-pointer items-center gap-1.5 text-[12px] text-muted select-none">
            <input
              type="checkbox"
              className="h-3 w-3 accent-primary"
              checked={showPluginOwned}
              onChange={(e) => setShowPluginOwned(e.target.checked)}
            />
            {t("library.showPluginOwned")}
          </label>
        )}
        <div className="ml-auto flex items-center gap-2">
          {isAgent && visibleAssets.length > 0 && (
            <button
              type="button"
              onClick={() => (isMultiSelect ? exitMultiSelect() : setIsMultiSelect(true))}
              className={cn(
                "rounded px-2 py-1 text-[12px] font-medium transition-colors outline-none",
                isMultiSelect
                  ? "bg-surface-active text-secondary"
                  : "text-muted hover:text-tertiary"
              )}
              title={isMultiSelect ? t("mySkills.cancelSelect") : t("mySkills.selectMode")}
            >
              {isMultiSelect ? t("mySkills.cancelSelect") : t("mySkills.selectMode")}
            </button>
          )}
          <button
            type="button"
            onClick={() => setImportOpen(true)}
            className="app-button app-button-secondary flex items-center gap-1.5 text-sm"
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("library.importFromWorkspace")}
          </button>
        </div>
      </div>

      {/* Agent multi-select toolbar */}
      {isAgent && isMultiSelect && (
        <MultiSelectToolbar
          selectedCount={selectedIds.size}
          isAllSelected={selectedIds.size === visibleAssets.length}
          anyDisabled={anyUnsynced}
          showToggle={selectedIds.size > 0}
          labels={{
            hint: t("mySkills.selectHint"),
            selected: t("mySkills.selectedCount", { count: selectedIds.size }),
            selectAll: t("mySkills.selectAll"),
            deselectAll: t("mySkills.deselectAll"),
            delete: t("mySkills.deleteSelected", { count: selectedIds.size }),
            enable: t("mySkills.batchEnable", { count: selectedIds.size }),
            disable: t("mySkills.batchDisable", { count: selectedIds.size }),
            cancel: t("common.cancel"),
          }}
          onToggle={() => void handleBatchSync(anyUnsynced)}
          onSelectAll={selectAll}
          onDelete={() => {
            // Bulk delete not wired for agents yet - exit select mode
            exitMultiSelect();
          }}
          onCancel={exitMultiSelect}
        />
      )}

      {visibleAssets.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 py-16 text-center">
          <p className="text-sm font-medium text-secondary">
            {t("library.emptyTitle", { type: assetType })}
          </p>
          <p className="text-[12px] text-muted">{t("library.emptyHint")}</p>
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {visibleAssets.map((asset) =>
            isAgent ? (
              <AgentAssetRow
                key={asset.id}
                asset={asset}
                skill={skillById.get(asset.id) ?? null}
                selected={selectedIds.has(asset.id)}
                isMultiSelect={isMultiSelect}
                pendingTool={
                  pendingToggle?.agentId === asset.id ? pendingToggle.tool : null
                }
                onToggleTool={(tool, enabled) => void handleToggleTool(asset.id, tool, enabled)}
                onRemoved={() => { void load(); void refreshManagedSkills(); }}
                onSelectToggle={() => toggleSelected(asset.id)}
              />
            ) : (
              <AssetRow key={asset.id} asset={asset} onRemoved={() => void load()} />
            )
          )}
        </div>
      )}

      <ImportWorkspaceSheet
        assetType={assetType}
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={handleImported}
      />
    </>
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

  // Plugins have their own dedicated panel (different data shape from generic assets).
  if (activeType === "plugin") {
    return (
      <div className="app-page">
        <TabBar activeType={activeType} onSelect={handleTabClick} />
        <PluginsPanel />
      </div>
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
