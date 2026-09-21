---
title: bp status
---

# `bp status`

Git's own status, across every member of a plane, in one command.

```
bp status [-p <plane>]
```

This is most of what a multi-repo plane manager is for: the state of six
worktrees without six `cd`s.

`status` **stores nothing**. It shells out to git, renders the answers, and
keeps none of them — so what it tells you cannot go stale. It is git's answer,
not `bp`'s opinion.

## Arguments

None. The plane comes from the current directory, or from `--plane`. See
[how the current directory resolves](./show.md#how-the-current-directory-resolves).

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane to report on. |

Plus the [global flags](../global-flags.md#global-flags).

## Output

One line per member, then the details for the ones that have any.

```
$ bp status
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api   feat-login  3 modified, 1 untracked, 2 ahead
  @web   feat-login  clean
  @docs  feat-login  clean, 1 ahead
```

| word | what it counts |
| --- | --- |
| `modified` | tracked files with changes, staged or not |
| `untracked` | files git does not know about |
| `ahead` | commits on this branch not contained in any `origin/*` ref |
| `clean` | none of the above |

`ahead` uses the same predicate as the [`unpushed`
refusal](../refusals-and-waivers.md#the-five-waivable-reasons): containment in any
remote-tracking ref of `origin`, never `git branch -d`'s upstream comparison,
which reports a branch already safe on the forge as unmerged.

No fetch is performed. A stale remote-tracking ref can only make `ahead` too
large, never too small.

## None of this is drift

A worktree with uncommitted changes, untracked files, unpushed commits, a
branch you switched, a rebase in progress — all of that is **ordinary use**, and
`bp status` reports it at exit `0`.

`bp` owns a worktree's **existence and location**; its contents and its `HEAD`
are yours. Between `bp create` and `bp destroy` the directory is yours to do
anything with, and `bp` never commits, pushes, stashes, checks out or resets.

What `bp status` exits `3` for is a finding about the things `bp` does own —
see [drift](../exit-codes.md#3-is-a-success-and-it-is-not-about-your-work).

:::note[This read makes git write]

`git status` refreshes the per-worktree index, so running `bp status` can touch
`index.lock`. That lock is per-worktree and never shared, and `bp` itself stores
nothing — the "a read never writes" rule is about `bp`'s own state. Recorded
here so it is not rediscovered as a bug.

:::

## Examples

### A plane with work in it

Exit `0`. Nothing here is a problem.

```
$ bp status
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api   feat-login  3 modified, 1 untracked, 2 ahead
  @web   feat-login  clean
```

### A member whose worktree is gone

Exit `3`. git has nothing to say about a directory that is not there, and that
absence **is** a finding: `bp` owns the worktree's existence.

```
$ bp status
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api   feat-login  3 modified, 1 untracked
  @web   -           worktree missing

findings
  @web  the worktree at acme/web is not there
        bp rm @web drops it; bp add @web:<branch> puts it back
```

### A detached HEAD

Exit `0`. Detaching is ordinary use.

```
$ bp status
```
```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api  (detached at 9f2c1ab)  clean
```

## Exit codes

| code | when |
| --- | --- |
| `0` | every member answered, whatever it said |
| `2` | bad arguments, or no plane found |
| `3` | the plane reported a finding |
| `4` | git is missing, unusable or older than 2.36 |

## See also

- [Exit codes](../exit-codes.md#3-is-a-success-and-it-is-not-about-your-work) — what `3` does and does not mean
- [`bp show`](./show.md) — the plane itself rather than git's view of it
