---
title: Refusals and waivers
---

# Refusals and waivers

Every way `bp` declines to do something, the exact words it uses, and — where
one exists at all — the waiver that answers it.

## The error envelope

Every failure is one line of JSON on **stderr**, whether or not you passed
`--json`. stdout stays clean for results.

```json
{"error":"lock_timeout","code":5,"message":"timed out waiting for the lock on /Users/alfonz/planes/bp-a3f9c2e1/.bitplane/lock","problems":[{"subject":"/Users/alfonz/planes/bp-a3f9c2e1/.bitplane/lock","message":"is locked by another process"}],"remedy":"Another bitplane process holds it; retry once that one finishes."}
```

| field | what it is |
| --- | --- |
| `error` | a stable machine tag. Match on this; never render it. |
| `code` | the number the process exits with. See [Exit codes](./exit-codes.md). |
| `message` | one human sentence saying what went wrong. |
| `problems` | the individual things that were wrong. A refusal across four members is four problems. |
| `remedy` | what to do about it, where `bp` knows. `null` where it does not. |

Each problem is `{subject, message}`. The subject is the member, plane id or
path the problem is about, rendered for display, or `null` where the problem is
about the operation as a whole.

The tables below give `error`, `message` and `remedy` verbatim. Values that vary
are shown as they would appear for a concrete case.

## Refusals

A **refusal** is `bp` declining to destroy work. `bp destroy` and `bp rm` check
**every** member before touching anything and return every reason at once: a
half-destroyed plane is worse than a refused one. Exit code is `1`, and nothing
has been done.

### The five waivable reasons

| reason | problem message | checked how |
| --- | --- | --- |
| `uncommitted` | `feat-login has uncommitted changes` | git's own status, tracked files only |
| `untracked` | `feat-login has untracked files` | git's own status |
| `unpushed` | `feat-login has commits that are not on origin` | the branch tip is not contained in any `refs/remotes/origin/*` |
| `locked_worktree` | `the worktree is locked: migration running` | git's `locked` flag, with git's reason |
| `source_repo_missing` | `the source repo is missing from ~/.local/share/bitplane/projects/api/repo.git` | one `stat` |

`unpushed` is deliberately **not** `git branch -d`'s check. A plane branch is
created off `origin/main` and keeps `origin/main` as its upstream, so `-d`
refuses a branch whose commits are already safe on the forge at `origin/feat`.
The question that matters is *is this work recoverable*, and containment in any
`origin/*` ref answers it.

`destroy` performs **no implicit fetch**. A stale remote-tracking ref can only
make the check more conservative — a false refusal you waive — never more
permissive.

`source_repo_missing` is different in kind from the other four. They mean *I
know there is work here and I accept losing it*; this one means *I cannot check
at all*. Folding it into `uncommitted` would let a waiver granted for a diff you
looked at silently authorise deleting a directory nobody looked at.

### What a refusal prints

```
$ bp destroy
```
```json
{"error":"refused","code":1,"message":"refusing to destroy bp-a3f9c2e1: 2 of 3 members have work that would be lost","problems":[{"subject":"@api","message":"feat-login has uncommitted changes"},{"subject":"@api","message":"feat-login has commits that are not on origin"},{"subject":"@web","message":"feat-login has untracked files"}],"remedy":"Inspect the members listed. Re-run with --waive uncommitted --waive untracked --waive unpushed to accept losing that work."}
```

The remedy names **exactly the reasons that were raised**, in the order of the
table above, so it can be pasted. It never names a reason that did not come up.

### `--waive`

```
--waive <reason>
```

Repeatable. The value is the reason's tag, spelled exactly as it appears in the
table — `locked_worktree`, not `locked-worktree` — so the string you read in a
refusal is the string you type back.

```
bp destroy --waive uncommitted --waive unpushed
```

A waiver is granted **per reason and per invocation**. It is never stored, never
inherited by a later command, and there is no blanket `--force`: you cannot
accidentally override the refusal you did not mean to. The response echoes what
was waived, so a forced destruction is visible in a transcript.

Waiving a reason that was not raised is not an error.

### The unwaivable ones

A waiver says *"I accept losing **this** work, which I am looking at."* Where
that sentence cannot honestly be said, there is no waiver.

#### `project_in_use`

Removing a project whose worktrees are live in some plane damages planes you did
not mention and are not looking at, so no consent given in that moment is
informed.

```json
{"error":"project_in_use","code":1,"message":"@codestyle has worktrees in 2 planes","problems":[{"subject":"bp-a3f9c2e1","message":"holds a worktree of @codestyle"},{"subject":"auth-work","message":"holds a worktree of @codestyle"}],"remedy":"Destroy those planes, or run bp rm @codestyle in each, then try again."}
```

The whole value of an unwaivable refusal is that the way out is obvious, so the
blocking planes are always listed **by id**.

#### `plane_incomplete`

A plane that was claimed and never finished being created. Repairing or running
scripts in one is meaningless work on a thing headed for deletion.

