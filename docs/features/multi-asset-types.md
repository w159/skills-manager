# Multi-Asset-Type Management

Slice 1 of the generalization of skills-manager from a skills-only tool to a
six-type asset library. Landed on branch feat/multi-asset-types-slice1.
Spec: docs/superpowers/specs/2026-06-16-multi-asset-types-slice1-design.md.
Stage map: docs/plans/2026-06-16-slice1-stage-map.md.

---

## Asset types

| Type    | Description                                        |
|---------|----------------------------------------------------|
| skill   | Existing type. Markdown-formatted tool skill files.|
| agent   | Agent persona definitions (.md source).            |
| command | Slash-command definition files.                    |
| hook    | Event hook scripts.                                |
| script  | Utility/automation scripts.                        |
| rule    | Coding-standard rule files.                        |

The discriminator column `asset_type TEXT NOT NULL DEFAULT 'skill'` was added to
the `skills` table via an additive migration with a backfill to `'skill'` for all
pre-existing rows. No table was renamed or dropped. Source:
src-tauri/src/core/skill_store.rs (commit 8e67488).

---

## Delivery-mode matrix

Each cell is the mode used when delivering that asset type to that agent. Empty
cells mean the agent does not support that type in this slice.

| asset_type | Claude (~/.claude)          | Pi (~/.pi/agent)            | Codex (~/.codex)              | Copilot (~/.copilot)            |
|------------|-----------------------------|-----------------------------|-------------------------------|---------------------------------|
| skill      | skills/ symlink             | skills/ symlink             | skills/ copy                  | skills/ copy                    |
| agent      | agents/<id>.md symlink      | agents/<id>.md symlink      | agents/<id>.toml render       | agents/<id>.agent.md render     |
| command    | commands/ symlink           | commands/ symlink           | commands/ symlink             | (n/a)                           |
| hook       | hooks/ place                | hooks/ place                | (n/a)                         | (n/a)                           |
| script     | scripts/ place              | scripts/ place              | (n/a)                         | (n/a)                           |
| rule       | place (activate later)      | place (activate later)      | (n/a)                         | (n/a)                           |

Three delivery modes:
- symlink: a per-asset symlink inside the agent's target directory.
- render: the asset is transformed into an agent-specific format and written as a
  regular file.
- place: the file is copied to the target directory and tracked but no
  further activation step is performed in this slice (rule @import and hook
  settings.json registration are deferred).

Source: src-tauri/src/core/tool_adapters.rs (capability matrix, commit c927417),
src-tauri/src/core/deliver_asset.rs (dispatch logic, commit 9370fc9).

---

## Render formats

Agent assets are rendered (not symlinked) for Codex and Copilot because those
agents have their own schema:

- Codex: renders to a .toml file at agents/<id>.toml. Fields include name,
  description, model_reasoning_effort, and tool lists. Output is byte-identical
  to the output of agentic-tools/scripts/agent_assets.py (_render_codex_agent).
- Copilot: renders to an .agent.md file at agents/<id>.agent.md. Output is
  byte-identical to agent_assets.py (_render_copilot_agent).

Fidelity is enforced by golden-fixture tests that capture agent_assets.py output
and assert byte equality against the Rust renderer.
Source: src-tauri/src/core/asset_render.rs (commit c927417).

---

## Central-repo layout

The app's local store at ~/.skills-manager/ now contains one subdirectory per
type:

```
~/.skills-manager/
  skills/
  agents/
  commands/
  hooks/
  scripts/
  rules/
  skills-manager.db     (asset_type column added)
  .sync-metadata/
```

The helper that maps an AssetType to its subdir lives in
src-tauri/src/core/central_repo.rs (commit c927417).

---

## Importer (read-only)

The importer reads an agentic-tools workspace and seeds the app's central repo.
It never writes to the source workspace.

Behavior:
1. Reads agentic-tools/{agents,commands,hooks,scripts,rules,skills}/ and
   agentic-tools/registry/active.json.
2. Lists candidates; defaults the selection to the active set named in
   active.json.
3. On import, copies selected assets into ~/.skills-manager/<type>/ and inserts
   a row with source_type = 'import' and source_ref = the agentic-tools path.
4. Codex-specific fields (model_reasoning_effort, etc.) are read from
   registry/active.json codex_* fields and merged into the imported agent's
   frontmatter.

Current limitation: import_selected_assets reports errors at batch level (a
list of strings), not per-item. Individual asset failures are not distinguished
in the response. Source: src-tauri/src/core/importer.rs,
src-tauri/src/commands/importer.rs (commit 1c85db7).

---

## Coexistence and backup

When the app first takes over a target directory that already exists (e.g.
~/.claude/agents populated by the user's setup.py), it renames the directory to
<dir>.backup-<timestamp> before writing. The original content is preserved.

Known limitation: if two delivery operations target the same directory within
the same second, both generate the same backup name, creating a collision risk.
This is a latent concurrency issue, not triggered in normal single-user use.
Source: src-tauri/src/core/deliver_asset.rs (commit b556014).

---

## Command surface

Two new Tauri commands exposed to the frontend:
- get_managed_assets(asset_type) -> Vec<ManagedAsset>
- deliver_managed_asset(asset_id, tool) -> DeliveryResult

deliver_managed_asset checks tool enablement and returns a typed error if the
target file is missing rather than silently succeeding. canonical_agent_from_file
parses agent frontmatter from the central-repo path.
Source: src-tauri/src/commands/assets.rs (commits aac9bc6, 3bad200).

ManagedAssetDto is thin in this slice: no targets[] or tags fields. Per-agent
sync status badges are a follow-up.

---

## Frontend

Route: /library/:assetType (six valid values). Unknown param values fall back to
"skill". The skill tab reuses the existing MySkills view. All other type tabs
render AssetTabPanel, which lists assets and exposes Sync, Remove, and Import
action controls.

TypeScript types ManagedAsset and AssetType are defined in src/tauri.ts and
mirror ManagedAssetDto from the Rust command surface. Typed command wrappers call
the backend with the correct argument shapes.
Source: src/views/AssetLibrary.tsx (commits e365d76, 70fe48f),
src/tauri.ts (commit e365d76).

Frontend verification: build and lint green; tsc -b green; argument names and
field shapes independently verified against Rust types. Live desktop runtime
smoke (npm run tauri:dev) was not executed automatically due to headless
environment constraints -- it is the user's manual verification step.

---

## Core agents

| Agent   | Home directory |
|---------|----------------|
| Claude  | ~/.claude      |
| Pi      | ~/.pi/agent    |
| Codex   | ~/.codex       |
| Copilot | ~/.copilot     |

Non-core agents registered in the app default to skill-only capability.

---

## Deferred (explicit out-of-scope for this slice)

- MCP servers: config-merge delivery into per-agent MCP config.
- Plugins: cached repos, versions, marketplace.
- Hook activation: writing event registrations into each agent's settings.json.
- Rule activation: managing the @import block in AGENTS.md/CLAUDE.md.
- Health/maintenance dashboard: broken links, drift, orphans, validation.
- GitHub browsing and installable directory/marketplace.

Each item is its own future slice.

---

## Test coverage

330 Rust tests green at merge. Stages verified independently:
- Unit: AssetType round-trip, migration backfill.
- Unit: per-mode sync (symlink, copy, render, place) with tempdir homes.
- Unit: Codex .toml and Copilot .agent.md renderers against golden fixtures.
- Integration: importer round-trip against a fixture agentic-tools tree.
- Integration: coexistence backup with pre-existing dirs.
- End-to-end: import -> parse -> render -> disk path (commit aac9bc6).
