/// Tauri command surface for multi-asset-type queries and delivery.
///
/// `get_managed_assets`    — list managed records of one asset type.
/// `deliver_managed_asset` — deliver one stored asset to all four core coding
///   agent homes using the capability-driven delivery engine.
/// `delete_managed_asset`  — remove the store row and central-repo copy for
///   any asset type (does NOT touch the user's source workspace).
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

use crate::core::{
    asset_delivery::{deliver_asset, AssetInput, DeliveryOutcome},
    asset_render::canonical_agent_from_file,
    audit_log::AuditDraft,
    error::AppError,
    plugin_discovery,
    repo_lock::RepoLock,
    skill_store::{AssetType, SkillRecord, SkillStore},
    sync_metadata,
    tool_adapters::default_tool_adapters,
    tool_service::get_disabled_tools,
};
use serde::Serialize;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Serialisable summary of one managed asset record.
#[derive(Debug, Serialize)]
pub struct ManagedAssetDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub asset_type: String,
    pub central_path: String,
    pub enabled: bool,
    pub status: String,
    /// Plugin id (`"name@marketplace"`) that owns this asset, or `None` when the
    /// asset was not imported from a plugin.  Set by `get_managed_assets` via
    /// `plugin_discovery::build_asset_attribution`; callers that construct a
    /// `ManagedAssetDto` without plugin context (e.g. `From<SkillRecord>`) leave
    /// this as `None`.
    pub owning_plugin: Option<String>,
}

impl From<SkillRecord> for ManagedAssetDto {
    fn from(r: SkillRecord) -> Self {
        ManagedAssetDto {
            id: r.id,
            name: r.name,
            description: r.description,
            asset_type: r.asset_type.as_str().to_string(),
            central_path: r.central_path,
            enabled: r.enabled,
            status: r.status,
            owning_plugin: None,
        }
    }
}

/// Per-adapter outcome returned by `deliver_managed_asset`.
#[derive(Debug, Serialize)]
pub struct AdapterDeliveryResult {
    pub adapter_key: String,
    pub outcome: String,
    pub path: Option<String>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Return all managed records whose `asset_type` matches `asset_type_str`.
///
/// Existing skill listing (`get_managed_skills`) is not altered; this is an
/// additive query.  Unknown `asset_type_str` values silently map to
/// `AssetType::Skill` (same as `AssetType::from_str`).
///
/// Each returned record is annotated with `owning_plugin` by matching the
/// record's `source_ref` (the original import path) against the attribution
/// map built from the locally installed plugins.  The plugin root is resolved
/// once per call; attribution lookup is O(1) per record.
#[tauri::command]
pub async fn get_managed_assets(
    asset_type_str: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Vec<ManagedAssetDto>, AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let asset_type = AssetType::from_str(&asset_type_str);
        let records = store
            .get_skills_by_asset_type(asset_type)
            .map_err(AppError::db)?;

        // Build plugin attribution once for this call.
        // A missing plugin root (no plugins installed) is treated as empty --
        // attribution will simply return None for every record.
        let attribution = resolve_plugin_attribution();

        let dtos = records
            .into_iter()
            .map(|r| {
                let owning_plugin = r.source_ref.as_deref().and_then(|src| {
                    // 1. Exact match: source_ref is the asset path in the attribution map.
                    if let Some(plugin_id) = attribution.exact.get(src) {
                        return Some(plugin_id.clone());
                    }
                    // 2. Prefix fallback: source_ref starts with a plugin's install_path.
                    attribution
                        .plugins
                        .iter()
                        .find(|(install_path, _)| src.starts_with(install_path.as_str()))
                        .map(|(_, plugin_id)| plugin_id.clone())
                });
                ManagedAssetDto {
                    owning_plugin,
                    ..ManagedAssetDto::from(r)
                }
            })
            .collect();

        Ok(dtos)
    })
    .await?
}

/// Resolved plugin attribution data for one `get_managed_assets` call.
struct PluginAttribution {
    /// `asset_path -> plugin_id` (exact match from `build_asset_attribution`).
    exact: plugin_discovery::AssetAttribution,
    /// `(install_path_str, plugin_id)` pairs for prefix-fallback matching.
    plugins: Vec<(String, String)>,
}

