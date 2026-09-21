---
title: bp list
---

# `bp list`

Every plane on this host, with its members, their live branches and its health.

```
bp list [--health none|cheap|full]
```

`list` is a **read**: it takes no lock, creates no file, and repairs nothing.
That is what makes it safe to run from a script, from a shell prompt, or in a
loop.

## Arguments

None. `list` reads the [planes directory](../global-flags.md#directory-resolution).

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `--health <level>` | `cheap` | How hard to look. `none`, `cheap` or `full`. |

Plus the [global flags](../global-flags.md#global-flags).

### Health levels

| level | cost | what it finds |
| --- | --- | --- |
| `none` | one directory listing, one small file per plane | nothing — just membership |
| `cheap` | filesystem only, no git process | member worktree missing, create never completed, source repo missing, id mismatch |
| `full` | one git invocation per source repo | all of the above, plus git's `prunable` |

`cheap` is the default because `list` runs constantly and must not spawn git per
member. A member's **live branch is cheap**: the worktree's `.git` file points
at `<source-repo>/worktrees/<name>/`, whose `HEAD` is a one-line file, so `bp`
reads real branches without starting a process.

## What counts as a plane

**A plane is exactly a directory containing `plane.toml`.** A directory in the
planes directory without one is silently skipped, so an unrelated directory
dropped in there is ignored rather than being an error.

A `plane.toml` that does not parse is **listed as a row in an error state**
naming the file and the error, and the scan continues. The listing is never
aborted by one bad file, and nothing is auto-repaired: a file `bp` cannot read
is a file it has no business rewriting.

## Output

One block per plane: the id, its directory, when it was created, then its
members.

```
$ bp list
```
```
auth-work    ~/planes/auth-work    created 2026-09-18
  @api   feat-login
  @web   feat-login

bp-a3f9c2e1  ~/planes/bp-a3f9c2e1  created 2026-09-21
  @api                             feat-x
  /Users/alfonz/projects/bitplane  main
```

The creation time comes from the plane **directory's** birth time, which is the
`mkdir` that claimed it and survives every rewrite, every worktree add, and a
same-filesystem `mv`. It is **absent** where the filesystem cannot supply one —
NFS, older ext4, or a plane moved across filesystems — and nothing depends on
it.

## Examples

### Everything is fine

Exit `0`.

```
$ bp list
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1  created 2026-09-21
  @api  feat-login
  @web  feat-login
```

No planes at all is also exit `0`, with nothing on stdout.

### Something has drifted

Exit `3`. The command **worked**; `3` says look at what it found.

```
$ bp list
```
```
auth-work    ~/planes/auth-work    created 2026-09-18
  @api   feat-login
  @web   -                         worktree missing

bp-7c1e0d44  ~/planes/bp-7c1e0d44  created 2026-09-21
  create never completed, started 3 days ago

broken.plane  ~/planes/broken.plane
  plane.toml: unknown key "status" at line 4
```

`create never completed` is the [latch](./create.md#what-it-writes), and it says
something no other signal can: this plane **never worked**, as opposed to having
worked and since lost a worktree. That is what makes it safe to discard with
[`bp destroy`](./destroy.md#a-plane-that-was-never-finished-being-created) without a single refusal check.

## Exit codes

| code | when |
| --- | --- |
| `0` | every plane is healthy |
| `2` | bad arguments |
| `3` | at least one plane reported a finding |
| `4` | git is missing, unusable or older than 2.36 |

`list` never exits `1` or `5`. It takes no locks, and a plane it cannot read is
a row rather than a failure.

## See also

- [`bp show`](./show.md) — one plane, in detail
- [`bp status`](./status.md) — git's own view of every member
- [`bp doctor`](./doctor.md) — what the planes directory structurally cannot see
