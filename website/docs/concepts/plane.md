---
title: The plane
---

# The plane

A **plane** is a named set of git worktrees that share one lifecycle. You make
them together, you work in them together, and you take them away together. One
plane is one directory:

```
~/planes/auth-work/
  plane.toml
  acme/api/          a worktree of the api repository
  acme/web/          a worktree of the web repository
  .bitplane/         bitplane's own files for this plane
```

That directory is the plane. Its **name is its identity** — `auth-work` is the
plane id, and renaming the plane is moving the directory. No index anywhere
lists your planes; `bp list` finds them by looking in `~/planes` for directories
with a `plane.toml` in them.

Multi-repo is the normal case, not a feature. A plane holding one worktree is
perfectly legal — it is just the smallest plane.

## One worktree per repository

A plane holds **at most one worktree of any given repository**. You cannot have
`api` on `feat-login` and `api` on `hotfix` in the same plane. Two branches of
one repository is two planes.

This is not a limitation that got left in; it is what makes a worktree's
location predictable. A worktree's path inside the plane is derived from where
its repository came from, with no branch component to tell two of them apart:

| the repository came from | the worktree lands at |
| --- | --- |
| `git@gitlab.com:acme/codestyle.git` | `acme/codestyle` |
| `~/projects/bitplane` | `projects/bitplane` |

Two worktrees of one repository would derive the same path, so the rule and the
layout are the same fact said twice.

The path is derived **once, when the member is created**, and never recomputed.
An existing plane keeps the layout it was built with even if a later version of
bitplane derives paths differently.

## A plane has no branch

This surprises people, so it is worth saying flatly: **there is no such thing as
the plane's branch.**

```sh
bp create @api @web -b feat-login
```

`-b feat-login` applies `feat-login` to each member **at that moment**, and then
the name is gone. Nothing records it. From then on, a member's branch is
whatever its worktree is sitting on right now, read out of git when you ask.
Switch `@api` to something else and bitplane will report the something else,
without comment — your `HEAD` is yours.

Which is why [`bp add`](../reference/plane/add.md) makes you name a branch every
time, rather than reusing "the plane's branch": there is nothing to reuse, and
guessing from what the other members happen to be on today would make the
command depend on what you did in an unrelated worktree last week.

## The plane file

`plane.toml`, at the root of the plane directory — one per plane, not one per
worktree.

```toml
version = 1
id = "auth-work"

[members]
"acme/api" = "@api"
"acme/web" = "@web"
"projects/bitplane" = "/Users/alfonz/projects/bitplane"
```

Three keys, and that is the whole format. `[members]` is keyed by where the
worktree is, relative to the plane directory; each value says what it is a
worktree **of** — `@name` for a [project](./project.md), an absolute path for an
[ad-hoc member](./member.md#ad-hoc-members).

What is **not** in it is the interesting part. No branch, because git knows the
branch and knows it better. No status, because reading a file cannot tell you
whether a worktree still exists. No host, no timestamps, no "last known good".
Every candidate field was either something git or the filesystem already stored,
or something that would have made reading a plane *write* to it.

It lives at the plane directory root, outside every repository in the plane, so
it can never be committed by accident and needs no ignore rules anywhere.

`.bitplane/` beside it is reserved for bitplane's own per-plane files — script
logs and the lock. A repository whose derived path would start with that segment
is refused rather than allowed to collide.

## Plane ids are meant to be changed

Create a plane without naming it and you get a generated id:

```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1
```

The `bp-` prefix means *nobody chose this name*. It is reserved: you cannot pick
an id starting with `bp-`, which keeps the signal reliable. Chosen ids are
lowercase, `[a-z0-9][a-z0-9._-]*`, 64 characters at most — lowercase only so two
ids can never collide on a case-insensitive filesystem.

A wall of `bp-a3f9c2e1` directories is unnavigable, so
[`bp rename`](../reference/plane/rename.md) exists and renaming is expected.
The consequence is the thing to remember: **a plane id is not safe to store.**
Nothing outside bitplane should write one down and expect it to resolve later —
there is no alias to a former id, and worse, the old id may since have been
taken by a different plane.

## You are usually standing in one

Almost every command works out which plane you mean from your current
directory: bitplane walks up from where you are until it finds a `plane.toml`,
and the innermost one wins.

```sh
cd ~/planes/auth-work/acme/api
bp status              # reports auth-work — the whole plane, not just @api
```

Note what that does *not* do. Standing inside `@api`'s worktree does not make
`@api` the subject; a path resolves to a **plane**, never to a member. `bp rm`
still makes you name the member. And a plane is never a positional argument —
`-p auth-work` is the only way to name one explicitly, which keeps every
positional slot meaning a member, a branch, a script or a project.

## The lifecycle

| | |
| --- | --- |
| [`bp create`](../reference/plane/create.md) | make a plane and its worktrees |
| [`bp add`](../reference/plane/add.md) · [`bp rm`](../reference/plane/rm.md) | put a member in, take one out |
| [`bp status`](../reference/plane/status.md) · [`bp show`](../reference/plane/show.md) · [`bp list`](../reference/plane/list.md) | look, without changing anything |
| [`bp rename`](../reference/plane/rename.md) · [`bp repair`](../reference/plane/repair.md) | move it, or reconnect it after a move |
| [`bp destroy`](../reference/plane/destroy.md) | take the whole thing away |

`create` is the only one that is **all-or-nothing**: it either builds a whole
plane or leaves none behind. Everything destructive **converges** instead —
interrupt a `destroy` and run it again, and it finishes, reporting the members
that are already gone as `already gone`.

## A plane that never worked

Between the moment `create` claims its directory and the moment the last
worktree lands, a plane carries an **incomplete marker** — a file at
`.bitplane/incomplete` saying it was never finished. That window is cheap by
design: the fetch and the scripts are deliberately outside it, so everything
inside it is discardable work nobody has touched.

If `create` dies in there, the marker survives, and it says something no other
signal can — **this plane never worked**, as opposed to having worked and since
lost a worktree. It is a one-way latch: written at the claim, removed when the
last worktree lands, never written again, and only ever by `create`. That is why
every other command refuses to touch a latched plane and points at the one that
clears it:

```
error[plane_incomplete]: bp-a3f9c2e1 was never finished being created

remedy: Nothing in it is yours; run bp destroy -p bp-a3f9c2e1 to clear it.
```

And it is why `destroy` runs **no refusal checks at all** there. Nothing in that
directory was ever yours to lose. See
[Recover a plane that broke](../guides/recover-a-broken-plane.md).

## Next

- [The project](./project.md) — what a member is usually a worktree of
- [The member](./member.md) — including the repositories you never registered
- [The ownership boundary](./ownership-boundary.md) — what bitplane will never do
  to any of this
