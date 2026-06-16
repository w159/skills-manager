/// Pure render functions that transform a canonical agent source
/// (Claude-style markdown: YAML frontmatter + body) into target formats.
///
/// These functions are deterministic and have no filesystem side effects.
/// Output fidelity is validated against the reference Python implementation
/// in `agentic-tools/scripts/agent_assets.py` (`_render_codex_agent`,
/// `_render_copilot_agent`, `_frontmatter`, `_body`, `_title`).
use anyhow::{bail, Result};

/// Parsed representation of a canonical agent source file.
///
/// Fields map 1-to-1 to the YAML frontmatter keys the Python registry uses.
#[derive(Debug, Clone)]
pub struct CanonicalAgent {
    /// Machine identifier, e.g. `"backend-architect"`.
    pub id: String,
    /// Human-readable title override; falls back to title-casing `id` when absent.
    pub display_name: Option<String>,
    /// One-sentence description used in all rendered formats.
    pub description: String,
    /// Tool names, e.g. `["Read", "Grep", "Bash"]`.
    pub tools: Vec<String>,
    /// Codex reasoning effort override; defaults to `"medium"`.
    pub codex_reasoning_effort: Option<String>,
    /// Codex sandbox mode override; defaults to `"workspace-write"`.
    pub codex_sandbox_mode: Option<String>,
    /// The raw markdown body (everything after the frontmatter block).
    pub body: String,
}

// ── Builder: parse an imported agent .md into CanonicalAgent ──────────────

/// Parse an imported agent markdown file into a `CanonicalAgent`.
///
/// The file format is:
/// ```text
/// ---
/// name: backend-architect
/// display_name: Backend Architect        # optional
/// description: One sentence purpose.
/// tools:
///   - Read
///   - Grep
/// codex_reasoning_effort: high           # optional
/// codex_sandbox_mode: workspace-write    # optional
/// ---
/// Body markdown here.
/// ```
///
/// `name` is used as the `id`.  Any field absent in the frontmatter falls
/// back to the same defaults the render functions use at render time.
pub fn canonical_agent_from_file(path: &std::path::Path) -> Result<CanonicalAgent> {
    let content = std::fs::read_to_string(path)?;
    let trimmed = content.trim();

    if !trimmed.starts_with("---") {
        bail!(
            "canonical_agent_from_file: {} has no YAML frontmatter (must start with '---')",
            path.display()
        );
    }

    // Split off the opening "---", then find the closing "\n---"
    let rest = &trimmed[3..]; // skip leading "---"
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("canonical_agent_from_file: no closing '---' in {}", path.display()))?;

    let yaml_str = &rest[..end];
    let body = rest[end + 4..].to_string(); // skip "\n---"

    let yaml: serde_yaml::Value = serde_yaml::from_str(yaml_str).map_err(|e| {
        anyhow::anyhow!(
            "canonical_agent_from_file: YAML parse error in {}: {}",
            path.display(),
            e
        )
    })?;

    let get_str = |key: &str| -> Option<String> {
        yaml.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
    };

    // `name` is the machine id (e.g. "backend-architect").
    let id = get_str("name").ok_or_else(|| {
        anyhow::anyhow!(
            "canonical_agent_from_file: missing 'name' field in {}",
            path.display()
        )
    })?;

    let description = get_str("description").unwrap_or_default();

    let tools: Vec<String> = yaml
        .get("tools")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    Ok(CanonicalAgent {
        id,
        display_name: get_str("display_name"),
        description,
        tools,
        codex_reasoning_effort: get_str("codex_reasoning_effort"),
        codex_sandbox_mode: get_str("codex_sandbox_mode"),
        body,
    })
}

// ── Internal helpers (mirrors Python helpers exactly) ──────────────────────

