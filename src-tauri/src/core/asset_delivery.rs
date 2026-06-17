/// Capability-driven delivery of a single asset to a single agent home.
///
/// This module is the execution layer for the matrix defined by
/// `ToolAdapter::asset_capability`.  Given an asset (its type, canonical
/// source path, id/name, and — for agents — a `CanonicalAgent`) and a
/// target adapter + home directory, `deliver_asset` consults the capability
/// and performs the correct delivery without altering existing skill-sync
/// behavior.
use std::path::{Path, PathBuf};

use anyhow::Result;

// ---------------------------------------------------------------------------
// Internal path-safety helpers
// ---------------------------------------------------------------------------

/// Return `true` when `path` (resolving symlinks) is strictly inside `root`
/// (resolving symlinks).  Returns `false` on any canonicalization error.
fn resolves_inside(path: &Path, root: &Path) -> bool {
    match (std::fs::canonicalize(path), std::fs::canonicalize(root)) {
        (Ok(p), Ok(r)) => p.starts_with(&r),
        _ => false,
    }
}

/// Return `true` when `target` is a symlink whose resolved destination lies
/// anywhere inside `central_root` (the skills-manager central repo root as
/// resolved by `central_repo::base_dir()`).  This lets `backup_foreign_target`
/// recognise a link placed by a previous sync cycle as "already managed" even
/// though it does not point at the exact same source file (e.g. a different
/// release of the same asset).
fn symlink_resolves_to_central_repo(target: &Path, central_root: &Path) -> bool {
    if !target.is_symlink() {
        return false;
    }
    resolves_inside(target, central_root)
}

use crate::core::asset_render::{render_codex, render_copilot, CanonicalAgent};
use crate::core::tool_adapters::Renderer;
use crate::core::skill_store::AssetType;
use crate::core::sync_engine::{sync_skill, write_rendered_file, SyncMode};
use crate::core::tool_adapters::ToolAdapter;

// ---------------------------------------------------------------------------
// Backup helpers
// ---------------------------------------------------------------------------

/// If `target` exists and is NOT already the managed artifact we are about to
/// create, rename it to `<target>.backup-<timestamp>` (format `%Y%m%d-%H%M%S`,
/// consistent with `git_backup.rs`) and return the backup path.
///
/// "Already ours" means:
/// - Symlink/Place: the target is a symlink that resolves to `source`, OR the
///   target is a symlink that resolves anywhere inside `central_root` (the
///   central repo root returned by `central_repo::base_dir()`, i.e.
///   `~/.skills-manager` by default — a link created by a previous sync cycle).
/// - Render: the target is a regular file whose content equals `render_bytes`.
///
/// If the target does not exist, or is already ours, returns `None` (no backup
/// created).  This is the idempotency guarantee: re-delivering an unchanged
/// asset never churns backups.
fn backup_foreign_target(
    target: &Path,
    source: &Path,
    mode: SyncMode,
    render_bytes: Option<&[u8]>,
    central_root: Option<&Path>,
) -> Result<Option<PathBuf>> {
    // Does the target exist at all (follow through: check lstat)?
    if std::fs::symlink_metadata(target).is_err() {
        // Nothing there; nothing to back up.
        return Ok(None);
    }

    // Is this already our managed artifact?
    let already_ours = match mode {
        SyncMode::Symlink | SyncMode::Place => {
            // Exact match: symlink resolves to this specific source file.
            symlink_points_to_source(target, source)
                // Broad match: symlink resolves anywhere inside the central
                // repo — a link written by a prior sync cycle.
                || central_root
                    .map(|root| symlink_resolves_to_central_repo(target, root))
                    .unwrap_or(false)
        }
        SyncMode::Render => {
            if let Some(bytes) = render_bytes {
                match std::fs::read(target) {
                    Ok(existing) => existing == bytes,
                    Err(_) => false,
                }
            } else {
                false
            }
        }
        SyncMode::Copy => false,
    };

    if already_ours {
        return Ok(None);
    }

    // Foreign artifact: rename it to a timestamped backup.
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let backup = PathBuf::from(format!("{}.backup-{}", target.display(), ts));
    std::fs::rename(target, &backup).map_err(|e| {
        anyhow::anyhow!(
            "backup_foreign_target: failed to rename {:?} -> {:?}: {}",
            target,
            backup,
            e
        )
    })?;
    Ok(Some(backup))
}

