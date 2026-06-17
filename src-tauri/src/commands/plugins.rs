/// Tauri command surface for plugin discovery and enablement.
///
/// `list_installed_plugins` — enumerate every installed plugin found in
/// `~/.claude/plugins`, resolving each plugin's manifest and bundled assets.
/// Each plugin also carries an `enabled` flag read from `~/.claude/settings.json`.
///
/// `set_plugin_enabled` — toggle the `enabledPlugins` key in
/// `~/.claude/settings.json` for a single plugin.  The write is a
/// structure-preserving read-modify-write: every other top-level key in the
/// file is left untouched.  A `.bak-<ts>` copy is written before any write.
///
/// This module is intentionally separate from the asset delivery / sync
/// engine: plugins are containers discovered from disk, not managed records
/// in the SkillStore.  The only disk writes here target settings.json.
use serde::Serialize;
use std::path::{Path, PathBuf};

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
    /// Whether the plugin is enabled in `~/.claude/settings.json`.
    ///
    /// Semantics: absent key in `enabledPlugins` means **enabled** (opt-out
    /// model), matching Claude Code's own behaviour where a freshly installed
    /// plugin is active until the user explicitly disables it.
    pub enabled: bool,
    pub assets: Vec<BundledAssetDto>,
}

// ---------------------------------------------------------------------------
// Path resolvers
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

/// Resolve `~/.claude/settings.json`.
fn resolve_settings_path() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir().ok_or_else(|| {
        AppError::internal("cannot determine home directory for settings.json resolution")
    })?;
    Ok(home.join(".claude").join("settings.json"))
}

// ---------------------------------------------------------------------------
// settings.json read helpers
// ---------------------------------------------------------------------------

/// Read `enabledPlugins` from a settings.json `Value` and return whether
/// the given `plugin_id` is enabled.
///
/// Absent key -> `true` (opt-out default: installed = enabled).
fn read_plugin_enabled(settings: &serde_json::Value, plugin_id: &str) -> bool {
    settings
        .get("enabledPlugins")
        .and_then(|ep| ep.get(plugin_id))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Load and parse `settings.json` if it exists.  Returns `None` when the
/// file is absent (all plugins default to enabled).  Returns an error when
/// the file exists but is not valid JSON (caller must not silently ignore a
/// corrupt settings file).
fn load_settings_value(settings_path: &Path) -> Result<Option<serde_json::Value>, AppError> {
    if !settings_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(settings_path).map_err(AppError::io)?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| AppError::invalid_input(e.to_string()))?;
    Ok(Some(value))
}

// ---------------------------------------------------------------------------
// settings.json write -- testable inner function
// ---------------------------------------------------------------------------

/// Structure-preserving read-modify-write for `enabledPlugins` in a
/// settings.json file at `settings_path`.
///
/// Contract:
/// - If the file is missing or not valid JSON: return an error; do NOT write.
/// - Back up the file to `<settings_path>.bak-<unix_ms>` before writing.
/// - Set `value["enabledPlugins"][plugin_id] = enabled`.
/// - Write back with `serde_json::to_string_pretty`; every other key is kept.
///
/// Accepts an explicit `settings_path` so unit tests can point it at a
/// temp file instead of `~/.claude/settings.json`.
pub(crate) fn toggle_plugin_enabled_in_file(
    settings_path: &Path,
    plugin_id: &str,
    enabled: bool,
) -> Result<(), AppError> {
    // --- Read and parse (error if missing or corrupt) ---
    if !settings_path.exists() {
        return Err(AppError::not_found(
            "settings.json not found; cannot modify plugin enablement",
        ));
    }
    let raw = std::fs::read_to_string(settings_path).map_err(AppError::io)?;
    let mut root: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| AppError::invalid_input(e.to_string()))?;

    // --- Backup before mutating ---
    let ts = chrono::Utc::now().timestamp_millis();
    let bak_path = settings_path.with_file_name(format!(
        "{}.bak-{}",
        settings_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        ts
    ));
    std::fs::copy(settings_path, &bak_path).map_err(AppError::io)?;

    // --- Mutate the enabledPlugins key only ---
    let ep = root
        .as_object_mut()
        .ok_or_else(|| AppError::invalid_input("settings.json root is not a JSON object"))?
        .entry("enabledPlugins")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

    ep.as_object_mut()
        .ok_or_else(|| AppError::invalid_input("enabledPlugins is not a JSON object"))?
        .insert(plugin_id.to_string(), serde_json::Value::Bool(enabled));

    // --- Write back (pretty-printed) ---
    let new_raw =
        serde_json::to_string_pretty(&root).map_err(|e| AppError::internal(e.to_string()))?;
    std::fs::write(settings_path, new_raw).map_err(AppError::io)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Return all installed plugins discovered in `~/.claude/plugins`.
