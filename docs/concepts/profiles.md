---
title: Profiles
description: Gate synced entries by machine — work, personal, devbox.
---

Profiles let you keep one shared lockfile while allowing different skills,
subagents, and MCP servers to land on different machines.

Each machine declares its profile set at `init`:

```bash
agents init --scaffold --profiles work,shared
agents init <git-url> --profiles personal
```

The list lives in `<repo>/.agents/machine.toml` (gitignored). Each machine writes
its own.

## Tagging a skill with profile gates

```bash
agents skills add canva/agent-skills --skill internal --profile work -y
```

The lockfile entry gets `profiles = ["work"]`. `agents apply` only installs the
entry on machines whose profile set intersects `["work"]`. A `personal`-only
machine skips it.

The same flag works for subagents and MCP servers:

```bash
agents subagents add ./agents/reviewer.md --profile work -y
agents mcp add otter --profile work -- otter mcp serve
```

## When to skip profiles entirely

Profiles are optional. A lockfile entry without a `profiles` field applies to
all machines. If your work and personal machines should look identical, don't
tag anything — every install lands everywhere.
