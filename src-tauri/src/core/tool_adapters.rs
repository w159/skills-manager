use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::skill_store::AssetType;
use crate::core::sync_engine::SyncMode;

// ---------------------------------------------------------------------------
// Asset capability types
// ---------------------------------------------------------------------------

/// Which renderer to invoke when `mode` is `SyncMode::Render`.
/// Carried in `AssetCapability` so the orchestration stage can select the
/// correct render function without knowing the adapter identity at that point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    Codex,
    Copilot,
}

/// How a single `(adapter, AssetType)` pair is delivered.
///
/// - `target_subdir`: path segment appended to the adapter's home dir
///   (e.g. `"agents"` -> `~/.claude/agents/`).
/// - `mode`: the `SyncMode` the delivery engine should use.
/// - `filename_rule`: `None` for directory-per-asset types (Skill, Hook,
///   Script, Rule with Place mode); `Some(template)` for file-per-asset
///   types where `{id}` and `{name}` are interpolated by the caller.
/// - `renderer`: populated only when `mode == SyncMode::Render`; identifies
///   which render function to call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCapability {
    pub target_subdir: &'static str,
    pub mode: SyncMode,
    pub filename_rule: Option<&'static str>,
    pub renderer: Option<Renderer>,
}

/// Top-level grouping for sidebar/overview display. Does not affect skill
/// deployment, sync, or any other backend behavior — purely a UI taxonomy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCategory {
    /// Coding agents (Claude Code, Cursor, Codex, etc.). The default.
    #[default]
    Coding,
    /// Lobster-class personal AI assistants (OpenClaw ecosystem, Hermes Agent).
    Lobster,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolAdapter {
    pub key: String,
    pub display_name: String,
    pub relative_skills_dir: String,
    pub relative_detect_dir: String,
    /// Additional directories to scan for skills (e.g. plugin/marketplace dirs).
    /// These are only used for discovery, not for deployment.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub additional_scan_dirs: Vec<String>,
    /// When set, overrides the computed skills_dir with this absolute path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_skills_dir: Option<String>,
    /// Whether this is a user-defined custom agent (not built-in).
    #[serde(default)]
    pub is_custom: bool,
    /// When true, scan the skills directory recursively for skill directories
    /// (directories containing SKILL.md) instead of treating immediate children as skills.
    /// Used by tools with nested category directories (e.g., Hermes Agent).
    #[serde(default)]
    pub recursive_scan: bool,
    /// Optional override for the project-level skills path. When `None`, the
    /// project-level path falls back to `relative_skills_dir`. Used by tools
    /// like OpenCode where the global path (`~/.config/opencode/skills`)
    /// differs from the project path (`.opencode/skills`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_relative_skills_dir: Option<String>,
    /// UI grouping. See [`ToolCategory`].
    #[serde(default)]
    pub category: ToolCategory,
}

/// Serializable custom tool definition stored in settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolDef {
    pub key: String,
    pub display_name: String,
    pub skills_dir: String,
    #[serde(default)]
    pub project_relative_skills_dir: Option<String>,
    #[serde(default)]
    pub category: ToolCategory,
}

impl ToolAdapter {
    fn home() -> PathBuf {
        dirs::home_dir().expect("Cannot determine home directory")
    }

    fn candidate_paths(relative: &str) -> Vec<PathBuf> {
        let mut candidates = vec![Self::home().join(relative)];

        if let Some(suffix) = relative.strip_prefix(".config/") {
            if let Some(config_dir) = dirs::config_dir() {
                let config_path = config_dir.join(suffix);
                if !candidates.contains(&config_path) {
                    candidates.push(config_path);
                }
            }
        }

        candidates
    }

    fn select_existing_or_default(paths: &[PathBuf]) -> PathBuf {
        paths
            .iter()
            .find(|path| path.exists())
            .cloned()
            .unwrap_or_else(|| paths[0].clone())
    }

    pub fn skills_dir(&self) -> PathBuf {
        if let Some(ref abs) = self.override_skills_dir {
            return PathBuf::from(abs);
        }
        let candidates = Self::candidate_paths(&self.relative_skills_dir);
        Self::select_existing_or_default(&candidates)
    }