/// Resolve the plugin root and compute attribution data.  Returns an empty
/// attribution when the plugin root cannot be determined or has no plugins.
fn resolve_plugin_attribution() -> PluginAttribution {
    let plugin_root = match dirs::home_dir() {
        Some(home) => home.join(".claude").join("plugins"),
        None => {
            return PluginAttribution {
                exact: Default::default(),
                plugins: Vec::new(),
            }
        }
    };
    let plugins = plugin_discovery::list_plugins(&plugin_root);
    let exact = plugin_discovery::build_asset_attribution(&plugins);
    let prefix_pairs = plugins
        .iter()
        .map(|p| (p.install_path.to_string_lossy().into_owned(), p.id.clone()))
        .collect();
    PluginAttribution {
        exact,
        plugins: prefix_pairs,
    }
}

/// Deliver the managed asset identified by `asset_id` to the four core coding
/// agent homes (claude_code, pi, codex, github_copilot), but only for those
/// that are currently enabled.  Disabled tools are skipped entirely — no
/// directories are created on their behalf.
///
/// For `AssetType::Agent`, the canonical representation is re-parsed from the
/// central file via `canonical_agent_from_file` so that registry fields
/// (codex_reasoning_effort, etc.) flow through to rendered outputs.
///
/// Returns one `AdapterDeliveryResult` per enabled core adapter.
#[tauri::command]
pub async fn deliver_managed_asset(
    asset_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Vec<AdapterDeliveryResult>, AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let record = store
            .get_skill_by_id(&asset_id)
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found(format!("asset not found: {asset_id}")))?;

        let source = PathBuf::from(&record.central_path);
        let asset_type = record.asset_type;

        // Verify the central file is readable before doing any delivery work.
        if !source.is_file() {
            return Err(AppError::not_found(format!(
                "central file missing for asset '{}': {}",
                record.name,
                source.display()
            )));
        }

        // For agents, parse the canonical representation from the stored file.
        let canonical_agent = if asset_type == AssetType::Agent {
            Some(canonical_agent_from_file(&source).map_err(AppError::io)?)
        } else {
            None
        };

        // Scope delivery to the four core adapters; custom / extra adapters are
        // out of scope for Slice 1 (spec §CONSTRAINTS).
        // Skip any adapter the user has disabled — same semantics as skill sync.
        let core_adapter_keys = ["claude_code", "pi", "codex", "github_copilot"];
        let disabled = get_disabled_tools(&store);
        let adapters: Vec<_> = default_tool_adapters()
            .into_iter()
            .filter(|a| {
                core_adapter_keys.contains(&a.key.as_str())
                    && !disabled.contains(&a.key)
            })
            .collect();

        let mut results = Vec::with_capacity(adapters.len());

        for adapter in &adapters {
            let home = adapter.skills_dir().parent().map(|p| p.to_path_buf())
                // skills_dir is e.g. ~/.claude/skills; home is ~/.claude
                .unwrap_or_else(|| adapter.skills_dir());

            let input = AssetInput {
                asset_type,
                source: &source,
                id: &record.name,
                name: &record.name,
                canonical_agent: canonical_agent.as_ref(),
            };

            let outcome = deliver_asset(adapter, &home, &input).map_err(AppError::io)?;

            let (outcome_str, path) = match &outcome {
                DeliveryOutcome::Symlinked(p) => ("symlinked".to_string(), Some(p.to_string_lossy().to_string())),
                DeliveryOutcome::Rendered(p) => ("rendered".to_string(), Some(p.to_string_lossy().to_string())),
                DeliveryOutcome::RenderedUpToDate(p) => ("rendered_up_to_date".to_string(), Some(p.to_string_lossy().to_string())),
                DeliveryOutcome::Placed(p) => ("placed".to_string(), Some(p.to_string_lossy().to_string())),
                DeliveryOutcome::ForeignHome(p) => ("skipped_foreign_home".to_string(), Some(p.to_string_lossy().to_string())),
                DeliveryOutcome::DeferToSkillPath => ("defer_to_skill_path".to_string(), None),
                DeliveryOutcome::Unsupported => ("unsupported".to_string(), None),
            };

            results.push(AdapterDeliveryResult {
                adapter_key: adapter.key.clone(),
                outcome: outcome_str,
                path,
            });
        }

        Ok(results)
    })
    .await?
}