/// Convert a kebab-case id to title case: `"backend-architect"` -> `"Backend Architect"`.
fn title(value: &str) -> String {
    value
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Escape backslashes and double-quotes for use inside a TOML double-quoted string.
fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape `"""` sequences so they cannot close the TOML multi-line literal.
fn toml_multiline(value: &str) -> String {
    value.replace("\"\"\"", "\\\"\\\"\\\"")
}

/// Build the rendered body: title heading + provenance line + raw body.
///
/// Mirrors Python's `_body(agent, body)`:
/// ```python
/// f"# {display_name}\n\n"
/// "These instructions were generated from `/Users/jerry/.agents/registry/active.json`.\n\n"
/// f"{body.strip()}\n"
/// ```
fn build_body(agent: &CanonicalAgent) -> String {
    let display = agent
        .display_name
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| title(&agent.id));
    format!(
        "# {}\n\nThese instructions were generated from `/Users/jerry/.agents/registry/active.json`.\n\n{}\n",
        display,
        agent.body.trim(),
    )
}

/// Render a YAML frontmatter block followed by `body`.
///
/// Each metadata value is emitted as-is unless it is a list, in which case
/// it is rendered as a YAML inline sequence with double-quoted items:
/// `tools: ["Read", "Grep"]`.
///
/// Mirrors Python's `_frontmatter(meta, body)`.
fn frontmatter(fields: &[(&str, FrontmatterValue)], body: &str) -> String {
    let mut lines = vec!["---".to_string()];
    for (key, value) in fields {
        match value {
            FrontmatterValue::Str(s) => lines.push(format!("{key}: {s}")),
            FrontmatterValue::List(items) => {
                let rendered = items
                    .iter()
                    .map(|item| format!("\"{item}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("{key}: [{rendered}]"));
            }
        }
    }
    lines.push("---".to_string());
    lines.push(String::new());
    lines.push(body.trim_end().to_string());
    lines.push(String::new());
    lines.join("\n")
}

enum FrontmatterValue<'a> {
    Str(&'a str),
    List(Vec<&'a str>),
}

// ── Public render functions ─────────────────────────────────────────────────

/// Render a canonical agent into Codex `.toml` format.
///
/// Output structure (mirrors `_render_codex_agent`):
/// ```toml
/// name = "backend_architect"
/// description = "..."
/// model_reasoning_effort = "medium"
/// sandbox_mode = "workspace-write"
/// developer_instructions = """
/// # Backend Architect
/// ...
/// """
/// ```
pub fn render_codex(agent: &CanonicalAgent) -> String {
    let name = agent.id.replace('-', "_");
    let description = toml_escape(&agent.description);
    let reasoning_effort = agent
        .codex_reasoning_effort
        .as_deref()
        .unwrap_or("medium");
    let sandbox_mode = agent
        .codex_sandbox_mode
        .as_deref()
        .unwrap_or("workspace-write");

    let body = build_body(agent);
    let body_escaped = toml_multiline(&body);

    // The body already ends with \n (from build_body); the Python join adds
    // one more \n between the body element and the closing """ element.
    format!(
        "name = \"{name}\"\ndescription = \"{description}\"\nmodel_reasoning_effort = \"{reasoning_effort}\"\nsandbox_mode = \"{sandbox_mode}\"\ndeveloper_instructions = \"\"\"\n{body_escaped}\n\"\"\"\n",
    )
}

/// Render a canonical agent into Copilot `.agent.md` format.
///
/// Output structure (mirrors `_render_copilot_agent`):
/// ```markdown
/// ---
/// name: Backend Architect
/// description: ...
/// tools: ["Read", "Grep"]
/// ---
///
/// # Backend Architect
/// ...
/// ```
///
/// Note: tools is a YAML inline list with quoted items, NOT a comma string.
pub fn render_copilot(agent: &CanonicalAgent) -> String {
    let display = agent
        .display_name
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| title(&agent.id));

    let tool_refs: Vec<&str> = agent.tools.iter().map(|s| s.as_str()).collect();
    let fields: &[(&str, FrontmatterValue)] = &[
        ("name", FrontmatterValue::Str(&display)),
        ("description", FrontmatterValue::Str(&agent.description)),
        ("tools", FrontmatterValue::List(tool_refs)),
    ];
    let body = build_body(agent);
    frontmatter(fields, &body)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture agent — same inputs the independent Python verifier will use.
    fn fixture_agent() -> CanonicalAgent {
        CanonicalAgent {
            id: "backend-architect".to_string(),
            display_name: None,
            description: "Use for APIs, services, data flows, backend architecture, schema boundaries, reliability, and integration design.".to_string(),
            tools: vec![
                "Read".to_string(),
                "Grep".to_string(),
                "Glob".to_string(),
                "Bash".to_string(),
                "Edit".to_string(),
                "Write".to_string(),
            ],
            codex_reasoning_effort: None,
            codex_sandbox_mode: None,
            body: "You are a senior backend architect.\n\nDesign scalable systems.".to_string(),
        }
    }

    // Golden output produced by running agent_assets.py with the same inputs.
    // An independent verifier MUST confirm these match the Python output.

    const EXPECTED_CODEX: &str = concat!(
        "name = \"backend_architect\"\n",
        "description = \"Use for APIs, services, data flows, backend architecture, schema boundaries, reliability, and integration design.\"\n",
        "model_reasoning_effort = \"medium\"\n",
        "sandbox_mode = \"workspace-write\"\n",
        "developer_instructions = \"\"\"\n",
        "# Backend Architect\n",
        "\n",
        "These instructions were generated from `/Users/jerry/.agents/registry/active.json`.\n",
        "\n",
        "You are a senior backend architect.\n",
        "\n",
        "Design scalable systems.\n",
        "\n",
        "\"\"\"\n",
    );

    const EXPECTED_COPILOT: &str = concat!(
        "---\n",
        "name: Backend Architect\n",
        "description: Use for APIs, services, data flows, backend architecture, schema boundaries, reliability, and integration design.\n",
        "tools: [\"Read\", \"Grep\", \"Glob\", \"Bash\", \"Edit\", \"Write\"]\n",
        "---\n",
        "\n",
        "# Backend Architect\n",
        "\n",
        "These instructions were generated from `/Users/jerry/.agents/registry/active.json`.\n",
        "\n",
        "You are a senior backend architect.\n",
        "\n",
        "Design scalable systems.\n",
    );

    #[test]
    fn render_codex_golden_output() {
        let agent = fixture_agent();
        let output = render_codex(&agent);
        assert_eq!(output, EXPECTED_CODEX);
    }

    #[test]
    fn render_copilot_golden_output() {
        let agent = fixture_agent();
        let output = render_copilot(&agent);
        assert_eq!(output, EXPECTED_COPILOT);
    }

    // ── Behavioural edge cases ──────────────────────────────────────────────

    #[test]
    fn render_codex_uses_display_name_in_body_title() {
        let mut agent = fixture_agent();
        agent.display_name = Some("My Custom Name".to_string());
        let output = render_codex(&agent);
        assert!(output.contains("# My Custom Name\n"));
    }

    #[test]
    fn render_codex_kebab_id_becomes_snake_case_name_field() {
        let agent = fixture_agent(); // id = "backend-architect"
        let output = render_codex(&agent);
        assert!(output.starts_with("name = \"backend_architect\"\n"));
    }

    #[test]
    fn render_codex_applies_reasoning_effort_override() {
        let mut agent = fixture_agent();
        agent.codex_reasoning_effort = Some("high".to_string());
        let output = render_codex(&agent);
        assert!(output.contains("model_reasoning_effort = \"high\"\n"));
    }

    #[test]
    fn render_codex_applies_sandbox_mode_override() {
        let mut agent = fixture_agent();
        agent.codex_sandbox_mode = Some("sandbox".to_string());
        let output = render_codex(&agent);
        assert!(output.contains("sandbox_mode = \"sandbox\"\n"));
    }

    #[test]
    fn render_codex_escapes_double_quotes_in_description() {
        let mut agent = fixture_agent();
        agent.description = r#"Say "hello""#.to_string();
        let output = render_codex(&agent);
        assert!(output.contains(r#"description = "Say \"hello\"""#));
    }

    #[test]
    fn render_codex_escapes_triple_quotes_in_body() {
        let mut agent = fixture_agent();
        agent.body = r#"Use """triple""" here."#.to_string();
        let output = render_codex(&agent);
        assert!(output.contains("\\\"\\\"\\\"triple\\\"\\\"\\\""));
    }

    #[test]
    fn render_copilot_tools_as_quoted_list_not_comma_string() {
        let agent = fixture_agent();
        let output = render_copilot(&agent);
        // Must be YAML inline list with quoted items, NOT "Read, Grep, ..."
        assert!(output.contains("tools: [\"Read\", \"Grep\""));
    }

    #[test]
    fn render_copilot_uses_display_name_for_name_field() {
        let mut agent = fixture_agent();
        agent.display_name = Some("Arch Expert".to_string());
        let output = render_copilot(&agent);
        assert!(output.contains("name: Arch Expert\n"));
    }

    #[test]
    fn render_copilot_falls_back_to_title_case_id() {
        let agent = fixture_agent(); // no display_name
        let output = render_copilot(&agent);
        assert!(output.contains("name: Backend Architect\n"));
    }

    #[test]
    fn render_copilot_empty_tools_list() {
        let mut agent = fixture_agent();
        agent.tools = vec![];
        let output = render_copilot(&agent);
        assert!(output.contains("tools: []\n"));
    }

    #[test]
    fn title_helper_capitalises_each_segment() {
        assert_eq!(title("backend-architect"), "Backend Architect");
        assert_eq!(title("orc-db-prober"), "Orc Db Prober");
        assert_eq!(title("plain"), "Plain");
    }

    #[test]
    fn toml_escape_handles_backslashes_and_quotes() {
        assert_eq!(toml_escape(r#"a\b"c""#), r#"a\\b\"c\""#);
    }

    #[test]
    fn render_is_deterministic() {
        let agent = fixture_agent();
        assert_eq!(render_codex(&agent), render_codex(&agent));
        assert_eq!(render_copilot(&agent), render_copilot(&agent));
    }

    // ── canonical_agent_from_file ───────────────────────────────────────────

    fn write_agent_md(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(format!("{}.md", name));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn canonical_agent_from_file_parses_all_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_agent_md(
            tmp.path(),
            "backend-architect",
            "---\nname: backend-architect\ndisplay_name: Backend Architect\ndescription: Design scalable systems.\ntools:\n  - Read\n  - Grep\ncodex_reasoning_effort: high\ncodex_sandbox_mode: workspace-write\n---\nBody content here.\n",
        );
        let agent = canonical_agent_from_file(&path).unwrap();
        assert_eq!(agent.id, "backend-architect");
        assert_eq!(agent.display_name.as_deref(), Some("Backend Architect"));
        assert_eq!(agent.description, "Design scalable systems.");
        assert_eq!(agent.tools, vec!["Read", "Grep"]);
        assert_eq!(agent.codex_reasoning_effort.as_deref(), Some("high"));
        assert_eq!(agent.codex_sandbox_mode.as_deref(), Some("workspace-write"));
        assert!(agent.body.contains("Body content here."));
    }

    #[test]
    fn canonical_agent_from_file_codex_reasoning_effort_high() {
        // Spec unit: frontmatter with codex_reasoning_effort="high" -> field = Some("high")
        let tmp = tempfile::tempdir().unwrap();
        let path = write_agent_md(
            tmp.path(),
            "test-agent",
            "---\nname: test-agent\ndescription: A test agent.\ncodex_reasoning_effort: high\n---\nBody.\n",
        );
        let agent = canonical_agent_from_file(&path).unwrap();
        assert_eq!(
            agent.codex_reasoning_effort.as_deref(),
            Some("high"),
            "codex_reasoning_effort must be Some(\"high\")"
        );
    }

    #[test]
    fn canonical_agent_from_file_optional_fields_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_agent_md(
            tmp.path(),
            "minimal",
            "---\nname: minimal\ndescription: Minimal agent.\n---\nBody.\n",
        );
        let agent = canonical_agent_from_file(&path).unwrap();
        assert_eq!(agent.id, "minimal");
        assert!(agent.display_name.is_none());
        assert!(agent.tools.is_empty());
        assert!(agent.codex_reasoning_effort.is_none());
        assert!(agent.codex_sandbox_mode.is_none());
    }

    #[test]
    fn canonical_agent_from_file_missing_name_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_agent_md(
            tmp.path(),
            "noname",
            "---\ndescription: No name field.\n---\nBody.\n",
        );
        assert!(
            canonical_agent_from_file(&path).is_err(),
            "missing 'name' must return Err"
        );
    }

    #[test]
    fn canonical_agent_from_file_no_frontmatter_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_agent_md(tmp.path(), "plain", "No frontmatter here.\n");
        assert!(
            canonical_agent_from_file(&path).is_err(),
            "file without frontmatter must return Err"
        );
    }
}
