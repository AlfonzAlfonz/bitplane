---
title: Adopt an existing checkout
sidebar_label: Adopt a checkout
---

# Adopt an existing checkout

:::not-implemented

[`bp project adopt`](../reference/project/adopt.md) does not exist yet, so
nothing on this page can be registered — and with no `project.toml` to declare
them in, neither can the scripts adopting is mostly *for*. The
[ad-hoc route](#the-shortcut-skip-adoption-entirely) at the bottom of this page
works today, and is the same thing without the name.

:::

You already have `~/projects/bitplane`, you have had it for two years, and there
are shell aliases, editor projects and an `.envrc` pointing at it. You want it
in planes without any of that breaking.

**Adopt it.** bitplane records where it is and leaves it exactly where it is.

```sh
bp project adopt ~/projects/bitplane
```

```
@bitplane  adopted
  source     /Users/alfonz/projects/bitplane
  directory  ~/.local/share/bitplane/projects/bitplane
  default    main
```

Your checkout has not moved, has not been copied, and has not been converted
into anything. What was created is a small directory elsewhere holding a
`project.toml` that points at it. There is no `repo.git` — your checkout **is**
the repository.

Name it something else with `--name` if the last path segment is not what you
want to type.

## What you get

`@bitplane` now works everywhere a project is accepted:

```sh
bp create @bitplane @api -b feat-x
```

And the two things a path could never give you:

- **[Scripts](../concepts/script.md).** Declare them in the project's
  `project.toml` and they fire when a worktree is made or removed, and run on
  demand with `bp run @bitplane <script>`.
- **`bin/` on `PATH`.** Drop an executable in
  `~/.local/share/bitplane/projects/bitplane/bin/` and this project's scripts
  find it first.

## What bitplane will not do to it

This is the sharpest asymmetry in the design, and it is on purpose: **bitplane
owns the refs and the config in a repository it built, and owns nothing in a
repository it merely pointed at.**

| | owned | **adopted** |
| --- | --- | --- |
| writes git config in it | yes | **no** |
| deletes the member's branch on `destroy` | yes | **no** |
| has anything to fetch | yes | **no** |

With nothing recorded, bitplane cannot tell whether a branch in your repository
is one it created last Tuesday or one you have had since 2023. So it deletes
none of them. Destroying a plane removes the worktree; every branch stays.

The only mark it leaves inside your repository is git's own administrative
record of each worktree, under `.git/worktrees/`. Destroying the plane that owns
one removes it, and
[`bp doctor --footprint`](../reference/plane/doctor.md#--footprint) lists every
one that exists.

## The one thing that will bite you

Your checkout is sitting on a branch, and git will not let two worktrees of one
repository hold the same branch. Ask a plane for the branch you are standing on
and it is refused:

```sh
bp create @bitplane -b main
```

```
error[branch_occupied]: main is already checked out in /Users/alfonz/projects/bitplane

  /Users/alfonz/projects/bitplane  its working tree is on main

remedy: Check out a different branch there, or give this member a branch no worktree holds.
```

This is why neither `bp create` nor `bp add` will fall back to a project's
default branch when you leave `-b` off. For an adopted project the default
branch is usually exactly the branch your checkout is occupying, so the
convenience would resolve straight into this refusal for the members it was
meant to help. The default branch is a **base** to cut new branches from, never
a destination.

Two ways out: name a different branch, or switch your own checkout to something
else.

### A stale record of a worktree that is gone

If you deleted a plane's directory by hand rather than destroying it, git still
records the worktree and still considers its branch taken. The refusal is the
same; only the remedy changes:

```
error[branch_occupied]: feat-login is already checked out in /Users/alfonz/planes/bp-a3f9c2e1/projects/bitplane

  /Users/alfonz/planes/bp-a3f9c2e1/projects/bitplane  is gone, but git still records it as holding the branch

remedy: Run git worktree prune in /Users/alfonz/projects/bitplane to clear the stale record, then try again.
```

bitplane prints the command rather than running it. Pruning is repository-wide
and this repository is yours.

## Fetching

An adopted project has **nothing to fetch**, and naming one in
[`bp project fetch`](../reference/project/fetch.md) is not an error — it reports
`nothing to fetch (adopted)` and moves on.

The reason is not a prohibition. An owned `repo.git` is a cache with exactly one
writer, so if bitplane does not fetch it, nobody does. Your checkout already has
a writer: you. The consequence is worth knowing — `bp create` fetches every
owned project before it resolves branches, so an adopted member's new branch is
cut from whatever your checkout last fetched. A week-stale checkout gives you a
week-stale branch, quietly. Run `git fetch` there yourself before you build a
plane you care about.

## Adopted, or owned?

**Adopt** when the checkout is somewhere you already work: you have tooling
pointed at it, or you want to keep using it directly as well as through planes.

**`bp project add <url>`** when you do not have a checkout, or do not care about
the one you have. bitplane builds a bare repository it owns, which is sitting on
no branch — so nothing can ever collide — and which it can clean branches out of
when planes are destroyed.

You cannot convert one into the other, and that is deliberate: adopting never
touches your checkout, and adding always builds a repository. If you want the
owned behaviour for a repository you already have, register it from its URL and
keep your checkout as an unrelated directory.

## The shortcut: skip adoption entirely

You do not have to register anything. Name the path and bitplane makes a
worktree of it:

```sh
bp create ~/projects/bitplane -b feat-x
```

That is an [ad-hoc member](../concepts/member.md#ad-hoc-members). It is
adopted-shaped minus the registration: nothing moves, no branch is ever deleted,
nothing is fetched, and the `branch_occupied` refusal above applies identically.
What it lacks is the name, the scripts and `bin/` — it has no `project.toml` to
declare them in, and `bp run` against it is an error.

Use it for a repository you are touching once. When you notice you have typed
the same path three weeks running, adopt it — nothing breaks, and the planes you
already built carry on working.

## Next

- [The project](../concepts/project.md) — owned and adopted, side by side
- [Set up per-project scripts](./per-project-scripts.md) — the main thing
  adopting buys you
- [`bp project adopt`](../reference/project/adopt.md) — the full reference