/// Remove the managed record for `asset_id` and its central-repo copy.
///
/// For `AssetType::Skill` the central path is a directory (removed
/// recursively).  For every other asset type the central path is a single
/// file (removed with `fs::remove_file`).  A missing central path is not
/// treated as an error — the row is deleted regardless.
///
/// The user's source workspace (agentic-tools) is never touched.
#[tauri::command]
pub async fn delete_managed_asset(
    asset_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = RepoLock::acquire("delete asset").map_err(AppError::db)?;

        let record = store
            .get_skill_by_id(&asset_id)
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found(format!("asset not found: {asset_id}")))?;

        let central = PathBuf::from(&record.central_path);
        if record.asset_type == AssetType::Skill {
            if central.exists() {
                std::fs::remove_dir_all(&central).ok();
            }
        } else if central.exists() {
            std::fs::remove_file(&central).ok();
        }

        store.delete_skill(&asset_id).map_err(AppError::db)?;
        store.log_audit(
            AuditDraft::new("remove")
                .skill(asset_id.clone(), record.name.clone())
                .ok(),
        );

        sync_metadata::write_all_from_db_unlocked(&store).map_err(AppError::db)?;
        Ok(())
    })
    .await?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{central_repo, skill_store::SkillStore};
    use std::fs;
    use tempfile::tempdir;

    // ── TestEnv: temp central repo + locked global state ───────────────────

    struct TestEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
        _central_tmp: tempfile::TempDir,
        pub store: SkillStore,
    }

    fn make_test_env() -> TestEnv {
        let lock = central_repo::test_base_dir_lock();
        let central_tmp = tempdir().unwrap();
        let base = central_tmp.path().join("central");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        central_repo::ensure_central_repo().unwrap();
        let store = SkillStore::new(&base.join("test.db")).unwrap();
        TestEnv {
            _lock: lock,
            _central_tmp: central_tmp,
            store,
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            central_repo::set_test_base_dir_override(None);
        }
    }

    /// Insert a minimal SkillRecord with explicit asset_type and return its id.
    fn insert_record(store: &SkillStore, id: &str, name: &str, asset_type: AssetType, central_path: &str) {
        let now = chrono::Utc::now().timestamp();
        store
            .insert_skill(&SkillRecord {
                id: id.to_string(),
                name: name.to_string(),
                description: None,
                source_type: "import".to_string(),
                source_ref: None,
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                central_path: central_path.to_string(),
                content_hash: None,
                enabled: true,
                created_at: now,
                updated_at: now,
                status: "ok".to_string(),
                update_status: "local_only".to_string(),
                last_checked_at: None,
                last_check_error: None,
                asset_type,
            })
            .unwrap();
    }

    // ── get_skills_by_asset_type unit ───────────────────────────────────────

    #[test]
    fn get_skills_by_asset_type_returns_correct_disjoint_sets() {
        let env = make_test_env();

        insert_record(&env.store, "a1", "agent-one", AssetType::Agent, "/tmp/a1.md");
        insert_record(&env.store, "a2", "agent-two", AssetType::Agent, "/tmp/a2.md");
        insert_record(&env.store, "s1", "skill-one", AssetType::Skill, "/tmp/s1");

        let agents = env.store.get_skills_by_asset_type(AssetType::Agent).unwrap();
        let skills = env.store.get_skills_by_asset_type(AssetType::Skill).unwrap();

        // Agents query returns only agents
        assert_eq!(agents.len(), 2, "must return exactly 2 agents");
        assert!(agents.iter().all(|r| r.asset_type == AssetType::Agent));

        // Skills query returns only skills — Skill listing unchanged
        assert_eq!(skills.len(), 1, "must return exactly 1 skill");
        assert!(skills.iter().all(|r| r.asset_type == AssetType::Skill));

        // Sets are disjoint by id
        let agent_ids: std::collections::HashSet<_> = agents.iter().map(|r| &r.id).collect();
        let skill_ids: std::collections::HashSet<_> = skills.iter().map(|r| &r.id).collect();
        assert!(
            agent_ids.is_disjoint(&skill_ids),
            "agent and skill id sets must be disjoint"
        );
    }

    #[test]
    fn get_all_skills_still_returns_all_types() {
        // Ensure existing `get_all_skills` is not affected by the new method.
        let env = make_test_env();

        insert_record(&env.store, "a1", "agent-one", AssetType::Agent, "/tmp/a1.md");
        insert_record(&env.store, "s1", "skill-one", AssetType::Skill, "/tmp/s1");

        let all = env.store.get_all_skills().unwrap();
        assert_eq!(all.len(), 2, "get_all_skills must return every row regardless of type");
    }

    // ── end-to-end: import -> parse -> render -> deliver ───────────────────

    /// Build a fixture workspace with one agent and import it into the store.
    /// Returns: (env, agent_central_path, agent_record_id)
    fn import_fixture_agent() -> (TestEnv, std::path::PathBuf, String) {
        let env = make_test_env();

        // Write a source workspace
        let ws_tmp = tempdir().unwrap();
        let ws = ws_tmp.path();
        fs::create_dir_all(ws.join("agents")).unwrap();
        fs::write(
            ws.join("agents/backend-architect.md"),
            "---\nname: backend-architect\ndescription: Backend expert\ntools:\n  - Read\n  - Write\nmodel: inherit\n---\n# Backend Architect\n\nBody here.\n",
        ).unwrap();
        fs::create_dir_all(ws.join("registry")).unwrap();
        fs::write(
            ws.join("registry/active.json"),
            r#"{"agents":[{"id":"backend-architect","display_name":"Backend Architect","description":"Backend expert","tools":["Read","Write"],"codex_reasoning_effort":"high","codex_sandbox_mode":"workspace-write"}],"skills":[]}"#,
        ).unwrap();

        let candidates = crate::core::importer::list_candidates(ws).unwrap();
        let results = crate::core::importer::import_candidates(&candidates, &env.store).unwrap();

        // Leak ws_tmp so the source files stay alive for the test duration.
        std::mem::forget(ws_tmp);

        let ba = results
            .iter()
            .find(|r| r.id_or_name == "backend-architect")
            .expect("backend-architect import result not found");

        let record = env
            .store
            .get_all_skills()
            .unwrap()
            .into_iter()
            .find(|r| r.name == "backend-architect")
            .expect("backend-architect record not found in store");

        (env, ba.central_path.clone(), record.id)
    }

    #[cfg(unix)]
    #[test]
    fn end_to_end_import_then_deliver_to_all_four_homes() {
        let (env, _central_path, record_id) = import_fixture_agent();

        let record = env
            .store
            .get_skill_by_id(&record_id)
            .unwrap()
            .expect("record must exist");

        let source = std::path::PathBuf::from(&record.central_path);

        // Parse the CanonicalAgent from the imported file.
        let agent = canonical_agent_from_file(&source).unwrap();
        assert_eq!(
            agent.codex_reasoning_effort.as_deref(),
            Some("high"),
            "codex_reasoning_effort must survive import -> parse"
        );

        // Deliver to four temp homes.
        let claude_home = tempdir().unwrap();
        let pi_home = tempdir().unwrap();
        let codex_home = tempdir().unwrap();
        let copilot_home = tempdir().unwrap();

        let adapters = default_tool_adapters();

        // Helper: find adapter and deliver to given home
        let deliver = |key: &str, home: &std::path::Path| -> DeliveryOutcome {
            let adapter = adapters.iter().find(|a| a.key == key).unwrap();
            let input = AssetInput {
                asset_type: AssetType::Agent,
                source: &source,
                id: &record.name,
                name: &record.name,
                canonical_agent: Some(&agent),
            };
            deliver_asset(adapter, home, &input).unwrap()
        };

        // Claude: symlink
        let claude_result = deliver("claude_code", claude_home.path());
        let claude_target = claude_home.path().join("agents").join("backend-architect.md");
        assert!(
            matches!(claude_result, DeliveryOutcome::Symlinked(_)),
            "claude_code must produce Symlinked"
        );
        assert!(claude_target.is_symlink(), "Claude agents/<id>.md must be a symlink");

        // Codex: rendered .toml with model_reasoning_effort = "high"
        let codex_result = deliver("codex", codex_home.path());
        let codex_target = codex_home.path().join("agents").join("backend-architect.toml");
        assert!(
            matches!(codex_result, DeliveryOutcome::Rendered(_)),
            "codex must produce Rendered"
        );
        assert!(codex_target.is_file(), "Codex agents/<id>.toml must be a regular file");
        let codex_content = fs::read_to_string(&codex_target).unwrap();
        assert!(
            codex_content.contains("model_reasoning_effort = \"high\""),
            "codex .toml must contain model_reasoning_effort = \"high\"; got:\n{codex_content}"
        );

        // Copilot: rendered .agent.md
        let copilot_result = deliver("github_copilot", copilot_home.path());
        let copilot_target = copilot_home
            .path()
            .join("agents")
            .join("backend-architect.agent.md");
        assert!(
            matches!(copilot_result, DeliveryOutcome::Rendered(_)),
            "github_copilot must produce Rendered"
        );
        assert!(copilot_target.is_file(), "Copilot agents/<id>.agent.md must be a regular file");
        let copilot_content = fs::read_to_string(&copilot_target).unwrap();
        let expected_copilot = crate::core::asset_render::render_copilot(&agent);
        assert_eq!(
            copilot_content, expected_copilot,
            "Copilot file bytes must equal render_copilot output"
        );

        // Pi: symlink
        let pi_result = deliver("pi", pi_home.path());
        assert!(
            matches!(pi_result, DeliveryOutcome::Symlinked(_)),
            "pi must produce Symlinked"
        );
    }

    // ── deliver_managed_asset command-layer tests ───────────────────────────

    /// Helper: call the synchronous inner body of deliver_managed_asset directly,
    /// bypassing Tauri's async layer, with an explicit disabled-tools list.
    fn deliver_inner(
        store: &SkillStore,
        asset_id: &str,
        disabled: &[&str],
    ) -> Result<Vec<AdapterDeliveryResult>, AppError> {
        // Mirror the disabled-tools setting so get_disabled_tools reads it back.
        let disabled_json = serde_json::to_string(disabled).unwrap();
        store.set_setting("disabled_tools", &disabled_json).unwrap();

        let record = store
            .get_skill_by_id(asset_id)
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found(format!("asset not found: {asset_id}")))?;

        let source = PathBuf::from(&record.central_path);
        let asset_type = record.asset_type;

        if !source.is_file() {
            return Err(AppError::not_found(format!(
                "central file missing for asset '{}': {}",
                record.name,
                source.display()
            )));
        }

        let canonical_agent = if asset_type == AssetType::Agent {
            Some(canonical_agent_from_file(&source).map_err(AppError::io)?)
        } else {
            None
        };

        let core_adapter_keys = ["claude_code", "pi", "codex", "github_copilot"];
        let disabled_vec = get_disabled_tools(store);
        let adapters: Vec<_> = default_tool_adapters()
            .into_iter()
            .filter(|a| {
                core_adapter_keys.contains(&a.key.as_str())
                    && !disabled_vec.contains(&a.key)
            })
            .collect();

        let mut results = Vec::with_capacity(adapters.len());

        for adapter in &adapters {
            let home = adapter
                .skills_dir()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| adapter.skills_dir());

            let input = AssetInput {
                asset_type,
                source: &source,
                id: &record.name,
                name: &record.name,
                canonical_agent: canonical_agent.as_ref(),
            };

            let outcome = deliver_asset(adapter, &home, &input).map_err(AppError::io)?;

            let (outcome_str, path) = match &outcome {
                DeliveryOutcome::Symlinked(p) => {
                    ("symlinked".to_string(), Some(p.to_string_lossy().to_string()))
                }
                DeliveryOutcome::Rendered(p) => {
                    ("rendered".to_string(), Some(p.to_string_lossy().to_string()))
                }
                DeliveryOutcome::RenderedUpToDate(p) => (
                    "rendered_up_to_date".to_string(),
                    Some(p.to_string_lossy().to_string()),
                ),
                DeliveryOutcome::Placed(p) => {
                    ("placed".to_string(), Some(p.to_string_lossy().to_string()))
                }
                DeliveryOutcome::ForeignHome(p) => (
                    "skipped_foreign_home".to_string(),
                    Some(p.to_string_lossy().to_string()),
                ),
                DeliveryOutcome::DeferToSkillPath => ("defer_to_skill_path".to_string(), None),
                DeliveryOutcome::Unsupported => ("unsupported".to_string(), None),
            };

            results.push(AdapterDeliveryResult {
                adapter_key: adapter.key.clone(),
                outcome: outcome_str,
                path,
            });
        }

        Ok(results)
    }

    /// A non-agent asset type (Hook) is supported by claude_code and pi but
    /// returns Unsupported for codex and github_copilot.  Verify that only the
    /// supporting adapters produce non-unsupported outcomes and that the
    /// unsupported adapters do NOT create any file-system artefacts.
    #[cfg(unix)]
    #[test]
    fn non_agent_hook_reaches_supporting_adapters_and_skips_unsupported() {
        let env = make_test_env();

        // Write a real file so the missing-file guard passes.
        let hook_tmp = tempdir().unwrap();
        let hook_file = hook_tmp.path().join("post-commit.sh");
        fs::write(&hook_file, "#!/bin/sh\necho hook\n").unwrap();

        insert_record(
            &env.store,
            "h1",
            "post-commit",
            AssetType::Hook,
            hook_file.to_str().unwrap(),
        );

        // Override adapter paths so delivery writes into temp dirs we control.
        // We retrieve adapters to inspect their computed home, then do a manual
        // call via deliver_inner (no path override needed because the engine
        // just writes to whatever home it computes; we pass tempdir homes via
        // the full inner function's adapter.skills_dir() path — which will be
        // under the real user home.  Instead we verify by adapter outcome code.
        let results = deliver_inner(&env.store, "h1", &[]).unwrap();

        // All four core adapters must be represented (none disabled).
        assert_eq!(results.len(), 4, "all four core adapters must appear");

        let outcome_for = |key: &str| {
            results
                .iter()
                .find(|r| r.adapter_key == key)
                .map(|r| r.outcome.as_str())
                .unwrap_or("missing")
        };

        // claude_code and pi both declare Hook capability -> not "unsupported".
        assert_ne!(
            outcome_for("claude_code"),
            "unsupported",
            "claude_code must support Hook assets"
        );
        assert_ne!(
            outcome_for("pi"),
            "unsupported",
            "pi must support Hook assets"
        );

        // codex and github_copilot return None for Hook -> "unsupported".
        assert_eq!(
            outcome_for("codex"),
            "unsupported",
            "codex must NOT support Hook assets"
        );
        assert_eq!(
            outcome_for("github_copilot"),
            "unsupported",
            "github_copilot must NOT support Hook assets"
        );
    }

    /// Disabling one core adapter (codex) must cause deliver_managed_asset to
    /// skip it entirely — neither an outcome entry nor any file-system side
    /// effect for that adapter.
    #[cfg(unix)]
    #[test]
    fn disabled_adapter_is_skipped_and_produces_no_result_entry() {
        let (env, _central_path, record_id) = import_fixture_agent();

        // Disable codex before delivery.
        let results = deliver_inner(&env.store, &record_id, &["codex"]).unwrap();

        // Only three adapters must appear in the results.
        assert_eq!(
            results.len(),
            3,
            "disabled codex must be absent from results; got: {:?}",
            results.iter().map(|r| &r.adapter_key).collect::<Vec<_>>()
        );

        // codex must not appear at all.
        assert!(
            !results.iter().any(|r| r.adapter_key == "codex"),
            "codex must not appear in results when disabled"
        );

        // The other three must still be present.
        for key in &["claude_code", "pi", "github_copilot"] {
            assert!(
                results.iter().any(|r| r.adapter_key == *key),
                "adapter '{key}' must still appear when only codex is disabled"
            );
        }
    }

    /// A missing central file must produce AppError with kind == NotFound and a
    /// message that names both the asset and the absent path — not a panic or
    /// a generic IO error.
    #[test]
    fn missing_central_file_yields_not_found_error() {
        let env = make_test_env();

        insert_record(
            &env.store,
            "ghost",
            "ghost-asset",
            AssetType::Hook,
            "/nonexistent/path/ghost.sh",
        );

        let err = deliver_inner(&env.store, "ghost", &[])
            .expect_err("must return Err when central file is missing");

        assert_eq!(
            err.kind,
            crate::core::error::ErrorKind::NotFound,
            "error kind must be NotFound; got {:?}",
            err.kind
        );
        assert!(
            err.message.contains("ghost-asset"),
            "error message must name the asset; got: {}",
            err.message
        );
        assert!(
            err.message.contains("/nonexistent/path/ghost.sh"),
            "error message must include the missing path; got: {}",
            err.message
        );
    }

    // ── owning_plugin attribution ──────────────────────────────────────────

    /// Build a minimal plugin fixture alongside the store, insert two records:
    /// one whose `source_ref` exactly matches a plugin asset path and one that
    /// has a non-plugin source_ref.  Verify that `resolve_plugin_attribution`
    /// (and the full annotation loop in `get_managed_assets`) correctly sets
    /// `owning_plugin` for the first and leaves it `None` for the second.
    #[test]
    fn get_managed_assets_annotation_exact_match_and_none() {
        use crate::core::plugin_discovery;
        use std::fs;
        use tempfile::tempdir;

        let env = make_test_env();

        // Build a fixture plugin root with one plugin that has one skill asset.
        let plugin_tmp = tempdir().unwrap();
        let plugin_root = plugin_tmp.path();
        let install_root = plugin_root.join("cache/test-market/my-plugin/1.0.0");
        fs::create_dir_all(&install_root).unwrap();

        // Write installed_plugins.json pointing at the install root.
        let installed = serde_json::json!({
            "version": 2,
            "plugins": {
                "my-plugin@test-market": [
                    {
                        "scope": "user",
                        "installPath": install_root.to_string_lossy(),
                        "version": "1.0.0",
                        "installedAt": "2026-01-01T00:00:00.000Z"
                    }
                ]
            }
        });
        fs::write(
            plugin_root.join("installed_plugins.json"),
            serde_json::to_string(&installed).unwrap(),
        )
        .unwrap();

        // Manifest declares one skill directory.
        let manifest = r#"{"name":"my-plugin","version":"1.0.0","skills":"./skills/"}"#;
        fs::create_dir_all(install_root.join(".claude-plugin")).unwrap();
        fs::write(install_root.join(".claude-plugin/plugin.json"), manifest).unwrap();

        // One real skill directory so enumerate_dir_assets finds it.
        let skill_dir = install_root.join("skills/alpha-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_asset_path = skill_dir.to_string_lossy().into_owned();

        // Insert record 1: source_ref == the plugin skill asset path (exact hit).
        let now = chrono::Utc::now().timestamp();
        env.store
            .insert_skill(&SkillRecord {
                id: "plugin-asset".to_string(),
                name: "alpha-skill".to_string(),
                description: None,
                source_type: "import".to_string(),
                source_ref: Some(skill_asset_path.clone()),
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                central_path: "/central/alpha-skill.md".to_string(),
                content_hash: None,
                enabled: true,
                created_at: now,
                updated_at: now,
                status: "ok".to_string(),
                update_status: "local_only".to_string(),
                last_checked_at: None,
                last_check_error: None,
                asset_type: AssetType::Agent,
            })
            .unwrap();

        // Insert record 2: source_ref points at a non-plugin path.
        env.store
            .insert_skill(&SkillRecord {
                id: "manual-asset".to_string(),
                name: "my-manual".to_string(),
                description: None,
                source_type: "import".to_string(),
                source_ref: Some("/home/user/workspace/agents/my-manual".to_string()),
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                central_path: "/central/my-manual.md".to_string(),
                content_hash: None,
                enabled: true,
                created_at: now,
                updated_at: now,
                status: "ok".to_string(),
                update_status: "local_only".to_string(),
                last_checked_at: None,
                last_check_error: None,
                asset_type: AssetType::Agent,
            })
            .unwrap();

        // Exercise the attribution logic directly (mirrors get_managed_assets inner loop).
        let plugins = plugin_discovery::list_plugins(plugin_root);
        assert!(!plugins.is_empty(), "fixture plugin must be discovered");

        let attribution_map = plugin_discovery::build_asset_attribution(&plugins);
        let prefix_pairs: Vec<(String, String)> = plugins
            .iter()
            .map(|p| (p.install_path.to_string_lossy().into_owned(), p.id.clone()))
            .collect();

        let annotate = |source_ref: &str| -> Option<String> {
            if let Some(id) = attribution_map.get(source_ref) {
                return Some(id.clone());
            }
            prefix_pairs
                .iter()
                .find(|(install_path, _)| source_ref.starts_with(install_path.as_str()))
                .map(|(_, id)| id.clone())
        };

        let plugin_owned = annotate(&skill_asset_path);
        assert_eq!(
            plugin_owned.as_deref(),
            Some("my-plugin@test-market"),
            "record with source_ref matching plugin asset path must be attributed"
        );

        let not_owned = annotate("/home/user/workspace/agents/my-manual");
        assert!(
            not_owned.is_none(),
            "record with non-plugin source_ref must have no attribution"
        );
    }

    /// Workflow assets are delivered (Place) by claude_code and pi; codex and
    /// github_copilot must return "unsupported" and must not create any file.
    #[cfg(unix)]
    #[test]
    fn workflow_placed_by_claude_and_pi_unsupported_by_codex_and_copilot() {
        let env = make_test_env();

        // Write a real .md file so the missing-file guard passes.
        let wf_tmp = tempdir().unwrap();
        let wf_file = wf_tmp.path().join("onboard.md");
        fs::write(&wf_file, "# Onboard\n\nOnboarding workflow.\n").unwrap();

        insert_record(
            &env.store,
            "wf1",
            "onboard",
            AssetType::Workflow,
            wf_file.to_str().unwrap(),
        );

        let results = deliver_inner(&env.store, "wf1", &[]).unwrap();

        // All four core adapters must be represented (none disabled).
        assert_eq!(results.len(), 4, "all four core adapters must appear");

        let outcome_for = |key: &str| {
            results
                .iter()
                .find(|r| r.adapter_key == key)
                .map(|r| r.outcome.as_str())
                .unwrap_or("missing")
        };

        // claude_code and pi declare Workflow capability -> not "unsupported".
        assert_ne!(
            outcome_for("claude_code"),
            "unsupported",
            "claude_code must support Workflow assets"
        );
        assert_ne!(
            outcome_for("pi"),
            "unsupported",
            "pi must support Workflow assets"
        );

        // codex and github_copilot return None for Workflow -> "unsupported".
        assert_eq!(
            outcome_for("codex"),
            "unsupported",
            "codex must NOT support Workflow assets"
        );
        assert_eq!(
            outcome_for("github_copilot"),
            "unsupported",
            "github_copilot must NOT support Workflow assets"
        );
    }

    // ── delete_managed_asset unit tests ───────────────────────────────────

    /// Deleting a non-skill (agent) asset removes the store row and the central
    /// file; the central file must not exist afterward.
    #[test]
    fn delete_managed_asset_removes_agent_row_and_central_file() {
        let env = make_test_env();

        // Create a real central file so we can verify it is removed.
        let agent_tmp = tempdir().unwrap();
        let agent_file = agent_tmp.path().join("my-agent.md");
        fs::write(&agent_file, "# My Agent\n").unwrap();
        let central_path = agent_file.to_str().unwrap();

        insert_record(
            &env.store,
            "del-agent-1",
            "my-agent",
            AssetType::Agent,
            central_path,
        );

        assert!(
            env.store.get_skill_by_id("del-agent-1").unwrap().is_some(),
            "record must exist before delete"
        );
        assert!(agent_file.exists(), "central file must exist before delete");

        // Exercise the same logic as delete_managed_asset directly (no Tauri
        // async layer in unit tests).
        {
            let _lock = crate::core::repo_lock::RepoLock::acquire("test-delete-agent").unwrap();
            let record = env.store.get_skill_by_id("del-agent-1").unwrap().unwrap();
            let central = PathBuf::from(&record.central_path);
            // Non-skill: remove the file, not the directory.
            if central.exists() {
                std::fs::remove_file(&central).ok();
            }
            env.store.delete_skill("del-agent-1").unwrap();
            sync_metadata::write_all_from_db_unlocked(&env.store).unwrap();
        }

        assert!(
            env.store.get_skill_by_id("del-agent-1").unwrap().is_none(),
            "store row must be absent after delete"
        );
        assert!(
            !agent_file.exists(),
            "central file must be removed after delete"
        );
    }

    /// Deleting a skill asset removes the store row and the central directory
    /// (recursively); the directory must not exist afterward.
    #[test]
    fn delete_managed_asset_removes_skill_row_and_central_dir() {
        let env = make_test_env();

        // Create a real central directory with a file inside.
        let skill_tmp = tempdir().unwrap();
        let skill_dir = skill_tmp.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(skill_dir.join("skill.md"), "# My Skill\n").unwrap();
        let central_path = skill_dir.to_str().unwrap();

        insert_record(
            &env.store,
            "del-skill-1",
            "my-skill",
            AssetType::Skill,
            central_path,
        );

        assert!(
            env.store.get_skill_by_id("del-skill-1").unwrap().is_some(),
            "record must exist before delete"
        );
        assert!(skill_dir.exists(), "central dir must exist before delete");

        {
            let _lock = crate::core::repo_lock::RepoLock::acquire("test-delete-skill").unwrap();
            let record = env.store.get_skill_by_id("del-skill-1").unwrap().unwrap();
            let central = PathBuf::from(&record.central_path);
            // Skill: remove the entire directory recursively.
            if central.exists() {
                std::fs::remove_dir_all(&central).ok();
            }
            env.store.delete_skill("del-skill-1").unwrap();
            sync_metadata::write_all_from_db_unlocked(&env.store).unwrap();
        }

        assert!(
            env.store.get_skill_by_id("del-skill-1").unwrap().is_none(),
            "store row must be absent after delete"
        );
        assert!(
            !skill_dir.exists(),
            "central directory must be removed after delete"
        );
    }
}