///
/// Each plugin's `enabled` field is read from `~/.claude/settings.json`
/// (`enabledPlugins[id]`).  An absent key means enabled (opt-out default).
///
/// The function is non-blocking (I/O is dispatched to the blocking thread
/// pool) and never returns a hard error: any plugin whose manifest is absent
/// or malformed is silently skipped by the discovery layer.
#[tauri::command]
pub async fn list_installed_plugins() -> Result<Vec<PluginDto>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let plugin_root = resolve_plugin_root()?;
        let settings_path = resolve_settings_path()?;
        let settings = load_settings_value(&settings_path)?;

        let plugins = plugin_discovery::list_plugins(&plugin_root);
        let dtos = plugins
            .into_iter()
            .map(|p| {
                let enabled = settings
                    .as_ref()
                    .map(|s| read_plugin_enabled(s, &p.id))
                    .unwrap_or(true);
                PluginDto {
                    id: p.id,
                    name: p.name,
                    marketplace: p.marketplace,
                    version: p.version,
                    install_path: p.install_path.to_string_lossy().into_owned(),
                    description: p.description,
                    source: p.source,
                    blocked: p.blocked,
                    enabled,
                    assets: p.assets.into_iter().map(BundledAssetDto::from).collect(),
                }
            })
            .collect();
        Ok(dtos)
    })
    .await?
}

