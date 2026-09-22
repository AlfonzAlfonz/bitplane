---
title: Set up per-project scripts
sidebar_label: Per-project scripts
---

# Set up per-project scripts

:::not-implemented

Scripts do not run yet, and neither [`bp run`](../reference/plane/run.md) nor
[`bp project show`](../reference/project/show.md) exists. `--no-scripts` is not
accepted anywhere. This guide is what is being built.

:::

A fresh worktree is a checkout and nothing else. No `node_modules`, no symlink
to your local tooling, no `.env`. Scripts are how a project makes its worktrees
usable on arrival, and how it cleans up on the way out.

## Write one

Scripts are declared in the project's `project.toml`, which lives in the
project's directory — not in the repository:

```sh
$EDITOR ~/.local/share/bitplane/projects/api/project.toml
```

```toml
version = 1
name = "api"

[source]
type = "owned"
url = "git@gitlab.com:acme/api.git"

[scripts.install]
shell = "pnpm i && pnpm build"
post_worktree_create = true
```

That is it. The next `bp create @api …` runs `pnpm i && pnpm build` in the new
worktree.

**The file is yours.** bitplane rewrites `name` and `[source]` when you rename a
project and never touches `[scripts]`, so nothing you write here moves. And
because the file lives in your own data directory rather than in the repository,
running what it says needs no more trust than a shell alias does — which is why
declaring scripts inside the repository is out of scope rather than merely
unbuilt.

## Order is declaration order, and it matters

Scripts run **sequentially**, in the order the tables appear in the file:

```toml
[scripts.link-tooling]
argv = ["ln", "-s", "../../.tooling", ".tooling"]
post_worktree_create = true

[scripts.install]
shell = "pnpm i && pnpm build"
post_worktree_create = true
```

`link-tooling` is first, so the symlink is there before the build looks for it.
Moving the table moves the step. bitplane never reorders them.

## `argv` or `shell`

```toml
[scripts.link-tooling]
argv = ["ln", "-s", "../../.tooling", ".tooling"]   # no shell, no quoting rules

[scripts.install]
shell = "pnpm i && pnpm build"                          # a shell, because you want one
```

Use `argv` when you are running one command — there are no quoting rules to get
wrong. Use `shell` when you want `&&`, a pipe, or a glob.

:::tip[Single-quote any `shell` value with a `$VAR` in it]

```toml
[scripts.show-branch]
shell = 'echo "on $BITPLANE_BRANCH"'
```

A TOML double-quoted string does its own escaping and will fight the shell's. A
literal (single-quoted) string does not. It is the first thing everyone gets
wrong.

:::

## Tear down as well as set up

The counterpart point fires before a worktree is removed:

```toml
[scripts.stop-stack]
shell = "docker compose down --remove-orphans"
pre_worktree_remove = true
```

Now `bp destroy` stops the containers before it removes the directory they were
running out of.

**A pre-remove script blocks.** Exit non-zero and the pass aborts and **nothing
is removed** — that is what makes it a usable veto:

```
error[script_blocked]: stop-stack exited 1 in @api; nothing was removed

  @api  stop-stack exited 1

remedy: Fix the script, or re-run with --no-scripts.
```

