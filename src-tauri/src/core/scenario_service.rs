use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use super::{
    asset_delivery::{deliver_asset, AssetInput},
    asset_render::canonical_agent_from_file,
    error::AppError,
    skill_store::{AssetType, ScenarioRecord, SkillStore, SkillTargetRecord},
    sync_engine, tool_adapters, tool_service,
};

#[derive(Debug, Clone)]
pub struct ScenarioSyncTarget {
    pub skill_id: String,
    pub skill_name: String,
    pub tool: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub mode: sync_engine::SyncMode,
    /// Current content hash of the central skill source, copied from
    /// `SkillRecord.content_hash`. Compared against the previously
    /// synced `SkillTargetRecord.source_hash` to skip redundant
    /// Copy-mode resyncs at startup (issue #153).
    pub source_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPreviewTarget {
    pub skill_id: String,
    pub skill_name: String,
    pub tool: String,
    pub target_path: String,
    pub mode: String,
}

pub fn ensure_scenario_exists(store: &SkillStore, scenario_id: &str) -> Result<(), AppError> {
    let exists = store
        .get_all_scenarios()
        .map_err(AppError::db)?
        .iter()
        .any(|s| s.id == scenario_id);
    if !exists {
        return Err(AppError::not_found("Scenario not found"));
    }
    Ok(())
}

pub fn enabled_installed_adapters_for_scenario_skill(
    store: &SkillStore,
    scenario_id: &str,
    skill_id: &str,
) -> Result<Vec<tool_adapters::ToolAdapter>, AppError> {
    let adapters = tool_adapters::enabled_installed_adapters(store);
    let adapter_keys: Vec<String> = adapters.iter().map(|a| a.key.clone()).collect();

    store
        .ensure_scenario_skill_tool_defaults(scenario_id, skill_id, &adapter_keys)
        .map_err(AppError::db)?;

    let enabled = store
        .get_enabled_tools_for_scenario_skill(scenario_id, skill_id)
        .map_err(AppError::db)?;
    let enabled_set: HashSet<String> = enabled.into_iter().collect();

    Ok(adapters
        .into_iter()
        .filter(|adapter| enabled_set.contains(&adapter.key))
        .collect())
}

pub fn collect_scenario_sync_targets(
    store: &SkillStore,
    scenario_id: &str,
) -> Result<Vec<ScenarioSyncTarget>, AppError> {
    let skills = store
        .get_skills_for_scenario(scenario_id)
        .map_err(AppError::db)?;
    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    let mut targets = Vec::new();

    for skill in &skills {
        let source = PathBuf::from(&skill.central_path);
        let target_name = sync_engine::target_dir_name(&source, &skill.name);
        let adapters =
            enabled_installed_adapters_for_scenario_skill(store, scenario_id, &skill.id)?;
        for adapter in &adapters {
            let target = adapter.skills_dir().join(&target_name);
            let mode = sync_engine::sync_mode_for_tool(&adapter.key, configured_mode.as_deref());
            targets.push(ScenarioSyncTarget {
                skill_id: skill.id.clone(),
                skill_name: skill.name.clone(),
                tool: adapter.key.clone(),
                source: source.clone(),
                target,
                mode,
                source_hash: skill.content_hash.clone(),
            });
        }
    }

    Ok(targets)
}

pub fn preview_scenario_sync(
    store: &SkillStore,
    scenario_id: &str,
) -> Result<Vec<SyncPreviewTarget>, AppError> {
    collect_scenario_sync_targets(store, scenario_id).map(|targets| {
        targets
            .into_iter()
            .map(|target| SyncPreviewTarget {
                skill_id: target.skill_id,
                skill_name: target.skill_name,
                tool: target.tool,
                target_path: target.target.to_string_lossy().to_string(),
                mode: target.mode.as_str().to_string(),
            })
            .collect()
    })
}

/// Decide which `SyncMode` `is_target_current` should compare against, or
/// `None` if the existing target's mode is incompatible with the desired
/// mode and the skip path must be refused.
///
/// Returns `Some(existing)` when both modes match exactly. Also returns
/// `Some(Copy)` when the existing record is `"copy"` but the desired
/// mode is `Symlink` — this is the Windows fallback case (issue #153):
/// `symlink_dir()` failed on a prior run and we landed in copy mode, so
/// every subsequent startup would re-attempt symlink, fail again, and
/// trigger a full recursive copy. Treating the existing copy as
/// compatible lets the hash gate skip when the source hasn't changed.
///
/// The reverse direction (existing `"symlink"`, desired `Copy`) returns
/// `None` because the user actively changed the `sync_mode` setting and
/// the on-disk symlink doesn't reflect that intent.
fn skip_check_mode(
    existing_mode: &str,
    desired: sync_engine::SyncMode,
) -> Option<sync_engine::SyncMode> {
    match (existing_mode, desired) {
        ("symlink", sync_engine::SyncMode::Symlink) => Some(sync_engine::SyncMode::Symlink),
        ("copy", sync_engine::SyncMode::Copy) => Some(sync_engine::SyncMode::Copy),
        ("copy", sync_engine::SyncMode::Symlink) => Some(sync_engine::SyncMode::Copy),
        _ => None,
    }
}

pub fn sync_desired_targets(
    store: &SkillStore,
    desired_targets: &[ScenarioSyncTarget],
) -> Result<(), AppError> {
    let batch_start = Instant::now();
    let existing_targets: HashMap<(String, String), SkillTargetRecord> = store
        .get_all_targets()
        .map_err(AppError::db)?
        .into_iter()
        .map(|target| ((target.skill_id.clone(), target.tool.clone()), target))
        .collect();

    let mut synced_count = 0usize;
    let mut skipped_count = 0usize;
    let mut failed_count = 0usize;

    for desired in desired_targets {
        let target_start = Instant::now();
        let key = (desired.skill_id.clone(), desired.tool.clone());
        if let Some(existing) = existing_targets.get(&key) {
            let target_path = PathBuf::from(&existing.target_path);
            if target_path != desired.target {
                if let Err(e) = sync_engine::remove_target(&target_path) {
                    log::warn!(
                        "Failed to remove stale target {}: {e}",
                        target_path.display()
                    );
                }
                if let Err(e) = store.delete_target(&desired.skill_id, &desired.tool) {
                    log::warn!(
                        "Failed to delete stale target record for skill {}, tool {}: {e}",
                        desired.skill_id,
                        desired.tool
                    );
                }
            } else if existing.status == "ok" {
                if let Some(check_mode) = skip_check_mode(&existing.mode, desired.mode) {
                    if sync_engine::is_target_current(
                        &desired.source,
                        &desired.target,
                        check_mode,
                        existing.source_hash.as_deref(),
                        desired.source_hash.as_deref(),
                    ) {
                        // Surface the Windows fallback case in logs so operators
                        // can tell when a target is permanently on Copy because
                        // an earlier symlink_dir() failed (issue #153). Helpful
                        // when a user later enables Developer Mode and wonders
                        // why Symlink isn't being re-attempted.
                        if existing.mode == "copy"
                            && matches!(desired.mode, sync_engine::SyncMode::Symlink)
                        {
                            log::debug!(
                                "sync_desired_targets: skill {} ({}) staying on copy fallback for {} (content unchanged); trigger a manual resync to retry symlink",
                                desired.skill_id,
                                desired.skill_name,
                                desired.tool
                            );
                        }
                        skipped_count += 1;
                        continue;
                    }
                }
            }
        }

        match sync_engine::sync_skill(&desired.source, &desired.target, desired.mode) {
            Ok(actual_mode) => {
                let now = chrono::Utc::now().timestamp_millis();
                let target_record = SkillTargetRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    skill_id: desired.skill_id.clone(),
                    tool: desired.tool.clone(),
                    target_path: desired.target.to_string_lossy().to_string(),
                    mode: actual_mode.as_str().to_string(),
                    status: "ok".to_string(),
                    synced_at: Some(now),
                    last_error: None,
                    // Record the hash that was just synced so the next
                    // run of this loop can short-circuit when the central
                    // skill content has not changed (issue #153).
                    source_hash: desired.source_hash.clone(),
                };
                if let Err(e) = store.insert_target(&target_record) {
                    log::warn!(
                        "Failed to insert sync target for skill {}: {e}",
                        desired.skill_id
                    );
                }
                synced_count += 1;
                let elapsed = target_start.elapsed().as_millis();
                if elapsed >= 200 {
                    log::warn!(
                        "sync_desired_targets: slow sync ({elapsed} ms, mode={}) for skill {} ({}) -> {}",
                        actual_mode.as_str(),
                        desired.skill_id,
                        desired.skill_name,
                        desired.target.display()
                    );
                }
            }
            Err(e) => {
                failed_count += 1;
                log::warn!(
                    "Failed to sync skill {} ({}) to {} after {} ms: {e}",
                    desired.skill_id,
                    desired.skill_name,
                    desired.target.display(),
                    target_start.elapsed().as_millis()
                );
            }
        }
    }

