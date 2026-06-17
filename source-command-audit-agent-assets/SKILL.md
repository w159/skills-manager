---
name: "source-command-audit-agent-assets"
description: "Review inactive imported agents, skills, plugins, hooks, and commands for consolidation opportunities."
---

# source-command-audit-agent-assets

Use this skill when the user asks to run the migrated source command `audit-agent-assets`.

## Command Template

Inspect `/Users/jerry/.agents/registry/active.json` first. Compare the active set against inactive source files under `agents/`, `skills/`, and `plugins/cache/`.

Prioritize:

1. Missing active capability that should be promoted.
2. Duplicate active capability that should be consolidated.
3. Vendor/cache material that should remain inactive rather than loaded by every agent runtime.

Finish by running:

```bash
python3 /Users/jerry/.agents/scripts/agent_assets.py --validate
```
