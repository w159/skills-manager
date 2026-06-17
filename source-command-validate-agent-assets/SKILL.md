---
name: "source-command-validate-agent-assets"
description: "Validate the shared .agents registry, active skills, active plugins, generated agent compatibility, and hook configuration."
---

# source-command-validate-agent-assets

Use this skill when the user asks to run the migrated source command `validate-agent-assets`.

## Command Template

Run:

```bash
python3 /Users/jerry/.agents/scripts/agent_assets.py --validate
python3 -m unittest discover -s /Users/jerry/.agents/tests
```

Treat `error` entries as blockers. Treat `warning` entries as inventory notes unless the user asks to reduce the inactive source pool further.
