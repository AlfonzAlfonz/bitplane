---
title: Introduction
slug: /
---

# Introduction

A change rarely stops at one repository. The API needs a field, the web client
needs to read it, the docs need to say so — three repositories, three branches
that ought to have the same name, three working trees you have to make, keep
straight, and remember to take away again. A **plane** is that set: a named
group of git worktrees, one per repository, created together, torn down
together, living in one directory you can `cd` into. bitplane makes and unmakes
planes. That is the whole tool.

```
~/planes/auth-work/
  acme/api      on feat-login
  acme/web      on feat-login
  acme/docs     on feat-login
```

Three worktrees under one roof. `bp status` tells you what all three look like
without three `cd`s, and `bp destroy` takes all three away in one command —
refusing, member by member, if any of them still holds work you have not
committed or pushed.

## What it is not

bitplane is a thing that makes directories. Inside them, you are alone with git.

**It never commits, pushes, stashes, checks out or resets.** It creates a
worktree, moves it, repairs it and removes it; everything you do in between —
committing, rebasing, switching branches, abandoning the whole thing — is
ordinary use, not something to be tracked or corrected. See
[the ownership boundary](./concepts/ownership-boundary.md), which is the rule
the rest of the design is derived from.

**It speaks git, not GitHub.** No pull requests, no merges, no reviews, no CI
status. And no credentials of its own: bitplane shells out to `git` and lets
your credential helpers and `ssh-agent` do the work, so a private repository you
cannot `git clone` by hand is one bitplane cannot use either.

**It remembers almost nothing.** There is no database and no index. A plane *is*
a directory containing a `plane.toml`; a project *is* a directory containing a
`project.toml`. Every question about a branch, a status or a worktree's health
is answered by asking git at the moment you ask, which is why the answer cannot
be stale.

## Why not just `git worktree`

You can do all of this by hand, and plenty of people do. What you cannot easily
do by hand is the second half: knowing which six directories belonged together,
and taking them away without losing something. Planes exist for the teardown as
much as the setup — that is why `bp destroy` checks every member for uncommitted
changes, untracked files and unpushed commits before it removes anything, and
why accepting the loss is [one named reason at a
time](./reference/refusals-and-waivers.md#--waive) rather than a `--force`.

## The five words

Everything else on this site is built from these.

| word | what it means |
| --- | --- |
| **plane** | a named set of worktrees sharing one lifecycle, in one directory |
| **member** | one worktree in a plane, and what it is a worktree of |
| **project** | one repository, registered with bitplane, plus its scripts |
| **script** | a named command a project declares, run on demand or when a worktree is made or removed |
| **`bp`** | the command you type. `bitplane` is the project; `bp` is the binary |

## Where to go next

- [Installation](./install.md) — get `bp` onto your machine.
- [Run a task across several repos](./guides/run-a-task-across-repos.md) — the
  guide to read first, start to finish.
- [The plane](./concepts/plane.md) — the one noun everything else is defined
  against.
- [CLI reference](./reference/global-flags.md) — every command, flag and exit
  code.
