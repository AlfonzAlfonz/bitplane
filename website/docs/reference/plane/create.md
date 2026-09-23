---
title: bp create
---

# `bp create`

:::implemented

Everything on this page works as described.

:::

Create a plane and the worktrees of every member named on the command line.

```
bp create <member>... [-b <branch>] [--id <id>] [--new-branch | --existing-branch]
          [--no-fetch] [--no-scripts]
```

`create` is **strict, not idempotent**: it either makes a whole plane or it
makes none. There is no "finish the half-built one" — the abort window holds
only cheap, discardable work, so throwing it away and running the command again
is the repair.

## Arguments

| argument | required | what it is |
| --- | --- | --- |
| `<member>...` | yes, one or more | A [member](../member-syntax.md): `@name` for a registered project, a path for an ad-hoc member. May carry a `:branch` suffix. |

Every member needs a branch, from its own suffix or from `-b`. A member with
neither is [`branch_unspecified`](../refusals-and-waivers.md#usage-failures).

Defaulting to each member's own default branch was considered and rejected: for
an **adopted** project or an **ad-hoc member** that branch is the one the user's
checkout is sitting on, so the convenience would resolve straight into
[`branch_occupied`](../refusals-and-waivers.md#branch_occupied) for exactly the
members it was meant to make easy. The default branch is the **base** a plane
branch is cut from, not a branch to land on.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `-b`, `--branch <branch>` | — | The branch for every member that does not carry its own. Applied at this moment and **not remembered**: a plane has no branch. |
| `--id <id>` | a generated `bp-<hex8>` | The plane id, which is also its directory name. |
| `--new-branch` | — | The branch must not already exist. Fails if it does. |
| `--existing-branch` | — | The branch must already exist. Fails if it does not. |
| `--no-fetch` | fetch first | Skip the fetch of every owned project. |
| `--no-scripts` | run them | Do not run `post_worktree_create` scripts. |

Plus the [global flags](../global-flags.md#global-flags). `create` takes no
`--plane`: it is making one.

### Branch intent

With neither `--new-branch` nor `--existing-branch`, `bp` **resolves**: it
checks the branch out if it exists after the fetch and creates it otherwise, and
the output says which happened. That is the right default and it is why the
fetch matters.

**Exists means resolves**, from either namespace. For an owned project
`refs/heads/*` holds only the branches plane members were created on, so a
colleague's branch is never there — it is at `refs/remotes/origin/<branch>`, and
a worktree cut from it tracks it. That is *joining*, so the row is not marked as
a new branch.

`--no-fetch` with the default intent is
[refused at validation](../refusals-and-waivers.md#branch_intent_requires_fetch),
because resolving against a stale source repo silently creates a new, unrelated
branch. Pass `--new-branch` to say you meant a new one.

### The id

A generated id is random hex with a `bp-` prefix, claimed by an atomic `mkdir`
and retried on collision, five times at most. The `bp-` prefix is **reserved**
for generated ids, so an id starting with it is
[`reserved_plane_id`](../refusals-and-waivers.md#usage-failures) — "starts with `bp-`"
stays a reliable signal that nobody chose this name.

A chosen id is lowercase `[a-z0-9][a-z0-9._-]*`, 64 characters at most.
Lowercase only, so two ids can never collide on a case-insensitive filesystem.

Ids are **not safe to cache**. A plane can be renamed, no alias to a former id
is kept, and a stale reference is simply an error.

## Output

The plane id and directory, then [one row per member](../global-flags.md#how-a-fan-out-prints)
in the order you named them.

```
$ bp create @api @web -b feat-login
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api  feat-login  created  acme/api
  @web  feat-login  created  acme/web
```

A branch that had to be created is marked, because it is the difference between
joining a colleague's work and starting your own:

```
$ bp create @api ~/projects/bitplane -b feat-login
```
```
bp-7c1e0d44  ~/planes/bp-7c1e0d44

  @api                             feat-login  created               acme/api
  /Users/alfonz/projects/bitplane  feat-login  created (new branch)  projects/bitplane
```

## Examples

### The id is already taken

Exit `2`. The path is stat'd and classified, so the remedy names what is
actually there.

```
$ bp create @api -b feat-login --id auth-work
```
```
error[plane_id_in_use]: auth-work exists but was never completed

remedy: Nothing in it is yours; run bp destroy -p auth-work to clear it.
```

### git is too old

Exit `4`. Checked once, before anything is touched, so no plane is left
half-built.

```
$ bp create @api -b feat-login
```
```
error[git_too_old]: git 2.35.9 is too old; bitplane requires git 2.36 or newer

remedy: Upgrade git to 2.36 or newer.
```

### A member fails partway through

Exit `1`. Scheduling stops, every worktree this run created is unwound, the
plane directory is removed, and the rows ride inside the error.

```
$ bp create @api @web @docs -b feat-login
```
```
error[create_aborted]: create did not finish; bp-a3f9c2e1 was removed

  @web  failed: git worktree add exited 128: fatal: invalid reference: feat-login

remedy: Nothing was left behind. Fix what the rows report, then run bp create again.
```

A member that had **not been picked up yet** when the failure landed rides in
the same error as `skipped: aborted after an earlier failure`. Up to four
worktrees are added at once, so a plane this small has every member in flight
before the first one can fail and there is nothing left to skip.

A forge that will not answer never gets this far: the fetch runs **before** the
claim, so it fails as [`fetch_failed`](../refusals-and-waivers.md#typed-failures)
with no plane directory to clear up.

`@api` succeeded and is not in `problems`; it was unwound with the rest. The
unwind passes git's force flag unconditionally, which is safe **only** here: by
construction nothing in the abort window is yours yet. That force is scoped to
this path and is never reused by [`bp destroy`](./destroy.md).

### Ctrl-C

Exit `130`. The first interrupt stops scheduling, waits for the children already
running, unwinds, and prints what it has. A second exits immediately and leaves
whatever was in flight where it fell.

```
$ bp create @api @web @docs -b feat-login
^C
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api   feat-login  removed  acme/api
  @web   -           skipped: interrupted  acme/web
  @docs  -           skipped: interrupted  acme/docs
```

### A post-create script fails

Exit `1`, and the plane is **kept**. Scripts run after the point of no return,
so a failure there unwinds nothing — but exiting `0` would hide it.

```
error[script_failed]: install exited 1 in @api; the worktree was created

  @api  install exited 1; see ~/planes/bp-a3f9c2e1/.bitplane/logs/20260921T140312Z-api-install.log

remedy: See the log, fix the cause, then run bp run @api install.
```

## Exit codes

| code | when |
| --- | --- |
| `0` | the plane was built |
| `1` | the run aborted, or a `post_worktree_create` script failed |
| `2` | bad arguments, a missing branch, a taken or reserved id, or a branch intent that cannot be honoured |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted |

## What it writes

In this order, and the order is the design:

1. `mkdir <plane-dir>` — the claim. Atomic; `EEXIST` is fatal.
2. `.bitplane/incomplete` — the latch, written before anything else.
3. `plane.toml` — the membership.
4. one `git worktree add` per member — **the abort window**.
5. the latch is unlinked — the point of no return.
6. `post_worktree_create` scripts.

Everything expensive or side-effect-bearing is outside the window. The fetch
writes the source repo, not the plane, so a retry finds it warm; scripts run
after the latch clears. That is what makes discarding cheap.

The latch says one thing nothing else can: **this plane never worked**, as
opposed to having worked and since lost a worktree. It is diagnostic only —
nothing parses its contents — and only `create` ever writes it.

## See also

- [Member syntax](../member-syntax.md) · [Global flags](../global-flags.md)
- [`bp add`](./add.md) — put another member into a plane that already exists
- [`bp destroy`](./destroy.md) — take one apart
