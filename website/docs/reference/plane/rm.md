---
title: bp rm
---

# `bp rm`

Take one member out of a plane, leaving the rest of the plane alone.

```
bp rm <member>... [-p <plane>] [--waive <reason>]... [--no-scripts]
```

`bp rm` carries **rules identical to [`bp destroy`](./destroy.md)**. Removing a
member destroys exactly as much work as destroying a one-member plane, so a
lighter rule here would be a hole in that one.

:::danger[`bp rm` removes a member, not a project]

`bp rm @codestyle` takes `@codestyle`'s worktree out of **this plane**. It does
not touch the project, its source repo, or any other plane.
[`bp project rm`](../project/rm.md) is how a project is removed, and project
removal is **never** spelled as a bare verb.

:::

## Arguments

| argument | required | what it is |
| --- | --- | --- |
| `<member>...` | yes, one or more | A [member](../member-syntax.md) of this plane. No branch suffix — a member is already on a branch. |

A member that is not in the plane is
[`project_not_in_plane`](../refusals-and-waivers.md#usage-failures). Standing inside a
member's worktree does not name it: a path resolves to a **plane**, never to a
member.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane to remove from. |
| `--waive <reason>` | — | Accept one refusal reason, for this invocation. Repeatable. See [waivers](../refusals-and-waivers.md#--waive). |
| `--no-scripts` | run them | Do not run `pre_worktree_remove` scripts. |

Plus the [global flags](../global-flags.md#global-flags).

## What it does

1. Collect **every** refusal across **every** named member, and stop if there
   are any.
2. Run `pre_worktree_remove` scripts, sequentially. A non-zero exit aborts and
   removes nothing.
3. Remove each worktree, and delete the branch it is on where the member is an
   owned project.
4. Rewrite `plane.toml` **last**.

The file is rewritten last so a crash between steps 3 and 4 leaves a *listed*
member with no worktree — visible, and converging on a re-run — rather than an
unlisted worktree nothing can see.

### Branch deletion

For an **owned** project, `rm` deletes the branch the worktree is **actually on
at that moment**. Nothing records the branch it was created on, so no other
answer is available.

This needs no extra waiver and no second gate: the `unpushed` check has already
run, so a branch that got this far is provably safe to delete. `bp rm` refuses,
names the member and the branch, and `--waive unpushed` proceeds and deletes.

For an **adopted** project or an **ad-hoc member**, the worktree goes and **no
ref is touched**. `bp` cannot tell a branch it created from one you have had for
two years in a repo it merely pointed at, and it owns nothing there. The output
says so per member.

## Output

```
$ bp rm @docs
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @docs  feat-login  removed  acme/docs (branch deleted)
```

A member whose worktree was already gone converges silently:

```
$ bp rm @docs
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @docs  -  already gone  -
```

## Examples

### Refused

Exit `1`, and nothing has been touched. See
[refusals](../refusals-and-waivers.md#refusals) for all five reasons.

```
$ bp rm @api @web
```
```
error[refused]: refusing to remove 2 of 2 members from bp-a3f9c2e1: they have work that would be lost

  @api  feat-login has uncommitted changes
  @web  feat-login has untracked files

remedy: Inspect the members listed. Re-run with --waive uncommitted --waive untracked to accept losing that work.
```

The count is over the members **you named**, not over the plane.

### The plane was never finished being created

Exit `1`. A latched plane is on its way to being discarded, so `rm` declines
and points at the one verb that clears it.

```
error[plane_incomplete]: bp-a3f9c2e1 was never finished being created

remedy: Nothing in it is yours; run bp destroy -p bp-a3f9c2e1 to clear it.
```

Whittling a latched plane down one member at a time leaves a latched plane,
still headed for deletion, with a shorter membership list — meaningless work.
[`bp destroy`](./destroy.md#a-plane-that-was-never-finished-being-created) does
the whole thing in one command and with no refusal checks at all, because on a
latched plane every refusal reason is structurally impossible.

### Waived

```
$ bp rm @api @web --waive uncommitted --waive untracked
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api  feat-login  removed  acme/api (branch deleted, waived: uncommitted)
  @web  feat-login  removed  acme/web (branch deleted, waived: untracked)
```

The waiver is echoed, so a forced removal is visible in a transcript.

## Exit codes

| code | when |
| --- | --- |
| `0` | every named member is out of the plane |
| `1` | refused, the plane is incomplete, a `pre_worktree_remove` script blocked, or a removal failed |
| `2` | bad arguments, or a member that is not in this plane |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted |

## See also

- [Refusals and waivers](../refusals-and-waivers.md) — the five reasons and their wording
- [`bp destroy`](./destroy.md) — the same rules, for the whole plane
- [`bp project rm`](../project/rm.md) — removing a **project**