A `post_worktree_create` script is the opposite: it unwinds nothing. By the time
it runs the plane exists and is usable, so a failed `pnpm i` is reported and the
plane stays. See [pre blocks, post does
not](../concepts/script.md#pre-blocks-post-does-not).

`--no-scripts` works on every command that runs them, so nothing you write here
can make a plane undestroyable.

## Scripts you only ever run by hand

Leave off both toggles:

```toml
[scripts.reset-db]
shell = "pnpm db:reset && seed-fixtures"
```

Nothing fires it. You run it when you want it:

```sh
bp run @api reset-db
```

This is often the right shape. Resetting a database is not something you want
happening automatically every time you make a worktree.

## Run one by hand

```sh
bp run @api install                  # one
bp run @api link-tooling install     # several, in the order you named them
```

```
@api  install  ok  4.2s
```

Output streams live to your terminal as it runs, and is tee'd to
`<plane-dir>/.bitplane/logs/`. Every failure message names the log file.

`bp run` exists because re-running a failed script by hand means reconstructing
eight environment variables, a `PATH` entry and the right working directory,
which nobody gets right — and getting it wrong silently is worse than not
retrying. It takes **script names, never a point name**; a failed `create`
already told you which script to name.

It is **not idempotent**, and does not pretend to be. Re-running `ln -s` fails
the second time.

## What a script can rely on

The working directory is the worktree. `<project-dir>/bin` is on `PATH` ahead of
everything. Your whole environment is inherited, untouched — including
`SSH_AUTH_SOCK` and every credential helper. Plus eight variables:

| variable | useful for |
| --- | --- |
| `BITPLANE_WORKTREE` | the worktree path — also the working directory |
| `BITPLANE_PLANE_DIR` | reaching a sibling member's worktree |
| `BITPLANE_PROJECT_DIR` | copying a file **you** keep next to the project, like an `.env` |
| `BITPLANE_BRANCH` | the live `HEAD` — **unset** when detached, so `git switch "$BITPLANE_BRANCH"` fails loudly instead of detaching again |

The full list of eight is on
[`bp run`](../reference/plane/run.md#environment).

**Persist none of them.** A plane id is mutable by design and the plane
directory moves with it, so a value written into a file today can be wrong
tomorrow.

### Copying an `.env` into a new worktree

There is no built-in for this, deliberately: silently copying a gitignored
`.env` into a new directory is bitplane acting unbidden, and `.env` is exactly
where a surprise copy is a security event. Write it yourself, from a file you
keep in the project directory:

```toml
[scripts.seed-env]
shell = 'cp "$BITPLANE_PROJECT_DIR/env.template" .env'
post_worktree_create = true
```

`BITPLANE_PROJECT_DIR` works for owned and adopted projects alike, which the
source repository would not — an owned one is bare and has no working tree to
copy from.

## Ship an executable with the project

`<project-dir>/bin` is prepended to `PATH` for this project's scripts only:

```sh
mkdir -p ~/.local/share/bitplane/projects/api/bin
cat > ~/.local/share/bitplane/projects/api/bin/seed-fixtures <<'EOF'
#!/bin/sh
exec pnpm exec tsx scripts/seed.ts "$@"
EOF
chmod +x ~/.local/share/bitplane/projects/api/bin/seed-fixtures
```

Now `seed-fixtures` is a command any of `@api`'s scripts can call. bitplane
never writes into `bin/` — it is yours, which is what keeps the trust story
above true.

An executable in `bin/` and a script in `project.toml` are different things: one
is a file on `PATH`, the other a declaration you run by name.

## Check what you wrote

```sh
bp project show @api
```

```
@api  owned
  source     git@gitlab.com:acme/api.git
  ...

scripts
  link-tooling  ln -s ../../.tooling .tooling                post_worktree_create
  install       pnpm i && pnpm build                         post_worktree_create
  stop-stack    docker compose down --remove-orphans         pre_worktree_remove
  reset-db      pnpm db:reset && seed-fixtures

post_worktree_create   link-tooling, install
pre_worktree_remove    stop-stack
```

The last two lines are the point of the command: *what runs at each point, in
what order*, which no single place in the file answers. It is also where a typo
surfaces:

```
error[parse_error]: ~/.local/share/bitplane/projects/api/project.toml: unknown key "post_worktree_created" in [scripts.install]

remedy: Legal keys are argv, shell, post_worktree_create and pre_worktree_remove.
```

An unknown key is an error rather than a shrug, precisely so that a misspelled
toggle fails here instead of silently never running and leaving you with a
worktree that is subtly wrong.

## One script per project, not per plane

Scripts belong to projects, so in a six-repo plane each member runs its own —
and the whole pass is sequential, member by member and script by script. Six
`pnpm i` run one after another. That is a real latency cost, accepted because
the escape only works one way: a script that wants concurrency can background
its own work, while a script that needs serialisation cannot invent it.

## Next

- [Scripts and script points](../concepts/script.md) — the concept in full
- [`bp run`](../reference/plane/run.md) — every flag, and the eight variables
- [`bp project show`](../reference/project/show.md) — the resolved per-point
  sequence
