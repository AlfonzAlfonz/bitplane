---
title: Scripts and script points
sidebar_label: Scripts
---

# Scripts and script points

A fresh worktree is rarely usable on arrival. Dependencies are not installed,
a symlink to your local tooling is missing, a `.env` is not there. A **script**
is a named command a [project](./project.md) declares so that bitplane can do
that for you — and, just as importantly, so you can run it again by hand when it
fails.

```toml
# ~/.local/share/bitplane/projects/api/project.toml

[scripts.install]
shell = "pnpm i && pnpm build"
post_worktree_create = true
```

```sh
bp run @api install     # on demand, any time
```

It is `package.json`'s word for the same idea on purpose: `bp run @api install`
is meant to read like `npm run install`.

## Two points, and only two

A script may be bound to a moment in a worktree's life. There are exactly two:

| point | fires | on |
| --- | --- | --- |
| `post_worktree_create` | once per member, after its worktree is made | `bp create`, `bp add` |
| `pre_worktree_remove` | once per member, before its worktree is taken away | `bp destroy`, `bp rm` |

They are named for the **worktree**, not for the plane, because scripts belong
to projects: in a plane of six repositories, each one runs its own.

A script may be bound to one point, to both, or to **neither**. A script bound
to nothing is legal and useful — `reset-db` is not something you want fired
automatically, but it is something you want to be able to type.

There is no rename point, no post-fetch point and no project-lifecycle point.
The last two would run before any worktree exists, which means half the
environment below would be undefined — a different contract wearing the same
name.

## Pre blocks. Post does not.

This is the rule to hold on to, and it is one sentence: **pre blocks and
short-circuits; post does neither.**

**A `pre_worktree_remove` script that exits non-zero aborts the whole pass, and
nothing is removed.**

```
error[script_blocked]: stop-stack exited 1 in @api; nothing was removed

  @api  stop-stack exited 1

remedy: Fix the script, or re-run with --no-scripts.
```

That is what makes a pre-script a usable veto: `docker compose down` failing
should stop you throwing the plane away.

**A `post_worktree_create` script that fails unwinds nothing.** By the time it
runs, the worktrees exist and the plane is real; the failure is reported, the
command exits `1`, and the plane stays. Exiting `0` would hide a real failure,
but tearing down a finished plane over a failed `pnpm i` would be worse.

```
error[script_failed]: install exited 1 in @api; the worktree was created

  @api  install exited 1; see ~/planes/bp-a3f9c2e1/.bitplane/logs/20260921T140312Z-api-install.log

remedy: See the log, fix the cause, then run bp run @api install.
```

### A project can never make a plane undestroyable

`--no-scripts` is available on every command that runs them, so a blocking
pre-script is always something you can step over. That is the entire answer to
the obvious worry, and it needed no special mechanism.

A blocking script is also **not** a refusal, and no `--waive` covers it. A
refusal is bitplane saying *you are about to lose work I can see*; a script's
exit code is a program's opinion. Keeping them apart is what stops the waiver
list from growing a `--waive script_failed` that would mean nothing.

## How they run

**Sequentially. Always.** Across the members of a plane in plane-file order, and
within a project in declaration order. Two `pnpm i` racing on one shared store
is the obvious first bug, with two `docker compose up` on one port right behind
it.

The cost is real — a six-repo plane running `pnpm i` runs six installs one after
another — and it is accepted because the escape works in only one direction: a
script that wants concurrency can background its own work, while a script that
needs serialisation cannot invent it.

Order is **declaration order in `project.toml`**, and that order is semantic:

```toml
[scripts.link-tooling]
argv = ["ln", "-s", "../../.tooling", ".tooling"]
post_worktree_create = true

[scripts.install]
shell = "pnpm i && pnpm build"
post_worktree_create = true
```

`link-tooling` is declared first, so it runs first, which is what lets the build
read `.tooling`. bitplane never rewrites that table, so the order cannot change
behind your back.

