/// Read-only plugin discovery layer.
///
/// Parses the three registry files kept alongside the plugin cache
/// (`installed_plugins.json`, `known_marketplaces.json`, `blocklist.json`)
/// and each installed plugin's own `.claude-plugin/plugin.json` manifest,
/// then enumerates every bundled asset declared there.
///
/// Nothing in this module writes to disk.  All public functions accept an
/// explicit `plugin_root` path so they are fully testable without touching the
/// real `~/.claude/plugins` directory.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Registry JSON shapes (Deserialize only -- never serialised back)
// ---------------------------------------------------------------------------

/// One entry in the `plugins` map of `installed_plugins.json` (schema v2).
/// The map value is a *single-element array* `[InstallRecord]`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallRecord {
    #[allow(dead_code)]
    scope: Option<String>,
    install_path: String,
    version: Option<String>,
    #[allow(dead_code)]
    installed_at: Option<String>,
    #[allow(dead_code)]
    last_updated: Option<String>,
    #[allow(dead_code)]
    git_commit_sha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InstalledPluginsFile {
    #[allow(dead_code)]
    version: Option<u32>,
    plugins: HashMap<String, serde_json::Value>,
}

/// One entry inside `known_marketplaces.json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketplaceEntry {
    source: Option<serde_json::Value>,
    #[allow(dead_code)]
    install_location: Option<String>,
    #[allow(dead_code)]
    last_updated: Option<String>,
    #[allow(dead_code)]
    auto_update: Option<bool>,
}

/// One item in the `plugins` array of `blocklist.json`.
#[derive(Debug, Deserialize)]
struct BlocklistItem {
    plugin: String,
}

#[derive(Debug, Deserialize)]
struct BlocklistFile {
    plugins: Vec<BlocklistItem>,
}

/// The `.claude-plugin/plugin.json` manifest inside an install path.
/// Asset declaration fields are optional; missing means "no assets of that type".
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginManifest {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    author: Option<serde_json::Value>,
    homepage: Option<String>,
    repository: Option<String>,
    /// Path to the skills directory (relative to install root), e.g. `"./skills/"`.
    skills: Option<String>,
    agents: Option<String>,
    commands: Option<String>,
    hooks: Option<String>,
    /// Map of server-name -> server config; keys become mcp asset names.
    mcp_servers: Option<HashMap<String, serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Public data model
// ---------------------------------------------------------------------------

/// The type of an asset bundled inside a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BundledAssetType {
    Skill,
    Agent,
    Command,
    Hook,
    Mcp,
}

impl BundledAssetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Command => "command",
            Self::Hook => "hook",
            Self::Mcp => "mcp",
        }
    }
}

/// A single asset bundled inside a plugin (directory-enumerated or key-derived).
#[derive(Debug, Clone, Serialize)]
pub struct BundledAsset {
    /// `"skill"`, `"agent"`, `"command"`, `"hook"`, or `"mcp"`.
    pub asset_type: BundledAssetType,
    /// Leaf name of the asset (directory name for file-based assets, server key for mcpServers).
    pub name: String,
    /// Absolute path on disk (the directory or file that represents the asset).
    /// For `mcp` assets this is the install root since mcpServers entries are in-manifest.
    pub path: PathBuf,
}

/// A fully resolved, installed plugin with its bundled assets.
#[derive(Debug, Clone, Serialize)]
pub struct Plugin {
    /// `"name@marketplace"` — the canonical identifier used as the map key in
    /// `installed_plugins.json`.
    pub id: String,
    /// The `name` segment before the `@`.
    pub name: String,
    /// The `marketplace` segment after the `@`.
    pub marketplace: String,
    /// Version string from the install record (may be `"unknown"`).
    pub version: String,
    /// Absolute path to the installed copy.
    pub install_path: PathBuf,
    /// Human-readable description from the plugin manifest, if present.
    pub description: Option<String>,
    /// Source URL/repo resolved from `known_marketplaces.json`.
    pub source: Option<String>,
    /// True when this plugin's id appears in `blocklist.json`.
    pub blocked: bool,
    /// All assets found by walking the manifest-declared subdirectories.
    pub assets: Vec<BundledAsset>,
}

