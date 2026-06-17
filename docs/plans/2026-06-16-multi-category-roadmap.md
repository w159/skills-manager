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
- [x] delete_managed_asset command + frontend wiring (RC3) - VERIFIED (351 tests).
- [ ] GATED: migrate setup.py home symlinks -> skills-manager-managed (W0.3). Still pending.
      EFFECT until done: delivery to Codex/Copilot is REFUSED by the foreign-home guard
      (their agents/ dirs symlink into agentic-tools). Claude/Pi deliver fine.
- [x] Re-seed central repo (W0.9) - DONE. Reality: DB had 79 rows (the "156" was a filesystem
      dir count, not DB rows). Only 2 genuine junk skill rows existed; removed. Backup at
      ~/.skills-manager/skills-manager.db.reseed-backup-20260617-005659.

### Wave 1 - Plugins (marquee; fixes the original "plugin contents leak")
Real structure (mapped): installed_plugins.json (plugins keyed name@marketplace + version +
installPath + gitCommitSha), known_marketplaces.json (git/github sources + installLocation),
blocklist.json. Payload at cache/{marketplace}/{name}/{version}/ with .claude-plugin/plugin.json
declaring bundled skills/agents/commands/hooks/mcpServers/rules. Marketplaces cloned under
marketplaces/{name}/ (plugins/ + .claude-plugin/marketplace.json). Identity = name@marketplace +
version. iCloud junk = " 2" suffix / 0700 perms -> ignore.

Sub-slices:
- P1 (read-only): backend plugin discovery from installed_plugins.json + manifest parse +
  bundled-asset attribution model; frontend Plugins tab = inventory + drill-down. NO writes.
  P1 DONE - VERIFIED (backend: 349 tests, 15 real plugins, zero writes, isolated from deliver;
  frontend: PluginsPanel read-only inventory + drill-down, DTO field-match verified, build green).
  Display decision: TAG + FILTER (plugin-owned assets show in flat tabs with a "from <plugin>"
  badge + show/hide toggle) -> that is P2, NEXT.
  Tidy-up: dead_code warnings on unread PluginManifest fields.
- P2: reflect plugin ownership in the flat Skills/Agents/Commands lists (the leak fix) per the
  display decision below. Depends on P1 attribution.
- P3 (GATED writes): enable/disable installed plugins (Claude Code config).
- P4 (GATED, large): browse + install/remove from marketplaces (git clone + payload install).

Display decision (pending user): how plugin-owned assets appear in the flat type lists.

### Wave 2 - MCP servers
Discover mcp.json server defs -> manage -> deliver into each agent's MCP config (config-merge).

### Wave 3 - Workflows  [DONE - VERIFIED]
Simple type like commands: discover workflows/*.md -> Place into <home>/workflows/ for
Claude+Pi (n/a Codex/Copilot). Shipped: AssetType::Workflow end-to-end + Library tab/nav.
Verified independently: 343 tests; 8 real agentic-tools workflows discoverable; delivery
correct. Done out of order (before Plugins) as the pattern-proving vertical slice.

### Wave 4 - Extensions
Minimal shell (empty source today); category + tab, no-op delivery until content exists.

### Wave 5 - Marketplace / GitHub discovery
Generalize the existing GitHub skills install flow with category filters (plugins, mcp,
workflows, extensions, tools) using the signed-in GitHub account/API.

## Verification posture
Every shipping change: red->green evidence + independent verifier (law 5). Desktop-visual
checks are the user's manual relaunch step (Tauri webview not headlessly runnable);
backend covered by cargo test; frontend by build+lint + user runtime confirmation.