```json
{"error":"plane_incomplete","code":1,"message":"bp-a3f9c2e1 was never finished being created","problems":[],"remedy":"Nothing in it is yours; run bp destroy -p bp-a3f9c2e1 to clear it."}
```

Raised by every command that would act on a latched plane:
[`bp add`](./plane/add.md), [`bp rm`](./plane/rm.md),
[`bp repair`](./plane/repair.md), [`bp rename`](./plane/rename.md) and
[`bp run`](./plane/run.md).

[`bp destroy`](./plane/destroy.md) is the one way out, and it never raises it —
on a latched plane it runs no refusal checks at all, because every one of them
is structurally impossible there. That single exit is the point: the latch means
*nothing in here is yours*, so every other verb declines and points at the one
that clears it.

[`bp project rm`](./project/rm.md) does not raise it either. A latched plane
holding the project blocks the removal, but the error that says so is
[`project_in_use`](#project_in_use), which names the plane.

#### `branch_occupied`

Git refuses `worktree add` on a branch that is checked out in any worktree of
the same repo. `bp` reports it in its own words rather than passing git's
message through.

```json
{"error":"branch_occupied","code":1,"message":"main is already checked out in /Users/alfonz/projects/api","problems":[{"subject":"/Users/alfonz/projects/api","message":"its working tree is on main"}],"remedy":"Check out a different branch there, or give this member a branch no worktree holds."}
```

This can only happen for an **adopted** project or an **ad-hoc member**, whose
source repo is your own checkout and occupies whatever branch you are sitting
on. A bitplane-owned source repo is bare and occupies nothing.

#### `script_blocked`

A `pre_worktree_remove` script exited non-zero. The pass aborts and **nothing is
removed**.

```json
{"error":"script_blocked","code":1,"message":"stop-stack exited 1 in @api; nothing was removed","problems":[{"subject":"@api","message":"stop-stack exited 1"}],"remedy":"Fix the script, or re-run with --no-scripts."}
```

It is not a refusal and no waiver covers it — a script's exit code is a
program's opinion, not work you are looking at. `--no-scripts` is available on
every command that runs scripts, so a `project.toml` can never make a plane
undestroyable.

## Usage failures

Exit code `2`. The request was malformed, or what it asked to create already
exists. Re-running identically will never help.

| `error` | `message` | `remedy` |
| --- | --- | --- |
| `no_command` | `no command given` | `Run` `bp --help` `to see what bp can do.` |
| `bad_usage` | clap's first line, trimmed | `Run` `bp --help` `to see what bp can do.` |
| `plane_id_in_use` | `auth-work is already a plane` | `Choose another id, or destroy the existing plane first.` |
| `plane_id_in_use` | `auth-work exists but was never completed` | `Nothing in it is yours; run` `bp destroy -p auth-work` `to clear it.` |
| `plane_id_in_use` | `auth-work is a claimed directory with no plane file` | `Run` `bp doctor` `to see what is in it, then remove the directory by hand.` |
| `reserved_plane_id` | `bp- is reserved for generated plane ids` | `Choose an id that does not start with bp-.` |
| `invalid_plane_id` | `Auth_Work is not a valid plane id` | `Use lowercase letters, digits and . _ - ; start with a letter or digit; 64 characters at most.` |
| `project_name_taken` | `codestyle is already a project` | `signageos-codestyle is free; re-run with --name signageos-codestyle.` |
| `reserved_path_segment` | `a worktree of this repo would land at .bitplane/api, which is reserved` | `Move the repository out of a directory called .bitplane.` |
| `duplicate_member` | `@api is already a member of bp-a3f9c2e1` | `Run` `bp rm @api`, `then` `bp add @api:feat-login.` |
| `branch_unspecified` | `no branch given for @api` | `Pass -b <branch>, or write the member as @api:<branch>.` |
| `branch_intent_requires_fetch` | `--no-fetch cannot be combined with the default branch intent` | `Drop --no-fetch, or pass --new-branch to create feat-x deliberately.` |
| `member_path_ambiguous` | `~/projects/weird:name could be a path or a member with a branch suffix` | `Register it with` `bp project adopt` `and use its @name instead.` |
| `plane_not_found` | `no plane contains /Users/alfonz/src` | `cd into a plane, or name one with --plane.` |
| `plane_not_found` | `there is no plane called auth-work` | `Run` `bp list` `to see what planes exist.` |
| `project_not_found` | `there is no project called codestyle` | `Run` `bp project list` `to see what projects exist.` |
| `project_not_in_plane` | `@web is not a member of bp-a3f9c2e1` | `Run` `bp show` `to see the members of this plane.` |
| `script_not_found` | `@api has no script called intall` | `Run` `bp project show @api` `to see the scripts it declares.` |

The three `plane_id_in_use` messages are three different things `bp` found at
the path. `create` stats the one path it already knows and classifies it, so the
remedy names what is actually there rather than saying "already in use" and
leaving you to look.

### `branch_intent_requires_fetch`

Refused at request validation, before anything is touched. The default branch
intent creates a branch when it does not resolve, so
`bp add @api:colleagues-branch --no-fetch` against a stale source repo would
silently create a **new, unrelated** branch of that name — and you would find
out at push time, having already committed.

## Typed failures

Exit code `1`. The operation ran and did not succeed.

| `error` | `message` | `remedy` |
| --- | --- | --- |
| `create_aborted` | `create did not finish; bp-a3f9c2e1 was removed` | `Nothing was left behind. Fix what the rows report, then run bp create again.` |
| `create_aborted` | `create did not finish and bp-a3f9c2e1 could not be fully removed` | `Run` `bp destroy -p bp-a3f9c2e1` `to clear the remnant.` |
| `add_aborted` | `add did not finish; bp-a3f9c2e1 is unchanged` | `Fix what the rows report, then run bp add again.` |
| `project_add_aborted` | `@codestyle was not registered` | `The objects fetched so far were kept at ~/.local/share/bitplane/projects/codestyle/repo.git; re-running bp project add will reuse them.` |
| `script_failed` | `install exited 1 in @api; the plane was created` | `See the log, fix the cause, then run bp run @api install.` |
| `script_failed` | `install exited 1 in @api` | `See the log, fix the cause, then run bp run @api install.` |
| `parse_error` | `~/.local/share/bitplane/projects/api/project.toml: unknown key "post_worktree_created" in [scripts.install]` | `Legal keys are argv, shell, post_worktree_create and pre_worktree_remove.` |
| `fetch_failed` | `1 of 2 fetchable projects could not be fetched` | none |
| `repair_failed` | `@style was renamed; 1 of 2 planes could not be repaired` | `Fix what the rows report, then run bp repair in each plane they name.` |
| `io` | `~/planes/bp-a3f9c2e1/plane.toml: permission denied` | none |

`repair_failed` is raised by [`bp rename`](./plane/rename.md),
[`bp repair`](./plane/repair.md) and [`bp project rename`](./project/rename.md)
alike. Its `message` names what did land before the repairs were attempted; its
rows name the planes that still need one, and `bp repair` is idempotent, so
running it there is always safe.

A failed `create` or `add` carries its per-member rows **inside** the error, so
you still see which member failed and whether its cleanup worked. `create` is
all-or-nothing: it either made a plane or it did not.

`script_failed` is a `post_worktree_create` script, or one named on
[`bp run`](./plane/run.md). It unwinds nothing — by the time it runs the plane
is complete and usable — but `bp create` exiting `0` would hide a real failure.
The `remedy` is the same wherever it is raised — re-running the script is the
only move either way. The `message` is what differs: from `bp create` or
`bp add` it says the plane was built, and from [`bp run`](./plane/run.md) there
is no plane to report on, so it says only what failed.

### `parse_error`

An unrecognised key in `plane.toml` or `project.toml` is an error, not a shrug.
`version = 1` owns schema evolution, so silence would buy nothing and cost you a
script that never runs with no trace of why. The message always names the
offending key and the remedy lists what was legal.

`plane.toml` additionally refuses a member key that is absolute, contains `..`,
starts with `./`, or begins with `.bitplane`; a nested table under `[members]`;
and two members resolving to the same thing.

A plane whose file does not parse is still **listed** by
[`bp list`](./plane/list.md) as a row in an error state naming the file and the
error. The scan continues, and nothing is auto-repaired: a file `bp` cannot read
is a file it has no business rewriting.

## Prerequisite and lock failures

| `error` | `code` | `message` | `remedy` |
| --- | --- | --- | --- |
| `git_missing` | `4` | `git was not found on PATH; bitplane requires git 2.36 or newer` | `Install git 2.36 or newer and put it on PATH.` |
| `git_unusable` | `4` | `git could not be run (exited 1); bitplane requires git 2.36 or newer` | `Check that` `git --version` `reports git 2.36 or newer.` |
| `git_version_unreadable` | `4` | `git's version could not be read from "not a version at all"; bitplane requires git 2.36 or newer` | `Check that` `git --version` `reports git 2.36 or newer.` |
| `git_too_old` | `4` | `git 2.35.9 is too old; bitplane requires git 2.36 or newer` | `Upgrade git to 2.36 or newer.` |
| `lock_timeout` | `5` | `timed out waiting for the lock on ~/planes/bp-a3f9c2e1/.bitplane/lock` | `Another bitplane process holds it; retry once that one finishes.` |

A lock is tried non-blocking first; on failure `bp` prints `waiting for
@codestyle…` to stderr and blocks for up to 120 seconds — a cold fetch of a
large repo genuinely takes that long — then fails naming the contended object.
The common case stays silent. The failure names the object rather than a pid,
because `flock` does not identify its holder.

## See also

- [Exit codes](./exit-codes.md) — what each `code` means
- [Global flags](./global-flags.md#--json) — why the envelope is on stderr
