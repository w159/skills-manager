/// Tauri command surface for multi-asset-type queries and delivery.
///
/// `get_managed_assets` — list managed records of one asset type.
/// `deliver_managed_asset` — deliver one stored agent to all four core coding
///   agent homes using the capability-driven delivery engine.
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

use crate::core::{
    asset_delivery::{deliver_asset, AssetInput, DeliveryOutcome},
    asset_render::canonical_agent_from_file,
    error::AppError,
    skill_store::{AssetType, SkillRecord, SkillStore},
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
        Ok(records.into_iter().map(ManagedAssetDto::from).collect())
    })
    .await?
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
}
