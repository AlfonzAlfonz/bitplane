---
title: bp add
---

# `bp add`

:::in-progress

Everything on this page works: the branch rules, branch intent and its
refusal of `--no-fetch`, the typed duplicate, the surgical unwind, and the
latch that `add` never sets. One part is not there yet:

- **`--no-scripts` is not accepted.** No `post_worktree_create` script runs,
  so there is nothing to switch off.

:::

Put one or more members into a plane that already exists.

```
bp add <member>:<branch>... [-p <plane>] [-b <branch>]
       [--new-branch | --existing-branch] [--no-fetch] [--no-scripts]
```

`add` cannot abort-and-remove the way [`bp create`](./create.md) does — the
plane holds other members full of work. On failure it force-removes only the
worktrees **it** created, drops only **its own** entries, and leaves the plane
exactly as it found it.

## Arguments

| argument | required | what it is |
| --- | --- | --- |
| `<member>:<branch>...` | yes, one or more | A [member](../member-syntax.md) and the branch its worktree goes on. |

**The branch is mandatory**, from the suffix or from `-b`. Nothing is derived
from what the plane's existing members happen to be on: that would make `add`
depend on what you did in an unrelated worktree last week, which is worse than
asking.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane to add to. |
| `-b`, `--branch <branch>` | — | The branch for every member that does not carry its own. |
| `--new-branch` | — | The branch must not already exist. |
| `--existing-branch` | — | The branch must already exist. |
| `--no-fetch` | fetch first | Skip the fetch of every owned project being added. |
| `--no-scripts` | run them | Do not run `post_worktree_create` scripts. |

Plus the [global flags](../global-flags.md#global-flags). Branch intent behaves
exactly as it does on [`bp create`](./create.md#branch-intent), including the
refusal of `--no-fetch` with the default intent.

## Output

[One row per member added](../global-flags.md#how-a-fan-out-prints). Members already in the
plane are not reprinted — `add` reports what it did.

```
$ bp add @docs:feat-login
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @docs  feat-login  created  acme/docs
```

## Examples

### The member is already in the plane

Exit `2`. A plane holds at most one worktree per project, and there is no
adoption magic: `add` will not quietly take over a listed member whose worktree
has gone missing.

```
$ bp add @api:feat-other
```
```
error[duplicate_member]: @api is already a member of bp-a3f9c2e1

remedy: Run bp rm @api, then bp add @api:feat-other.
```

Two commands, deliberately. It is the same strictness `create` has, and it makes
the removal visible instead of implied.

### Another `bp` holds the plane

Exit `5`. `bp` tries the lock without blocking, then prints a line to stderr and
blocks for up to 120 seconds before giving up.

```
$ bp add @docs:feat-login
waiting for bp-a3f9c2e1…
```
```
error[lock_timeout]: timed out waiting for the lock on ~/planes/bp-a3f9c2e1/.bitplane/lock

  ~/planes/bp-a3f9c2e1/.bitplane/lock  is locked by another process

remedy: Another bitplane process holds it; retry once that one finishes.
```

`5` means **try again** and nothing else: when the other process finishes, the
same command works. The plane lock exists because two concurrent `bp add`s on
one plane would otherwise silently drop one member while leaving its worktree on
disk, orphaned and invisible.

### The plane was never finished being created

Exit `1`. A latched plane is on its way to being discarded, so `add` declines
and points at the one verb that clears it.

```
error[plane_incomplete]: bp-a3f9c2e1 was never finished being created

remedy: Nothing in it is yours; run bp destroy -p bp-a3f9c2e1 to clear it.
```

Refusing here is what keeps [`bp destroy`](./destroy.md#a-plane-that-was-never-finished-being-created)'s
refusal-free path honest: if `add` could put real work into a latched plane,
the latch's promise that *nothing in here is yours* would become a trapdoor.

### The run aborts

Exit `1`. The plane returns to its prior state.

```
error[add_aborted]: add did not finish; bp-a3f9c2e1 is unchanged

  @docs  failed: git worktree add exited 128: fatal: 'feat-login' is already used by worktree

remedy: Fix what the rows report, then run bp add again.
```

**`add` never sets the incomplete latch**, under any failure. The latch means
*this plane was never completed, nothing in it is yours*, and that is what
licenses a [refusal-free destroy](./destroy.md#a-plane-that-was-never-finished-being-created). A plane full of real
work flagged as free to discard would be a trapdoor.

The fetch happens **before** the plane's lock is taken, so a forge that will not
answer stops the run with nothing added and no lock held.

If the unwind itself dies, the plane file lists a member with no worktree, and
the failure says so rather than claiming the plane is unchanged:

```
error[add_aborted]: add did not finish and bp-a3f9c2e1 could not be returned to its prior state

remedy: Run bp show -p bp-a3f9c2e1 to see which members have no worktree, then bp rm them.
```

An interrupt whose unwind could not finish says the same thing on stdout, since
`add` still answers with its rows there:

```
bp-a3f9c2e1 still lists members with no worktree; run bp rm to clear them.
```

That shows up as a finding on [`bp show`](./show.md), and
[`bp rm @docs`](./rm.md) clears it — `rm` tolerates a member whose worktree is
already gone.

## Exit codes

| code | when |
| --- | --- |
| `0` | every member was added |
| `1` | the run aborted, the plane is incomplete, or a `post_worktree_create` script failed |
| `2` | bad arguments, a missing branch, a duplicate member, or a branch intent that cannot be honoured |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted |

## See also

- [Member syntax](../member-syntax.md) — the `:branch` suffix
- [`bp create`](./create.md) · [`bp rm`](./rm.md)
