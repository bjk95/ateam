---
title: Profiles
description: Gate skills by machine — work, personal, devbox.
---

Profiles let you keep one shared lockfile while allowing different skills to
land on different machines.

Each machine declares its profile set at `init`:

```bash
ateam init --scaffold --profiles work,shared
ateam init <git-url> --profiles personal
```

The list lives in `<repo>/.ateam/machine.toml` (gitignored). Each machine writes
its own.

## Tagging a skill with profile gates

```bash
ateam add canva/agent-skills --skill internal --profile work -y
```

The lockfile entry gets `profiles = ["work"]`. `ateam apply` only installs the
entry on machines whose profile set intersects `["work"]`. A `personal`-only
machine skips it.

## When to skip profiles entirely

Profiles are optional. A lockfile entry without a `profiles` field applies to
all machines. If your work and personal machines should look identical, don't
tag anything — every install lands everywhere.
