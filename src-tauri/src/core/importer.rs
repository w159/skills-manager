use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::central_repo;
use super::skill_metadata::is_valid_skill_dir;
use super::skill_store::{AssetType, SkillRecord, SkillStore};

// ── Public types ──────────────────────────────────────────────────────────────

/// One discoverable asset in the source workspace.
#[derive(Debug, Clone, Serialize)]
pub struct ImportCandidate {
    pub asset_type: String,
    pub id_or_name: String,
    pub source_path: PathBuf,
    pub in_active_set: bool,
    // Agent-only fields (None for non-agents)
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub tools: Option<Vec<String>>,
    pub codex_sandbox_mode: Option<String>,
    pub codex_reasoning_effort: Option<String>,
}

/// Result of importing one asset.
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub asset_type: String,
    pub id_or_name: String,
    pub central_path: PathBuf,
}

// ── Registry deserialization structs ─────────────────────────────────────────

#[derive(Deserialize)]
struct RegistryFile {
    agents: Option<Vec<ActiveAgent>>,
    skills: Option<Vec<RegistrySkill>>,
}

#[derive(Deserialize, Clone)]
struct ActiveAgent {
    id: String,
    display_name: Option<String>,
    #[allow(dead_code)]
    source: Option<String>,
    description: Option<String>,
    tools: Option<Vec<String>>,
    codex_sandbox_mode: Option<String>,
    codex_reasoning_effort: Option<String>,
}

#[derive(Deserialize)]
struct RegistrySkill {
    id: String,
}

struct ActiveRegistry {
    agents: Vec<ActiveAgent>,
    skill_ids: HashSet<String>,
}

// ── Registry loader ───────────────────────────────────────────────────────────