/// Enable or disable a plugin by writing to `~/.claude/settings.json`.
///
/// The write is structure-preserving: only `enabledPlugins[plugin_id]` is
/// touched; all other keys are carried through unchanged.  A `.bak-<ts>`
/// copy is created before writing.  If `settings.json` is missing or
/// unparseable, the command returns an error without writing anything.
///
/// Changes take effect the next time an agent session loads.
#[tauri::command]
pub async fn set_plugin_enabled(plugin_id: String, enabled: bool) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings_path = resolve_settings_path()?;
        toggle_plugin_enabled_in_file(&settings_path, &plugin_id, enabled)
    })
    .await?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Seed a settings.json with a mix of keys including pre-existing
    /// enabledPlugins, permissions, and hooks to confirm preservation.
    fn write_seed_settings(dir: &TempDir, content: &str) -> PathBuf {
        let path = dir.path().join("settings.json");
        fs::write(&path, content).unwrap();
        path
    }

    // ── test 1: flip a known plugin to false; verify value + key preservation ──

    #[test]
    fn toggle_existing_plugin_to_disabled_preserves_other_keys() {
        let dir = TempDir::new().unwrap();
        let seed = r#"{
  "enabledPlugins": {
    "context-mode@context-mode": true,
    "other-plugin@market": true
  },
  "permissions": {
    "allow": ["Bash(git:*)"],
    "deny": []
  },
  "hooks": {
    "PreToolUse": []
  }
}"#;
        let path = write_seed_settings(&dir, seed);

        toggle_plugin_enabled_in_file(&path, "context-mode@context-mode", false).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();

        // The toggled plugin is now false.
        assert_eq!(
            v["enabledPlugins"]["context-mode@context-mode"],
            serde_json::Value::Bool(false),
            "toggled plugin should be false"
        );
        // The other plugin key is untouched.
        assert_eq!(
            v["enabledPlugins"]["other-plugin@market"],
            serde_json::Value::Bool(true),
            "other enabledPlugins key should be preserved"
        );
        // permissions and hooks survive unchanged.
        assert_eq!(
            v["permissions"]["allow"][0],
            serde_json::json!("Bash(git:*)"),
            "permissions.allow must survive"
        );
        assert!(
            v["hooks"]["PreToolUse"].is_array(),
            "hooks.PreToolUse must survive"
        );

        // A .bak file must have been created.
        let bak_count = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("settings.json.bak-")
            })
            .count();
        assert_eq!(bak_count, 1, "exactly one .bak file must exist");
    }

    // ── test 2: plugin id not previously present is added ──

    #[test]
    fn toggle_absent_plugin_id_adds_it() {
        let dir = TempDir::new().unwrap();
        let seed = r#"{
  "enabledPlugins": {},
  "permissions": { "allow": [], "deny": [] }
}"#;
        let path = write_seed_settings(&dir, seed);

        toggle_plugin_enabled_in_file(&path, "brand-new@market", false).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(
            v["enabledPlugins"]["brand-new@market"],
            serde_json::Value::Bool(false),
            "new plugin key should be inserted as false"
        );
        // permissions key still there.
        assert!(
            v["permissions"].is_object(),
            "permissions object must survive"
        );
    }

    // ── test 3: enabledPlugins object created when absent ──

    #[test]
    fn toggle_creates_enabled_plugins_key_when_absent() {
        let dir = TempDir::new().unwrap();
        // Settings file with no enabledPlugins key at all.
        let seed = r#"{
  "permissions": { "allow": [], "deny": [] },
  "hooks": {}
}"#;
        let path = write_seed_settings(&dir, seed);

        toggle_plugin_enabled_in_file(&path, "my-plugin@my-market", true).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(
            v["enabledPlugins"]["my-plugin@my-market"],
            serde_json::Value::Bool(true),
            "enabledPlugins should be created with the new entry"
        );
        // Other keys intact.
        assert!(v["permissions"].is_object());
        assert!(v["hooks"].is_object());
    }

    // ── test 4: missing file -> error, no write ──

    #[test]
    fn toggle_missing_file_returns_error_without_writing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("settings.json"); // does not exist

        let result = toggle_plugin_enabled_in_file(&path, "any@market", false);

        assert!(result.is_err(), "should return error when file is missing");
        // No bak file should have been created either.
        let bak_count = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("settings.json.bak-")
            })
            .count();
        assert_eq!(bak_count, 0, "no .bak should be created when file is missing");
    }

    // ── test 5: unparseable JSON -> error, no write ──

    #[test]
    fn toggle_unparseable_json_returns_error_without_writing() {
        let dir = TempDir::new().unwrap();
        let path = write_seed_settings(&dir, "{ this is not json }");

        let result = toggle_plugin_enabled_in_file(&path, "any@market", false);

        assert!(result.is_err(), "should return error when JSON is invalid");
        // The original bad content should be unchanged (no partial write).
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(after, "{ this is not json }");
    }

    // ── test 6: read_plugin_enabled defaults ──

    #[test]
    fn read_plugin_enabled_defaults_to_true_when_key_absent() {
        let s = serde_json::json!({ "enabledPlugins": {} });
        assert!(read_plugin_enabled(&s, "missing@market"));
    }

    #[test]
    fn read_plugin_enabled_respects_explicit_false() {
        let s = serde_json::json!({ "enabledPlugins": { "p@m": false } });
        assert!(!read_plugin_enabled(&s, "p@m"));
    }

    #[test]
    fn read_plugin_enabled_defaults_when_no_enabled_plugins_key() {
        let s = serde_json::json!({ "permissions": {} });
        assert!(read_plugin_enabled(&s, "p@m"));
    }

    // ── test 7: preserve_order — top-level key order must survive a toggle ──
    //
    // Without the `preserve_order` feature serde_json::Map is BTreeMap-backed
    // and serialisation sorts keys alphabetically, turning every plugin toggle
    // into a large disruptive diff on the user's hand-maintained settings.json.
    // With `preserve_order` the backing store is IndexMap and insertion order
    // is preserved on the round-trip.  This test locks that invariant.
    #[test]
    fn toggle_preserves_top_level_key_order() {
        let dir = TempDir::new().unwrap();
        // Keys are in a deliberately NON-alphabetical order:
        //   mcpServers -> permissions -> enabledPlugins -> hooks
        // Alphabetical order would be:
        //   enabledPlugins -> hooks -> mcpServers -> permissions
        // If preserve_order is not active the written file will be sorted and
        // the order assertion below will fail.
        let seed = r#"{
  "mcpServers": {},
  "permissions": {
    "allow": ["Bash(git:*)"],
    "deny": []
  },
  "enabledPlugins": {
    "existing-plugin@market": true
  },
  "hooks": {
    "PreToolUse": []
  }
}"#;
        let path = write_seed_settings(&dir, seed);

        toggle_plugin_enabled_in_file(&path, "new-plugin@market", false).unwrap();

        // --- value correctness ---
        let raw = fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v["enabledPlugins"]["new-plugin@market"],
            serde_json::Value::Bool(false),
            "toggled plugin should be false"
        );
        assert_eq!(
            v["enabledPlugins"]["existing-plugin@market"],
            serde_json::Value::Bool(true),
            "pre-existing plugin key must be unchanged"
        );

        // --- key-order preservation ---
        // Extract top-level key names in the order they appear in the raw text.
        let keys: Vec<&str> = raw
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                // Match lines like `  "keyName": ...`
                if trimmed.starts_with('"') {
                    trimmed.split('"').nth(1)
                } else {
                    None
                }
            })
            // Only keep the four top-level keys we seeded (skip nested keys).
            .filter(|k| matches!(*k, "mcpServers" | "permissions" | "enabledPlugins" | "hooks"))
            // Each top-level key appears exactly once at depth 0; dedup keeps
            // first occurrence so nested keys with the same name are ignored.
            .collect::<Vec<_>>()
            .into_iter()
            .fold(Vec::new(), |mut acc, k| {
                if !acc.contains(&k) {
                    acc.push(k);
                }
                acc
            });

        assert_eq!(
            keys,
            vec!["mcpServers", "permissions", "enabledPlugins", "hooks"],
            "top-level key order must match the original file (preserve_order required)"
        );
    }
}