/// Return true when `target` is a symlink that resolves to the same inode as
/// `source` (cross-platform: falls back to false when symlinks are unsupported).
fn symlink_points_to_source(target: &Path, source: &Path) -> bool {
    // Only meaningful on platforms with symlink support.
    if !target.is_symlink() {
        return false;
    }
    match (std::fs::canonicalize(target), std::fs::canonicalize(source)) {
        (Ok(t), Ok(s)) => t == s,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// Outcome of a single `deliver_asset` call.
#[derive(Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// A symlink was created (or already existed) at `path`.
    Symlinked(PathBuf),
    /// A rendered file was written (bytes were new) at `path`.
    Rendered(PathBuf),
    /// A rendered file already had the same content; the write was skipped.
    RenderedUpToDate(PathBuf),
    /// A Place symlink was created at `path`.
    Placed(PathBuf),
    /// The agent home (or a subdirectory within it) resolves via symlink to a
    /// location outside the nominal home root.  Delivery was refused and
    /// nothing was written.  The `PathBuf` carries the resolved foreign path
    /// that triggered the refusal.
    ForeignHome(PathBuf),
    /// The asset type is `Skill`; the caller should use the existing skill
    /// delivery path (scenario_service / sync_engine).
    DeferToSkillPath,
    /// This adapter has no capability for the given asset type; nothing was
    /// written.
    Unsupported,
}

// ---------------------------------------------------------------------------
// Inputs for a single delivery
// ---------------------------------------------------------------------------

/// Everything the delivery engine needs to know about the asset being delivered.
pub struct AssetInput<'a> {
    /// The asset type (Skill, Agent, Command, Hook, Script, Rule).
    pub asset_type: AssetType,
    /// Absolute path of the canonical source file (or directory for Place
    /// types such as Hook/Script/Rule).
    pub source: &'a Path,
    /// Machine identifier, e.g. `"backend-architect"`.
    pub id: &'a str,
    /// Human-readable name, e.g. `"backend-architect"` or a command filename
    /// stem.  Used for `{name}` substitution in `filename_rule`.
    pub name: &'a str,
    /// Required when `asset_type == AssetType::Agent` and the adapter's
    /// capability uses `SyncMode::Render`.  Unused otherwise.
    pub canonical_agent: Option<&'a CanonicalAgent>,
}

// ---------------------------------------------------------------------------
// Core delivery function
// ---------------------------------------------------------------------------

