---
title: bp destroy
---

# `bp destroy`

Remove every worktree in a plane and the plane directory with them.

```
bp destroy [-p <plane>] [--waive <reason>]... [--no-scripts]
```

`destroy` **converges**. Rollback is not available — you cannot un-remove a
worktree — so an interrupted run is finished by running it again: refusals are
re-checked against what survived, and the members already gone are reported
`already gone`.

## Arguments

None. The plane comes from the current directory, or from `--plane`.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane to destroy. |
| `--waive <reason>` | — | Accept one refusal reason, for this invocation. Repeatable. See [waivers](../refusals-and-waivers.md#--waive). |
| `--no-scripts` | run them | Do not run `pre_worktree_remove` scripts. |

Plus the [global flags](../global-flags.md#global-flags).

There is **no `--force`**. Every reason is waived individually, so you cannot
accidentally override the one you did not mean to.

## What it does

1. Collect **every** refusal across **every** member and return them in one
   error. A half-destroyed plane is worse than a refused one.
2. Run `pre_worktree_remove` scripts as one sequential pass in `plane.toml`
   order, aborting on the first non-zero exit and removing nothing.
3. Remove each worktree, deleting its branch where the member is an owned
   project — see [branch deletion](./rm.md#branch-deletion).
4. Delete `plane.toml`.
5. `rmdir` the plane directory.

`plane.toml` is the **first thing written** by `create` and the **last thing
removed** here, so the window in which a plane is visible strictly contains the
window in which it exists. Anything that exists can be found and named.

Scripts cannot join the refusal pass: running `docker compose down` and *then*
refusing over uncommitted work would leave the containers stopped and the plane
intact.

Anything in the plane directory that no member names — your scratch files, a
notes file — goes with the directory. The plane directory is yours between
`create` and `destroy`, and `destroy` is the end of that.

## Output

```
$ bp destroy
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api   feat-login  removed  acme/api (branch deleted)
  @web   feat-login  removed  acme/web (branch deleted)
  @docs  -           already gone  acme/docs

destroyed bp-a3f9c2e1
```

## Examples

### Refused

Exit `1`, and **nothing has been touched**. Every reason across every member
comes back at once, so one re-run with the right waivers finishes the job.

```
$ bp destroy
```
```
error[refused]: refusing to destroy bp-a3f9c2e1: 2 of 3 members have work that would be lost

  @api  feat-login has uncommitted changes
  @api  feat-login has commits that are not on origin
  @web  feat-login has untracked files

remedy: Inspect the members listed. Re-run with --waive uncommitted --waive untracked --waive unpushed to accept losing that work.
```

The remedy names exactly the reasons that were raised, so it can be pasted.

### A plane that was never finished being created

`destroy` is the **only** command that does not refuse on a latched plane, and
it runs no refusal checks there at all. No script ever ran, so there are no
untracked files; the branches are seconds old, so nothing is uncommitted or
unpushed. Every refusal reason is structurally impossible.

```
$ bp destroy -p auth-work
```
```
auth-work  ~/planes/auth-work

  @api  feat-login  removed  acme/api

destroyed auth-work (create had never completed)
```

That property is why [`bp add` never sets the latch](./add.md#the-run-aborts).

### A pre-remove script blocks

Exit `1`, and nothing is removed.

```
error[script_blocked]: stop-stack exited 1 in @api; nothing was removed

  @api  stop-stack exited 1

remedy: Fix the script, or re-run with --no-scripts.
```

`--no-scripts` always works, which is the whole answer to *can a `project.toml`
make a plane undestroyable*. It cannot.

A block partway through the pass leaves earlier members' teardown already done
with nothing removed. That is the accepted cost of letting one script both veto
and tear down; the unwind is yours.

## Exit codes

| code | when |
| --- | --- |
| `0` | the plane is gone |
| `1` | refused, a `pre_worktree_remove` script blocked, or a removal failed |
| `2` | bad arguments, or no plane found |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted — re-run to converge |

## See also

- [Refusals and waivers](../refusals-and-waivers.md)
- [`bp rm`](./rm.md) — the same rules, for one member
- [`bp repair`](./repair.md) — for a plane that moved rather than one you are done with
