---
title: The ownership boundary
sidebar_label: Ownership boundary
---

# The ownership boundary

One sentence decides what bitplane is:

> **bitplane owns a worktree's existence and its location. You own its contents
> and its `HEAD`.**

bitplane makes a worktree, moves it, repairs git's record of it, and removes it.
Between the making and the removing, the directory is yours. It will **never**
commit, push, stash, check out, merge, rebase or reset in there — not helpfully,
not on request, not with a flag.

## What follows from it

| bitplane does | bitplane does not |
| --- | --- |
| `git worktree add` / `remove` / `repair` | `git commit`, `push`, `stash`, `checkout`, `reset` |
| create the branch a new worktree lands on | move a branch you are working on |
| delete a member's branch on `destroy`, in a repository it built | delete any branch in a repository you brought |
| read `git status` across a plane and render it | store what it read, or form an opinion about it |
| refuse to destroy work you have not saved | decide your work is disposable because it was merged |

Stated as ownership rather than as a list of permitted commands because it
decides the cases a list would not. `git worktree repair` is obviously in
scope — it fixes a worktree's location, which bitplane owns. `git stash` is
obviously out — it touches contents, which it does not. Neither needed a rule of
its own.

## Working in a plane is not drift

Everything in this list is **ordinary use**, and bitplane reports all of it at
exit `0` without comment:

- uncommitted changes, staged or not
- untracked files
- commits you have not pushed
- switching a worktree to a different branch
- detaching `HEAD`
- an interrupted rebase or merge sitting in the worktree
- your own scratch files anywhere in the plane directory

There is nothing to "reconcile", because nothing was recorded that could
disagree with reality. A plane file carries no branch and no status, so a member
on a different branch than the one you created it on is not a discrepancy — it
is Tuesday.

**Drift** is a narrower word, and it is only about the two things bitplane owns:

| finding | what it means |
| --- | --- |
| member worktree missing | the plane lists a member whose directory is not there |
| prunable | git reports a worktree at a path that no longer exists |
| source repo missing | the repository a member came from is gone |
| create never completed | the plane was claimed and never finished |
| id mismatch | the plane file's id disagrees with its directory name |

Those, plus a file bitplane needed and could not parse, are what
[exit `3`](../reference/exit-codes.md#3-is-a-success-and-it-is-not-about-your-work)
means. Your filthy worktree is never one of them.

## Reading is allowed; writing is not

[`bp status`](../reference/plane/status.md) shells out to `git status` in every
member and renders the answers. That is not a violation of "bitplane keeps no
state" — it is the opposite of one. Nothing is stored, so nothing can go stale;
what you get is git's answer at the moment you asked.

The objection to tracking state is about **storage and judgement**, and reading
is neither.

:::note[One honest asterisk]

`git status` refreshes the per-worktree index, so running `bp status` does make
git write a file. That index is per-worktree and never shared, and it is git's,
not bitplane's. *bitplane* stores nothing — that is the rule, and this is the
one place worth spelling it out so it is not later rediscovered as a bug.

:::

## Destroying is where the boundary gets tested

`bp destroy` is the one command whose whole job is removing your directories, so
it is where the boundary has to be sharpest.

It checks **every** member before touching **anything**, and reports every
reason at once — a half-destroyed plane is worse than a refused one. Five
reasons block it: uncommitted changes, untracked files, unpushed commits, a
locked worktree, and a source repository it cannot find.

Accepting the loss is done **one named reason at a time**:

```sh
bp destroy --waive uncommitted --waive unpushed
```

There is **no `--force`**. A waiver says *"I accept losing this work, which I am
looking at"*, and that sentence is only honest about a specific thing you were
shown. A blanket flag would let a waiver you meant for one member silently cover
a reason you never saw. Waivers are per invocation and never stored.

Where that sentence cannot be said honestly, there is no waiver at all. Removing
a project whose worktrees are live in other planes damages planes you did not
mention and are not looking at, so it is refused outright — and the error lists
the blocking planes by id, because the whole value of an unwaivable refusal is
that the way out is obvious.

### What is not a reason to destroy something

bitplane will not judge your branch merged, and therefore disposable. *"The
branch is merged into its base"* and *"the branch is gone on the remote"* are
**not** staleness signals and never become ones. They are opinions about how you
use git — plenty of people keep a branch alive after merge, and plenty of forges
delete branches on merge whether you were finished or not.

The signals bitplane is willing to act on are only things it owns: the worktree
directory is missing, git says `prunable`, the source repository is gone, the
plane never finished being created, nothing has touched it in N days. Every one
of those is about the *existence* of something bitplane made.

And the veto beats all of them. Uncommitted or unpushed work overrides every
staleness signal there is.

## Repositories you brought

The boundary hardens at the edge of a repository bitplane did not build. In an
[adopted project](./project.md#adopted) or an
[ad-hoc member](./member.md#ad-hoc-members), bitplane:

- writes **no** config;
- creates and deletes **no** branches — a worktree goes, no ref is touched;
- has nothing to fetch, because the repository has a writer already: you.

It cannot tell a branch it created from one you have had for two years, because
it recorded nothing. So it touches none of them. The single mark it leaves is
git's own administrative record of a worktree, inside your repository's `.git/`,
and destroying the plane that owns it removes it.
[`bp doctor --footprint`](../reference/plane/doctor.md#--footprint) lists every
one of them.

## Two more edges

**git protocol yes, forge API no.** bitplane speaks git. It does not open pull
requests, merge, review, or read CI status. That is a different tool's job and
it is not a feature waiting to be added.

**git's credentials, never bitplane's.** It shells out to `git` and lets your
credential helpers and `ssh-agent` do the work — it never reads, stores, prompts
for or forwards a credential. A private repository you cannot `git clone` by
hand is one bitplane cannot use either. That is the auth-shaped restatement of
the same rule, and it is what keeps a secrets story out of the tool entirely.

## Next

- [Recover a plane that broke](../guides/recover-a-broken-plane.md) — the
  boundary in practice, when something has gone wrong
- [Refusals and waivers](../reference/refusals-and-waivers.md) — every refusal,
  with its exact wording
- [Exit codes](../reference/exit-codes.md#3-is-a-success-and-it-is-not-about-your-work)
  — what `3` does and does not mean