fn load_registry(workspace_root: &Path) -> Result<ActiveRegistry> {
    let registry_path = workspace_root.join("registry").join("active.json");
    if !registry_path.exists() {
        return Ok(ActiveRegistry {
            agents: vec![],
            skill_ids: HashSet::new(),
        });
    }
    let raw = fs::read_to_string(&registry_path)?;
    let file: RegistryFile = serde_json::from_str(&raw)?;

    let agents = file.agents.unwrap_or_default();
    let skill_ids = file
        .skills
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.id)
        .collect();

    Ok(ActiveRegistry { agents, skill_ids })
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Discover all importable assets in the agentic-tools workspace at
/// `workspace_root`. Returns candidates sorted by (asset_type, id_or_name).
pub fn list_candidates(workspace_root: &Path) -> Result<Vec<ImportCandidate>> {
    if !workspace_root.exists() {
        bail!("workspace root does not exist: {}", workspace_root.display());
    }

    let registry = load_registry(workspace_root)?;
    let active_agent_map: std::collections::HashMap<String, ActiveAgent> = registry
        .agents
        .into_iter()
        .map(|a| (a.id.clone(), a))
        .collect();

    let mut candidates: Vec<ImportCandidate> = Vec::new();

    // ── agents/ ──
    scan_md_files(
        workspace_root,
        "agents",
        "agent",
        &mut candidates,
        |stem, path| {
            let in_active = active_agent_map.contains_key(stem);
            let agent_info = active_agent_map.get(stem);
            ImportCandidate {
                asset_type: "agent".to_string(),
                id_or_name: stem.to_string(),
                source_path: path,
                in_active_set: in_active,
                display_name: agent_info.and_then(|a| a.display_name.clone()),
                description: agent_info.and_then(|a| a.description.clone()),
                tools: agent_info.and_then(|a| a.tools.clone()),
                codex_sandbox_mode: agent_info.and_then(|a| a.codex_sandbox_mode.clone()),
                codex_reasoning_effort: agent_info.and_then(|a| a.codex_reasoning_effort.clone()),
            }
        },
    );

    // ── commands/ ──
    scan_md_files(
        workspace_root,
        "commands",
        "command",
        &mut candidates,
        |stem, path| ImportCandidate {
            asset_type: "command".to_string(),
            id_or_name: stem.to_string(),
            source_path: path,
            in_active_set: false,
            display_name: None,
            description: None,
            tools: None,
            codex_sandbox_mode: None,
            codex_reasoning_effort: None,
        },
    );

    // ── hooks/ ──
    scan_flat_files(workspace_root, "hooks", "hook", &mut candidates);

    // ── scripts/ ──
    scan_flat_files(workspace_root, "scripts", "script", &mut candidates);

    // ── rules/ ──
    scan_md_files(
        workspace_root,
        "rules",
        "rule",
        &mut candidates,
        |stem, path| ImportCandidate {
            asset_type: "rule".to_string(),
            id_or_name: stem.to_string(),
            source_path: path,
            in_active_set: false,
            display_name: None,
            description: None,
            tools: None,
            codex_sandbox_mode: None,
            codex_reasoning_effort: None,
        },
    );

    // ── workflows/ ──
    scan_md_files(
        workspace_root,
        "workflows",
        "workflow",
        &mut candidates,
        |stem, path| ImportCandidate {
            asset_type: "workflow".to_string(),
            id_or_name: stem.to_string(),
            source_path: path,
            in_active_set: false,
            display_name: None,
            description: None,
            tools: None,
            codex_sandbox_mode: None,
            codex_reasoning_effort: None,
        },
    );

    // ── skills/ ──
    let skills_dir = workspace_root.join("skills");
    if skills_dir.exists() {
        if let Ok(entries) = fs::read_dir(&skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Only treat a subdirectory as a skill candidate when it contains
                // a recognised skill marker file (SKILL.md or skill.md).
                // Without this gate every non-skill directory (.system, assets, ci,
                // etc.) becomes a phantom skill candidate.
                if !is_valid_skill_dir(&path) {
                    continue;
                }
                let in_active = registry.skill_ids.contains(&dir_name);
                candidates.push(ImportCandidate {
                    asset_type: "skill".to_string(),
                    id_or_name: dir_name,
                    source_path: path,
                    in_active_set: in_active,
                    display_name: None,
                    description: None,
                    tools: None,
                    codex_sandbox_mode: None,
                    codex_reasoning_effort: None,
                });
            }
        }
    }

    candidates.sort_by(|a, b| {
        a.asset_type
            .cmp(&b.asset_type)
            .then_with(|| a.id_or_name.cmp(&b.id_or_name))
    });

    Ok(candidates)
}

