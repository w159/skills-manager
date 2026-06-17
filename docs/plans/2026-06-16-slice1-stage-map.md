# Slice 1 Stage Map - Multi-Asset Types

Living document. Spec: `docs/superpowers/specs/2026-06-16-multi-asset-types-slice1-design.md`.
Decisions: both waves this run; temp-dir sandbox only (real ~/.claude etc. untouched);
core agents Claude/Pi/Codex/Copilot; agent rendering in scope; hooks/rules placement-only.

Status legend: TODO / WIP / VERIFIED / REJECTED.

## Wave A - backend (evidence: cargo test + CLI red->green, tempdir homes)

| # | Stage | Failable check | Status |
|---|---|---|---|
| A0 | Baseline `cargo test` green; scaffold `docs/.run/` | suite green before changes | VERIFIED (265/265, clean rebuild ~46s) |
| A1 | `AssetType` enum + `asset_type` migration on `skills` + backfill `'skill'` | fresh + upgraded DB both have column; old rows = `'skill'`; tree compiles; existing tests green | VERIFIED (267 tests; commit 8e67488). Scope trimmed to skills table; targets/discovered/scenario asset_type folded into consumer stages. |
| A2 | `central_repo.rs` per-type subdir helper | unit: all 6 types -> correct subdir | VERIFIED (commit c927417) |
| A3 | `tool_adapters.rs` per-(agent x type) capability map | unit: matrix matches spec; non-core agents skill-only | VERIFIED (commit c927417) |
| A4 | `sync_engine.rs` `Render`+`Place`; new `asset_render.rs` (Codex `.toml`, Copilot `.agent.md`) | golden-fixture byte tests both renders; per-mode sync test | VERIFIED - Python cross-check byte-identical (commit c927417). NOTE: render write path (`write_rendered_file`) wired by A5; `CanonicalAgent` codex_* fields come from registry (A6 importer must capture). |
| A5a | capability-driven delivery engine (symlink/render/place per matrix) | tempdir per-mode + unsupported-cell absence | VERIFIED (commit 9370fc9) |
| A5b | asset command surface (get_managed_assets, deliver_managed_asset) + canonical_agent_from_file + e2e | e2e import->parse->render->disk; Codex .toml model_reasoning_effort='high' | VERIFIED (commit aac9bc6) |
| A6 | read-only importer from `agentic-tools/` (+ `registry/active.json`) | active-set flags, right subdir, source unchanged (read-only), codex fields survive | VERIFIED (commit 1c85db7) |
| A7 | coexistence backup before takeover | tempdir: foreign target -> `.backup-<ts>` w/ content; idempotent no-churn | VERIFIED (commit b556014). Note: same-second backup-name collision is a latent concurrency risk. |

Wave A backend COMPLETE: 327 tests green, every stage independently verified, end-to-end red->green proven.

## Wave B - frontend (evidence: build+lint green + runtime smoke)

| # | Stage | Failable check | Status |
|---|---|---|---|
| B1 | `tauri.ts` `ManagedAsset`+`asset_type` wrappers (+AdapterDeliveryResult fixed to Rust shape) | `tsc -b` green | VERIFIED (commit e365d76) |
| B2 | `/library/:assetType` route + redirect + 6-type tab bar; skill tab reuses MySkills | `build`+`lint` green | VERIFIED (commit e365d76) |
| B-actions | wire Sync (deliver) / Remove / Import-from-workspace controls; backend respects tool enablement | build+lint green; wrapper args + SelectedAsset shape match Rust | VERIFIED (commits 3bad200, 70fe48f) |
| B3 | live desktop runtime smoke | each tab renders; actions invoke typed commands; rendered preview | STATIC-VERIFIED only - live `npm run tauri:dev` is a manual step (Tauri desktop webview + real IPC not auto-runnable headlessly). Build/lint/arg-names/field-shapes all independently verified; backend paths covered by 330 Rust tests + e2e. |

Wave B COMPLETE (code): asset tabs list + sync + remove + import, all calling independently-verified commands. Live-runtime smoke is the user's manual `npm run tauri:dev` step.

## Known follow-ups (logged, not blockers)
- ManagedAssetDto is thin (no targets[]/tags); per-agent sync badges need it later.
- import_selected_assets reports batch-level errors, not per-item.
- Same-second backup-name collision is a latent concurrency risk in delivery.
- B3 live runtime not auto-executed (headless limitation).

## Red->green (law 3, new behavior)

Before: app cannot list/sync any agent asset. After: an imported agent syncs as
symlinked `.md` to Claude/Pi, rendered `.toml` to Codex, rendered `.agent.md` to
Copilot (positive); `command->Copilot` correctly absent (negative). All against
tempdir homes.

## Write-gates

- A1 migration: code only this run (runs against test/dev DBs; no real DB write without approval).
- Real agent-dir takeover: OUT this run (temp-dir sandbox decision).
- No dependency installs planned.
