---
title: bp repair
---

# `bp repair`

Reconnect a plane's worktrees to their source repos after something moved.

```
bp repair [-p <plane>]
```

Three moves break a worktree's link to its repo, and `repair` is the cure for
all three:

- the **plane directory** moved, by a `mv` you ran rather than a
  [`bp rename`](./rename.md);
- a **worktree** moved within the plane;
- the **project directory** behind a member moved.

`repair` is **idempotent**, because `git worktree repair` is. Running it on a
healthy plane does nothing and says so.

## Why a worktree cannot just be moved

A plain `mv` of a worktree inside its plane leaves git reporting the worktree at
its **old** path and marking it `prunable` — git's word for "the directory is
gone". That is a staleness signal, so a moved worktree looks like garbage at a
path where nothing can find your work to veto on.

The move is therefore not a free act. `repair` is what makes it a supported one.

## Arguments

None. The plane comes from the current directory, or from `--plane`.

Discovery has **no planes-directory constraint** — a plane found outside the
configured planes directory still resolves. Refusing it would make `repair`
impossible to aim at the one plane that most needs it.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane to repair. |

Plus the [global flags](../global-flags.md#global-flags).

## What it does

1. **Scan the plane directory** for worktrees.
2. `git worktree repair` on each, one invocation per source repo.
3. Rewrite `plane.toml`'s member keys to where the worktrees actually are.

`repair` is the **one operation licensed to scan the plane directory**, because
it is an explicit request rather than a read. That is what keeps
[`bp list`](./list.md) and [`bp show`](./show.md) to one directory listing and
one small file per plane.

It repairs a plane. It does not **converge** one: a member listed in
`plane.toml` with no worktree anywhere is reported, not recreated. `bp rm` drops
it and `bp add` puts it back.

A directory in the plane that no member names and that is not a worktree is
ignored, not reported. It is almost always your own scratch space.

## Output

```
$ bp repair
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api  feat-login  repaired  signageos/api
  @web  feat-login  repaired  signageos/web (moved from signageos/www)
```

Nothing to do:

```
$ bp repair
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api  feat-login  unchanged  signageos/api
  @web  feat-login  unchanged  signageos/web

nothing to repair
```

## Examples

### A member's worktree is missing entirely

Exit `3` — the command succeeded and reported a finding.

```
$ bp repair
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api   feat-login  repaired  signageos/api
  @docs  -           failed: no worktree found  signageos/docs
```

### The plane was never finished being created

Exit `1`.

```json
{"error":"plane_incomplete","code":1,"message":"bp-a3f9c2e1 was never finished being created","problems":[],"remedy":"Nothing in it is yours; run bp destroy -p bp-a3f9c2e1 to clear it."}
```

## Exit codes

| code | when |
| --- | --- |
| `0` | every worktree is connected |
| `1` | the plane is incomplete, or a repair failed |
| `2` | bad arguments, or no plane found |
| `3` | repaired, with a finding left that `repair` cannot fix |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted — re-run to converge |

## See also

- [`bp rename`](./rename.md) — a move `bp` performs, which repairs as it goes
- [`bp doctor`](./doctor.md) — for what is broken outside a plane