/// Copy each selected candidate into the central repo and insert a store row.
pub fn import_candidates(
    selected: &[ImportCandidate],
    store: &SkillStore,
) -> Result<Vec<ImportResult>> {
    let now = chrono::Utc::now().timestamp();
    let mut results = Vec::new();

    for candidate in selected {
        let asset_type_enum = AssetType::from_str(&candidate.asset_type);
        let dest_dir = central_repo::asset_type_dir(asset_type_enum);
        fs::create_dir_all(&dest_dir)?;

        let central_path: PathBuf = match asset_type_enum {
            AssetType::Skill => {
                // Copy entire directory recursively.
                let dest = dest_dir.join(&candidate.id_or_name);
                copy_dir_recursive(&candidate.source_path, &dest)?;
                dest
            }
            _ => {
                // Copy single file.
                let file_name = candidate
                    .source_path
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("source path has no file name"))?;
                let dest = dest_dir.join(file_name);
                fs::copy(&candidate.source_path, &dest)?;

                // For agents: merge registry fields into destination frontmatter.
                if asset_type_enum == AssetType::Agent {
                    merge_agent_frontmatter(&dest, candidate)?;
                }

                dest
            }
        };

        let record = SkillRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: candidate.id_or_name.clone(),
            description: candidate.description.clone(),
            source_type: "import".to_string(),
            source_ref: Some(candidate.source_path.to_string_lossy().to_string()),
            source_ref_resolved: None,
            source_subpath: None,
            source_branch: None,
            source_revision: None,
            remote_revision: None,
            central_path: central_path.to_string_lossy().to_string(),
            content_hash: None,
            enabled: true,
            created_at: now,
            updated_at: now,
            status: "ok".to_string(),
            update_status: "local_only".to_string(),
            last_checked_at: None,
            last_check_error: None,
            asset_type: asset_type_enum,
        };

        store.insert_skill(&record)?;

        results.push(ImportResult {
            asset_type: candidate.asset_type.clone(),
            id_or_name: candidate.id_or_name.clone(),
            central_path,
        });
    }

    Ok(results)
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Scan a subdirectory for `.md` files (non-backup) and push candidates.
/// The closure receives (stem, absolute_path) and returns a candidate.
fn scan_md_files<F>(
    workspace_root: &Path,
    subdir: &str,
    _asset_type: &str,
    candidates: &mut Vec<ImportCandidate>,
    make_candidate: F,
) where
    F: Fn(&str, PathBuf) -> ImportCandidate,
{
    let dir = workspace_root.join(subdir);
    if !dir.exists() {
        return;
    }
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // Only plain .md files; skip .backup-* render artifacts and .toml configs.
        // Render artifacts like `foo.agent.md` have a dot before the final ".md"
        // segment — the stem would contain a dot, producing phantom duplicates.
        if !file_name.ends_with(".md")
            || file_name.contains(".backup")
            || file_name.ends_with(".toml")
        {
            continue;
        }
        // stem: strip the final ".md" — owned so we can move path afterward
        let stem = file_name[..file_name.len() - 3].to_string();
        // Skip render artifacts: stems that still contain a dot are compound
        // extensions like `<name>.agent` produced by render pipelines.
        if stem.contains('.') {
            continue;
        }
        candidates.push(make_candidate(&stem, path));
    }
}

/// Scan a flat file directory (hooks, scripts): one file per entry, no subdirs,
/// skip dotfiles and __pycache__.
fn scan_flat_files(
    workspace_root: &Path,
    subdir: &str,
    asset_type: &str,
    candidates: &mut Vec<ImportCandidate>,
) {
    let dir = workspace_root.join(subdir);
    if !dir.exists() {
        return;
    }
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // Skip dotfiles, __pycache__, and documentation/config files.
        // Hooks and scripts are executable files; .md and .toml are never scripts.
        if file_name.starts_with('.')
            || file_name == "__pycache__"
            || file_name.ends_with(".md")
            || file_name.ends_with(".toml")
        {
            continue;
        }
        candidates.push(ImportCandidate {
            asset_type: asset_type.to_string(),
            // id_or_name for hooks/scripts uses full filename (e.g. "pre-commit")
            id_or_name: file_name.to_string(),
            source_path: path,
            in_active_set: false,
            display_name: None,
            description: None,
            tools: None,
            codex_sandbox_mode: None,
            codex_reasoning_effort: None,
        });
    }
}

