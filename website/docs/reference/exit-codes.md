---
title: Exit codes
---

# Exit codes

Six codes, plus the one the shell gives you. Every command uses the same set,
and every code below is reachable from a worked example on a command page.

| code | name | meaning | example |
| --- | --- | --- | --- |
| `0` | ok | The command did what was asked. | [`bp list`](./plane/list.md#everything-is-fine) |
| `1` | failure | The operation ran and did not succeed — including every refusal. | [`bp destroy`](./plane/destroy.md#refused) |
| `2` | usage | The request was malformed, or what it asked to create already exists. | [`bp create --id`](./plane/create.md#the-id-is-already-taken) |
| `3` | drift | The command **succeeded** and reported a finding about a plane or a project. | [`bp status`](./plane/status.md#a-member-whose-worktree-is-gone) |
| `4` | prerequisite missing | git is absent, unusable, or older than 2.36. | [`bp create`](./plane/create.md#git-is-too-old) |
| `5` | busy | A lock could not be taken in time. Try again. | [`bp add`](./plane/add.md#another-bp-holds-the-plane) |
| `130` | interrupted | `128 + SIGINT`. The shell's convention, not `bp`'s. | [`bp create`](./plane/create.md#ctrl-c) |

## Why these three are separate

`1`, `2` and `5` all mean "it did not happen", and a script has to tell them
apart to know what to do:

- **`2` will never succeed** however many times you run it. The command line was
  wrong, or the name is taken.
- **`5` is "try again"** and nothing else. Another `bp` holds the lock; when it
  finishes, yours works. Retrying is sane.
- **`1` is everything else.** Retrying identically is not obviously sane —
  something has to change first, and the envelope's `remedy` usually says what.

## `3` is a success, and it is not about your work

`bp` exits `3` when the command **worked** and there is a finding. The result is
on stdout; `3` is the flag saying "look at it".

Most findings are about a plane. One is not: a `project.toml` that will not
parse is a finding on a **project**, which is why
[`bp project list`](./project/list.md) can exit `3` with no plane in sight.

**Drift is never your uncommitted work.** A worktree with unstaged changes, a
branch you switched, a rebase in progress — all of that is ordinary use, and
`bp status` reports it at exit `0`.

A finding is one of two things: something `bp` owns is not as recorded, or a
file `bp` needs could not be read. `bp` owns a worktree's **existence and
location**, so the first kind is always about one of those:

| finding | what it means |
| --- | --- |
| member worktree missing | `plane.toml` lists a member whose directory is not there |
| create never completed | the plane was claimed and never finished being built |
| source repo missing | the repo a member is a worktree of is gone |
| id mismatch | `plane.toml`'s `id` disagrees with the directory name — the plane was moved outside `bp` |
| prunable | git reports the worktree at a path that no longer exists |

The second kind is one row:

| finding | what it means |
| --- | --- |
| unreadable | a `plane.toml` or `project.toml` did not parse; the listing printed it as a row in an error state and carried on |

The reads exit `3`: [`bp list`](./plane/list.md),
[`bp show`](./plane/show.md), [`bp status`](./plane/status.md),
[`bp project list`](./project/list.md) and [`bp doctor`](./plane/doctor.md).

One mutation joins them. [`bp repair`](./plane/repair.md) is aimed at findings,
so a finding it cannot fix is the one thing it has to say — it exits `3`.
Every other mutation either does the thing or fails.

**A listing tolerates a file it cannot read; a `show` does not.** `bp list` and
`bp project list` report an unreadable file as a row and keep scanning, which is
exit `3`. `bp show` and `bp project show` have exactly one file to read, so
failing to read it is exit `1`.

## `4` is checked once, before anything is touched

Every command verifies git before it does any work, so `4` never leaves a plane
half-built. `bp` requires **git 2.36 or newer**, because
`git worktree list --porcelain -z` — the only form that handles paths and lock
reasons containing unsafe bytes correctly — landed in 2.36.0.

Four failures produce it, and their messages are fixed:

| tag | message |
| --- | --- |
| `git_missing` | `git was not found on PATH; bitplane requires git 2.36 or newer` |
| `git_unusable` | `git could not be run (<what happened>); bitplane requires git 2.36 or newer` |
| `git_version_unreadable` | `git's version could not be read from "<what git printed>"; bitplane requires git 2.36 or newer` |
| `git_too_old` | `git 2.35.9 is too old; bitplane requires git 2.36 or newer` |

## `130`, and what Ctrl-C actually does

SIGINT reaches the whole process group, so an in-flight `git worktree add` gets
it too. `bp` cannot protect its children; all it controls is what it records and
prints.

**The first Ctrl-C** stops scheduling new members, waits for the children
already running, runs whatever unwind the command has, prints the rows it has on
stdout, and exits `130`. Those rows are not a courtesy — they are the repair
instruction.

**The second Ctrl-C** exits immediately, and whatever was in flight is left
where it fell. [`bp repair`](./plane/repair.md) and
[`bp destroy`](./plane/destroy.md) are how you clean up after that.

## See also

- [Refusals and waivers](./refusals-and-waivers.md) — every `1`, with its exact wording
- [Global flags](./global-flags.md#--json) — where the code appears on the wire