/// Deliver one asset to one agent home according to the adapter's declared
/// capability.
///
/// # Dispatch
///
/// | capability.mode | action                                                    |
/// |-----------------|-----------------------------------------------------------|
/// | `Symlink`       | symlink `source` to `<home>/<subdir>/<filename>`         |
/// | `Render`        | render bytes via `render_codex`/`render_copilot`, write  |
/// | `Place`         | symlink `source` dir to `<home>/<subdir>/` (live edits)  |
/// | None + Skill    | return `DeferToSkillPath`                                |
/// | None + other    | return `Unsupported`                                     |
///
/// # Safety guard
///
/// If the resolved `home` (following any symlink) lies outside the nominal
/// `home` path, delivery is refused and `ForeignHome` is returned.  This
/// prevents writes from escaping into a foreign repository when the agent
/// home directory (or a subdirectory inside it) is itself a symlink into
/// another git repo.
///
/// The function is additive — it does not touch existing skill delivery paths.
pub fn deliver_asset(
    adapter: &ToolAdapter,
    home: &Path,
    asset: &AssetInput<'_>,
) -> Result<DeliveryOutcome> {
    // --- Skill: always defer to the existing engine ---
    if asset.asset_type == AssetType::Skill {
        return Ok(DeliveryOutcome::DeferToSkillPath);
    }

    let capability = match adapter.asset_capability(asset.asset_type) {
        Some(c) => c,
        None => return Ok(DeliveryOutcome::Unsupported),
    };

    // Resolve the target file/directory path.
    let subdir = home.join(capability.target_subdir);
    let target = match capability.filename_rule {
        Some(rule) => {
            // Interpolate {id} and {name} in the filename template.
            let filename = rule
                .replace("{id}", asset.id)
                .replace("{name}", asset.name);
            subdir.join(filename)
        }
        // Place mode: target is a symlink inside the subdir named after the
        // source's directory name (hooks/<hook-dir-name>).
        None => subdir.join(
            asset
                .source
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new(asset.id)),
        ),
    };

    // ── Foreign-home guard ────────────────────────────────────────────────────
    // If `subdir` exists on disk and resolves (following symlinks) to a
    // location outside `home`, refuse delivery.  This catches the case where
    // ~/.codex/agents or ~/.copilot/agents is a symlink into another git repo.
    if subdir.exists() && !resolves_inside(&subdir, home) {
        // canonicalize gives us the true resolved path for the error message.
        let resolved = std::fs::canonicalize(&subdir).unwrap_or_else(|_| subdir.clone());
        return Ok(DeliveryOutcome::ForeignHome(resolved));
    }

    // Derive the central repo root via central_repo::base_dir(), which is the
    // same source of truth used everywhere else in the codebase and honours the
    // test-override set by set_test_base_dir_override.  This lets
    // backup_foreign_target recognise any symlink resolving into the real
    // central repo (~/.skills-manager by default) as "already managed".
    let central_root: Option<PathBuf> =
        Some(crate::core::central_repo::base_dir());

    match capability.mode {
        // ── Symlink ──────────────────────────────────────────────────────────
        SyncMode::Symlink => {
            std::fs::create_dir_all(&subdir)?;
            // Back up any pre-existing foreign file before symlinking.
            backup_foreign_target(
                &target,
                asset.source,
                SyncMode::Symlink,
                None,
                central_root.as_deref(),
            )?;
            sync_skill(asset.source, &target, SyncMode::Symlink)?;
            Ok(DeliveryOutcome::Symlinked(target))
        }

        // ── Render ───────────────────────────────────────────────────────────
        SyncMode::Render => {
            let agent = asset.canonical_agent.ok_or_else(|| {
                anyhow::anyhow!(
                    "deliver_asset: Render mode requires canonical_agent for asset '{}'",
                    asset.id
                )
            })?;
            let bytes = match capability.renderer {
                Some(Renderer::Codex) => render_codex(agent).into_bytes(),
                Some(Renderer::Copilot) => render_copilot(agent).into_bytes(),
                None => anyhow::bail!(
                    "deliver_asset: Render mode has no renderer for asset '{}'",
                    asset.id
                ),
            };
            std::fs::create_dir_all(&subdir)?;
            // Back up any pre-existing foreign file before writing.  Pass the
            // render bytes so the check can recognise our own prior output.
            backup_foreign_target(
                &target,
                asset.source,
                SyncMode::Render,
                Some(&bytes),
                central_root.as_deref(),
            )?;
            let (written, _hash) = write_rendered_file(&target, &bytes)?;
            if written {
                Ok(DeliveryOutcome::Rendered(target))
            } else {
                Ok(DeliveryOutcome::RenderedUpToDate(target))
            }
        }

        // ── Place ─────────────────────────────────────────────────────────────
        SyncMode::Place => {
            std::fs::create_dir_all(&subdir)?;
            // Back up any pre-existing foreign file/directory before placing.
            backup_foreign_target(
                &target,
                asset.source,
                SyncMode::Place,
                None,
                central_root.as_deref(),
            )?;
            sync_skill(asset.source, &target, SyncMode::Place)?;
            Ok(DeliveryOutcome::Placed(target))
        }

        // Copy is not emitted by any asset_capability today; treat it as
        // unsupported to avoid silently picking up a future extension.
        SyncMode::Copy => Ok(DeliveryOutcome::Unsupported),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::asset_render::CanonicalAgent;
    use crate::core::tool_adapters::ToolAdapter;
    use std::fs;
    use tempfile::tempdir;

    /// Return the named adapter from the default registry.
    ///
    /// `home` is passed to `deliver_asset` as the agent home; the adapter
    /// itself is taken as-is from the default list (its `skills_dir` is not
    /// used by the delivery engine — the caller-supplied `home` governs where
    /// files land).
    fn adapter(key: &str, _home: &Path) -> ToolAdapter {
        crate::core::tool_adapters::default_tool_adapters()
            .into_iter()
            .find(|a| a.key == key)
            .unwrap_or_else(|| panic!("adapter '{}' not found in default registry", key))
    }

    /// Sample canonical agent with NON-default codex_reasoning_effort = "high"
    /// to prove registry fields flow through render, not hardcoded defaults.
    fn sample_agent() -> CanonicalAgent {
        CanonicalAgent {
            id: "backend-architect".to_string(),
            display_name: Some("Backend Architect".to_string()),
            description: "Use for APIs, services, and integration design.".to_string(),
            tools: vec![
                "Read".to_string(),
                "Grep".to_string(),
                "Glob".to_string(),
                "Bash".to_string(),
                "Edit".to_string(),
                "Write".to_string(),
            ],
            codex_reasoning_effort: Some("high".to_string()),
            codex_sandbox_mode: None,
            body: "You are a senior backend architect.".to_string(),
        }
    }

    // ── POSITIVE: agent delivered to all four homes ───────────────────────────

    #[cfg(unix)]
    #[test]
    fn deliver_agent_to_claude_creates_symlink() {
        let home = tempdir().unwrap();
        // Write a dummy source file.
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        let agent = sample_agent();
        let a = adapter("claude_code", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();
        let target = home.path().join("agents").join("backend-architect.md");
        assert!(
            matches!(result, DeliveryOutcome::Symlinked(ref p) if p == &target),
            "expected Symlinked, got {:?}",
            result
        );
        assert!(target.exists(), "target must exist");
        assert!(target.is_symlink(), "Claude agent must be a symlink");
        assert_eq!(fs::canonicalize(&target).unwrap(), fs::canonicalize(&src).unwrap(), "symlink must resolve to source file");
    }

    #[cfg(unix)]
    #[test]
    fn deliver_agent_to_pi_creates_symlink() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        let agent = sample_agent();
        let a = adapter("pi", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();
        let target = home.path().join("agents").join("backend-architect.md");
        assert!(
            matches!(result, DeliveryOutcome::Symlinked(ref p) if p == &target),
            "expected Symlinked, got {:?}",
            result
        );
        assert!(target.is_symlink(), "Pi agent must be a symlink");
        assert_eq!(fs::canonicalize(&target).unwrap(), fs::canonicalize(&src).unwrap(), "symlink must resolve to source file");
    }

    #[cfg(unix)]
    #[test]
    fn deliver_agent_to_codex_writes_toml_with_high_reasoning_effort() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        let agent = sample_agent();
        let a = adapter("codex", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();
        let target = home.path().join("agents").join("backend-architect.toml");

        assert!(
            matches!(result, DeliveryOutcome::Rendered(ref p) if p == &target),
            "expected Rendered, got {:?}",
            result
        );
        assert!(target.is_file(), "Codex agent must be a regular file");

        let content = fs::read_to_string(&target).unwrap();
        // The key assertion: registry field must flow through, not a default.
        assert!(
            content.contains("model_reasoning_effort = \"high\""),
            "codex render must carry codex_reasoning_effort=high from registry; got:\n{}",
            content
        );
        // Also confirm bytes equal render_codex output.
        let expected = render_codex(&agent);
        assert_eq!(content, expected, "file bytes must equal render_codex output");
    }

    #[cfg(unix)]
    #[test]
    fn deliver_agent_to_copilot_writes_agent_md() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        let agent = sample_agent();
        let a = adapter("github_copilot", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();
        let target = home
            .path()
            .join("agents")
            .join("backend-architect.agent.md");

        assert!(
            matches!(result, DeliveryOutcome::Rendered(ref p) if p == &target),
            "expected Rendered, got {:?}",
            result
        );
        assert!(target.is_file(), "Copilot agent must be a regular file");

        let content = fs::read_to_string(&target).unwrap();
        let expected = render_copilot(&agent);
        assert_eq!(content, expected, "file bytes must equal render_copilot output");
    }

    #[cfg(unix)]
    #[test]
    fn deliver_command_to_claude_creates_symlink() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("deploy.md");
        fs::write(&src, b"# Deploy command").unwrap();

        let a = adapter("claude_code", home.path());
        let input = AssetInput {
            asset_type: AssetType::Command,
            source: &src,
            id: "deploy",
            name: "deploy",
            canonical_agent: None,
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();
        let target = home.path().join("commands").join("deploy.md");

        assert!(
            matches!(result, DeliveryOutcome::Symlinked(ref p) if p == &target),
            "expected Symlinked, got {:?}",
            result
        );
        assert!(target.is_symlink(), "command must be a symlink");
    }

    #[cfg(unix)]
    #[test]
    fn deliver_hook_to_claude_places_symlink() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let hook_dir = src_dir.path().join("pre-commit");
        fs::create_dir_all(&hook_dir).unwrap();
        fs::write(hook_dir.join("run.sh"), b"#!/bin/sh\necho ok").unwrap();

        let a = adapter("claude_code", home.path());
        let input = AssetInput {
            asset_type: AssetType::Hook,
            source: &hook_dir,
            id: "pre-commit",
            name: "pre-commit",
            canonical_agent: None,
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();
        let target = home.path().join("hooks").join("pre-commit");

        assert!(
            matches!(result, DeliveryOutcome::Placed(ref p) if p == &target),
            "expected Placed, got {:?}",
            result
        );
        assert!(target.exists(), "placed hook must exist");
    }

    // ── NEGATIVE: unsupported cells create NO files ───────────────────────────

    #[test]
    fn deliver_command_to_copilot_is_unsupported() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("deploy.md");
        fs::write(&src, b"# Deploy").unwrap();

        let a = adapter("github_copilot", home.path());
        let input = AssetInput {
            asset_type: AssetType::Command,
            source: &src,
            id: "deploy",
            name: "deploy",
            canonical_agent: None,
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();

        assert_eq!(
            result,
            DeliveryOutcome::Unsupported,
            "command must be unsupported for Copilot"
        );
        // Confirm no file was created.
        assert!(
            !home.path().join("commands").join("deploy.md").exists(),
            "no file must exist for an unsupported cell"
        );
    }

    #[test]
    fn deliver_hook_to_codex_is_unsupported() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let hook_dir = src_dir.path().join("pre-commit");
        fs::create_dir_all(&hook_dir).unwrap();

        let a = adapter("codex", home.path());
        let input = AssetInput {
            asset_type: AssetType::Hook,
            source: &hook_dir,
            id: "pre-commit",
            name: "pre-commit",
            canonical_agent: None,
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();

        assert_eq!(
            result,
            DeliveryOutcome::Unsupported,
            "hook must be unsupported for Codex"
        );
        assert!(
            !home.path().join("hooks").join("pre-commit").exists(),
            "no file must exist for an unsupported cell"
        );
    }

    #[test]
    fn deliver_skill_defers_to_skill_path() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();

        let a = adapter("claude_code", home.path());
        let input = AssetInput {
            asset_type: AssetType::Skill,
            source: src_dir.path(),
            id: "some-skill",
            name: "some-skill",
            canonical_agent: None,
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();
        assert_eq!(result, DeliveryOutcome::DeferToSkillPath);
    }

    #[test]
    fn render_up_to_date_returns_rendered_up_to_date_on_second_call() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"placeholder").unwrap();

        let agent = sample_agent();
        let a = adapter("codex", home.path());
        let input = || AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        // First call: written.
        let r1 = deliver_asset(&a, home.path(), &input()).unwrap();
        assert!(matches!(r1, DeliveryOutcome::Rendered(_)));
        // Second call: same bytes, should be up-to-date.
        let r2 = deliver_asset(&a, home.path(), &input()).unwrap();
        assert!(
            matches!(r2, DeliveryOutcome::RenderedUpToDate(_)),
            "second identical render should be RenderedUpToDate, got {:?}",
            r2
        );
    }

    // ── BACKUP / COEXISTENCE ─────────────────────────────────────────────────

    /// Takeover (symlink): a pre-existing real file at the target path is
    /// backed up before the managed symlink is created.
    #[cfg(unix)]
    #[test]
    fn takeover_symlink_backs_up_preexisting_file() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        // Pre-create a real (non-managed) file at the target location.
        let agents_dir = home.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        let target = agents_dir.join("backend-architect.md");
        fs::write(&target, b"PREEXISTING").unwrap();

        let agent = sample_agent();
        let a = adapter("claude_code", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();

        // (a) The delivery succeeded and the managed symlink is in place.
        assert!(
            matches!(result, DeliveryOutcome::Symlinked(_)),
            "expected Symlinked, got {:?}",
            result
        );
        assert!(target.is_symlink(), "target must now be a managed symlink");
        assert_eq!(
            fs::canonicalize(&target).unwrap(),
            fs::canonicalize(&src).unwrap(),
            "symlink must resolve to source"
        );

        // (b) A backup file exists containing the original "PREEXISTING" content.
        let backup = fs::read_dir(&agents_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("backend-architect.md.backup-")
            })
            .expect("a .backup-<ts> file must exist after takeover");
        let backup_content = fs::read(&backup.path()).unwrap();
        assert_eq!(
            backup_content, b"PREEXISTING",
            "backup must contain the original file content"
        );
    }

    /// Idempotency (symlink): delivering the same symlink asset twice to a
    /// clean home must not create any backup file on either call.
    #[cfg(unix)]
    #[test]
    fn idempotent_symlink_delivery_creates_no_backup() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        let agent = sample_agent();
        let a = adapter("claude_code", home.path());
        let mk_input = || AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };

        // First delivery.
        let r1 = deliver_asset(&a, home.path(), &mk_input()).unwrap();
        assert!(matches!(r1, DeliveryOutcome::Symlinked(_)));

        // Second delivery of the identical asset.
        let r2 = deliver_asset(&a, home.path(), &mk_input()).unwrap();
        assert!(matches!(r2, DeliveryOutcome::Symlinked(_)));

        // No backup file must exist in the agents dir.
        let agents_dir = home.path().join("agents");
        let has_backup = fs::read_dir(&agents_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".backup-"));
        assert!(
            !has_backup,
            "re-delivering an unchanged symlink asset must not create a backup"
        );
    }

    /// Takeover (render): a pre-existing non-managed file at the Codex .toml
    /// path is backed up before the rendered file is written.
    #[cfg(unix)]
    #[test]
    fn takeover_render_backs_up_preexisting_file() {
        let home = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"placeholder").unwrap();

        // Pre-create a non-managed file where the rendered .toml will land.
        let agents_dir = home.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        let target = agents_dir.join("backend-architect.toml");
        fs::write(&target, b"PREEXISTING_TOML").unwrap();

        let agent = sample_agent();
        let a = adapter("codex", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();

        // Managed rendered file is now in place.
        assert!(
            matches!(result, DeliveryOutcome::Rendered(_)),
            "expected Rendered, got {:?}",
            result
        );
        assert!(target.is_file(), "rendered .toml must exist");

        // A backup containing the original bytes must exist.
        let backup = fs::read_dir(&agents_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("backend-architect.toml.backup-")
            })
            .expect("a .backup-<ts> file must exist after render takeover");
        let backup_content = fs::read(&backup.path()).unwrap();
        assert_eq!(
            backup_content, b"PREEXISTING_TOML",
            "backup must contain the original file content"
        );
    }

    // ── SAFETY GUARD: foreign home ────────────────────────────────────────────

    /// When the target subdir is a symlink pointing OUTSIDE the nominal home
    /// root, `deliver_asset` must return `ForeignHome` and write NOTHING to
    /// the foreign location.
    #[cfg(unix)]
    #[test]
    fn deliver_refuses_when_subdir_symlinks_outside_home() {
        // The "home" directory for this adapter.
        let home = tempdir().unwrap();
        // A separate directory that lives OUTSIDE the home — simulates the
        // user's agentic-tools source tree on iCloud.
        let foreign_dir = tempdir().unwrap();
        let foreign_agents = foreign_dir.path().join("agents");
        fs::create_dir_all(&foreign_agents).unwrap();

        // Plant a sentinel file in the foreign location so we can assert it is
        // untouched after the refused delivery.
        let sentinel = foreign_agents.join("sentinel.txt");
        fs::write(&sentinel, b"DO NOT TOUCH").unwrap();

        // Create <home>/agents as a symlink pointing into the foreign tree.
        let home_agents = home.path().join("agents");
        std::os::unix::fs::symlink(&foreign_agents, &home_agents).unwrap();

        // Source file lives in a separate tempdir (central repo simulation).
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        let agent = sample_agent();
        let a = adapter("claude_code", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };

        let result = deliver_asset(&a, home.path(), &input).unwrap();

        // Must be refused with ForeignHome.
        assert!(
            matches!(result, DeliveryOutcome::ForeignHome(_)),
            "expected ForeignHome, got {:?}",
            result
        );

        // The foreign directory must be completely untouched.
        let foreign_entry_count = fs::read_dir(&foreign_agents).unwrap().count();
        assert_eq!(
            foreign_entry_count, 1,
            "foreign agents dir must still contain exactly the sentinel file, found {} entries",
            foreign_entry_count
        );
        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"DO NOT TOUCH",
            "sentinel file must be unmodified"
        );
    }

    /// When the target is a symlink that resolves into the skills-manager
    /// central repo root (as resolved by `central_repo::base_dir()`),
    /// `deliver_asset` must NOT create a `.backup-*` file — no churn on
    /// re-sync.  The test exercises the real runtime derivation by pointing the
    /// base-dir override at a tempdir; no fake `src-tauri` marker is needed or
    /// used.
    #[cfg(unix)]
    #[test]
    fn no_backup_when_target_symlink_resolves_inside_central_repo() {
        use crate::core::central_repo::{
            set_test_base_dir_override, test_base_dir_lock,
        };

        // Hold the global base-dir mutex for the duration of the test.
        let _guard = test_base_dir_lock();

        // The tempdir is the central repo root for this test run.  No
        // src-tauri marker is created — the real resolver does not look for
        // one; only the old walk-up heuristic did.
        let central = tempdir().unwrap();
        set_test_base_dir_override(Some(central.path().to_path_buf()));

        // Source file inside the central repo tree (as delivered by the real
        // sync pipeline: central_repo::base_dir()/agents/backend-architect.md).
        let agents_src_dir = central.path().join("agents");
        fs::create_dir_all(&agents_src_dir).unwrap();
        let src = agents_src_dir.join("backend-architect.md");
        fs::write(&src, b"# Backend Architect").unwrap();

        // A *different* file that also lives inside the central repo — this
        // simulates an older release of the same asset left by a prior cycle.
        let older = agents_src_dir.join("older-version.md");
        fs::write(&older, b"# Older").unwrap();

        // The "home" directory where the adapter delivers the asset.
        let home = tempdir().unwrap();
        let agents_home_dir = home.path().join("agents");
        fs::create_dir_all(&agents_home_dir).unwrap();

        // Pre-place a symlink at the expected target location pointing at the
        // older file inside the central repo (not the current source file).
        // deliver_asset must recognise this as "already managed" and skip the
        // backup.
        let target = agents_home_dir.join("backend-architect.md");
        std::os::unix::fs::symlink(&older, &target).unwrap();

        // Call deliver_asset — this exercises the real central_root derivation
        // via central_repo::base_dir(), which picks up the override above.
        let agent = sample_agent();
        let a = adapter("claude_code", home.path());
        let input = AssetInput {
            asset_type: AssetType::Agent,
            source: &src,
            id: "backend-architect",
            name: "backend-architect",
            canonical_agent: Some(&agent),
        };
        let result = deliver_asset(&a, home.path(), &input).unwrap();

        // Restore the override before any assertion that might panic.
        set_test_base_dir_override(None);

        // The delivery must succeed (Symlinked outcome).
        assert!(
            matches!(result, DeliveryOutcome::Symlinked(_)),
            "expected Symlinked, got {:?}",
            result
        );

        // No .backup-* file must exist: the pre-existing symlink pointed into
        // the central repo and was therefore already managed.
        let has_backup = fs::read_dir(&agents_home_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".backup-"));
        assert!(
            !has_backup,
            "no .backup-* file must exist when the target symlink resolves \
             inside the central repo root"
        );
    }
}
