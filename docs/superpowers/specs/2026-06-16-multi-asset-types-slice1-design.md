# Multi-Asset Types - Slice 1 Design

Date: 2026-06-16
Status: Approved (design); plan pending
Scope owner: skills-manager (Tauri + React + Rust)

## Context

skills-manager today manages one asset type, "skills," for ~20 coding agents. It
keeps its own central repo at `~/.skills-manager/` (a `skills/` dir plus a SQLite
DB), resolves per-agent target directories through tool adapters, and syncs each
skill to each enabled agent by symlink or copy. Presets, projects, git-backup,
and source-diff all hang off that one type.

Separately, the user hand-maintains a single source of truth at
`agentic-tools/` (agents, commands, hooks, mcp, plugins, rules, scripts, skills,
plus `registry/active.json`) and wires the active subset into `~/.claude`,
`~/.codex`, `~/.pi/agent`, and `~/.copilot` with two Python scripts
(`scripts/setup.py`, `scripts/agent_assets.py`).

Goal: extend skills-manager to manage agents, commands, hooks, scripts, and
rules in addition to skills, through the GUI, with per-agent sync. The app
becomes the system of record over time; `agentic-tools/` stays untouched now and
is phased out once this works.

## Decisions (locked in brainstorming)

1. Source of truth: the app becomes the system. `agentic-tools/` is read-only
   input now, phased out later. The app keeps its own central repo.
2. First slice: the generic asset-type foundation plus the file-shaped types
   (agents, commands, hooks, scripts, rules). MCP, plugins, health dashboard,
   and marketplace are later slices.
3. Hooks and rules: place/track the files now; defer `settings.json` hook
   registration and the `AGENTS.md` `@import` rule activation to a later slice.
4. Target agents: the core set - Claude Code (`~/.claude`), Codex (`~/.codex`),
   Copilot (`~/.copilot`), Pi (`~/.pi/agent`).
5. Agent rendering: in scope for this slice. Agent assets are rendered to
   Codex `.toml` and Copilot `.agent.md` (not symlinked), ported from
   `agentic-tools/scripts/agent_assets.py`.

## Scope

In scope:
- A generic `asset_type` dimension across the data model and sync engine.
- Five new/extended asset types: skill (existing), agent, command, hook,
  script, rule.
- A third delivery mode, "render," plus the existing symlink/copy and a plain
  "place" path.
- Per-(agent x asset_type) target matrix for the four core agents.
- A read-only importer that seeds the app's central repo from `agentic-tools/`.
- Coexistence/backup so existing runtime dirs are not clobbered.
- GUI generalization from a skills-only view to an asset-type-tabbed library.

Out of scope (explicit, each its own later slice):
- MCP servers (config-merge delivery).
- Plugins (cached repos, versions, marketplace).
- Hook activation: writing event registrations into each agent's
  `settings.json`.
- Rule activation: managing the `@import` block in `AGENTS.md`/`CLAUDE.md`.
- Health/maintenance dashboard (broken links, drift, orphans, validation).
- GitHub browsing and the installable directory/marketplace.

## Architecture

### Delivery modes

The sync engine currently knows `Symlink` and `Copy`
(`src-tauri/src/core/sync_engine.rs`). This slice adds:

- `Render`: transform a canonical source asset into an agent-specific file on
  sync (Codex `.toml`, Copilot `.agent.md`). Output is a generated file, not a
  link, so freshness is gated on a hash of the rendered bytes, not on symlink
  identity.
- `Place`: a file/dir drop with no transform, tracked but not activated, used
  for hooks, scripts, and rules in this slice. Mechanically a symlink on
  Claude/Pi (same as other file drops, so edits stay live); semantically
  "present but not registered" so a later activation slice can wire it
  (`settings.json` hook events, `AGENTS.md` rule imports) without re-placing.

Each `(agent, asset_type)` cell selects exactly one mode.

### Per-(agent x asset_type) matrix

Grounded in `agentic-tools/scripts/setup.py` (KNOWN_HOMES, plan/apply selective)
and `agent_assets.py` (render functions, MCP table). Empty cell = not supported
by that agent in this slice.