The accepted cost of declaring bindings as toggles on each script is that *"what
runs at `post_worktree_create`, in what order"* is not answerable from any one
place in the file — you would read every table in order.
[`bp project show`](../reference/project/show.md) prints the resolved sequence,
which turns a file-reading problem into a command.

## What a script gets

- **The working directory is the worktree.**
- **`<project-dir>/bin` is prepended to `PATH`**, so a project can ship its own
  executables and have them win.
- **Your environment is inherited in full.** A sanitised one would break
  `SSH_AUTH_SOCK`, `ssh-agent` and every credential helper — bitplane passes
  your environment through untouched for the same reason it never handles a
  credential itself.
- **Eight `BITPLANE_*` variables** describing the script, the project, the
  worktree, the branch and the plane. They are listed on
  [`bp run`](../reference/plane/run.md#environment).
- **No timeout.** A real `pnpm i && pnpm build` outlives any default worth
  shipping, and a build killed at ten minutes is a worse failure than a hang you
  can see.

What makes "no timeout" safe is the visibility half: script output is
**streamed live** as it runs — merged, on bitplane's stderr — and tee'd to a log
under `<plane-dir>/.bitplane/logs/`. A hang looks like a hang rather than a
freeze. Live output is on stderr rather than stdout because stdout carries
results, and a `pnpm` banner in the middle of a JSON response is a broken
contract.

**None of the `BITPLANE_*` values is stable**, and a script must persist none of
them. A plane id is mutable by design and the directory moves with it.

## Re-running one by hand

```sh
bp run @api install
bp run @api link-tooling install     # in the order you named them
```

`bp run` exists because reconstructing eight environment variables, a `PATH`
entry and the right working directory by hand is not something anyone gets
right, and getting it wrong silently is worse than not retrying at all.

It takes **script names only, never a point name**. Points are bindings, not
things you can invoke, so the two never share a namespace. Recovering from a
failed `create` means naming the scripts — which the failure already told you.

`bp run` is **not idempotent and does not pretend to be.** Re-running `ln -s`
fails the second time. That failure is yours to own, exactly as the first one
was.

## `argv` or `shell`

Each script is one or the other:

```toml
[scripts.link-tooling]
argv = ["ln", "-s", "../../.tooling", ".tooling"]

[scripts.install]
shell = "pnpm i && pnpm build"
```

`argv` runs the command directly, with no quoting rules to get wrong. `shell`
exists because the realistic examples want a shell, and pretending otherwise
just makes everyone write `["sh", "-c", …]`.

:::tip[Single-quote any `shell` value containing `$VAR`]

TOML's own escaping in a double-quoted string will fight the shell's. Use a
literal string — `shell = 'echo $BITPLANE_BRANCH'` — and there is nothing to
fight about. It is the first thing people get wrong.

:::

A misspelled key is an error, not a shrug:

```
error[parse_error]: ~/.local/share/bitplane/projects/api/project.toml: unknown key "post_worktree_created" in [scripts.install]

remedy: Legal keys are argv, shell, post_worktree_create and pre_worktree_remove.
```

Silence would cost you a script that never runs with no trace of why — your
setup did not happen, your worktree is subtly wrong, and nothing in `bp create`'s
output points at the cause. It fails at
[`bp project show`](../reference/project/show.md) and
[`bp doctor`](../reference/plane/doctor.md) too, not only when a plane is built.

## Scripts and `bin/` are different things

A **script** is a declared entry in `project.toml`, run by name. An executable in
`<project-dir>/bin/` is a file on `PATH` while a script runs. One line separates
them; they are not two spellings of the same idea.

## Ad-hoc members have none

An [ad-hoc member](./member.md#ad-hoc-members) has no `project.toml`, so it has
no scripts. It behaves exactly like a project with none declared — nothing runs,
nothing is reported, no error. `bp run` against one is an error, because
`bp run` takes a project.

## Next

- [Set up per-project scripts](../guides/per-project-scripts.md) — the guide
- [`bp run`](../reference/plane/run.md) — the full contract, including the
  environment
- [`bp project show`](../reference/project/show.md) — what a project declares,
  and what runs at each point
