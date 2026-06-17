# Lessons: Multi-Asset-Type Slice 1 (2026-06-16)

From the implementation of feat/multi-asset-types-slice1.

---

## Render fidelity via Python cross-check

The Rust renderers for Codex (.toml) and Copilot (.agent.md) were validated by
capturing golden-fixture output from the existing Python implementation
(agentic-tools/scripts/agent_assets.py, functions _render_codex_agent and
_render_copilot_agent) and asserting byte equality in Rust tests. This approach
caught any encoding or whitespace divergence before integration and made the
parity claim verifiable rather than assumed. When porting a renderer from one
language to another, capturing byte-identical fixtures from the authoritative
source and running them as regression tests is cheaper than discovering drift at
runtime.

Source: src-tauri/src/core/asset_render.rs; stage A4 in
docs/plans/2026-06-16-slice1-stage-map.md.

---

## Registry fields via frontmatter

Codex-specific agent fields (model_reasoning_effort and similar) live in
registry/active.json as codex_* keys, not in the agent's .md source file. The
importer merges these fields into the imported agent's in-memory frontmatter
during the import step, so downstream renderers see a single unified struct and
do not need to know where each field came from. This avoids coupling the render
path to the registry format and keeps the .md source file unchanged.

Source: src-tauri/src/core/importer.rs (commit 1c85db7).

---

## Same-second backup-name collision risk

The coexistence backup renames an existing target directory to
<dir>.backup-<timestamp> using a second-resolution timestamp. If two delivery
operations target the same directory within the same second (unlikely in normal
single-user use but possible under automation or fast scripting), they generate
the same backup name and the second operation overwrites or errors against the
first backup. A sub-second or random suffix would eliminate this. Logged as a
known follow-up; not a blocker for single-user desktop use.

Source: src-tauri/src/core/deliver_asset.rs (commit b556014); also noted in
docs/plans/2026-06-16-slice1-stage-map.md under "Known follow-ups."

---

## Independent verification is not optional

During this slice, a subagent once reported a frontend stage complete before the
build had been run. The error was caught by running the build independently and
observing a type mismatch in the command argument shape. The principle this
enforces: a subagent's "done" is a claim, not a fact. Every failable check in
the stage map must be run and its output observed before the stage is marked
verified. Accepting a verbal "it should work" in place of command output leads to
integration failures that are expensive to unwind. The stage map format (one
failable check per stage, status recorded with evidence) exists precisely to
prevent this.

Applies to all agent-driven work, not just this slice.