| asset_type | Claude `~/.claude` | Pi `~/.pi/agent` | Codex `~/.codex` | Copilot `~/.copilot` |
|---|---|---|---|---|
| skill   | `skills/` symlink  | `skills/` symlink  | `skills/` copy           | `skills/` copy                |
| agent   | `agents/<id>.md` symlink | `agents/<id>.md` symlink | `agents/<id>.toml` render | `agents/<id>.agent.md` render |
| command | `commands/` symlink | `commands/` symlink | `commands/` symlink      | (n/a)                         |
| hook    | `hooks/` place     | `hooks/` place     | (n/a)                    | (n/a)                         |
| script  | `scripts/` place   | `scripts/` place   | (n/a)                    | (n/a)                         |
| rule    | place (activate later) | place (activate later) | (n/a)            | (n/a)                         |

Notes:
- Agent rendering for Codex/Copilot is ported from `_render_codex_agent` and
  `_render_copilot_agent` in `agent_assets.py`. Claude/Pi agents are symlinked
  because the canonical source is already Claude-format `.md`.
- Copilot has no `commands` concept (confirmed: `agent_assets.py` copy plan has
  no copilot/commands entry).
- Per-asset symlink (a real target dir holding one link per active asset) is the
  Claude/Pi strategy from `setup.py:apply_selective`; matches the app's existing
  per-skill symlink behavior.

### Data model

Single generic model, not parallel tables per type (DRY; reuses every existing
subsystem). Keep the existing physical table names this slice and add an
`asset_type` discriminator column to each (additive migration only - no table
renames, which keeps the migration safe on existing installs). A renaming pass
(`skills` -> `assets`, etc.) for naming clarity is a deferred non-functional
cleanup. Tables in `src-tauri/src/core/skill_store.rs`:

- `skills`: add `asset_type TEXT NOT NULL DEFAULT 'skill'`. Existing rows
  backfill to `'skill'`. `central_path` now points under the type subdir
  (see layout).
- `skill_targets`: add `asset_type` for fast filtering, and extend the existing
  `mode` value set with `render` and `place`. Render targets store the
  rendered-output hash in the existing `source_hash` column for freshness.
- Preset membership / `scenario_*`: carry `asset_type` so a preset can hold
  mixed types. `apply_preset_*` applies across types.
- Discovery (`discovered_skills`): add `asset_type` so the scanner can classify
  what it finds.

The Rust/TS API and DTOs use asset-typed names (`ManagedAsset`, `asset_type`)
over these tables, so the misleading physical names are not exposed at the
boundary. Migration: a new versioned migration in
`src-tauri/src/core/migrations.rs` adds the columns with the `'skill'` default
and backfills, so existing installs keep working with zero user action.

### Central repo layout

The app keeps `~/.skills-manager/` (`central_repo.rs`) and adds per-type subdirs
alongside the existing `skills/`:

```
~/.skills-manager/
  skills/   agents/   commands/   hooks/   scripts/   rules/
  skills-manager.db        (rows carry asset_type)
  .sync-metadata/          (git backup, unchanged shape)
```

`central_repo.rs` gains a helper that maps an `asset_type` to its subdir. New
assets install under the matching subdir; `central_path` stores the full path.

### Tool adapters

`src-tauri/src/core/tool_adapters.rs` currently exposes one `skills_dir` per
agent. Extend each adapter to declare, per `asset_type`: whether it is
supported, the target subdir, and the delivery mode. Concretely, replace the
single field with a per-type capability map so a cell can be absent (n/a). The
four core agents get fully-populated maps per the matrix above; other adapters
default to "skill only" so nothing regresses.

### Render port

Add a render module (e.g. `src-tauri/src/core/asset_render.rs`) implementing:
- `render_codex_agent(canonical) -> String` (TOML), ported from
  `agent_assets.py:_render_codex_agent`.
- `render_copilot_agent(canonical) -> String` (`.agent.md`, `tools` as a YAML
  list), ported from `agent_assets.py:_render_copilot_agent`.
- Claude/Pi need no render (source is already the target format).

Canonical source for an agent is the Claude-style `.md` with frontmatter
(`name`, `description`, `tools`, `model`). The renderers parse that frontmatter
(reuse `skill_metadata.rs` frontmatter parsing) and emit the target format.

### Importer (read-only from agentic-tools)

A new command surface + UI panel that:
- Reads `agentic-tools/{agents,commands,hooks,scripts,rules,skills}/` and
  `agentic-tools/registry/active.json`.
- Lists candidates, defaulting the selection to the active set named in
  `active.json`, with a toggle to show all.
- On import, copies the selected asset into the matching
  `~/.skills-manager/<type>/` subdir and inserts an `assets` row with
  `source_type = 'import'` and `source_ref` = the agentic-tools path.