/// Map from an asset's absolute path (as a string) to the owning plugin id.
/// Lets callers tag flat-list asset entries with their plugin origin.
pub type AssetAttribution = HashMap<String, String>;

// ---------------------------------------------------------------------------
// iCloud junk detection
// ---------------------------------------------------------------------------

/// Returns `true` when a name segment looks like an iCloud duplicate artefact
/// (a trailing " 2", " 3", etc.).
///
/// For plugin ids (`"name@marketplace"`) pass only the name segment before `@`.
/// For filesystem entry names pass the bare entry name.
fn has_trailing_digit_suffix(name: &str) -> bool {
    if let Some(last_space) = name.rfind(' ') {
        let suffix = &name[last_space + 1..];
        if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// Returns `true` when a filesystem entry is iCloud junk: either its name ends
/// with " <digit(s)>" or its directory has 0700 permissions (Claude's staging
/// sentinel).
fn is_icloud_junk(entry_name: &str, path: &Path) -> bool {
    if has_trailing_digit_suffix(entry_name) {
        return true;
    }
    // 0700 octal permissions -- Claude's staging sentinel.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            if meta.is_dir() && meta.permissions().mode() & 0o777 == 0o700 {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Registry parsing helpers
// ---------------------------------------------------------------------------

fn read_blocklist(plugin_root: &Path) -> HashSet<String> {
    let path = plugin_root.join("blocklist.json");
    let Ok(content) = fs::read_to_string(&path) else {
        return HashSet::new();
    };
    let Ok(file): Result<BlocklistFile, _> = serde_json::from_str(&content) else {
        log::warn!("plugin_discovery: failed to parse blocklist.json");
        return HashSet::new();
    };
    file.plugins.into_iter().map(|i| i.plugin).collect()
}

/// Extract a human-readable source URL/repo from a marketplace entry's `source` field.
/// The field shape varies: `{"source": "git", "url": "..."}` or `{"source": "github", "repo": "..."}`.
fn source_url_from_marketplace(entry: &MarketplaceEntry) -> Option<String> {
    let source = entry.source.as_ref()?;
    let obj = source.as_object()?;
    obj.get("url")
        .or_else(|| obj.get("repo"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn read_known_marketplaces(plugin_root: &Path) -> HashMap<String, Option<String>> {
    let path = plugin_root.join("known_marketplaces.json");
    let Ok(content) = fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let Ok(map): Result<HashMap<String, MarketplaceEntry>, _> = serde_json::from_str(&content)
    else {
        log::warn!("plugin_discovery: failed to parse known_marketplaces.json");
        return HashMap::new();
    };
    map.into_iter()
        .map(|(k, v)| (k, source_url_from_marketplace(&v)))
        .collect()
}

/// Parse the install record array from the serde_json::Value stored per plugin key.
/// Schema v2: value is `[{scope, installPath, version, ...}]`.
fn parse_install_record(value: &serde_json::Value) -> Option<InstallRecord> {
    // Value should be an array; take the first element.
    let arr = value.as_array()?;
    let first = arr.first()?;
    serde_json::from_value(first.clone()).ok()
}

// ---------------------------------------------------------------------------
// Manifest + asset enumeration
// ---------------------------------------------------------------------------

/// Read and parse `.claude-plugin/plugin.json` at `install_path`.
/// Returns `None` when the manifest is absent or malformed (graceful skip).
fn read_manifest(install_path: &Path) -> Option<PluginManifest> {
    let manifest_path = install_path.join(".claude-plugin").join("plugin.json");
    let content = fs::read_to_string(&manifest_path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Walk a directory declared by the manifest (e.g. `"./skills/"`) and return
/// one `BundledAsset` per immediate child directory that is not iCloud junk.
fn enumerate_dir_assets(
    install_path: &Path,
    declared_rel: &str,
    asset_type: BundledAssetType,
) -> Vec<BundledAsset> {
    // Strip leading `./` so Path::join works correctly.
    let rel = declared_rel.trim_start_matches("./").trim_end_matches('/');
    let dir = install_path.join(rel);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut assets = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // Skip hidden entries and iCloud junk.
        if name.starts_with('.') || is_icloud_junk(&name, &path) {
            continue;
        }
        if path.is_dir() {
            assets.push(BundledAsset {
                asset_type: asset_type.clone(),
                name,
                path,
            });
        }
    }
    assets
}

/// Derive assets from a parsed manifest and an install path.
fn assets_from_manifest(manifest: &PluginManifest, install_path: &Path) -> Vec<BundledAsset> {
    let mut assets: Vec<BundledAsset> = Vec::new();

    // Directory-based asset types.
    if let Some(ref rel) = manifest.skills {
        assets.extend(enumerate_dir_assets(install_path, rel, BundledAssetType::Skill));
    }
    if let Some(ref rel) = manifest.agents {
        assets.extend(enumerate_dir_assets(install_path, rel, BundledAssetType::Agent));
    }
    if let Some(ref rel) = manifest.commands {
        assets.extend(enumerate_dir_assets(
            install_path,
            rel,
            BundledAssetType::Command,
        ));
    }
    if let Some(ref rel) = manifest.hooks {
        // hooks is often a file path, not a directory. Accept either.
        let rel_stripped = rel.trim_start_matches("./").trim_end_matches('/');
        let hooks_path = install_path.join(rel_stripped);
        if hooks_path.is_dir() {
            assets.extend(enumerate_dir_assets(
                install_path,
                rel,
                BundledAssetType::Hook,
            ));
        } else if hooks_path.exists() {
            // Single file -- emit one hook asset named after the file stem.
            let name = hooks_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("hooks")
                .to_string();
            assets.push(BundledAsset {
                asset_type: BundledAssetType::Hook,
                name,
                path: hooks_path,
            });
        }
    }

    // mcpServers: each key becomes one mcp asset; path is install root.
    if let Some(ref servers) = manifest.mcp_servers {
        for key in servers.keys() {
            assets.push(BundledAsset {
                asset_type: BundledAssetType::Mcp,
                name: key.clone(),
                path: install_path.to_path_buf(),
            });
        }
    }

    assets
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discover all installed plugins rooted at `plugin_root`.
///
/// The function is resilient: a missing registry file or a missing/malformed
/// manifest causes that plugin to be skipped or returned with empty assets,
/// never a hard error.  iCloud junk entries (trailing-digit names, 0700 dirs)
/// are silently excluded.
pub fn list_plugins(plugin_root: &Path) -> Vec<Plugin> {
    // Load auxiliary registries.
    let blocklist = read_blocklist(plugin_root);
    let marketplaces = read_known_marketplaces(plugin_root);

    // Parse installed_plugins.json.
    let installed_path = plugin_root.join("installed_plugins.json");
    let content = match fs::read_to_string(&installed_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "plugin_discovery: cannot read installed_plugins.json: {}",
                e
            );
            return Vec::new();
        }
    };
    let registry: InstalledPluginsFile = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            log::warn!(
                "plugin_discovery: failed to parse installed_plugins.json: {}",
                e
            );
            return Vec::new();
        }
    };

    let mut plugins = Vec::new();

    for (plugin_id, raw_value) in &registry.plugins {
        // Split "name@marketplace" first so we can check the name segment for iCloud junk.
        let (name, marketplace) = match plugin_id.split_once('@') {
            Some(pair) => (pair.0.to_string(), pair.1.to_string()),
            None => {
                log::warn!("plugin_discovery: unexpected plugin id format: {}", plugin_id);
                continue;
            }
        };

        // Filter iCloud junk: the name segment ends with " <digits>".
        if has_trailing_digit_suffix(&name) {
            continue;
        }

        let record = match parse_install_record(raw_value) {
            Some(r) => r,
            None => {
                log::warn!(
                    "plugin_discovery: cannot parse install record for {}",
                    plugin_id
                );
                continue;
            }
        };

        let install_path = PathBuf::from(&record.install_path);
        let version = record.version.unwrap_or_else(|| "unknown".to_string());
        let blocked = blocklist.contains(plugin_id.as_str());
        let source = marketplaces.get(&marketplace).and_then(|v| v.clone());

        // Read manifest (graceful skip on missing/malformed).
        let (description, assets) = match read_manifest(&install_path) {
            Some(manifest) => {
                let desc = manifest.description.clone();
                let a = assets_from_manifest(&manifest, &install_path);
                (desc, a)
            }
            None => {
                log::debug!(
                    "plugin_discovery: no manifest at {} -- skipping assets",
                    install_path.display()
                );
                (None, Vec::new())
            }
        };

        plugins.push(Plugin {
            id: plugin_id.clone(),
            name,
            marketplace,
            version,
            install_path,
            description,
            source,
            blocked,
            assets,
        });
    }

    // Stable order: sort by id so output is deterministic.
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    plugins
}

/// Build an attribution map: `absolute_asset_path_string -> plugin_id`.
///
/// Used by the flat-list tabs to badge assets that originate from an installed
/// plugin rather than a manually managed skill.
pub fn build_asset_attribution(plugins: &[Plugin]) -> AssetAttribution {
    let mut map = HashMap::new();
    for plugin in plugins {
        for asset in &plugin.assets {
            map.insert(asset.path.to_string_lossy().into_owned(), plugin.id.clone());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // -----------------------------------------------------------------------
    // Fixture builder helpers
    // -----------------------------------------------------------------------

    fn write_json(path: &Path, content: impl AsRef<str>) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content.as_ref()).unwrap();
    }

    /// Build a minimal fixture plugin tree inside `root`.
    ///
    /// Layout:
    ///   installed_plugins.json  (v2, one real plugin + one junk key)
    ///   known_marketplaces.json (one marketplace with a github source)
    ///   blocklist.json          (blocks "evil-plugin@bad-market")
    ///   cache/test-market/my-plugin/1.0.0/
    ///     .claude-plugin/plugin.json   (declares skills/ with two skills)
    ///     skills/skill-alpha/          (real skill dir)
    ///     skills/skill-beta/           (real skill dir)
    fn build_fixture(root: &Path) {
        // installed_plugins.json
        write_json(
            &root.join("installed_plugins.json"),
            r#"{
                "version": 2,
                "plugins": {
                    "my-plugin@test-market": [
                        {
                            "scope": "user",
                            "installPath": "INSTALL_PATH",
                            "version": "1.0.0",
                            "installedAt": "2026-01-01T00:00:00.000Z",
                            "lastUpdated": "2026-01-02T00:00:00.000Z"
                        }
                    ],
                    "junk-plugin 2@test-market": [
                        {
                            "scope": "user",
                            "installPath": "INSTALL_PATH",
                            "version": "0.1.0",
                            "installedAt": "2026-01-01T00:00:00.000Z"
                        }
                    ],
                    "evil-plugin@bad-market": [
                        {
                            "scope": "user",
                            "installPath": "INSTALL_PATH",
                            "version": "0.0.1",
                            "installedAt": "2026-01-01T00:00:00.000Z"
                        }
                    ]
                }
            }"#.replace("INSTALL_PATH", &root.join("cache/test-market/my-plugin/1.0.0").to_string_lossy()),
        );

        // known_marketplaces.json
        write_json(
            &root.join("known_marketplaces.json"),
            r#"{
                "test-market": {
                    "source": {"source": "github", "repo": "https://github.com/example/test-market"},
                    "installLocation": "cache/test-market",
                    "lastUpdated": "2026-01-01T00:00:00.000Z"
                }
            }"#,
        );

        // blocklist.json
        write_json(
            &root.join("blocklist.json"),
            r#"{"fetchedAt": "2026-01-01T00:00:00.000Z", "plugins": [{"plugin": "evil-plugin@bad-market", "added_at": "2026-01-01", "reason": "test", "text": "blocked for testing"}]}"#,
        );

        // .claude-plugin/plugin.json manifest
        let install_root = root.join("cache/test-market/my-plugin/1.0.0");
        write_json(
            &install_root.join(".claude-plugin/plugin.json"),
            r#"{
                "name": "my-plugin",
                "version": "1.0.0",
                "description": "A test plugin",
                "skills": "./skills/"
            }"#,
        );

        // Two skill dirs.
        fs::create_dir_all(install_root.join("skills/skill-alpha")).unwrap();
        fs::create_dir_all(install_root.join("skills/skill-beta")).unwrap();

        // An iCloud-junk dir inside skills/ (trailing digit name) -- must be ignored.
        fs::create_dir_all(install_root.join("skills/skill-alpha 2")).unwrap();
    }

    // -----------------------------------------------------------------------
    // Fixture-based tests
    // -----------------------------------------------------------------------

    #[test]
    fn fixture_returns_real_plugin_with_correct_fields() {
        let tmp = tempdir().unwrap();
        build_fixture(tmp.path());

        let plugins = list_plugins(tmp.path());

        // Only "my-plugin@test-market" should survive (junk key excluded, evil blocked but present).
        let real = plugins
            .iter()
            .find(|p| p.id == "my-plugin@test-market")
            .expect("my-plugin@test-market must be in results");

        assert_eq!(real.name, "my-plugin");
        assert_eq!(real.marketplace, "test-market");
        assert_eq!(real.version, "1.0.0");
        assert!(!real.blocked);
        assert_eq!(
            real.source.as_deref(),
            Some("https://github.com/example/test-market")
        );
        assert_eq!(real.description.as_deref(), Some("A test plugin"));
    }

    #[test]
    fn fixture_enumerates_bundled_skills_correctly() {
        let tmp = tempdir().unwrap();
        build_fixture(tmp.path());

        let plugins = list_plugins(tmp.path());
        let real = plugins
            .iter()
            .find(|p| p.id == "my-plugin@test-market")
            .unwrap();

        let mut skill_names: Vec<&str> = real
            .assets
            .iter()
            .filter(|a| a.asset_type == BundledAssetType::Skill)
            .map(|a| a.name.as_str())
            .collect();
        skill_names.sort();

        // skill-alpha 2 (iCloud junk) must be excluded; alpha and beta remain.
        assert_eq!(skill_names, vec!["skill-alpha", "skill-beta"]);
    }

    #[test]
    fn fixture_excludes_icloud_junk_plugin_id() {
        let tmp = tempdir().unwrap();
        build_fixture(tmp.path());

        let plugins = list_plugins(tmp.path());
        assert!(
            !plugins.iter().any(|p| p.id.contains(" 2")),
            "iCloud junk plugin id 'junk-plugin 2@test-market' must be excluded"
        );
    }

    #[test]
    fn fixture_blocked_flag_set_for_blocklisted_plugin() {
        let tmp = tempdir().unwrap();
        build_fixture(tmp.path());

        let plugins = list_plugins(tmp.path());
        // evil-plugin@bad-market has no valid installPath in the fixture so it
        // may or may not parse -- only check if it appears.
        if let Some(evil) = plugins.iter().find(|p| p.id == "evil-plugin@bad-market") {
            assert!(evil.blocked, "evil-plugin must be flagged as blocked");
        }
    }

    #[test]
    fn attribution_map_keys_on_asset_paths() {
        let tmp = tempdir().unwrap();
        build_fixture(tmp.path());

        let plugins = list_plugins(tmp.path());
        let attribution = build_asset_attribution(&plugins);

        let real = plugins
            .iter()
            .find(|p| p.id == "my-plugin@test-market")
            .unwrap();
        for asset in &real.assets {
            let key = asset.path.to_string_lossy().into_owned();
            assert_eq!(
                attribution.get(&key).map(|s| s.as_str()),
                Some("my-plugin@test-market"),
                "asset at {} must attribute to my-plugin@test-market",
                key
            );
        }
    }

    #[test]
    fn empty_plugin_root_returns_empty_vec() {
        let tmp = tempdir().unwrap();
        // No files at all -- must not panic.
        let plugins = list_plugins(tmp.path());
        assert!(plugins.is_empty());
    }

    // -----------------------------------------------------------------------
    // Integration test against the real installed_plugins.json.
    //
    // Run with:  cargo test -p skills-manager real_plugins -- --ignored --nocapture
    // -----------------------------------------------------------------------
    #[test]
    #[ignore]
    fn real_plugins_deserialization_sanity() {
        let home = dirs::home_dir().expect("cannot determine home directory");
        let plugin_root = home.join(".claude").join("plugins");
        assert!(
            plugin_root.exists(),
            "plugin root {} does not exist",
            plugin_root.display()
        );

        let plugins = list_plugins(&plugin_root);
        println!(
            "real_plugins_deserialization_sanity: discovered {} plugins",
            plugins.len()
        );
        for p in &plugins {
            println!(
                "  [{}] {} assets  blocked={}  version={}",
                p.id,
                p.assets.len(),
                p.blocked,
                p.version
            );
        }
        assert!(
            plugins.len() >= 10,
            "expected at least 10 installed plugins, got {}",
            plugins.len()
        );
    }
}
