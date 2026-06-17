# Multi-Category Build Roadmap

Agreed scope after the user's first real run of the slice-1 build. Source of truth
for the generalization from a skills-only tool to a full coding-agent asset manager.

## Decisions locked
- Nav: one collapsible **Library** section surfacing every asset category. (DONE for the 6 existing types.)
- skills-manager is the SINGLE owner of the agent homes (~/.claude, ~/.pi/agent, ~/.codex, ~/.copilot).
- Categories to build: Plugins, MCP servers, Workflows, Extensions, plus Marketplace/GitHub discovery.
- Marketplace = GENERALIZE the existing GitHub skills discovery/install flow (skillssh_api.rs / git_fetcher.rs / Install view) with category filters, not a rebuild.
- Plugins surface: attribute bundled assets (fix the leak) + enable/disable installed + browse/install from marketplaces.
- Extensions: source dir is empty today -> build a minimal shell only.

## Source ground truth (agentic-tools/)
- plugins/: 65 entries = config.json (enabled + marketplaces) + cache/ (cloned marketplace repos) + per-plugin dirs.
- mcp/: mcp.json (server definitions).
- workflows/: 8 .md files.
- extensions/: empty.

## Status

### Wave 0 - Foundation (in progress)
- [x] Frontend rebuild (stale dist was the dominant "nothing works" cause) - VERIFIED green.
- [x] Foreign-home delivery guard (stop corrupting agentic-tools via symlinked homes) - VERIFIED.
- [x] central_root via base_dir() + no-churn already-managed links - VERIFIED.
- [x] Importer pollution filters (.agent.md/.toml artifacts, .md-as-script, SKILL.md validation) - VERIFIED (337 tests).
- [x] Nav/IA: grouped Library section, 6 types first-class - build+lint green; user visual check pending.
- [ ] delete_managed_asset command + frontend wiring (RC3). CODE, small.
- [ ] GATED: migrate setup.py home symlinks -> skills-manager-managed (W0.3).
- [ ] GATED: re-seed polluted central repo (156 "skills" -> ~29 real) (W0.9).

### Wave 1 - Plugins (marquee; fixes the original "plugin contents leak")
Backend AssetType::Plugin + plugin manifest/boundary detection + bundled-asset attribution
(nested skills/agents/commands belong to the plugin, not the flat lists) -> enable/disable
(config.json) -> frontend Library>Plugins tab -> install/remove from marketplaces.

### Wave 2 - MCP servers
Discover mcp.json server defs -> manage -> deliver into each agent's MCP config (config-merge).

### Wave 3 - Workflows
Simple type like commands: discover workflows/*.md -> deliver/place. (Good pattern-proving slice.)

### Wave 4 - Extensions
Minimal shell (empty source today); category + tab, no-op delivery until content exists.

### Wave 5 - Marketplace / GitHub discovery
Generalize the existing GitHub skills install flow with category filters (plugins, mcp,
workflows, extensions, tools) using the signed-in GitHub account/API.

## Verification posture
Every shipping change: red->green evidence + independent verifier (law 5). Desktop-visual
checks are the user's manual relaunch step (Tauri webview not headlessly runnable);
backend covered by cargo test; frontend by build+lint + user runtime confirmation.