- Never writes to `agentic-tools/`. The path is configurable; it defaults to the
  known workspace path.

### Coexistence and backup

Existing runtime dirs (e.g. `~/.claude/agents` already populated by the user's
`setup.py`) must not be clobbered. Reuse the app's existing backup-before-link
behavior: when the app first takes over a target dir, back it up to
`<dir>.backup-<ts>` (the app already does this for skills). During the
transition the user stops running `setup.py` for app-managed types. Document
this in a short migration note. Nothing in `agentic-tools/` is modified.

### UI generalization

- Route `/my-skills` becomes `/library/:assetType` (default `skill`); keep a
  redirect from the old path.
- Add a type tab bar (Skills | Agents | Commands | Hooks | Scripts | Rules) in
  the library view.
- Reuse `SkillDetailPanel`, `AddSkillsSheet`, `AgentToggleSection`,
  `SyncDots`, and the multi-select toolbar; they become asset-type-aware by
  passing `assetType` through instead of assuming "skill."
- `src/lib/tauri.ts`: rename/extend the `ManagedSkill` type to `ManagedAsset`
  with `asset_type`, keep a `ManagedSkill` alias to limit churn; thread
  `asset_type` through the invoke wrappers.
- The detail panel shows the rendered output (read-only) for render-mode targets
  so the user can see the `.toml`/`.agent.md` that will be written.

## Affected files (anchors)

Backend:
- `src-tauri/src/core/skill_store.rs` - asset_type columns, type-aware queries.
- `src-tauri/src/core/migrations.rs` - new migration.
- `src-tauri/src/core/central_repo.rs` - per-type subdir helper.
- `src-tauri/src/core/tool_adapters.rs` - per-type capability map.
- `src-tauri/src/core/sync_engine.rs` - `Render` and `Place` modes.
- `src-tauri/src/core/asset_render.rs` - new; Codex/Copilot renderers.
- `src-tauri/src/core/scenario_service.rs` - apply across asset types.
- `src-tauri/src/commands/skills.rs` (+ new `assets.rs` or generalized) -
  asset_type params on get/install/delete; new import commands.
- `src-tauri/src/commands/scan.rs` - classify discovered asset_type.

Frontend:
- `src/lib/tauri.ts` - `ManagedAsset`, invoke wrappers.
- `src/App.tsx` - `/library/:assetType` route + redirect.
- `src/views/MySkills.tsx` -> generalized library view with type tabs.
- `src/context/AppContext` - asset lists keyed by type.
- Reused components threaded with `assetType`.

## Testing

- Rust unit: store round-trip per asset_type; migration backfill to `'skill'`.
- Rust unit: sync per mode - symlink (Claude agent), copy (Codex skill),
  render (Codex `.toml` and Copilot `.agent.md` against golden fixtures),
  place (hook/script/rule).
- Rust unit: renderers match golden output captured from `agent_assets.py` for
  a representative agent.
- Rust integration: import round-trip against a fixture tree mirroring
  `agentic-tools/{agents,...}` + a small `active.json`; assert only active set
  selected by default and files land under the right subdir.
- Rust integration: coexistence - pre-existing `agents/` dir is backed up to
  `.backup-<ts>` before the app links, original contents preserved.
- Frontend: library view renders each type tab; add/remove/sync invoke the
  asset_type-aware commands. Loading/empty/error/success states for the new
  tabs.

## Risks and mitigations

- Three mechanisms can fight over the same runtime dirs (the app, `setup.py`,
  `agent_assets.py`). Mitigation: backup-before-takeover; documented migration
  note to stop `setup.py` for app-managed types; agentic-tools untouched.
- Render fidelity drift from the Python source. Mitigation: golden fixtures
  captured from `agent_assets.py` output; renderer tests assert byte-equality.
- Schema migration on existing installs. Mitigation: additive columns with
  `'skill'` default + backfill; no destructive change.
- Scope creep into MCP/plugins/health. Mitigation: explicit out-of-scope list;
  each is its own slice.

## Follow-up slices (not now)

1. MCP servers (config-merge delivery into per-agent mcp config).
2. Plugins (cached repos, versions, marketplace).
3. Hook activation (`settings.json` event registration).
4. Rule activation (`AGENTS.md` `@import` block management).
5. Health/maintenance dashboard.
6. GitHub browsing + installable directory/marketplace.