    log::info!(
        "sync_desired_targets: {} targets in {} ms (synced={synced_count}, skipped={skipped_count}, failed={failed_count})",
        desired_targets.len(),
        batch_start.elapsed().as_millis()
    );

    Ok(())
}

pub fn unsync_obsolete_scenario_targets(
    store: &SkillStore,
    old_scenario_id: &str,
    desired_targets: &[ScenarioSyncTarget],
) -> Result<(), AppError> {
    let desired_paths: HashMap<(String, String), PathBuf> = desired_targets
        .iter()
        .map(|target| {
            (
                (target.skill_id.clone(), target.tool.clone()),
                target.target.clone(),
            )
        })
        .collect();

    let old_skill_ids = store
        .get_skill_ids_for_scenario(old_scenario_id)
        .map_err(AppError::db)?;
    for skill_id in &old_skill_ids {
        let targets = store.get_targets_for_skill(skill_id).unwrap_or_default();
        for target in &targets {
            let path = PathBuf::from(&target.target_path);
            let key = (skill_id.clone(), target.tool.clone());
            if desired_paths.get(&key) == Some(&path) {
                continue;
            }

            if let Err(e) = sync_engine::remove_target(&path) {
                log::warn!("Failed to remove sync target {}: {e}", path.display());
            }
            if let Err(e) = store.delete_target(skill_id, &target.tool) {
                log::warn!(
                    "Failed to delete target record for skill {skill_id}, tool {}: {e}",
                    target.tool
                );
            }
        }
    }

    Ok(())
}

pub fn unsync_scenario_skills(store: &SkillStore, scenario_id: &str) -> Result<(), AppError> {
    let skill_ids = store
        .get_skill_ids_for_scenario(scenario_id)
        .map_err(AppError::db)?;

    for skill_id in &skill_ids {
        let targets = store.get_targets_for_skill(skill_id).unwrap_or_default();
        for target in &targets {
            let path = PathBuf::from(&target.target_path);
            if let Err(e) = sync_engine::remove_target(&path) {
                log::warn!("Failed to remove sync target {}: {e}", path.display());
            }
            if let Err(e) = store.delete_target(skill_id, &target.tool) {
                log::warn!(
                    "Failed to delete target record for skill {skill_id}, tool {}: {e}",
                    target.tool
                );
            }
        }
    }

    Ok(())
}

pub fn sync_scenario_skills(store: &SkillStore, scenario_id: &str) -> Result<(), AppError> {
    let desired_targets = collect_scenario_sync_targets(store, scenario_id)?;
    sync_desired_targets(store, &desired_targets)
}

pub fn apply_scenario_to_default(store: &SkillStore, scenario_id: &str) -> Result<(), AppError> {
    ensure_scenario_exists(store, scenario_id)?;
    let desired_targets = collect_scenario_sync_targets(store, scenario_id)?;

    if let Ok(Some(old_id)) = store.get_active_scenario_id() {
        if old_id != scenario_id {
            unsync_obsolete_scenario_targets(store, &old_id, &desired_targets)?;
        }
    }

    store
        .set_active_scenario(scenario_id)
        .map_err(AppError::db)?;
    sync_desired_targets(store, &desired_targets)
}

