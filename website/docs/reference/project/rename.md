---
title: bp project rename
---

# `bp project rename`

:::not-implemented

`bp project rename` does not exist yet. This page is the specification it is being built
against, not a description of the binary.

:::

Give a project a different name, and fix every plane that holds it.

```
bp project rename <project> <new-name>
```

This is the **one operation in the tool that mutates many planes at once**.
Moving the project directory breaks the gitdir pointer of every member worktree
derived from it, so the sweep is unavoidable.

`rename` **converges**: interrupt it and run it again, and it finishes.

## Arguments

| argument | required | what it is |
| --- | --- | --- |
| `<project>` | yes | The project to rename, as `@name` or `name`. |
| `<new-name>` | yes | Its new name, which is also its new directory name. |

A name is a `/`-separated path of segments, each lowercase
`[a-z0-9][a-z0-9._-]*`, and unique per host — `style` and
`acme/platform/style` are both names. A new name already in use by a
**different** project is
[`project_name_taken`](../refusals-and-waivers.md#usage-failures); one that
would nest inside **another** project, or that another project would nest
inside, is refused unwaivably.

The project being renamed is never its own blocker, so
`bp project rename @acme acme/core` — moving a project into a tree it is
currently the root of — is a legal rename, not a collision.

**Renaming a project to the name it already holds is legal**, and it is how an
interrupted rename is finished — see [converging](#converging) below.

## Flags

Only the [global flags](../global-flags.md#global-flags).

## What it does

1. **Acquire every holding plane's lock, then the project lock.** Plane before
   project, never the reverse — that ordering is what makes a deadlock cycle
   unconstructible.
2. `mv` the project directory, creating whatever intermediate directories the
   new name needs.
3. **Prune the intermediate directories the old name left behind**, bottom-up,
   stopping at the first one that is not empty.
4. `git worktree repair` per plane.
5. Rewrite each `plane.toml`'s member value.
6. Rewrite `project.toml`'s `name` **last**.

Holders are found by scanning the planes directory, because nothing maps a
project to its planes and nothing should: an index would be a cache of something
the filesystem already answers.

### Converging

`name` is written last for the same reason [`bp rename`](../plane/rename.md)
writes `id` last. `project.toml`'s `name` is an **integrity check**, not the
authority — the directory name wins — so a disagreement is a reliable "this
rename is unfinished" signal.

Step 2 is the point after which the project answers to its **new** name, because
the directory is the authority. So the command that finishes an interrupted
rename is `bp project rename @style style` — a rename to the name it already
holds, which is a no-op on a consistent project and does the outstanding work on
an inconsistent one. Steps 4 through 6 are each idempotent, so it can be run any
number of times.

Step 3 is the one that does **not** come back on a converge run: a rename to the
name it already holds has no old name, so it has no tree to prune. An
interruption between the move and the prune can therefore leave an empty
intermediate directory behind for good. That is not a fault to repair — it holds
no `project.toml`, so it is not a project, nothing lists it, and the next
`rm` or `rename` through that branch clears it.

### Renaming moves the project through the tree

Because a name is a path, a rename can change how deep the project sits.
`bp project rename @codestyle acme/platform/codestyle` creates `acme/` and
`acme/platform/` and moves the directory into them; renaming it back removes
them again. A project can even be renamed **into its own subtree** —
`@acme` to `acme/core` — which is the one move that cannot be a single `mv`,
since a directory cannot be moved inside itself; it goes via a temporary
sibling, and nothing else about the command changes. Intermediate directories are pruned **bottom-up, stopping at the
first one that is not empty**, so a rename out of a branch of the tree that
another project still uses leaves that branch alone. The projects directory
itself is never pruned.

Neither direction is a special case: every rename creates what the new name
needs and prunes what the old name no longer does, and a rename between two flat
names simply finds nothing to do in either step.

### Layout is not recomputed

Existing planes keep the subdirectories they were built with. A worktree's path
inside a plane derives from the project **source**, once, at create time — and
the source has not changed here. Renaming `@codestyle` to `@style` does not move
`acme/codestyle` inside any plane.

## Output

One row per **plane**, because a failure has to name the plane it happened in.

```
$ bp project rename @codestyle style
```
```
@codestyle is now @style

  bp-a3f9c2e1  repaired  1 member
  auth-work    repaired  1 member
```

A project no plane holds renames with no rows:

```
$ bp project rename @codestyle style
```
```
@codestyle is now @style
```

## Examples

### The new name is taken

Exit `2`. Nothing has been touched.

```
error[project_name_taken]: style is already a different project

remedy: Choose another name, or rename that project first.
```

"a **different** project" is load-bearing: `bp project rename @style style` is
not this error, it is [the converge command](#converging).

### A plane cannot be locked

Exit `5`. The locks are all taken before anything moves, so a busy plane stops
the rename before it starts rather than partway through.

```
error[lock_timeout]: timed out waiting for the lock on ~/planes/auth-work/.bitplane/lock

  ~/planes/auth-work/.bitplane/lock  is locked by another process

remedy: Another bitplane process holds it; retry once that one finishes.
```

### A repair fails in one plane

Exit `1`. The directory has already moved, so the project now answers to
`@style`; the rows say which plane still needs repairing. Run
`bp project rename @style style` to finish it, or
[`bp repair`](../plane/repair.md) in each named plane — both converge on the
same state.

```
error[repair_failed]: @style was renamed; 1 of 2 planes could not be repaired

  auth-work  git worktree repair: permission denied

remedy: Fix what the rows report, then run bp project rename @style style again; repairing is idempotent.
```

## Exit codes

| code | when |
| --- | --- |
| `0` | the project and every holding plane are consistent |
| `1` | the move or a repair failed |
| `2` | bad arguments, an invalid, taken or nesting name, or no such project |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted — re-run to converge |

## See also

- [`bp rename`](../plane/rename.md) — renaming a **plane**
- [`bp repair`](../plane/repair.md) — the same primitive, aimed at one plane
