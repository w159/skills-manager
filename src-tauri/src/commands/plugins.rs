/// Tauri command surface for read-only plugin discovery.
///
/// `list_installed_plugins` — enumerate every installed plugin found in
/// `~/.claude/plugins`, resolving each plugin's manifest and bundled assets.
///
/// This module is intentionally separate from the asset delivery / sync
/// engine: plugins are containers discovered from disk, not managed records
/// in the SkillStore.  Nothing here writes to disk.
use serde::Serialize;
use std::path::PathBuf;

use crate::core::{
    error::AppError,
    plugin_discovery::{self, BundledAsset, Plugin},
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Serialisable representation of a single bundled asset inside a plugin.
#[derive(Debug, Serialize)]
pub struct BundledAssetDto {
    /// `"skill"`, `"agent"`, `"command"`, `"hook"`, or `"mcp"`.
    pub asset_type: String,
    /// Leaf name of the asset (directory name or mcp server key).
    pub name: String,
    /// Absolute path on disk.
    pub path: String,
}

impl From<BundledAsset> for BundledAssetDto {
    fn from(a: BundledAsset) -> Self {
        BundledAssetDto {
            asset_type: a.asset_type.as_str().to_string(),
            name: a.name,
            path: a.path.to_string_lossy().into_owned(),
        }
    }
}

/// Serialisable summary of one installed plugin.
#[derive(Debug, Serialize)]
pub struct PluginDto {
    /// `"name@marketplace"` canonical id.
    pub id: String,
    pub name: String,
    pub marketplace: String,
    pub version: String,
    pub install_path: String,
    pub description: Option<String>,
    /// Source URL or repo string resolved from `known_marketplaces.json`.
    pub source: Option<String>,
    pub blocked: bool,
    pub assets: Vec<BundledAssetDto>,
}

impl From<Plugin> for PluginDto {
    fn from(p: Plugin) -> Self {
        PluginDto {
            id: p.id,
            name: p.name,
            marketplace: p.marketplace,
            version: p.version,
            install_path: p.install_path.to_string_lossy().into_owned(),
            description: p.description,
            source: p.source,
            blocked: p.blocked,
            assets: p.assets.into_iter().map(BundledAssetDto::from).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Path resolver
// ---------------------------------------------------------------------------

/// Resolve the plugin root path: `~/.claude/plugins`.
///
/// Uses the same home-directory strategy as `ToolAdapter::home()` in
/// `core/tool_adapters.rs` -- `dirs::home_dir()` resolved at runtime.
fn resolve_plugin_root() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir().ok_or_else(|| {
        AppError::internal("cannot determine home directory for plugin root resolution")
    })?;
    Ok(home.join(".claude").join("plugins"))
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// Return all installed plugins discovered in `~/.claude/plugins`.
///
/// The function is non-blocking (I/O is dispatched to the blocking thread
/// pool) and never returns a hard error: any plugin whose manifest is absent
/// or malformed is silently skipped by the discovery layer.
#[tauri::command]
pub async fn list_installed_plugins() -> Result<Vec<PluginDto>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let plugin_root = resolve_plugin_root()?;
        let plugins = plugin_discovery::list_plugins(&plugin_root);
        Ok(plugins.into_iter().map(PluginDto::from).collect())
    })
    .await?
}
