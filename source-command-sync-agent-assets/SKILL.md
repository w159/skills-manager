---
name: "source-command-sync-agent-assets"
description: "Build and install the curated .agents source of truth into Codex, Codex, and Copilot runtime directories."
---

# source-command-sync-agent-assets

Use this skill when the user asks to run the migrated source command `sync-agent-assets`.

## Command Template

Run:

```bash
python3 /Users/jerry/.agents/scripts/agent_assets.py --validate --install
```

This projects the active registry into:

- `~/.Codex/agents` and `~/.Codex/skills`
- `~/.codex/agents` and `~/.codex/skills`
- `~/.copilot/agents` and `~/.copilot/skills`

Do not hand-edit generated target files. Change `/Users/jerry/.agents/registry/active.json` or the source assets under `/Users/jerry/.agents`, then sync again.
