---
title: Recover a plane that broke
sidebar_label: Recover a broken plane
---

# Recover a plane that broke

:::in-progress

The diagnosis half works today: [`bp list`](../reference/plane/list.md),
[`bp show`](../reference/plane/show.md) and
[`bp status`](../reference/plane/status.md), plus the
[`bp destroy`](../reference/plane/destroy.md),
[`bp rm`](../reference/plane/rm.md) and [`bp add`](../reference/plane/add.md)
this page sends you to. [`bp repair`](../reference/plane/repair.md),
[`bp rename`](../reference/plane/rename.md) and
[`bp doctor`](../reference/plane/doctor.md) do not exist yet — the sections
using them describe what is being built.

:::

Something moved, something vanished, or a `create` died halfway. Every one of
these has a named shape and exactly one command that fixes it.

**Start by looking.** All four diagnostic commands are reads: no locks, no
files written, nothing repaired behind your back. Running them cannot make
anything worse.

```sh
bp list           # every plane, and its health
bp show           # this plane, in detail
bp status         # git's own view of every member
bp doctor         # what a plane-directory scan structurally cannot see
```

A finding is reported at **exit `3`**, which means *the command worked, now look
at what it found* — not *the command failed*. And a finding is never about your
uncommitted work; see
[the ownership boundary](../concepts/ownership-boundary.md#working-in-a-plane-is-not-drift).

## "create never completed"

```
bp-7c1e0d44  ~/planes/bp-7c1e0d44  created 2026-09-19
  create never completed, started 3 days ago
```

A `bp create` was interrupted — Ctrl-C, a crash, a dead network — and left a
directory behind.

**Throw it away.**

```sh
bp destroy -p bp-7c1e0d44
```

This is the one case where you can be certain without looking. The plane carries
an [incomplete marker](../concepts/plane.md#a-plane-that-never-worked) written
before anything else and removed only when the last worktree lands, so it says
something no other signal can: **this plane never worked.** Not *it worked and
has since lost something* — it never worked at all. No script ever ran, so there
are no untracked files; the branches are seconds old, so nothing is uncommitted
or unpushed.

Which is why `destroy` runs **no refusal checks at all** here, and why every
other command declines and points at it:

```
error[plane_incomplete]: bp-7c1e0d44 was never finished being created

remedy: Nothing in it is yours; run bp destroy -p bp-7c1e0d44 to clear it.
```

There is no "finish the half-built one". The window a `create` can die in holds
only cheap, discardable work by design — the fetch and the scripts are
deliberately outside it — so discarding and re-running *is* the repair.

## You moved the plane directory with `mv`

```sh
mv ~/planes/bp-a3f9c2e1 ~/planes/auth-work     # don't
```

git now looks for every worktree at its old path and finds nothing.

```sh
cd ~/planes/auth-work
bp repair
```

```
auth-work  ~/planes/auth-work

  @api  feat-login  repaired  acme/api
  @web  feat-login  repaired  acme/web
```

`repair` runs `git worktree repair` for each source repository and rewrites the
plane file's keys to where the worktrees actually are. It is **idempotent** —
running it on a healthy plane does nothing and says so — so there is never a
reason not to try it.

**Next time, use [`bp rename`](../reference/plane/rename.md)**, which moves the
directory and repairs as it goes:

```sh
bp rename auth-work
```

A plane id is mutable by design, so renaming is expected rather than exceptional.
Just remember the consequence: nothing outside bitplane should store a plane id
and expect it to resolve later.

### A rename that was interrupted

The plane file's `id` is written **last**, so a plane whose `id` disagrees with
its directory name is a reliable *this rename is unfinished* signal rather than
a lie. It is also exactly the state a hand-`mv` leaves, and the cure is the same
either way: `bp repair`, or run the `bp rename` again. Both converge.

## You moved a worktree inside the plane

```sh
mv acme/www acme/web     # don't
```

Same command, same reason:

```sh
bp repair
```

```
auth-work  ~/planes/auth-work

  @api  feat-login  unchanged  acme/api
  @web  feat-login  repaired   acme/web (moved from acme/www)
```

Worth knowing *why* this is not a free act. A plain `mv` leaves git reporting
the worktree at its old path and marking it `prunable` — git's word for *the
directory is gone*. That is one of the signals bitplane treats as staleness, so
a moved worktree looks like garbage at a path where nothing can find your work
to veto over. `repair` is what turns the move into a supported one.

## A member's worktree is gone

```
auth-work  ~/planes/auth-work

  @api   feat-login  3 modified, 1 untracked
  @web   -           worktree missing

findings
  @web  the worktree at acme/web is not there
        bp rm @web drops it; bp add @web:<branch> puts it back
```

The plane lists a member whose directory is not there. `repair` will **report**
this and not fix it — it repairs a plane, it does not converge one. Recreating a
worktree you deleted would be bitplane deciding you did not mean it.

Two commands, and which one you want depends on what you meant:

```sh
bp rm @web                    # I am done with it — drops the listing
bp add @web:feat-login        # I want it back — makes a new worktree
```

`bp rm` tolerates a member whose worktree is already gone; it reports
`already gone` and converges.

## The source repository is gone

```
  @api  -  source repo missing
```

The repository the worktree came from has been deleted. There is nothing left to
repair against — the worktree's directory may still be full of files, but git
cannot operate on it.

Rescue anything in there by hand first, because `bp rm @api` will refuse until
you say otherwise: a missing source repository is
[a refusal reason in its own right](../reference/refusals-and-waivers.md#the-five-waivable-reasons),
raised by a single `stat` whenever the repository is not there.

It is a different *kind* of reason from the other four. They mean *I know there
is work here and I accept losing it*; this one means *I cannot check at all*, so
`--waive source_repo_missing` is you saying you looked instead.

## You interrupted a destroy

Run it again.

```sh
bp destroy
```

```
auth-work  ~/planes/auth-work

  @api   feat-login  removed       acme/api
  @web   -           already gone  -

destroyed auth-work
```

`destroy` and `rm` **converge**: rollback is not available — you cannot
un-remove a worktree — so the design makes finishing safe instead. Refusals are
re-checked against what survived, and members already gone come back
`already gone`.

The same is true of `repair`, `rename` and `project rename`, all of which sit on
`git worktree repair`, which is idempotent by construction. Running any of them
a second time is always safe.

## A `plane.toml` that will not parse

```
broken.plane  ~/planes/broken.plane
  plane.toml: unknown key "status" at line 4
```

The listing keeps going — one unreadable file never aborts a scan — and
**nothing is auto-repaired**. A file bitplane cannot read is a file it has no
business rewriting.

Fix it by hand. A plane file has exactly three keys: `version`, `id` and
`[members]`. The same applies to a `project.toml` that will not parse; the error
always names the offending key and lists what was legal.

## Nothing is wrong but nothing works either

```sh
bp doctor
```

`bp list` reads the planes directory, so there are things it structurally cannot
see: a worktree belonging to a plane whose file was never written, a project
directory holding a half-fetched repository, a branch somebody switched away
from before the plane was destroyed. `doctor` sweeps the source repositories and
both data directories and reports everything bitplane has a claim on.

```
findings
  ~/.local/share/bitplane/projects/codestyle
    holds repo.git but no project.toml, so it is not a project
    bp project add git@gitlab.com:acme/codestyle.git will reuse the objects

1 finding
```

**It reports and never repairs.** Nothing is deleted, rewritten or fixed, and
every finding names the command that fixes it — sometimes a `git` command rather
than a `bp` one:

```
  @api  refs/heads/feat-a
    no worktree and no plane holds this branch
    git -C ~/.local/share/bitplane/projects/api/repo.git branch -D feat-a removes it
```

bitplane prints that command rather than running it. Deciding a branch is
disposable is a judgement about your work, and it is not bitplane's to make.

The branch sweep and the worktree sweep need `--sweep-source-repos`, which is
off by default because it costs one git invocation per project. Everything else
`doctor` does is filesystem-only.

## "Another bitplane process holds it"

```
error[lock_timeout]: timed out waiting for the lock on ~/planes/bp-a3f9c2e1/.bitplane/lock

remedy: Another bitplane process holds it; retry once that one finishes.
```

Exit `5`, and it means **try again** and nothing else. bitplane tried the lock
without blocking, printed a `waiting for …` line, and blocked for two minutes — a
cold fetch of a large repository genuinely takes that long — before giving up.
When the other process finishes, the identical command works.

Reads never take locks, so `bp list`, `bp show` and `bp status` are always
available, even mid-operation.

## The plane is fine, git is not

```
error[git_too_old]: git 2.35.9 is too old; bitplane requires git 2.36 or newer

remedy: Upgrade git to 2.36 or newer.
```

Exit `4`, checked once before anything is touched, so this never leaves a plane
half-built. See [the requirement](../install.md#before-anything-git-236).

## Quick reference

| what you see | what to run |
| --- | --- |
| `create never completed` | `bp destroy -p <id>` |
| `plane_incomplete` from anything else | `bp destroy -p <id>` |
| you `mv`'d the plane, or a worktree in it | `bp repair` |
| the plane file's id disagrees with its directory | `bp repair`, or `bp rename` again |
| `worktree missing` | `bp rm <member>`, or `bp add <member>:<branch>` |
| `source repo missing` | rescue by hand, then `bp rm <member> --waive source_repo_missing` |
| an interrupted `destroy` / `repair` / `rename` | run it again — they converge |
| `plane.toml: unknown key` | edit the file; nothing is auto-repaired |
| `lock_timeout`, exit `5` | wait, then run the same command |
| something nothing else can see | `bp doctor --sweep-source-repos` |

## Next

- [`bp repair`](../reference/plane/repair.md) ·
  [`bp doctor`](../reference/plane/doctor.md)
- [Exit codes](../reference/exit-codes.md) — what each one means, and why `3` is
  a success
- [The ownership boundary](../concepts/ownership-boundary.md) — why bitplane
  reports rather than fixes