pub fn sync_skill_to_active_scenario(
    store: &SkillStore,
    scenario_id: &str,
    skill_id: &str,
) -> Result<(), AppError> {
    if let Ok(Some(active_id)) = store.get_active_scenario_id() {
        if active_id == scenario_id {
            let adapters =
                enabled_installed_adapters_for_scenario_skill(store, scenario_id, skill_id)?;
            let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
            let Ok(Some(skill)) = store.get_skill_by_id(skill_id) else {
                return Ok(());
            };
            let source = PathBuf::from(&skill.central_path);
            let target_name = sync_engine::target_dir_name(&source, &skill.name);
            let old_targets = store.get_targets_for_skill(skill_id).unwrap_or_default();
            for adapter in &adapters {
                if let Some(old) = old_targets.iter().find(|t| t.tool == adapter.key) {
                    let old_path = PathBuf::from(&old.target_path);
                    if old_path != adapter.skills_dir().join(&target_name) {
                        if let Err(e) = sync_engine::remove_target(&old_path) {
                            log::warn!("Failed to remove stale target {}: {e}", old_path.display());
                        }
                        let _ = store.delete_target(skill_id, &adapter.key);
                    }
                }

                let target = adapter.skills_dir().join(&target_name);
                let mode =
                    sync_engine::sync_mode_for_tool(&adapter.key, configured_mode.as_deref());
                match sync_engine::sync_skill(&source, &target, mode) {
                    Ok(actual_mode) => {
                        let now = chrono::Utc::now().timestamp_millis();
                        let target_record = super::skill_store::SkillTargetRecord {
                            id: uuid::Uuid::new_v4().to_string(),
                            skill_id: skill_id.to_string(),
                            tool: adapter.key.clone(),
                            target_path: target.to_string_lossy().to_string(),
                            mode: actual_mode.as_str().to_string(),
                            status: "ok".to_string(),
                            synced_at: Some(now),
                            last_error: None,
                            source_hash: skill.content_hash.clone(),
                        };
                        if let Err(e) = store.insert_target(&target_record) {
                            log::warn!("Failed to insert sync target for skill {skill_id}: {e}");
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to sync skill {skill_id} to {}: {e}",
                            target.display()
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn ensure_default_startup_scenario(store: &SkillStore) -> Result<(), AppError> {
    let mut scenarios = store.get_all_scenarios().map_err(AppError::db)?;
    if scenarios.is_empty() {
        let now = chrono::Utc::now().timestamp_millis();
        let default_scenario = ScenarioRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Default".to_string(),
            description: Some("Default startup scenario".to_string()),
            icon: None,
            sort_order: 0,
            created_at: now,
            updated_at: now,
        };
        store
            .insert_scenario(&default_scenario)
            .map_err(AppError::db)?;
        scenarios.push(default_scenario);
    }

    let current_active = store.get_active_scenario_id().map_err(AppError::db)?;
    let preferred_default = store.get_setting("default_scenario").ok().flatten();

    let desired_active = preferred_default
        .filter(|id| scenarios.iter().any(|scenario| scenario.id == *id))
        .or_else(|| {
            current_active
                .clone()
                .filter(|id| scenarios.iter().any(|scenario| scenario.id == *id))
        })
        .unwrap_or_else(|| scenarios[0].id.clone());

    if current_active.as_deref() != Some(desired_active.as_str()) {
        if let Some(old_active) = current_active.as_deref() {
            unsync_scenario_skills(store, old_active)?;
        }
        store
            .set_active_scenario(&desired_active)
            .map_err(AppError::db)?;
    }

    sync_scenario_skills(store, &desired_active)
}

pub fn ensure_cli_scenario_state(store: &SkillStore) -> Result<(), AppError> {
    let mut scenarios = store.get_all_scenarios().map_err(AppError::db)?;
    if scenarios.is_empty() {
        let now = chrono::Utc::now().timestamp_millis();
        let default_scenario = ScenarioRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Default".to_string(),
            description: Some("Default startup scenario".to_string()),
            icon: None,
            sort_order: 0,
            created_at: now,
            updated_at: now,
        };
        store
            .insert_scenario(&default_scenario)
            .map_err(AppError::db)?;
        scenarios.push(default_scenario);
    }

    let current_active = store.get_active_scenario_id().map_err(AppError::db)?;
    if current_active
        .as_deref()
        .is_some_and(|id| scenarios.iter().any(|scenario| scenario.id == id))
    {
        return Ok(());
    }

    let preferred_default = store.get_setting("default_scenario").ok().flatten();
    let desired_active = preferred_default
        .filter(|id| scenarios.iter().any(|scenario| scenario.id == *id))
        .unwrap_or_else(|| scenarios[0].id.clone());

    store
        .set_active_scenario(&desired_active)
        .map_err(AppError::db)
}

pub fn restore_all_skills_sync_included(store: &SkillStore) -> Result<bool, AppError> {
    let mut changed = false;
    for skill in store.get_all_skills().map_err(AppError::db)? {
        if !skill.enabled {
            store
                .update_skill_enabled(&skill.id, true)
                .map_err(AppError::db)?;
            changed = true;
        }
    }
    Ok(changed)
}

pub fn sync_active_scenario_to_tool(store: &SkillStore, tool_key: &str) {
    if let Ok(Some(active_id)) = store.get_active_scenario_id() {
        let Ok(skill_ids) = store.get_skill_ids_for_scenario(&active_id) else {
            return;
        };
        for skill_id in skill_ids {
            if let Ok(adapters) =
                enabled_installed_adapters_for_scenario_skill(store, &active_id, &skill_id)
            {
                if adapters.iter().any(|adapter| adapter.key == tool_key) {
                    let _ = sync_skill_to_active_scenario(store, &active_id, &skill_id);
                }
            }
        }
    }
}

pub fn sync_single_skill_to_tool(
    store: &SkillStore,
    skill_id: &str,
    tool: &str,
) -> Result<(), AppError> {
    let adapter = tool_adapters::find_adapter_with_store(store, tool)
        .ok_or_else(|| AppError::not_found(format!("Unknown tool: {}", tool)))?;

    if !adapter.is_installed() {
        return Err(AppError::not_found(format!(
            "{} is not installed",
            adapter.display_name
        )));
    }

    if tool_service::get_disabled_tools(store).contains(&tool.to_string()) {
        return Err(AppError::invalid_input(format!(
            "{} is disabled",
            adapter.display_name
        )));
    }

    let skill = store
        .get_skill_by_id(skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    let source = PathBuf::from(&skill.central_path);
    let now = chrono::Utc::now().timestamp_millis();

    // Branch on asset_type: agents are delivered by the generic capability
    // engine (symlink for Claude/Pi, rendered file for Codex/Copilot).
    // Every other type (Skill and any future additions) continues on the
    // existing sync_engine path so its behavior is byte-for-byte unchanged.
    if skill.asset_type == AssetType::Agent {
        let canonical_agent = canonical_agent_from_file(&source).map_err(AppError::io)?;

        // home is the adapter root (e.g. ~/.claude); skills_dir is one level
        // deeper (e.g. ~/.claude/skills).  deliver_asset uses home, not
        // skills_dir, to resolve capability-specific subdirs.
        let home = adapter
            .skills_dir()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| adapter.skills_dir());

        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &source,
            id: &skill.name,
            name: &skill.name,
            canonical_agent: Some(&canonical_agent),
        };

        let outcome = deliver_asset(&adapter, &home, &input).map_err(AppError::io)?;

        // Map the real DeliveryOutcome to (target_path, mode_str) so the
        // skill_targets row reflects what actually landed on disk.
        //
        // Mirroring deliver_managed_asset (commands/assets.rs ~607-625):
        //   - Symlinked / Rendered / RenderedUpToDate / Placed: extract the
        //     concrete path and the mode string from the outcome variant.
        //   - ForeignHome / Unsupported / DeferToSkillPath: nothing was
        //     written; do NOT record a successful target row.  Return early
        //     instead so a refused or no-op delivery is never logged as "ok".
        use crate::core::asset_delivery::DeliveryOutcome;
        let (target_path, mode_str) = match &outcome {
            // Defect 1 fix: derive mode from the actual outcome, not a
            // hardcoded "symlink".  Symlinked -> "symlink", Rendered /
            // RenderedUpToDate -> "render" (matches the skill branch's
            // `actual_mode.as_str()` pattern), Placed -> "place".
            DeliveryOutcome::Symlinked(p) => {
                (p.to_string_lossy().to_string(), "symlink".to_string())
            }
            DeliveryOutcome::Rendered(p) | DeliveryOutcome::RenderedUpToDate(p) => {
                (p.to_string_lossy().to_string(), "render".to_string())
            }
            DeliveryOutcome::Placed(p) => (p.to_string_lossy().to_string(), "place".to_string()),
            // Defect 2 fix: refused / unsupported deliveries produce no
            // on-disk artifact; recording them as status "ok" / mode
            // "symlink" would be a false positive.  Return early without
            // inserting any skill_targets row, matching the no-insert path
            // in deliver_managed_asset (commands/assets.rs ~620-625).
            DeliveryOutcome::ForeignHome(_)
            | DeliveryOutcome::Unsupported
            | DeliveryOutcome::DeferToSkillPath => {
                return Ok(());
            }
        };

        let target_record = SkillTargetRecord {
            id: uuid::Uuid::new_v4().to_string(),
            skill_id: skill_id.to_string(),
            tool: tool.to_string(),
            target_path,
            mode: mode_str,
            status: "ok".to_string(),
            synced_at: Some(now),
            last_error: None,
            source_hash: skill.content_hash.clone(),
        };

        store.insert_target(&target_record).map_err(AppError::db)?;
        return Ok(());
    }

    // Skill (and any unhandled asset types): existing sync_engine path,
    // preserved byte-for-byte.
    let target = adapter
        .skills_dir()
        .join(sync_engine::target_dir_name(&source, &skill.name));
    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    let mode = sync_engine::sync_mode_for_tool(tool, configured_mode.as_deref());
    let actual_mode = sync_engine::sync_skill(&source, &target, mode).map_err(AppError::io)?;

    let target_record = SkillTargetRecord {
        id: uuid::Uuid::new_v4().to_string(),
        skill_id: skill_id.to_string(),
        tool: tool.to_string(),
        target_path: target.to_string_lossy().to_string(),
        mode: actual_mode.as_str().to_string(),
        status: "ok".to_string(),
        synced_at: Some(now),
        last_error: None,
        source_hash: skill.content_hash.clone(),
    };

    store.insert_target(&target_record).map_err(AppError::db)?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum BatchApplyMode {
    Add,
    Remove,
}

/// Apply a batch of `(skill_id × tool_key)` pairs in either Add or Remove mode
/// without touching `active_scenario_id` or `scenario_skill_tools` toggles.
///
/// This is the tray-side preset apply primitive. Unlike [`sync_single_skill_to_tool`]
/// (which is wrapped by the `sync_skill_to_tool` Tauri command and carries the
/// implicit active-preset toggle side-effect), this batch is a pure
/// "write/remove files + maintain `skill_targets` rows" operation.
///
/// Remove mode handles shared physical paths: a `target_path` may be referenced
/// by multiple `(skill_id, tool)` records when several tools resolve to the same
/// skills directory. The filesystem path is only removed when no remaining
/// `skill_targets` row references it after the batch deletions, so removing one
/// preset's tools never wipes another tool's still-active files.
pub fn apply_skills_to_tools(
    store: &SkillStore,
    skill_ids: &[String],
    tool_keys: &[String],
    mode: BatchApplyMode,
) -> Result<(), AppError> {
    if skill_ids.is_empty() || tool_keys.is_empty() {
        return Ok(());
    }

    match mode {
        BatchApplyMode::Add => apply_add(store, skill_ids, tool_keys),
        BatchApplyMode::Remove => apply_remove(store, skill_ids, tool_keys),
    }
}

fn apply_add(
    store: &SkillStore,
    skill_ids: &[String],
    tool_keys: &[String],
) -> Result<(), AppError> {
    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    let disabled = tool_service::get_disabled_tools(store);

    let mut adapters: HashMap<String, tool_adapters::ToolAdapter> = HashMap::new();
    for key in tool_keys {
        if disabled.contains(key) {
            log::debug!("apply_skills_to_tools: skipping disabled tool {key}");
            continue;
        }
        let Some(adapter) = tool_adapters::find_adapter_with_store(store, key) else {
            log::warn!("apply_skills_to_tools: unknown tool {key}");
            continue;
        };
        if !adapter.is_installed() {
            log::debug!(
                "apply_skills_to_tools: skipping uninstalled tool {} ({key})",
                adapter.display_name
            );
            continue;
        }
        adapters.insert(key.clone(), adapter);
    }

    let mut synced = 0usize;
    let mut failed = 0usize;
    for skill_id in skill_ids {
        let Ok(Some(skill)) = store.get_skill_by_id(skill_id) else {
            log::warn!("apply_skills_to_tools: skill {skill_id} not found");
            continue;
        };
        let source = PathBuf::from(&skill.central_path);

        // Agents are delivered by the capability engine (symlink for Claude/Pi,
        // rendered file for Codex/Copilot).  Route through sync_single_skill_to_tool
        // so every asset type gets the same logic as the single-command path.
        // Skills continue on the direct sync_engine path below (byte-for-byte
        // identical to the original loop body).
        if skill.asset_type == AssetType::Agent {
            for tool_key in adapters.keys() {
                match sync_single_skill_to_tool(store, skill_id, tool_key) {
                    Ok(()) => {
                        synced += 1;
                    }
                    Err(e) => {
                        failed += 1;
                        log::warn!(
                            "apply_skills_to_tools: failed to sync agent {skill_id} ({}) to {tool_key}: {e}",
                            skill.name,
                        );
                    }
                }
            }
            continue;
        }

        let target_name = sync_engine::target_dir_name(&source, &skill.name);
        for (tool_key, adapter) in &adapters {
            let target = adapter.skills_dir().join(&target_name);
            let mode = sync_engine::sync_mode_for_tool(tool_key, configured_mode.as_deref());
            match sync_engine::sync_skill(&source, &target, mode) {
                Ok(actual_mode) => {
                    let now = chrono::Utc::now().timestamp_millis();
                    let target_record = SkillTargetRecord {
                        id: uuid::Uuid::new_v4().to_string(),
                        skill_id: skill_id.clone(),
                        tool: tool_key.clone(),
                        target_path: target.to_string_lossy().to_string(),
                        mode: actual_mode.as_str().to_string(),
                        status: "ok".to_string(),
                        synced_at: Some(now),
                        last_error: None,
                        source_hash: skill.content_hash.clone(),
                    };
                    if let Err(e) = store.insert_target(&target_record) {
                        log::warn!(
                            "apply_skills_to_tools: failed to insert target for skill {skill_id} / {tool_key}: {e}"
                        );
                        failed += 1;
                    } else {
                        synced += 1;
                    }
                }
                Err(e) => {
                    failed += 1;
                    log::warn!(
                        "apply_skills_to_tools: failed to sync skill {skill_id} ({}) to {}: {e}",
                        skill.name,
                        target.display()
                    );
                }
            }
        }
    }

    log::info!(
        "apply_skills_to_tools(Add): skills={} tools={} synced={synced} failed={failed}",
        skill_ids.len(),
        adapters.len(),
    );
    Ok(())
}

fn apply_remove(
    store: &SkillStore,
    skill_ids: &[String],
    tool_keys: &[String],
) -> Result<(), AppError> {
    let tool_set: HashSet<&String> = tool_keys.iter().collect();

    let mut to_delete: Vec<(String, String, PathBuf)> = Vec::new();
    for skill_id in skill_ids {
        let targets = store.get_targets_for_skill(skill_id).unwrap_or_default();
        for target in targets {
            if tool_set.contains(&target.tool) {
                to_delete.push((
                    skill_id.clone(),
                    target.tool.clone(),
                    PathBuf::from(&target.target_path),
                ));
            }
        }
    }

    if to_delete.is_empty() {
        return Ok(());
    }

    // Phase 1: drop the DB rows first so the post-delete recount below sees
    // the new ground truth when deciding which filesystem paths to keep.
    for (skill_id, tool, _) in &to_delete {
        if let Err(e) = store.delete_target(skill_id, tool) {
            log::warn!(
                "apply_skills_to_tools(Remove): failed to delete target record for skill {skill_id} / {tool}: {e}"
            );
        }
    }

    // Phase 2: gather the paths the batch wanted to remove, then keep any path
    // a remaining (skill_id, tool) row still points at. This prevents wiping a
    // directory another adapter is sharing.
    let candidate_paths: HashSet<PathBuf> = to_delete.iter().map(|(_, _, p)| p.clone()).collect();
    let still_referenced: HashSet<PathBuf> = store
        .get_all_targets()
        .unwrap_or_default()
        .into_iter()
        .map(|t| PathBuf::from(&t.target_path))
        .collect();

    let mut removed = 0usize;
    for path in candidate_paths {
        if still_referenced.contains(&path) {
            log::debug!(
                "apply_skills_to_tools(Remove): keeping {} (still referenced by another target)",
                path.display()
            );
            continue;
        }
        if let Err(e) = sync_engine::remove_target(&path) {
            log::warn!(
                "apply_skills_to_tools(Remove): failed to remove {}: {e}",
                path.display()
            );
        } else {
            removed += 1;
        }
    }

    log::info!(
        "apply_skills_to_tools(Remove): pairs={} fs_removed={removed}",
        to_delete.len(),
    );
    Ok(())
}

#[cfg(test)]
mod sync_desired_targets_tests {
    use super::*;
    use crate::core::central_repo;
    use crate::core::skill_store::{SkillRecord, SkillStore, SkillTargetRecord};
    use std::fs;
    use tempfile::tempdir;

    /// Issue #153 regression: when the existing target was written in
    /// Copy mode (Windows symlink fallback) but the configured mode is
    /// Symlink, and the source content hash hasn't changed, the sync
    /// must be skipped. Prior to the fix the mode-equality guard would
    /// reject the skip branch and re-attempt the full recursive copy
    /// every startup.
    #[test]
    fn copy_fallback_target_with_matching_hash_is_skipped() {
        let _lock = central_repo::test_base_dir_lock();
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("repo");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        fs::create_dir_all(central_repo::skills_dir()).unwrap();
        let store = SkillStore::new(&base.join("test.db")).unwrap();

        // Real source dir with one file (the central skill).
        let source = central_repo::skills_dir().join("skill-a");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "real source").unwrap();

        // Pre-existing target dir with a marker file that would be wiped
        // by copy_dir_recursive's pre-clean step if a re-sync ran.
        let target = tmp.path().join("agent-skills").join("skill-a");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("MARKER.txt"), "do not wipe me").unwrap();

        // DB rows: skill content_hash = "h1"; existing target also at "h1",
        // mode "copy" (i.e. previously fell back from Symlink).
        let skill = SkillRecord {
            id: "skill-a".to_string(),
            name: "skill-a".to_string(),
            description: None,
            source_type: "import".to_string(),
            source_ref: Some(source.to_string_lossy().to_string()),
            source_ref_resolved: None,
            source_subpath: None,
            source_branch: None,
            source_revision: None,
            remote_revision: None,
            central_path: source.to_string_lossy().to_string(),
            content_hash: Some("h1".to_string()),
            enabled: true,
            created_at: 1,
            updated_at: 1,
            status: "ok".to_string(),
            update_status: "local_only".to_string(),
            last_checked_at: None,
            last_check_error: None,
            asset_type: crate::core::skill_store::AssetType::Skill,
        };
        store.insert_skill(&skill).unwrap();

        store
            .insert_target(&SkillTargetRecord {
                id: "target-1".to_string(),
                skill_id: "skill-a".to_string(),
                tool: "claude-code".to_string(),
                target_path: target.to_string_lossy().to_string(),
                mode: "copy".to_string(),
                status: "ok".to_string(),
                synced_at: Some(1),
                last_error: None,
                source_hash: Some("h1".to_string()),
            })
            .unwrap();

        // Desired target: same source/target/hash but Symlink mode
        // (the configured default that originally fell back to Copy).
        let desired = vec![ScenarioSyncTarget {
            skill_id: "skill-a".to_string(),
            skill_name: "skill-a".to_string(),
            tool: "claude-code".to_string(),
            source: source.clone(),
            target: target.clone(),
            mode: sync_engine::SyncMode::Symlink,
            source_hash: Some("h1".to_string()),
        }];

        sync_desired_targets(&store, &desired).unwrap();

        // The marker file proves no re-sync ran (a real re-sync would
        // have called copy_dir_recursive after wiping the target).
        assert!(
            target.join("MARKER.txt").exists(),
            "target dir was wiped — skip did not fire"
        );
        // The skill's actual SKILL.md should NOT have been copied in,
        // because we skipped the sync entirely.
        assert!(
            !target.join("SKILL.md").exists(),
            "SKILL.md appeared — sync ran instead of skipping"
        );

        central_repo::set_test_base_dir_override(None);
    }

    /// Companion: if the target has been manually deleted, even with a
    /// matching hash, we must NOT skip — the user's agent dir is
    /// otherwise left broken.
    #[test]
    fn deleted_target_with_matching_hash_forces_resync() {
        let _lock = central_repo::test_base_dir_lock();
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("repo");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        fs::create_dir_all(central_repo::skills_dir()).unwrap();
        let store = SkillStore::new(&base.join("test.db")).unwrap();

        let source = central_repo::skills_dir().join("skill-b");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "real source").unwrap();

        // Target path that does NOT exist on disk.
        let target = tmp.path().join("agent-skills").join("skill-b");

        let skill = SkillRecord {
            id: "skill-b".to_string(),
            name: "skill-b".to_string(),
            description: None,
            source_type: "import".to_string(),
            source_ref: Some(source.to_string_lossy().to_string()),
            source_ref_resolved: None,
            source_subpath: None,
            source_branch: None,
            source_revision: None,
            remote_revision: None,
            central_path: source.to_string_lossy().to_string(),
            content_hash: Some("h1".to_string()),
            enabled: true,
            created_at: 1,
            updated_at: 1,
            status: "ok".to_string(),
            update_status: "local_only".to_string(),
            last_checked_at: None,
            last_check_error: None,
            asset_type: crate::core::skill_store::AssetType::Skill,
        };
        store.insert_skill(&skill).unwrap();

        store
            .insert_target(&SkillTargetRecord {
                id: "target-2".to_string(),
                skill_id: "skill-b".to_string(),
                tool: "claude-code".to_string(),
                target_path: target.to_string_lossy().to_string(),
                mode: "copy".to_string(),
                status: "ok".to_string(),
                synced_at: Some(1),
                last_error: None,
                source_hash: Some("h1".to_string()),
            })
            .unwrap();

        let desired = vec![ScenarioSyncTarget {
            skill_id: "skill-b".to_string(),
            skill_name: "skill-b".to_string(),
            tool: "claude-code".to_string(),
            source: source.clone(),
            target: target.clone(),
            mode: sync_engine::SyncMode::Copy,
            source_hash: Some("h1".to_string()),
        }];

        sync_desired_targets(&store, &desired).unwrap();

        // Sync must have run — target should now exist with the source content.
        assert!(
            target.join("SKILL.md").exists(),
            "missing target was not re-synced"
        );

        central_repo::set_test_base_dir_override(None);
    }
}

#[cfg(test)]
mod skip_check_mode_tests {
    use super::skip_check_mode;
    use super::sync_engine::SyncMode;

    #[test]
    fn matching_modes_are_compatible() {
        assert!(matches!(
            skip_check_mode("symlink", SyncMode::Symlink),
            Some(SyncMode::Symlink)
        ));
        assert!(matches!(
            skip_check_mode("copy", SyncMode::Copy),
            Some(SyncMode::Copy)
        ));
    }

    #[test]
    fn copy_existing_with_symlink_desired_treated_as_copy() {
        // Windows fallback case (issue #153): record says copy because
        // symlink_dir failed previously. We accept that and let the hash
        // gate decide freshness, instead of re-attempting symlink and
        // triggering a full recopy on every startup.
        assert!(matches!(
            skip_check_mode("copy", SyncMode::Symlink),
            Some(SyncMode::Copy)
        ));
    }

    #[test]
    fn symlink_existing_with_copy_desired_is_incompatible() {
        // User flipped sync_mode setting from symlink to copy — the
        // on-disk symlink no longer reflects intent, must resync.
        assert!(skip_check_mode("symlink", SyncMode::Copy).is_none());
    }

    #[test]
    fn unknown_existing_mode_is_incompatible() {
        assert!(skip_check_mode("garbage", SyncMode::Symlink).is_none());
        assert!(skip_check_mode("", SyncMode::Copy).is_none());
    }
}

// ── Tests for sync_single_skill_to_tool with AssetType::Agent ────────────────
//
// These tests drive the FULL sync_single_skill_to_tool path with an Agent
// record and verify BOTH the on-disk artifact AND the skill_targets row for
// each of the three agent-capable adapters (claude_code, codex, github_copilot).
//
// They also confirm that a refused/unsupported delivery does NOT insert a
// successful-symlink row (Defect 2 regression guard).
#[cfg(test)]
#[cfg(unix)]
mod sync_single_skill_agent_tests {
    use super::*;
    use crate::core::{central_repo, skill_store::SkillRecord};
    use std::collections::HashMap;
    use std::fs;
    use tempfile::tempdir;

    // Minimal canonical-agent .md file.  The frontmatter must be parseable by
    // canonical_agent_from_file; name becomes the artifact filename stem.
    const AGENT_MD: &str = "\
---
name: test-agent
description: A test agent for sync tests.
tools:
  - Read
---

Body content here.
";

    /// Build a store with:
    ///  - central_repo redirected to a temp dir (so insert_skill does not
    ///    write to the real ~/.skills-manager repo)
    ///  - one Agent SkillRecord whose central_path points to a real .md file
    ///  - custom_tool_paths overriding claude_code / codex / github_copilot to
    ///    temp dirs so is_installed() returns true and delivery writes there
    ///
    /// Returns (store, lock, agent_file_tempdir, [claude_home, codex_home,
    /// copilot_home]).  Keep all TempDirs alive for the duration of the test.
    fn setup() -> (
        crate::core::skill_store::SkillStore,
        std::sync::MutexGuard<'static, ()>,
        tempfile::TempDir, // central repo temp dir (keeps lock alive)
        tempfile::TempDir, // claude home temp dir
        tempfile::TempDir, // codex home temp dir
        tempfile::TempDir, // copilot home temp dir
    ) {
        let lock = central_repo::test_base_dir_lock();
        let central_tmp = tempdir().unwrap();
        let base = central_tmp.path().join("repo");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        central_repo::ensure_central_repo().unwrap();
        let store = crate::core::skill_store::SkillStore::new(&base.join("test.db")).unwrap();

        // Write the agent .md file into the central tmp dir.
        let agent_file = central_tmp.path().join("test-agent.md");
        fs::write(&agent_file, AGENT_MD).unwrap();

        // Temp homes for each adapter.  override_skills_dir must point to the
        // skills sub-directory; home is derived as skills_dir().parent().
        let claude_tmp = tempdir().unwrap();
        let codex_tmp = tempdir().unwrap();
        let copilot_tmp = tempdir().unwrap();

        // Each override_skills_dir is <home>/skills so that home = <tmp_root>.
        // deliver_asset writes to <home>/agents/<id>.<ext>.
        let claude_skills = claude_tmp.path().join("skills");
        let codex_skills = codex_tmp.path().join("skills");
        let copilot_skills = copilot_tmp.path().join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        fs::create_dir_all(&codex_skills).unwrap();
        fs::create_dir_all(&copilot_skills).unwrap();

        // Persist the overrides so find_adapter_with_store picks them up.
        // custom_tool_paths is a HashMap<adapter_key, override_skills_dir>.
        let paths: HashMap<String, String> = [
            (
                "claude_code".to_string(),
                claude_skills.to_string_lossy().to_string(),
            ),
            (
                "codex".to_string(),
                codex_skills.to_string_lossy().to_string(),
            ),
            (
                "github_copilot".to_string(),
                copilot_skills.to_string_lossy().to_string(),
            ),
        ]
        .into_iter()
        .collect();
        store
            .set_setting("custom_tool_paths", &serde_json::to_string(&paths).unwrap())
            .unwrap();

        // Insert the SkillRecord.
        let now = chrono::Utc::now().timestamp();
        store
            .insert_skill(&SkillRecord {
                id: "test-agent".to_string(),
                name: "test-agent".to_string(),
                description: None,
                source_type: "import".to_string(),
                source_ref: None,
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                central_path: agent_file.to_string_lossy().to_string(),
                content_hash: None,
                enabled: true,
                created_at: now,
                updated_at: now,
                status: "ok".to_string(),
                update_status: "local_only".to_string(),
                last_checked_at: None,
                last_check_error: None,
                asset_type: crate::core::skill_store::AssetType::Agent,
            })
            .unwrap();

        (store, lock, central_tmp, claude_tmp, codex_tmp, copilot_tmp)
    }

    // ── claude_code: symlink at agents/test-agent.md ───────────────────────

    #[test]
    fn agent_sync_claude_code_creates_symlink_and_records_symlink_mode() {
        let (store, _lock, _central, claude_tmp, _codex, _copilot) = setup();

        sync_single_skill_to_tool(&store, "test-agent", "claude_code").unwrap();

        // On-disk: agents/test-agent.md must be a symlink.
        let artifact = claude_tmp.path().join("agents").join("test-agent.md");
        assert!(
            artifact.is_symlink(),
            "claude_code must place a symlink at agents/test-agent.md; path={artifact:?}"
        );

        // skill_targets row: mode == "symlink", status == "ok".
        let targets = store.get_targets_for_skill("test-agent").unwrap();
        let row = targets
            .iter()
            .find(|r| r.tool == "claude_code")
            .expect("skill_targets must have a row for claude_code after sync");
        assert_eq!(row.mode, "symlink", "claude_code mode must be 'symlink'");
        assert_eq!(row.status, "ok", "status must be ok");
        assert_eq!(
            row.target_path,
            artifact.to_string_lossy().as_ref(),
            "target_path must point to the on-disk artifact"
        );

        central_repo::set_test_base_dir_override(None);
    }

    // ── codex: rendered .toml at agents/test-agent.toml ──────────────────

    #[test]
    fn agent_sync_codex_renders_toml_and_records_render_mode() {
        let (store, _lock, _central, _claude, codex_tmp, _copilot) = setup();

        sync_single_skill_to_tool(&store, "test-agent", "codex").unwrap();

        // On-disk: agents/test-agent.toml must be a regular file (not a symlink).
        let artifact = codex_tmp.path().join("agents").join("test-agent.toml");
        assert!(
            artifact.is_file() && !artifact.is_symlink(),
            "codex must render a real file at agents/test-agent.toml; path={artifact:?}"
        );

        // Content must be valid render_codex output (contains TOML name key).
        let content = fs::read_to_string(&artifact).unwrap();
        assert!(
            content.contains("name = \"test_agent\""),
            "codex .toml must contain TOML name field; got:\n{content}"
        );

        // Verify bytes equal render_codex output so we know it is the rendered
        // content and not a copy of the source .md.
        let agent =
            crate::core::asset_render::canonical_agent_from_file(&std::path::PathBuf::from(
                store
                    .get_skill_by_id("test-agent")
                    .unwrap()
                    .unwrap()
                    .central_path,
            ))
            .unwrap();
        let expected = crate::core::asset_render::render_codex(&agent);
        assert_eq!(
            content, expected,
            "codex .toml content must equal render_codex output"
        );

        // skill_targets row: mode == "render", status == "ok".
        let targets = store.get_targets_for_skill("test-agent").unwrap();
        let row = targets
            .iter()
            .find(|r| r.tool == "codex")
            .expect("skill_targets must have a row for codex after sync");
        assert_eq!(row.mode, "render", "codex mode must be 'render'");
        assert_eq!(row.status, "ok", "status must be ok");
        assert_eq!(
            row.target_path,
            artifact.to_string_lossy().as_ref(),
            "target_path must point to the on-disk artifact"
        );

        central_repo::set_test_base_dir_override(None);
    }

    // ── github_copilot: rendered .agent.md at agents/test-agent.agent.md ──

    #[test]
    fn agent_sync_copilot_renders_agent_md_and_records_render_mode() {
        let (store, _lock, _central, _claude, _codex, copilot_tmp) = setup();

        sync_single_skill_to_tool(&store, "test-agent", "github_copilot").unwrap();

        // On-disk: agents/test-agent.agent.md must be a regular file.
        let artifact = copilot_tmp
            .path()
            .join("agents")
            .join("test-agent.agent.md");
        assert!(
            artifact.is_file() && !artifact.is_symlink(),
            "github_copilot must render a real file at agents/test-agent.agent.md; path={artifact:?}"
        );

        // Content must be valid render_copilot output (YAML frontmatter).
        let content = fs::read_to_string(&artifact).unwrap();
        assert!(
            content.contains("name:"),
            "copilot .agent.md must contain YAML name field; got:\n{content}"
        );

        // skill_targets row: mode == "render", status == "ok".
        let targets = store.get_targets_for_skill("test-agent").unwrap();
        let row = targets
            .iter()
            .find(|r| r.tool == "github_copilot")
            .expect("skill_targets must have a row for github_copilot after sync");
        assert_eq!(row.mode, "render", "github_copilot mode must be 'render'");
        assert_eq!(row.status, "ok", "status must be ok");
        assert_eq!(
            row.target_path,
            artifact.to_string_lossy().as_ref(),
            "target_path must point to the on-disk artifact"
        );

        central_repo::set_test_base_dir_override(None);
    }

    // ── Defect 2 regression guard: Unsupported outcome must NOT insert a row ──
    //
    // We verify this by calling sync_single_skill_to_tool with a tool whose
    // asset_capability returns None for AssetType::Agent (or equivalently, a
    // tool that would produce Unsupported).  We use a custom tool definition
    // with a skills_dir that lies outside the adapter home so the foreign-home
    // guard fires, OR simply assert that codex with a broken home produces no
    // "ok symlink" row.  The cleanest approach is to insert a second skill,
    // then use an adapter whose Agent capability is intentionally absent.
    //
    // We use "cursor" which has no AssetType::Agent capability
    // (asset_capability returns None -> Unsupported outcome).
    #[test]
    fn unsupported_adapter_does_not_insert_ok_symlink_row() {
        let (store, _lock, _central, _claude, _codex, _copilot) = setup();

        // Add cursor to custom_tool_paths with a real temp dir so is_installed()
        // passes.  cursor returns Unsupported for AssetType::Agent.
        let cursor_tmp = tempdir().unwrap();
        let cursor_skills = cursor_tmp.path().join("skills");
        fs::create_dir_all(&cursor_skills).unwrap();

        let mut paths: HashMap<String, String> = store
            .get_setting("custom_tool_paths")
            .unwrap()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        paths.insert(
            "cursor".to_string(),
            cursor_skills.to_string_lossy().to_string(),
        );
        store
            .set_setting("custom_tool_paths", &serde_json::to_string(&paths).unwrap())
            .unwrap();

        // sync_single_skill_to_tool must succeed (no error) but must NOT insert
        // a skill_targets row for cursor because Unsupported means nothing was
        // written.
        sync_single_skill_to_tool(&store, "test-agent", "cursor").unwrap();

        let targets = store.get_targets_for_skill("test-agent").unwrap();
        assert!(
            !targets.iter().any(|r| r.tool == "cursor"),
            "Unsupported delivery must not insert any skill_targets row; got: {targets:?}"
        );

        central_repo::set_test_base_dir_override(None);
    }

    // ── Agent unsync: apply_skills_to_tools(Remove) removes all artifact kinds ─
    //
    // For each of the three agent-capable adapters we:
    //   1. Sync via apply_skills_to_tools(Add) so the artifact exists on disk.
    //   2. Unsync via apply_skills_to_tools(Remove).
    //   3. Assert the on-disk artifact is gone.
    //   4. Assert the skill_targets row for that tool is cleared.

    #[test]
    fn agent_unsync_claude_code_removes_symlink_and_clears_row() {
        let (store, _lock, _central, claude_tmp, _codex, _copilot) = setup();

        // Phase 1: sync so the artifact exists.
        apply_skills_to_tools(
            &store,
            &["test-agent".to_string()],
            &["claude_code".to_string()],
            BatchApplyMode::Add,
        )
        .unwrap();

        let artifact = claude_tmp.path().join("agents").join("test-agent.md");
        assert!(
            artifact.exists(),
            "artifact must exist after sync; path={artifact:?}"
        );

        // Phase 2: unsync.
        apply_skills_to_tools(
            &store,
            &["test-agent".to_string()],
            &["claude_code".to_string()],
            BatchApplyMode::Remove,
        )
        .unwrap();

        // Phase 3: artifact gone.
        assert!(
            !artifact.exists(),
            "claude_code symlink must be removed after unsync; path={artifact:?}"
        );

        // Phase 4: row cleared.
        let targets = store.get_targets_for_skill("test-agent").unwrap();
        assert!(
            !targets.iter().any(|r| r.tool == "claude_code"),
            "skill_targets row for claude_code must be cleared after unsync; got: {targets:?}"
        );

        central_repo::set_test_base_dir_override(None);
    }

    #[test]
    fn agent_unsync_codex_removes_rendered_toml_and_clears_row() {
        let (store, _lock, _central, _claude, codex_tmp, _copilot) = setup();

        // Phase 1: sync.
        apply_skills_to_tools(
            &store,
            &["test-agent".to_string()],
            &["codex".to_string()],
            BatchApplyMode::Add,
        )
        .unwrap();

        let artifact = codex_tmp.path().join("agents").join("test-agent.toml");
        assert!(
            artifact.is_file(),
            "codex artifact must exist after sync; path={artifact:?}"
        );

        // Phase 2: unsync.
        apply_skills_to_tools(
            &store,
            &["test-agent".to_string()],
            &["codex".to_string()],
            BatchApplyMode::Remove,
        )
        .unwrap();

        // Phase 3: artifact gone.
        assert!(
            !artifact.exists(),
            "codex rendered .toml must be removed after unsync; path={artifact:?}"
        );

        // Phase 4: row cleared.
        let targets = store.get_targets_for_skill("test-agent").unwrap();
        assert!(
            !targets.iter().any(|r| r.tool == "codex"),
            "skill_targets row for codex must be cleared after unsync; got: {targets:?}"
        );

        central_repo::set_test_base_dir_override(None);
    }

    #[test]
    fn agent_unsync_github_copilot_removes_rendered_agent_md_and_clears_row() {
        let (store, _lock, _central, _claude, _codex, copilot_tmp) = setup();

        // Phase 1: sync.
        apply_skills_to_tools(
            &store,
            &["test-agent".to_string()],
            &["github_copilot".to_string()],
            BatchApplyMode::Add,
        )
        .unwrap();

        let artifact = copilot_tmp
            .path()
            .join("agents")
            .join("test-agent.agent.md");
        assert!(
            artifact.is_file(),
            "copilot artifact must exist after sync; path={artifact:?}"
        );

        // Phase 2: unsync.
        apply_skills_to_tools(
            &store,
            &["test-agent".to_string()],
            &["github_copilot".to_string()],
            BatchApplyMode::Remove,
        )
        .unwrap();

        // Phase 3: artifact gone.
        assert!(
            !artifact.exists(),
            "github_copilot rendered .agent.md must be removed after unsync; path={artifact:?}"
        );

        // Phase 4: row cleared.
        let targets = store.get_targets_for_skill("test-agent").unwrap();
        assert!(
            !targets.iter().any(|r| r.tool == "github_copilot"),
            "skill_targets row for github_copilot must be cleared after unsync; got: {targets:?}"
        );

        central_repo::set_test_base_dir_override(None);
    }

    // ── Batch path: sync >=2 agent ids, then batch-unsync removes all ─────────

    #[test]
    fn batch_sync_and_unsync_two_agents_delivers_and_removes_all() {
        let lock = central_repo::test_base_dir_lock();
        let central_tmp = tempdir().unwrap();
        let base = central_tmp.path().join("repo");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        central_repo::ensure_central_repo().unwrap();
        let store = crate::core::skill_store::SkillStore::new(&base.join("test.db")).unwrap();

        // Two distinct agent .md files.
        let agent_a_file = central_tmp.path().join("agent-alpha.md");
        let agent_b_file = central_tmp.path().join("agent-beta.md");
        let agent_a_md = "\
---
name: agent-alpha
description: Alpha agent.
tools:
  - Read
---
Body alpha.
";
        let agent_b_md = "\
---
name: agent-beta
description: Beta agent.
tools:
  - Read
---
Body beta.
";
        fs::write(&agent_a_file, agent_a_md).unwrap();
        fs::write(&agent_b_file, agent_b_md).unwrap();

        // One adapter (claude_code) proves the batch path end-to-end.
        let claude_tmp = tempdir().unwrap();
        let claude_skills = claude_tmp.path().join("skills");
        fs::create_dir_all(&claude_skills).unwrap();

        let paths: HashMap<String, String> = [(
            "claude_code".to_string(),
            claude_skills.to_string_lossy().to_string(),
        )]
        .into_iter()
        .collect();
        store
            .set_setting("custom_tool_paths", &serde_json::to_string(&paths).unwrap())
            .unwrap();

        let now = chrono::Utc::now().timestamp();
        for (id, file) in [
            ("agent-alpha", &agent_a_file),
            ("agent-beta", &agent_b_file),
        ] {
            store
                .insert_skill(&SkillRecord {
                    id: id.to_string(),
                    name: id.to_string(),
                    description: None,
                    source_type: "import".to_string(),
                    source_ref: None,
                    source_ref_resolved: None,
                    source_subpath: None,
                    source_branch: None,
                    source_revision: None,
                    remote_revision: None,
                    central_path: file.to_string_lossy().to_string(),
                    content_hash: None,
                    enabled: true,
                    created_at: now,
                    updated_at: now,
                    status: "ok".to_string(),
                    update_status: "local_only".to_string(),
                    last_checked_at: None,
                    last_check_error: None,
                    asset_type: crate::core::skill_store::AssetType::Agent,
                })
                .unwrap();
        }

        let skill_ids = vec!["agent-alpha".to_string(), "agent-beta".to_string()];
        let tool_keys = vec!["claude_code".to_string()];

        // Batch sync: both artifacts must appear.
        apply_skills_to_tools(&store, &skill_ids, &tool_keys, BatchApplyMode::Add).unwrap();

        let artifact_a = claude_tmp.path().join("agents").join("agent-alpha.md");
        let artifact_b = claude_tmp.path().join("agents").join("agent-beta.md");
        assert!(
            artifact_a.exists(),
            "agent-alpha artifact must exist after batch sync; path={artifact_a:?}"
        );
        assert!(
            artifact_b.exists(),
            "agent-beta artifact must exist after batch sync; path={artifact_b:?}"
        );

        // Batch unsync: both artifacts must be gone and rows cleared.
        apply_skills_to_tools(&store, &skill_ids, &tool_keys, BatchApplyMode::Remove).unwrap();

        assert!(
            !artifact_a.exists(),
            "agent-alpha artifact must be removed after batch unsync; path={artifact_a:?}"
        );
        assert!(
            !artifact_b.exists(),
            "agent-beta artifact must be removed after batch unsync; path={artifact_b:?}"
        );

        let targets_a = store.get_targets_for_skill("agent-alpha").unwrap();
        let targets_b = store.get_targets_for_skill("agent-beta").unwrap();
        assert!(
            !targets_a.iter().any(|r| r.tool == "claude_code"),
            "skill_targets row for agent-alpha/claude_code must be cleared; got: {targets_a:?}"
        );
        assert!(
            !targets_b.iter().any(|r| r.tool == "claude_code"),
            "skill_targets row for agent-beta/claude_code must be cleared; got: {targets_b:?}"
        );

        drop(lock);
        central_repo::set_test_base_dir_override(None);
    }
}