/// Recursively copy src directory into dest (create dest if needed).
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            // Skip .DS_Store
            if entry
                .file_name()
                .to_str()
                .map(|n| n == ".DS_Store")
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Merge agent registry fields (display_name, tools, codex_sandbox_mode,
/// codex_reasoning_effort) into the YAML frontmatter of the DESTINATION file.
/// The source file is never touched.
fn merge_agent_frontmatter(dest_path: &Path, candidate: &ImportCandidate) -> Result<()> {
    let content = fs::read_to_string(dest_path)?;
    let trimmed = content.trim();

    let (existing_yaml, body) = if trimmed.starts_with("---") {
        let rest = &trimmed[3..];
        if let Some(end) = rest.find("\n---") {
            let yaml_str = &rest[..end];
            let body = &rest[end + 4..]; // skip "\n---"
            (yaml_str.to_string(), body.to_string())
        } else {
            (String::new(), content.clone())
        }
    } else {
        (String::new(), content.clone())
    };

    let mut map: serde_yaml::Value = if existing_yaml.is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(&existing_yaml)
            .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
    };

    let m = map
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("frontmatter is not a mapping"))?;

    if let Some(dn) = &candidate.display_name {
        m.insert("display_name".into(), dn.clone().into());
    }
    if let Some(t) = &candidate.tools {
        let arr: Vec<serde_yaml::Value> = t.iter().map(|s| s.clone().into()).collect();
        m.insert("tools".into(), serde_yaml::Value::Sequence(arr));
    }
    if let Some(sm) = &candidate.codex_sandbox_mode {
        m.insert("codex_sandbox_mode".into(), sm.clone().into());
    }
    if let Some(re) = &candidate.codex_reasoning_effort {
        m.insert("codex_reasoning_effort".into(), re.clone().into());
    }

    let new_yaml = serde_yaml::to_string(&map)?;
    let new_content = format!("---\n{}---\n{}", new_yaml, body);
    fs::write(dest_path, new_content)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{central_repo, skill_store::SkillStore};
    use tempfile::{tempdir, TempDir};

    /// Build a fixture source workspace in a temp dir.
    fn make_fixture_workspace() -> TempDir {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        // agents/backend-architect.md with frontmatter + body
        fs::create_dir_all(root.join("agents")).unwrap();
        fs::write(
            root.join("agents/backend-architect.md"),
            "---\nname: backend-architect\ndescription: Backend expert\ntools: Read, Write\nmodel: inherit\n---\n# Backend Architect\n\nBody content here.\n",
        ).unwrap();
        // An agent NOT in the active set
        fs::write(
            root.join("agents/inactive-agent.md"),
            "---\nname: inactive-agent\n---\n# Inactive\n",
        ).unwrap();

        // commands/deploy.md
        fs::create_dir_all(root.join("commands")).unwrap();
        fs::write(root.join("commands/deploy.md"), "# Deploy\n\nDeploy command.\n").unwrap();

        // hooks/pre-commit (flat file, no extension)
        fs::create_dir_all(root.join("hooks")).unwrap();
        fs::write(root.join("hooks/pre-commit"), "#!/bin/sh\necho pre-commit\n").unwrap();

        // rules/security.md
        fs::create_dir_all(root.join("rules")).unwrap();
        fs::write(root.join("rules/security.md"), "# Security\n\nSecurity rules.\n").unwrap();

        // workflows/onboard.md + a .toml artifact that must be filtered out
        fs::create_dir_all(root.join("workflows")).unwrap();
        fs::write(root.join("workflows/onboard.md"), "# Onboard\n\nOnboarding workflow.\n").unwrap();
        fs::write(root.join("workflows/onboard.toml"), "[workflow]\nname = \"onboard\"\n").unwrap();

        // skills/foo/SKILL.md
        fs::create_dir_all(root.join("skills/foo")).unwrap();
        fs::write(root.join("skills/foo/SKILL.md"), "---\nname: foo\n---\n# Foo skill\n").unwrap();

        // registry/active.json
        fs::create_dir_all(root.join("registry")).unwrap();
        fs::write(
            root.join("registry/active.json"),
            r#"{
  "agents": [
    {
      "id": "backend-architect",
      "display_name": "Backend Architect",
      "source": "agents/backend-architect.md",
      "description": "Backend expert",
      "tools": ["Read", "Write"],
      "codex_sandbox_mode": "workspace-write",
      "codex_reasoning_effort": "high"
    }
  ],
  "skills": [{"id": "foo", "source": "skills/foo", "category": "core"}],
  "plugins": []
}"#,
        ).unwrap();

        tmp
    }

    /// Holds an active test env: locked global state + central repo temp dir + store.
    struct TestEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
        _central_tmp: TempDir,
        pub store: SkillStore,
    }

    fn make_test_env() -> TestEnv {
        let lock = central_repo::test_base_dir_lock();
        let central_tmp = tempdir().unwrap();
        let base = central_tmp.path().join("central");
        central_repo::set_test_base_dir_override(Some(base.clone()));
        crate::core::central_repo::ensure_central_repo().unwrap();
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

    #[test]
    fn list_candidates_returns_correct_types_and_active_set() {
        let workspace = make_fixture_workspace();
        let candidates = list_candidates(workspace.path()).unwrap();

        assert!(!candidates.is_empty(), "should find some candidates");

        // backend-architect should be in active set
        let ba = candidates
            .iter()
            .find(|c| c.id_or_name == "backend-architect")
            .expect("backend-architect not found");
        assert_eq!(ba.asset_type, "agent");
        assert!(ba.in_active_set, "backend-architect should be in active set");
        assert_eq!(ba.codex_reasoning_effort.as_deref(), Some("high"));
        assert_eq!(ba.codex_sandbox_mode.as_deref(), Some("workspace-write"));
        assert_eq!(ba.display_name.as_deref(), Some("Backend Architect"));
        assert!(ba
            .tools
            .as_ref()
            .map(|t| t.contains(&"Read".to_string()))
            .unwrap_or(false));

        // inactive-agent should NOT be in active set
        let ia = candidates
            .iter()
            .find(|c| c.id_or_name == "inactive-agent")
            .expect("inactive-agent not found");
        assert_eq!(ia.asset_type, "agent");
        assert!(!ia.in_active_set, "inactive-agent should NOT be in active set");
        assert!(ia.codex_reasoning_effort.is_none());

        // deploy command
        let deploy = candidates
            .iter()
            .find(|c| c.id_or_name == "deploy")
            .expect("deploy not found");
        assert_eq!(deploy.asset_type, "command");
        assert!(!deploy.in_active_set);

        // hook
        let hook = candidates
            .iter()
            .find(|c| c.id_or_name == "pre-commit")
            .expect("pre-commit hook not found");
        assert_eq!(hook.asset_type, "hook");
        assert!(!hook.in_active_set);

        // rule
        let rule = candidates
            .iter()
            .find(|c| c.id_or_name == "security")
            .expect("security rule not found");
        assert_eq!(rule.asset_type, "rule");
        assert!(!rule.in_active_set);

        // workflow onboard.md -> "onboard" candidate; onboard.toml must be filtered
        let wf = candidates
            .iter()
            .find(|c| c.id_or_name == "onboard" && c.asset_type == "workflow")
            .expect("onboard workflow not found");
        assert_eq!(wf.asset_type, "workflow");
        assert!(!wf.in_active_set);
        // .toml artifact must not appear as a workflow candidate
        assert_eq!(
            candidates.iter().filter(|c| c.asset_type == "workflow").count(),
            1,
            "exactly one workflow candidate (onboard.md); toml artifact must be filtered"
        );

        // skill foo
        let foo = candidates
            .iter()
            .find(|c| c.id_or_name == "foo" && c.asset_type == "skill")
            .expect("foo skill not found");
        assert!(foo.in_active_set, "foo skill should be in active set");
    }

    #[test]
    fn import_copies_files_and_creates_store_rows() {
        let workspace = make_fixture_workspace();
        let env = make_test_env();

        let candidates = list_candidates(workspace.path()).unwrap();
        let results = import_candidates(&candidates, &env.store).unwrap();

        assert!(!results.is_empty(), "should have import results");

        // Verify backend-architect was imported
        let ba_result = results
            .iter()
            .find(|r| r.id_or_name == "backend-architect")
            .expect("backend-architect not imported");
        assert!(
            ba_result.central_path.exists(),
            "central path for agent should exist"
        );

        // Verify hook pre-commit was imported
        let hook_result = results
            .iter()
            .find(|r| r.id_or_name == "pre-commit")
            .expect("pre-commit not imported");
        assert!(
            hook_result.central_path.exists(),
            "central path for hook should exist"
        );

        // Verify skill foo dir was imported
        let foo_result = results
            .iter()
            .find(|r| r.id_or_name == "foo")
            .expect("foo skill not imported");
        assert!(
            foo_result.central_path.exists(),
            "central path for skill dir should exist"
        );
        assert!(
            foo_result.central_path.join("SKILL.md").exists(),
            "SKILL.md should exist in imported skill dir"
        );

        // Verify store has rows with source_type="import"
        let all_skills = env.store.get_all_skills().unwrap();
        assert!(!all_skills.is_empty(), "store should have records");
        for rec in &all_skills {
            assert_eq!(
                rec.source_type, "import",
                "all imported rows should have source_type=import"
            );
        }
    }

    #[test]
    fn imported_agent_retains_codex_reasoning_effort() {
        let workspace = make_fixture_workspace();
        let env = make_test_env();

        let candidates = list_candidates(workspace.path()).unwrap();
        import_candidates(&candidates, &env.store).unwrap();

        let ba_result_path =
            central_repo::asset_type_dir(crate::core::skill_store::AssetType::Agent)
                .join("backend-architect.md");
        assert!(
            ba_result_path.exists(),
            "backend-architect.md should exist in agents dir"
        );

        let content = fs::read_to_string(&ba_result_path).unwrap();
        assert!(
            content.contains("codex_reasoning_effort"),
            "should contain codex_reasoning_effort"
        );
        assert!(
            content.contains("high"),
            "codex_reasoning_effort should be 'high'"
        );
        assert!(
            content.contains("workspace-write"),
            "codex_sandbox_mode should be 'workspace-write'"
        );
    }

    #[test]
    fn source_fixture_bytes_unchanged_after_import() {
        let workspace = make_fixture_workspace();
        let env = make_test_env();

        let src_agent = workspace.path().join("agents/backend-architect.md");
        let before = fs::read(&src_agent).unwrap();

        let candidates = list_candidates(workspace.path()).unwrap();
        import_candidates(&candidates, &env.store).unwrap();

        let after = fs::read(&src_agent).unwrap();
        assert_eq!(before, after, "source file must not be modified by import");
    }

    // ── Pollution-filter tests ─────────────────────────────────────────────────

    /// Render artifacts (.agent.md) and config files (.toml) must not appear as
    /// agent candidates; only the canonical plain `<name>.md` source is discovered.
    #[test]
    fn scan_md_excludes_agent_render_artifacts_and_toml() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("agents")).unwrap();

        // Canonical source — should be discovered
        fs::write(root.join("agents/backend-architect.md"), "# Agent\n").unwrap();
        // Render artifact — must be excluded
        fs::write(
            root.join("agents/backend-architect.agent.md"),
            "# Render artifact\n",
        )
        .unwrap();
        // Toml config — must be excluded
        fs::write(root.join("agents/backend-architect.toml"), "[agent]\n").unwrap();

        // No registry needed; list_candidates handles a missing registry gracefully.
        let candidates = list_candidates(root).unwrap();
        let agent_names: Vec<&str> = candidates
            .iter()
            .filter(|c| c.asset_type == "agent")
            .map(|c| c.id_or_name.as_str())
            .collect();

        assert!(
            agent_names.contains(&"backend-architect"),
            "canonical agent must be present; got: {:?}",
            agent_names
        );
        assert!(
            !agent_names.contains(&"backend-architect.agent"),
            "render artifact must be excluded; got: {:?}",
            agent_names
        );
        assert!(
            !agent_names
                .iter()
                .any(|n| n.ends_with(".toml") || n.contains("toml")),
            "toml config must be excluded; got: {:?}",
            agent_names
        );
    }

    /// Documentation files (.md) inside hooks/ or scripts/ must not be imported
    /// as hook/script candidates; only executable/script files are accepted.
    #[test]
    fn scan_flat_files_excludes_md_and_toml_docs() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("scripts")).unwrap();

        // Real script — should be discovered
        fs::write(root.join("scripts/deploy.sh"), "#!/bin/sh\n").unwrap();
        // Documentation file — must be excluded
        fs::write(root.join("scripts/README.md"), "# Scripts\n").unwrap();
        // Toml config — must be excluded
        fs::write(root.join("scripts/config.toml"), "[settings]\n").unwrap();

        let candidates = list_candidates(root).unwrap();
        let script_names: Vec<&str> = candidates
            .iter()
            .filter(|c| c.asset_type == "script")
            .map(|c| c.id_or_name.as_str())
            .collect();

        assert!(
            script_names.contains(&"deploy.sh"),
            "real script must be present; got: {:?}",
            script_names
        );
        assert!(
            !script_names.contains(&"README.md"),
            ".md doc must be excluded; got: {:?}",
            script_names
        );
        assert!(
            !script_names.contains(&"config.toml"),
            ".toml config must be excluded; got: {:?}",
            script_names
        );
    }

    /// Skill candidate discovery must require a SKILL.md marker file.
    /// Directories without SKILL.md (e.g. .system, assets, ci junk dirs) are excluded.
    #[test]
    fn skills_scan_requires_skill_md_marker() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("skills/real-skill")).unwrap();
        fs::write(root.join("skills/real-skill/SKILL.md"), "# Real skill\n").unwrap();

        // Junk directory with no marker — must be excluded
        fs::create_dir_all(root.join("skills/junk-dir")).unwrap();
        fs::write(root.join("skills/junk-dir/README.md"), "# Not a skill\n").unwrap();

        let candidates = list_candidates(root).unwrap();
        let skill_names: Vec<&str> = candidates
            .iter()
            .filter(|c| c.asset_type == "skill")
            .map(|c| c.id_or_name.as_str())
            .collect();

        assert!(
            skill_names.contains(&"real-skill"),
            "skill with SKILL.md must be present; got: {:?}",
            skill_names
        );
        assert!(
            !skill_names.contains(&"junk-dir"),
            "dir without SKILL.md must be excluded; got: {:?}",
            skill_names
        );
    }

    // ── Workflow-specific tests ────────────────────────────────────────────────

    #[test]
    fn workflow_round_trip_from_str_and_as_str() {
        use crate::core::skill_store::AssetType;
        assert_eq!(AssetType::from_str("workflow"), AssetType::Workflow);
        assert_eq!(AssetType::Workflow.as_str(), "workflow");
    }

    /// workflows/ scan yields "onboard" but not the .toml artifact.
    #[test]
    fn list_candidates_discovers_workflow_and_filters_toml_artifact() {
        let workspace = make_fixture_workspace();
        let candidates = list_candidates(workspace.path()).unwrap();

        let wf_names: Vec<&str> = candidates
            .iter()
            .filter(|c| c.asset_type == "workflow")
            .map(|c| c.id_or_name.as_str())
            .collect();

        assert!(
            wf_names.contains(&"onboard"),
            "onboard workflow must be discovered; got: {:?}",
            wf_names
        );
        assert_eq!(
            wf_names.len(),
            1,
            "exactly one workflow candidate expected (.toml must be filtered); got: {:?}",
            wf_names
        );
    }

    /// Importing a workflow candidate copies the file into the central repo.
    #[test]
    fn import_workflow_copies_file_to_central_repo() {
        let workspace = make_fixture_workspace();
        let env = make_test_env();

        let candidates = list_candidates(workspace.path()).unwrap();
        let results = import_candidates(&candidates, &env.store).unwrap();

        let wf_result = results
            .iter()
            .find(|r| r.id_or_name == "onboard" && r.asset_type == "workflow")
            .expect("onboard workflow must appear in import results");

        assert!(
            wf_result.central_path.exists(),
            "central path for workflow must exist: {}",
            wf_result.central_path.display()
        );

        // Verify it landed in the workflows/ subdir of the central repo.
        let expected_dir = central_repo::asset_type_dir(crate::core::skill_store::AssetType::Workflow);
        assert_eq!(
            wf_result.central_path.parent().unwrap(),
            expected_dir,
            "workflow file must be inside the workflows/ central subdir"
        );

        // Store must have a row for it.
        let rows = env.store.get_skills_by_asset_type(crate::core::skill_store::AssetType::Workflow).unwrap();
        assert_eq!(rows.len(), 1, "exactly one workflow row must be in the store");
        assert_eq!(rows[0].name, "onboard");
    }
}