    /// Project-relative skills path used when scanning workspaces. Falls back
    /// to `relative_skills_dir` when no project-specific override is set.
    pub fn project_relative_skills_dir(&self) -> &str {
        self.project_relative_skills_dir
            .as_deref()
            .unwrap_or(&self.relative_skills_dir)
    }

    /// Returns all directories to scan for skills: the primary skills_dir plus any additional scan dirs.
    pub fn all_scan_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.skills_dir()];
        for c in self.additional_existing_scan_dirs() {
            if !dirs.contains(&c) {
                dirs.push(c);
            }
        }
        dirs
    }

    /// Returns the existing additional discovery roots for this adapter.
    pub fn additional_existing_scan_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        for rel in &self.additional_scan_dirs {
            let candidates = Self::candidate_paths(rel);
            for c in candidates {
                if c.exists() && !dirs.contains(&c) {
                    dirs.push(c);
                }
            }
        }
        dirs
    }

    pub fn is_installed(&self) -> bool {
        // Product decision: when users explicitly provide a skills path (override/custom),
        // we treat the tool as available so sync can proceed without probing vendor install state.
        if self.is_custom || self.override_skills_dir.is_some() {
            return true;
        }
        Self::candidate_paths(&self.relative_detect_dir)
            .iter()
            .any(|path| path.exists())
    }

    /// Whether this adapter's skills_dir has been overridden from the default.
    pub fn has_path_override(&self) -> bool {
        self.override_skills_dir.is_some()
    }

    /// Returns how this adapter delivers the given `AssetType`, or `None` when
    /// the type is unsupported by this adapter.
    ///
    /// `AssetType::Skill` always returns `None` here — callers must continue
    /// using the existing `skills_dir()` + engine sync-mode path so that Skill
    /// delivery behavior is not altered.
    pub fn asset_capability(&self, asset_type: AssetType) -> Option<AssetCapability> {
        // Skill delivery is intentionally excluded: the existing engine path
        // (skills_dir + sync_mode_for_tool) owns that behavior.
        if asset_type == AssetType::Skill {
            return None;
        }

        match self.key.as_str() {
            // ------------------------------------------------------------------
            // Claude Code  (~/.claude)
            // ------------------------------------------------------------------
            "claude_code" => match asset_type {
                AssetType::Agent => Some(AssetCapability {
                    target_subdir: "agents",
                    mode: SyncMode::Symlink,
                    filename_rule: Some("{id}.md"),
                    renderer: None,
                }),
                AssetType::Command => Some(AssetCapability {
                    target_subdir: "commands",
                    mode: SyncMode::Symlink,
                    filename_rule: Some("{name}.md"),
                    renderer: None,
                }),
                AssetType::Hook => Some(AssetCapability {
                    target_subdir: "hooks",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                AssetType::Script => Some(AssetCapability {
                    target_subdir: "scripts",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                AssetType::Rule => Some(AssetCapability {
                    target_subdir: "rules",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                AssetType::Workflow => Some(AssetCapability {
                    target_subdir: "workflows",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                // Extension: no delivery target yet (shell only)
                AssetType::Extension => None,
                AssetType::Skill => unreachable!("guarded above"),
            },

            // ------------------------------------------------------------------
            // Pi  (~/.pi/agent)
            // ------------------------------------------------------------------
            "pi" => match asset_type {
                AssetType::Agent => Some(AssetCapability {
                    target_subdir: "agents",
                    mode: SyncMode::Symlink,
                    filename_rule: Some("{id}.md"),
                    renderer: None,
                }),
                AssetType::Command => Some(AssetCapability {
                    target_subdir: "commands",
                    mode: SyncMode::Symlink,
                    filename_rule: Some("{name}.md"),
                    renderer: None,
                }),
                AssetType::Hook => Some(AssetCapability {
                    target_subdir: "hooks",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                AssetType::Script => Some(AssetCapability {
                    target_subdir: "scripts",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                AssetType::Rule => Some(AssetCapability {
                    target_subdir: "rules",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                AssetType::Workflow => Some(AssetCapability {
                    target_subdir: "workflows",
                    mode: SyncMode::Place,
                    filename_rule: None,
                    renderer: None,
                }),
                // Extension: no delivery target yet (shell only)
                AssetType::Extension => None,
                AssetType::Skill => unreachable!("guarded above"),
            },

            // ------------------------------------------------------------------
            // Codex  (~/.codex)
            // ------------------------------------------------------------------
            "codex" => match asset_type {
                AssetType::Agent => Some(AssetCapability {
                    target_subdir: "agents",
                    mode: SyncMode::Render,
                    filename_rule: Some("{id}.toml"),
                    renderer: Some(Renderer::Codex),
                }),
                AssetType::Command => Some(AssetCapability {
                    target_subdir: "commands",
                    mode: SyncMode::Symlink,
                    filename_rule: Some("{name}.md"),
                    renderer: None,
                }),
                // Hook, Script, Rule, Workflow, Extension: unsupported for Codex
                AssetType::Hook
                | AssetType::Script
                | AssetType::Rule
                | AssetType::Workflow
                | AssetType::Extension => None,
                AssetType::Skill => unreachable!("guarded above"),
            },

            // ------------------------------------------------------------------
            // GitHub Copilot  (~/.copilot)
            // ------------------------------------------------------------------
            "github_copilot" => match asset_type {
                AssetType::Agent => Some(AssetCapability {
                    target_subdir: "agents",
                    mode: SyncMode::Render,
                    filename_rule: Some("{id}.agent.md"),
                    renderer: Some(Renderer::Copilot),
                }),
                // Command, Hook, Script, Rule, Workflow, Extension: unsupported for Copilot
                AssetType::Command
                | AssetType::Hook
                | AssetType::Script
                | AssetType::Rule
                | AssetType::Workflow
                | AssetType::Extension => None,
                AssetType::Skill => unreachable!("guarded above"),
            },

            // Every other adapter: no capability for non-Skill types.
            _ => None,
        }
    }
}

pub fn default_tool_adapters() -> Vec<ToolAdapter> {
    vec![
        ToolAdapter {
            key: "cursor".into(),
            display_name: "Cursor".into(),
            relative_skills_dir: ".cursor/skills".into(),
            relative_detect_dir: ".cursor".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "claude_code".into(),
            display_name: "Claude Code".into(),
            relative_skills_dir: ".claude/skills".into(),
            relative_detect_dir: ".claude".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            // Codex CLI reads user-level skills from `~/.codex/skills/` and
            // project-level skills from `<repo>/.codex/skills/`. The shared
            // `~/.agents/skills` location is kept as a discovery fallback so
            // skills synced there by other adapters (or by older skills-manager
            // versions that deployed Codex there by mistake) still surface in
            // the Codex tab.
            //
            // Note: `AGENT_SKILLS_PATH` (openai/codex#13074) is a proposed
            // env var that would let Codex load from `<custom>/.agents/skills`;
            // until it ships, `.codex/skills` is the only path Codex CLI
            // actually reads.
            key: "codex".into(),
            display_name: "Codex".into(),
            relative_skills_dir: ".codex/skills".into(),
            relative_detect_dir: ".codex".into(),
            additional_scan_dirs: vec![".agents/skills".into()],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            // Grok reads user-level skills from `~/.grok/skills/` and
            // project-level skills from `<repo>/.grok/skills/`.
            // See https://docs.x.ai/build/features/skills-plugins-marketplaces
            key: "grok".into(),
            display_name: "Grok".into(),
            relative_skills_dir: ".grok/skills".into(),
            relative_detect_dir: ".grok".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "opencode".into(),
            display_name: "OpenCode".into(),
            relative_skills_dir: ".config/opencode/skills".into(),
            relative_detect_dir: ".config/opencode".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: Some(".opencode/skills".into()),
        },
        ToolAdapter {
            key: "antigravity".into(),
            display_name: "Antigravity".into(),
            relative_skills_dir: ".gemini/antigravity/skills".into(),
            relative_detect_dir: ".gemini/antigravity".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "amp".into(),
            display_name: "Amp".into(),
            relative_skills_dir: ".config/agents/skills".into(),
            relative_detect_dir: ".config/agents".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "kilo_code".into(),
            display_name: "Kilo Code".into(),
            relative_skills_dir: ".kilocode/skills".into(),
            relative_detect_dir: ".kilocode".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "roo_code".into(),
            display_name: "Roo Code".into(),
            relative_skills_dir: ".roo/skills".into(),
            relative_detect_dir: ".roo".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "goose".into(),
            display_name: "Goose".into(),
            relative_skills_dir: ".config/goose/skills".into(),
            relative_detect_dir: ".config/goose".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "gemini_cli".into(),
            display_name: "Gemini CLI".into(),
            relative_skills_dir: ".gemini/skills".into(),
            relative_detect_dir: ".gemini".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "github_copilot".into(),
            display_name: "GitHub Copilot".into(),
            relative_skills_dir: ".copilot/skills".into(),
            relative_detect_dir: ".copilot".into(),
            // GitHub Copilot now reads skills from the unified `~/.agents/skills` location too.
            additional_scan_dirs: vec![".agents/skills".into()],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "openclaw".into(),
            display_name: "OpenClaw".into(),
            relative_skills_dir: ".openclaw/skills".into(),
            relative_detect_dir: ".openclaw".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Lobster,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "droid".into(),
            display_name: "Droid".into(),
            relative_skills_dir: ".factory/skills".into(),
            relative_detect_dir: ".factory".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "windsurf".into(),
            display_name: "Windsurf".into(),
            relative_skills_dir: ".codeium/windsurf/skills".into(),
            relative_detect_dir: ".codeium/windsurf".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "trae".into(),
            display_name: "TRAE IDE".into(),
            relative_skills_dir: ".trae/skills".into(),
            relative_detect_dir: ".trae".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "cline".into(),
            display_name: "Cline".into(),
            relative_skills_dir: ".agents/skills".into(),
            relative_detect_dir: ".cline".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "deepagents".into(),
            display_name: "Deep Agents".into(),
            relative_skills_dir: ".deepagents/agent/skills".into(),
            relative_detect_dir: ".deepagents".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "firebender".into(),
            display_name: "Firebender".into(),
            relative_skills_dir: ".firebender/skills".into(),
            relative_detect_dir: ".firebender".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "kimi".into(),
            display_name: "Kimi Code CLI".into(),
            relative_skills_dir: ".config/agents/skills".into(),
            relative_detect_dir: ".kimi".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "replit".into(),
            display_name: "Replit".into(),
            relative_skills_dir: ".config/agents/skills".into(),
            relative_detect_dir: ".replit".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "warp".into(),
            display_name: "Warp".into(),
            relative_skills_dir: ".agents/skills".into(),
            relative_detect_dir: ".warp".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "augment".into(),
            display_name: "Augment".into(),
            relative_skills_dir: ".augment/skills".into(),
            relative_detect_dir: ".augment".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "bob".into(),
            display_name: "IBM Bob".into(),
            relative_skills_dir: ".bob/skills".into(),
            relative_detect_dir: ".bob".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "codebuddy".into(),
            display_name: "CodeBuddy".into(),
            relative_skills_dir: ".codebuddy/skills".into(),
            relative_detect_dir: ".codebuddy".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "command_code".into(),
            display_name: "Command Code".into(),
            relative_skills_dir: ".commandcode/skills".into(),
            relative_detect_dir: ".commandcode".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "continue".into(),
            display_name: "Continue".into(),
            relative_skills_dir: ".continue/skills".into(),
            relative_detect_dir: ".continue".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "cortex".into(),
            display_name: "Cortex Code".into(),
            relative_skills_dir: ".snowflake/cortex/skills".into(),
            relative_detect_dir: ".snowflake/cortex".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "crush".into(),
            display_name: "Crush".into(),
            relative_skills_dir: ".config/crush/skills".into(),
            relative_detect_dir: ".config/crush".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "iflow".into(),
            display_name: "iFlow CLI".into(),
            relative_skills_dir: ".iflow/skills".into(),
            relative_detect_dir: ".iflow".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "junie".into(),
            display_name: "Junie".into(),
            relative_skills_dir: ".junie/skills".into(),
            relative_detect_dir: ".junie".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "kiro".into(),
            display_name: "Kiro CLI".into(),
            relative_skills_dir: ".kiro/skills".into(),
            relative_detect_dir: ".kiro".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "kode".into(),
            display_name: "Kode".into(),
            relative_skills_dir: ".kode/skills".into(),
            relative_detect_dir: ".kode".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "mcpjam".into(),
            display_name: "MCPJam".into(),
            relative_skills_dir: ".mcpjam/skills".into(),
            relative_detect_dir: ".mcpjam".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "mistral_vibe".into(),
            display_name: "Mistral Vibe".into(),
            relative_skills_dir: ".vibe/skills".into(),
            relative_detect_dir: ".vibe".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "mux".into(),
            display_name: "Mux".into(),
            relative_skills_dir: ".mux/skills".into(),
            relative_detect_dir: ".mux".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "neovate".into(),
            display_name: "Neovate".into(),
            relative_skills_dir: ".neovate/skills".into(),
            relative_detect_dir: ".neovate".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "openhands".into(),
            display_name: "OpenHands".into(),
            relative_skills_dir: ".openhands/skills".into(),
            relative_detect_dir: ".openhands".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "pi".into(),
            display_name: "Pi".into(),
            relative_skills_dir: ".pi/agent/skills".into(),
            relative_detect_dir: ".pi/agent".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "pochi".into(),
            display_name: "Pochi".into(),
            relative_skills_dir: ".pochi/skills".into(),
            relative_detect_dir: ".pochi".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "qoder".into(),
            display_name: "Qoder".into(),
            relative_skills_dir: ".qoder/skills".into(),
            relative_detect_dir: ".qoder".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "qwen_code".into(),
            display_name: "Qwen Code".into(),
            relative_skills_dir: ".qwen/skills".into(),
            relative_detect_dir: ".qwen".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "trae_cn".into(),
            display_name: "TRAE CN".into(),
            relative_skills_dir: ".trae-cn/skills".into(),
            relative_detect_dir: ".trae-cn".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "zencoder".into(),
            display_name: "Zencoder".into(),
            relative_skills_dir: ".zencoder/skills".into(),
            relative_detect_dir: ".zencoder".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "adal".into(),
            display_name: "AdaL".into(),
            relative_skills_dir: ".adal/skills".into(),
            relative_detect_dir: ".adal".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Coding,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "hermes".into(),
            display_name: "Hermes Agent".into(),
            relative_skills_dir: ".hermes/skills".into(),
            relative_detect_dir: ".hermes".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Lobster,
            is_custom: false,
            recursive_scan: true,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "qclaw".into(),
            display_name: "QClaw".into(),
            relative_skills_dir: ".qclaw/skills".into(),
            relative_detect_dir: ".qclaw".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Lobster,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "easyclaw".into(),
            display_name: "EasyClaw".into(),
            relative_skills_dir: ".easyclaw/skills".into(),
            relative_detect_dir: ".easyclaw".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Lobster,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "autoclaw".into(),
            display_name: "AutoClaw".into(),
            relative_skills_dir: ".openclaw-autoclaw/skills".into(),
            relative_detect_dir: ".openclaw-autoclaw".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Lobster,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
        ToolAdapter {
            key: "workbuddy".into(),
            display_name: "WorkBuddy".into(),
            relative_skills_dir: ".workbuddy/skills-marketplace/skills".into(),
            relative_detect_dir: ".workbuddy".into(),
            additional_scan_dirs: vec![],
            override_skills_dir: None,
            category: ToolCategory::Lobster,
            is_custom: false,
            recursive_scan: false,
            project_relative_skills_dir: None,
        },
    ]
}

/// Read custom tool path overrides from store.
pub fn custom_tool_paths(store: &crate::core::skill_store::SkillStore) -> HashMap<String, String> {
    store
        .get_setting("custom_tool_paths")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Read per-tool project-relative skills path overrides for built-in adapters.
/// Maps tool key -> project-relative path (e.g. `.cursor/skills`). Custom tools
/// store their project path inside [`CustomToolDef`] instead.
pub fn custom_tool_project_paths(
    store: &crate::core::skill_store::SkillStore,
) -> HashMap<String, String> {
    store
        .get_setting("custom_tool_project_paths")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Read user-defined custom tools from store.
pub fn custom_tools(store: &crate::core::skill_store::SkillStore) -> Vec<CustomToolDef> {
    store
        .get_setting("custom_tools")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Returns all tool adapters: built-in (with path overrides applied) + custom tools.
pub fn all_tool_adapters(store: &crate::core::skill_store::SkillStore) -> Vec<ToolAdapter> {
    let overrides = custom_tool_paths(store);
    let project_overrides = custom_tool_project_paths(store);
    let customs = custom_tools(store);

    let mut adapters: Vec<ToolAdapter> = default_tool_adapters()
        .into_iter()
        .map(|mut a| {
            if let Some(path) = overrides.get(&a.key) {
                a.override_skills_dir = Some(path.clone());
            }
            if let Some(project_path) = project_overrides.get(&a.key) {
                a.project_relative_skills_dir = Some(project_path.clone());
            }
            a
        })
        .collect();

    for ct in customs {
        adapters.push(ToolAdapter {
            key: ct.key,
            display_name: ct.display_name,
            relative_skills_dir: ct.project_relative_skills_dir.unwrap_or_default(),
            relative_detect_dir: String::new(),
            additional_scan_dirs: vec![],
            override_skills_dir: Some(ct.skills_dir),
            category: ct.category,
            is_custom: true,
            recursive_scan: false,
            project_relative_skills_dir: None,
        });
    }

    adapters
}

#[allow(dead_code)]
pub fn find_adapter(key: &str) -> Option<ToolAdapter> {
    default_tool_adapters().into_iter().find(|a| a.key == key)
}

/// Find an adapter by key, considering custom tools and path overrides.
pub fn find_adapter_with_store(
    store: &crate::core::skill_store::SkillStore,
    key: &str,
) -> Option<ToolAdapter> {
    if let Some(mut adapter) = default_tool_adapters().into_iter().find(|a| a.key == key) {
        if let Some(path) = custom_tool_paths(store).get(key) {
            adapter.override_skills_dir = Some(path.clone());
        }
        if let Some(project_path) = custom_tool_project_paths(store).get(key) {
            adapter.project_relative_skills_dir = Some(project_path.clone());
        }
        return Some(adapter);
    }

    custom_tools(store)
        .into_iter()
        .find(|ct| ct.key == key)
        .map(|ct| ToolAdapter {
            key: ct.key,
            display_name: ct.display_name,
            relative_skills_dir: ct.project_relative_skills_dir.unwrap_or_default(),
            relative_detect_dir: String::new(),
            additional_scan_dirs: vec![],
            override_skills_dir: Some(ct.skills_dir),
            category: ct.category,
            is_custom: true,
            recursive_scan: false,
            project_relative_skills_dir: None,
        })
}

/// Returns adapters that are installed and not in the disabled list.
pub fn enabled_installed_adapters(
    store: &crate::core::skill_store::SkillStore,
) -> Vec<ToolAdapter> {
    let disabled: Vec<String> = store
        .get_setting("disabled_tools")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    all_tool_adapters(store)
        .into_iter()
        .filter(|a| a.is_installed() && !disabled.contains(&a.key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{default_tool_adapters, AssetCapability, Renderer};
    use crate::core::skill_store::AssetType;
    use crate::core::sync_engine::SyncMode;

    #[test]
    fn antigravity_uses_current_default_skills_path() {
        let adapter = default_tool_adapters()
            .into_iter()
            .find(|adapter| adapter.key == "antigravity")
            .expect("antigravity adapter should exist");

        assert_eq!(adapter.relative_skills_dir, ".gemini/antigravity/skills");
    }

    #[test]
    fn claude_code_does_not_scan_plugin_marketplaces_by_default() {
        let adapter = default_tool_adapters()
            .into_iter()
            .find(|adapter| adapter.key == "claude_code")
            .expect("claude_code adapter should exist");

        assert!(adapter.additional_scan_dirs.is_empty());
    }

    #[test]
    fn opencode_uses_distinct_project_and_global_skill_paths() {
        let adapter = default_tool_adapters()
            .into_iter()
            .find(|adapter| adapter.key == "opencode")
            .expect("opencode adapter should exist");

        // Global path under home: ~/.config/opencode/skills
        assert_eq!(adapter.relative_skills_dir, ".config/opencode/skills");
        // Project path under workspace: .opencode/skills
        assert_eq!(adapter.project_relative_skills_dir(), ".opencode/skills");
    }

    // -----------------------------------------------------------------------
    // asset_capability matrix
    // -----------------------------------------------------------------------

    fn find(key: &str) -> super::ToolAdapter {
        default_tool_adapters()
            .into_iter()
            .find(|a| a.key == key)
            .unwrap_or_else(|| panic!("adapter '{}' not found", key))
    }

    #[test]
    fn asset_capability_skill_always_returns_none_for_all_four_core_agents() {
        // Skill delivery must not be changed; asset_capability must return None
        // for Skill on every adapter so callers keep using the existing path.
        for key in &["claude_code", "pi", "codex", "github_copilot"] {
            assert!(
                find(key).asset_capability(AssetType::Skill).is_none(),
                "Skill capability must be None for '{}'",
                key
            );
        }
    }

    #[test]
    fn asset_capability_matrix_claude_code() {
        let a = find("claude_code");

        assert_eq!(
            a.asset_capability(AssetType::Agent),
            Some(AssetCapability {
                target_subdir: "agents",
                mode: SyncMode::Symlink,
                filename_rule: Some("{id}.md"),
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Command),
            Some(AssetCapability {
                target_subdir: "commands",
                mode: SyncMode::Symlink,
                filename_rule: Some("{name}.md"),
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Hook),
            Some(AssetCapability {
                target_subdir: "hooks",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Script),
            Some(AssetCapability {
                target_subdir: "scripts",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Rule),
            Some(AssetCapability {
                target_subdir: "rules",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Workflow),
            Some(AssetCapability {
                target_subdir: "workflows",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert!(a.asset_capability(AssetType::Extension).is_none());
    }

    #[test]
    fn asset_capability_matrix_pi() {
        let a = find("pi");

        assert_eq!(
            a.asset_capability(AssetType::Agent),
            Some(AssetCapability {
                target_subdir: "agents",
                mode: SyncMode::Symlink,
                filename_rule: Some("{id}.md"),
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Command),
            Some(AssetCapability {
                target_subdir: "commands",
                mode: SyncMode::Symlink,
                filename_rule: Some("{name}.md"),
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Hook),
            Some(AssetCapability {
                target_subdir: "hooks",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Script),
            Some(AssetCapability {
                target_subdir: "scripts",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Rule),
            Some(AssetCapability {
                target_subdir: "rules",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Workflow),
            Some(AssetCapability {
                target_subdir: "workflows",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            })
        );
        assert!(a.asset_capability(AssetType::Extension).is_none());
    }

    #[test]
    fn asset_capability_matrix_codex() {
        let a = find("codex");

        assert_eq!(
            a.asset_capability(AssetType::Agent),
            Some(AssetCapability {
                target_subdir: "agents",
                mode: SyncMode::Render,
                filename_rule: Some("{id}.toml"),
                renderer: Some(Renderer::Codex),
            })
        );
        assert_eq!(
            a.asset_capability(AssetType::Command),
            Some(AssetCapability {
                target_subdir: "commands",
                mode: SyncMode::Symlink,
                filename_rule: Some("{name}.md"),
                renderer: None,
            })
        );
        assert!(a.asset_capability(AssetType::Hook).is_none());
        assert!(a.asset_capability(AssetType::Script).is_none());
        assert!(a.asset_capability(AssetType::Rule).is_none());
        assert!(a.asset_capability(AssetType::Workflow).is_none());
        assert!(a.asset_capability(AssetType::Extension).is_none());
    }

    #[test]
    fn asset_capability_matrix_github_copilot() {
        let a = find("github_copilot");

        assert_eq!(
            a.asset_capability(AssetType::Agent),
            Some(AssetCapability {
                target_subdir: "agents",
                mode: SyncMode::Render,
                filename_rule: Some("{id}.agent.md"),
                renderer: Some(Renderer::Copilot),
            })
        );
        assert!(a.asset_capability(AssetType::Command).is_none());
        assert!(a.asset_capability(AssetType::Hook).is_none());
        assert!(a.asset_capability(AssetType::Script).is_none());
        assert!(a.asset_capability(AssetType::Rule).is_none());
        assert!(a.asset_capability(AssetType::Workflow).is_none());
        assert!(a.asset_capability(AssetType::Extension).is_none());
    }

    #[test]
    fn asset_capability_non_core_adapter_returns_none_for_all_new_types_and_still_has_skill_none() {
        // Spot-check a non-core adapter (cursor) to confirm it falls into the
        // wildcard arm and returns None for every new type.
        let a = find("cursor");
        for t in [
            AssetType::Agent,
            AssetType::Command,
            AssetType::Hook,
            AssetType::Script,
            AssetType::Rule,
            AssetType::Workflow,
            AssetType::Extension,
        ] {
            assert!(
                a.asset_capability(t).is_none(),
                "cursor should return None for {:?}",
                t
            );
        }
        // Skill is also None (existing path unchanged).
        assert!(a.asset_capability(AssetType::Skill).is_none());
    }

    #[test]
    fn asset_capability_workflow_place_for_claude_and_pi_none_for_codex_and_copilot() {
        // Claude Code: Place into workflows/
        assert_eq!(
            find("claude_code").asset_capability(AssetType::Workflow),
            Some(AssetCapability {
                target_subdir: "workflows",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            }),
            "claude_code must Place workflows"
        );
        // Pi: Place into workflows/
        assert_eq!(
            find("pi").asset_capability(AssetType::Workflow),
            Some(AssetCapability {
                target_subdir: "workflows",
                mode: SyncMode::Place,
                filename_rule: None,
                renderer: None,
            }),
            "pi must Place workflows"
        );
        // Codex: unsupported
        assert!(
            find("codex")
                .asset_capability(AssetType::Workflow)
                .is_none(),
            "codex must return None for Workflow"
        );
        // GitHub Copilot: unsupported
        assert!(
            find("github_copilot")
                .asset_capability(AssetType::Workflow)
                .is_none(),
            "github_copilot must return None for Workflow"
        );
    }

    #[test]
    fn asset_capability_extension_is_none_for_all_adapters() {
        // Extension is a shell only — no adapter delivers it yet.
        for key in ["claude_code", "pi", "codex", "github_copilot", "cursor"] {
            assert!(
                find(key).asset_capability(AssetType::Extension).is_none(),
                "{key} must return None for Extension (shell only)"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Agent delivery contract — regression anchor for the symlink scheme
    // -----------------------------------------------------------------------
    //
    // Each tool's Agent row must match the user's manual deployment scheme
    // exactly.  If any of these assertions fail the capability table has
    // drifted from the expected contract and must be investigated before
    // shipping.
    //
    //   Claude Code : Symlink  {id}.md       renderer = None
    //   Codex       : Render   {id}.toml     renderer = Codex
    //   Copilot     : Render   {id}.agent.md renderer = Copilot
    #[test]
    fn agent_delivery_capability_matches_manual_symlink_scheme() {
        // Claude Code: symlink the canonical .md file unchanged
        assert_eq!(
            find("claude_code").asset_capability(AssetType::Agent),
            Some(AssetCapability {
                target_subdir: "agents",
                mode: SyncMode::Symlink,
                filename_rule: Some("{id}.md"),
                renderer: None,
            }),
            "claude_code Agent must use Symlink to {{id}}.md with no renderer"
        );

        // Codex: render the agent into .toml format via the Codex renderer
        assert_eq!(
            find("codex").asset_capability(AssetType::Agent),
            Some(AssetCapability {
                target_subdir: "agents",
                mode: SyncMode::Render,
                filename_rule: Some("{id}.toml"),
                renderer: Some(Renderer::Codex),
            }),
            "codex Agent must use Render to {{id}}.toml with Renderer::Codex"
        );

        // GitHub Copilot: render the agent into .agent.md format via the Copilot renderer
        assert_eq!(
            find("github_copilot").asset_capability(AssetType::Agent),
            Some(AssetCapability {
                target_subdir: "agents",
                mode: SyncMode::Render,
                filename_rule: Some("{id}.agent.md"),
                renderer: Some(Renderer::Copilot),
            }),
            "github_copilot Agent must use Render to {{id}}.agent.md with Renderer::Copilot"
        );
    }
}
