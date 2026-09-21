---
title: bp project rename
---

# `bp project rename`

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

A name is lowercase `[a-z0-9][a-z0-9._-]*`, flat, and unique per host. A new
name already in use by a **different** project is
[`project_name_taken`](../refusals-and-waivers.md#usage-failures).

**Renaming a project to the name it already holds is legal**, and it is how an
interrupted rename is finished — see [converging](#converging) below.

## Flags

Only the [global flags](../global-flags.md#global-flags).

## What it does

1. **Acquire every holding plane's lock, then the project lock.** Plane before
   project, never the reverse — that ordering is what makes a deadlock cycle
   unconstructible.
2. `mv` the project directory.
3. `git worktree repair` per plane.
4. Rewrite each `plane.toml`'s member value.
5. Rewrite `project.toml`'s `name` **last**.

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
an inconsistent one. Steps 3, 4 and 5 are each idempotent, so it can be run any
number of times.

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

```json
{"error":"project_name_taken","code":2,"message":"style is already a different project","problems":[],"remedy":"Choose another name, or rename that project first."}
```

"a **different** project" is load-bearing: `bp project rename @style style` is
not this error, it is [the converge command](#converging).

### A plane cannot be locked

Exit `5`. The locks are all taken before anything moves, so a busy plane stops
the rename before it starts rather than partway through.

```json
{"error":"lock_timeout","code":5,"message":"timed out waiting for the lock on ~/planes/auth-work/.bitplane/lock","problems":[{"subject":"~/planes/auth-work/.bitplane/lock","message":"is locked by another process"}],"remedy":"Another bitplane process holds it; retry once that one finishes."}
```

### A repair fails in one plane

Exit `1`. The directory has already moved, so the project now answers to
`@style`; the rows say which plane still needs repairing. Run
`bp project rename @style style` to finish it, or
[`bp repair`](../plane/repair.md) in each named plane — both converge on the
same state.

```json
{"error":"repair_failed","code":1,"message":"@style was renamed; 1 of 2 planes could not be repaired","problems":[{"subject":"auth-work","message":"git worktree repair: permission denied"}],"remedy":"Fix what the rows report, then run bp project rename @style style again; repairing is idempotent."}
```

## Exit codes

| code | when |
| --- | --- |
| `0` | the project and every holding plane are consistent |
| `1` | the move or a repair failed |
| `2` | bad arguments, an invalid or taken name, or no such project |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted — re-run to converge |

## See also

- [`bp rename`](../plane/rename.md) — renaming a **plane**
- [`bp repair`](../plane/repair.md) — the same primitive, aimed at one plane
