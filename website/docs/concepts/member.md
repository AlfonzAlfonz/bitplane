---
title: The member
---

# The member

A **member** is one worktree in a plane, together with what it is a worktree of.
A three-repo plane has three members.

```
bp-a3f9c2e1  ~/planes/bp-a3f9c2e1

  @api                             feat-login  acme/api
  @web                             feat-login  acme/web
  /Users/alfonz/projects/bitplane  main        projects/bitplane
```

Each row is a member. The first column is how you name it — and it is also how
bitplane names it back to you, everywhere, in fan-out rows, in refusals and in
findings.

## Naming a member

Two forms, and they cannot be confused with one another:

| form | example | what it is |
| --- | --- | --- |
| `@<name>` | `@api` | a [project](./project.md) you registered |
| a path | `~/projects/bitplane` | an [ad-hoc member](#ad-hoc-members) |

A project name can contain neither `@` nor `/`, so anything that is not a
project name is read as a path. A path may be relative, absolute or start with
`~`; bitplane canonicalises it, following symlinks, and the **absolute** result
is what the plane records.

The full grammar, including the `:branch` suffix and the one path that cannot
carry one, is on [Member syntax](../reference/member-syntax.md).

## A member is not a path, and not a project

Three distinctions that catch people out.

**A member is not its worktree's path.** `bp rm acme/api` does not work;
`bp rm @api` does. The path is where the worktree happens to live today, and
[`bp repair`](../reference/plane/repair.md) is allowed to change it.

**Standing in a worktree does not name its member.** `cd`ing into `acme/api` and
running a command resolves the **plane**, never the member. `bp rm` still makes
you say `@api`. A subject that is sometimes implicit and sometimes not is a
subject you cannot predict.

**Removing a member is not removing a project.**
[`bp rm @api`](../reference/plane/rm.md) takes `@api`'s worktree out of *this
plane* and touches nothing else — not the project, not its repository, not any
other plane. Removing a project is [`bp project rm`](../reference/project/rm.md),
and project removal is never spelled as a bare verb.

## Every member names its own branch

A member's branch is given when the member is made, either by `-b` for all of
them or by a suffix for one:

```sh
bp create @api @web:hotfix -b feat-login
# @api -> feat-login
# @web -> hotfix
```

After that, nothing records it. The branch you see in any listing is read from
the worktree's `HEAD` at the moment you ask, so it is true by construction, and
switching branches in a worktree is ordinary use that bitplane simply reports.

Neither `create` nor `add` will guess a branch for you. There is no plane-level
branch to inherit, and falling back to the repository's own default branch would
be worst exactly where it was meant to help: for an adopted project or an ad-hoc
member, that default is the branch your checkout is already sitting on, so the
convenience would resolve straight into `branch_occupied`.

## Ad-hoc members

You do not have to register a repository to put it in a plane. Name a path and
bitplane makes a worktree of it:

```sh
bp create ~/projects/bitplane -b feat-x
```

That is an **ad-hoc member**. It behaves exactly like an
[adopted project](./project.md#adopted), minus the registration: bitplane never
moves it, never converts it, never deletes a branch in it, and has nothing to
fetch for it.

It is a kind of **member**, not a kind of project. It has no `project.toml`, so:

- it has **no name** — it is identified by its absolute path, which is what
  appears in every row and every message;
- it has **no `bin/`** on `PATH`;
- it has **no scripts**, and behaves exactly like a project with none declared —
  silently, not as an error. [`bp run`](../reference/plane/run.md) against one
  *is* an error, because `bp run` takes a project.

This is the right default for a repository you are touching once. If you find
yourself typing the same path into `bp create` every week, that is the moment to
[adopt it](../guides/adopt-an-existing-checkout.md) — the checkout does not move
and nothing you have breaks, and you get `@name`, scripts and `bin/`.

Ad-hoc members are not second-class in the things that matter.
[`bp status`](../reference/plane/status.md) reports them,
[`bp destroy`](../reference/plane/destroy.md) checks them for uncommitted work
and refuses over it exactly as hard, and
[`bp doctor --sweep-source-repos`](../reference/plane/doctor.md#the-four-sweeps)
sweeps their repositories, whose paths it reads out of the plane files.

## One worktree per repository, checked properly

A plane holds at most one worktree of any given repository, and bitplane means
*repository*, not *the string you typed*. Naming the same repository twice by
two different paths is caught:

```
error[same_repository]: /Users/alfonz/projects/api/../side is a worktree of the same repository as /Users/alfonz/projects/api

remedy: A plane holds at most one worktree per repository; name one of them.
```

Two *different* repositories that would land in the same place are caught too,
since worktree paths are derived from the source and two sources can derive
alike:

```
error[member_path_collision]: /Users/alfonz/one/repos/api and /Users/alfonz/two/repos/api would both land at repos/api

remedy: A plane holds one worktree per derived path; put one of them in another plane.
```

## Next

- [Member syntax](../reference/member-syntax.md) — the exact grammar
- [The project](./project.md) — what registering one buys you
- [The ownership boundary](./ownership-boundary.md) — what bitplane does inside
  a worktree, and what it never does
