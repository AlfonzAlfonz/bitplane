---
title: Run a task across several repos
sidebar_label: Across several repos
---

# Run a task across several repos

:::in-progress

Every step on this page works today, with one exception: **scripts do not run
yet**, so step 2 leaves you with a plain checkout rather than an installed one.
Each step links to the reference page that states exactly what is real.

:::

A ticket lands: logins need a new field. It touches the API, the web client and
the docs — three repositories, one change. This guide takes you from nothing to
three worktrees on one branch, through the work, and back to nothing.

## 1. Register the repositories, once

```sh
bp project add git@gitlab.com:acme/api.git
bp project add git@gitlab.com:acme/web.git
bp project add git@gitlab.com:acme/docs.git
```

```
@api  owned
  source     git@gitlab.com:acme/api.git
  directory  ~/.local/share/bitplane/projects/api
  default    main
```

This is a once-per-repository step, not a once-per-task one. Each one builds a
bare repository under `~/.local/share/bitplane/projects/` that bitplane owns and
fetches into. See [`bp project add`](../reference/project/add.md).

Already have one of them checked out and want to keep working in it? Register
that instead, in place, with
[`bp project adopt`](../guides/adopt-an-existing-checkout.md) — nothing moves.

## 2. Make the plane

```sh
bp create @api @web @docs -b feat-login --id auth-work
```

```
auth-work  ~/planes/auth-work

  @api   feat-login  created (new branch)  acme/api
  @web   feat-login  created (new branch)  acme/web
  @docs  feat-login  created (new branch)  acme/docs
```

Three worktrees, all on `feat-login`, under one directory. What happened, in
order: each owned project was fetched, then the branch was resolved — it did not
exist, so it was created off each project's default branch — then the worktrees
were built, then any [`post_worktree_create` scripts](./per-project-scripts.md)
ran.

**`--id auth-work` is optional.** Without it you get a generated
`bp-a3f9c2e1`, which is fine for something you will finish today and
unnavigable once you have four of them. You can always
[rename it later](../reference/plane/rename.md).

**`-b feat-login` is not optional.** Every member needs a branch, and there is
no plane-level branch that could supply one — see
[a plane has no branch](../concepts/plane.md#a-plane-has-no-branch). One member
can differ:

```sh
bp create @api @web @docs:existing-docs-branch -b feat-login
```

If the branch already exists — say a colleague pushed it — `create` checks it
out instead of creating it, and the output tells you which happened. That
distinction is the difference between joining someone's work and starting your
own, which is why it is marked rather than implied. To be certain, say so:
`--new-branch` fails if it exists, `--existing-branch` fails if it does not.

## 3. Work

```sh
cd ~/planes/auth-work
```

Three ordinary git checkouts, in `acme/api`, `acme/web` and `acme/docs`. Nothing
about them is special — commit, rebase, switch branches, run your editor over
the whole plane directory at once. bitplane is not watching, and
[none of it counts as drift](../concepts/ownership-boundary.md#working-in-a-plane-is-not-drift).

What the plane buys you is asking about all three at once:

```sh
bp status
```

```
auth-work  ~/planes/auth-work

  @api   feat-login  3 modified, 1 untracked, 2 ahead
  @web   feat-login  clean
  @docs  feat-login  clean, 1 ahead
```

That is git's own answer for each member, rendered — nothing is stored, so
nothing can be stale. `ahead` counts commits not contained in any `origin/*`
ref, which is the question that actually matters (*is this recoverable if I lose
the directory*) rather than the one `git branch -d` asks. See
[`bp status`](../reference/plane/status.md).

Pushing is yours. bitplane never pushes.

## 4. It turns out to touch a fourth repository

```sh
bp project add git@gitlab.com:acme/infra.git     # not registered yet
bp add @infra:feat-login
```

```
auth-work  ~/planes/auth-work

  @infra  feat-login  created  acme/infra
```

The branch is mandatory here too, and for a sharper reason than on `create`:
deriving it from what the plane's other members happen to be on would make
`bp add` depend on what you did in an unrelated worktree last week. Write it as
a suffix, or pass `-b`. See [`bp add`](../reference/plane/add.md).

And if it turns out *not* to touch `@docs` after all:

```sh
bp rm @docs
```

Same rules as a destroy, for one member: it checks that member for work you have
not saved and refuses if it finds any. Removing a member does **not** remove the
project — [`bp rm`](../reference/plane/rm.md) takes a worktree out of this
plane and touches nothing else.

## 5. Take it away

Merged and pushed? Remove the whole thing in one command:

```sh
bp destroy
```

```
auth-work  ~/planes/auth-work

  @api    feat-login  removed  acme/api (branch deleted)
  @web    feat-login  removed  acme/web (branch deleted)
  @docs   feat-login  removed  acme/docs (branch deleted)
  @infra  feat-login  removed  acme/infra (branch deleted)

destroyed auth-work
```

The branches go with the worktrees, because these are **owned** projects: the
repository bitplane built holds the forge's branches somewhere else entirely, so
everything in its `refs/heads/*` is a branch a plane put there. In an
[adopted project or an ad-hoc member](../concepts/project.md#adopted) the
worktree goes and no ref is touched — bitplane owns nothing in a repository you
brought.

Everything else in the plane directory — your notes file, your scratch space —
goes with the directory. The plane directory is yours between `create` and
`destroy`, and `destroy` is the end of that.

### When it refuses

```
error[refused]: refusing to destroy auth-work: 2 of 4 members have work that would be lost

  @api  feat-login has uncommitted changes
  @api  feat-login has commits that are not on origin
  @web  feat-login has untracked files

remedy: Inspect the members listed. Re-run with --waive uncommitted --waive untracked --waive unpushed to accept losing that work.
```

**Nothing has been touched.** Every reason across every member came back at
once, deliberately — one look, one decision, one re-run.

Go and look at what it named. Then either save the work, or accept losing it one
reason at a time:

```sh
bp destroy --waive uncommitted --waive untracked --waive unpushed
```

The remedy line names exactly the reasons that were raised, in order, so you can
paste it. There is no `--force`, ever, and a waiver is never remembered: the
next `destroy` asks again. See
[Refusals and waivers](../reference/refusals-and-waivers.md).

## The whole thing

```sh
bp project add git@gitlab.com:acme/api.git      # once per repository
bp project add git@gitlab.com:acme/web.git
bp project add git@gitlab.com:acme/docs.git

bp create @api @web @docs -b feat-login --id auth-work
cd ~/planes/auth-work
# … work, commit, push …

bp status
bp destroy
```

## Next

- [Set up per-project scripts](./per-project-scripts.md) — make step 2 leave you
  with a worktree that is ready to use
- [Adopt an existing checkout](./adopt-an-existing-checkout.md) — use a
  repository you already have
- [Recover a plane that broke](./recover-a-broken-plane.md) — when one of these
  steps does not go to plan
