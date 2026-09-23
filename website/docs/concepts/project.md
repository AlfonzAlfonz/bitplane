---
title: The project
---

# The project

A **project** is one repository that bitplane knows about by name, plus whatever
[scripts](./script.md) and settings you have given it. Register a repository
once and you can put it in any plane by name for ever after:

```sh
bp project add git@gitlab.com:acme/api.git     # once
bp create @api @web -b feat-login              # every time after that
```

A project is exactly **a directory containing a `project.toml`**, under
`$XDG_DATA_HOME/bitplane/projects/`. The set of projects on your machine is the
set of directories in there, which is why nothing has to be kept in sync with
anything.

```
~/.local/share/bitplane/projects/api/
  project.toml
  repo.git/       the repository bitplane fetches into
  bin/            yours; put executables here and its scripts will find them
```

Registering is optional. A repository you have not registered can still go in a
plane — see [ad-hoc members](./member.md#ad-hoc-members) — it just has no name,
no scripts and nothing to fetch.

## Owned or adopted

There are exactly two kinds of project, and the difference decides everything
else.

|  | **owned** | **adopted** |
| --- | --- | --- |
| registered with | `bp project add <url>` | `bp project adopt <path>` |
| the repository is | one bitplane built, bare, at `repo.git` | **yours**, wherever it already is |
| bitplane writes config in it | yes | **never** |
| bitplane deletes branches in it | yes, on `destroy` | **never** |
| there is something to fetch | yes | no — it is your checkout |
| it can be sitting on a branch | no, it is bare | yes, whichever you are on |

The word is **ownership**, not where the repository came from, because ownership
is what every one of those behaviours turns on. Told that a project is owned,
you can predict the whole of the rest of that column.

### Owned

```sh
bp project add git@gitlab.com:acme/api.git
```

bitplane builds a bare repository at `repo.git`, points it at the URL, and
fetches. Bare means **no working tree**, which means it is sitting on no branch
at all — so no plane can ever collide with it.

And because bitplane built it, it owns what is in it. The forge's branches live
in `refs/remotes/origin/*`, and `refs/heads/*` holds **exactly** the branches
your plane members were created on. That is what lets
[`bp destroy`](../reference/plane/destroy.md) delete a member's branch when the
plane goes: the branch is the responsibility of the plane that made it, and
nothing else was ever in there.

Once you have registered it, [`bp project fetch`](../reference/project/fetch.md)
brings it up to date, and `bp create` fetches before it builds worktrees, so a
plane starts from your colleagues' latest work rather than from whenever you
last looked.

### Adopted

```sh
bp project adopt ~/projects/bitplane
```

Now `@bitplane` names the checkout you already have, **in place**. bitplane does
not move it, does not copy it, does not convert it into anything. Every path on
your machine pointing at that repository keeps working, including the shell
you are sitting in.

The price of that is an asymmetry worth stating plainly: **bitplane owns the
refs and the config in a repository it built, and owns nothing in a repository
it merely pointed at.** With nothing stored, it cannot tell whether a branch in
your repository is one it created last Tuesday or one you have had for two
years — so it touches none of them. Worktrees come and go; branches stay.

The one thing you will actually notice is that your checkout is sitting on a
branch, and git will not let two worktrees of one repository hold the same
branch:

```
error[branch_occupied]: main is already checked out in /Users/alfonz/projects/bitplane

  /Users/alfonz/projects/bitplane  its working tree is on main

remedy: Check out a different branch there, or give this member a branch no worktree holds.
```

That refusal can only happen for an adopted project or an ad-hoc member. An
owned project holds no branch to collide with.

## The name

A project's name is a **path**: `/`-separated segments, each lowercase
`[a-z0-9][a-z0-9._-]*`, unique on your machine. `--name` overrides the default,
and the default is *use the namespace when there is one*:

- from a **URL with a host**, the whole path after it —
  `@acme/platform/tooling/codestyle` from
  `git@gitlab.com:acme/platform/tooling/codestyle.git`. A forge path is a
  namespace: everyone who clones the repo agrees on it.
- from a **path on your disk**, the last segment — `@bitplane` from
  `~/projects/bitplane`. Where your home directory sits is an accident, and
  `@users/alfonz/projects/bitplane` would name your machine, not the project.

That is what stops `acme/platform/codestyle` and `acme/infra/codestyle` from
both wanting to be `@codestyle`, and it is why importing a whole group tree
needs no `--name` per repo.

A derived name is **lowercased silently** — two names differing only in case
are one directory on a case-insensitive filesystem, so there is nothing there
to guess at. A name you type is never rewritten.

Two names cannot **nest**: `@acme` and `@acme/codestyle` cannot both exist,
because the second would live inside the first's project directory, where
removing `@acme` would take it along without mentioning it. That refusal cannot
be waived.

When a name is already taken, bitplane **refuses**, rather than quietly picking
something else. If the project holding it came from the same URL, you already
have that repo and there is nothing to do; if it came from a different one, you
get the next free name:

```
error[project_name_taken]: acme/codestyle is already a project

remedy: acme/codestyle-2 is free; re-run with --name acme/codestyle-2.
```

Two commands instead of one, deliberately. A default that renames itself behind
your back is worse than an error that names the conflict.

Projects can be renamed later with
[`bp project rename`](../reference/project/rename.md), which moves the project
directory and sweeps every plane holding it. Planes keep the subdirectory names
they were built with.

## The `@` sigil

You write a project as `@api`, or `@acme/codestyle`. The `@` is **syntax, not
part of the name** —
bitplane strips it when it reads your command line, and the bare name is what
appears in `project.toml`, in `BITPLANE_PROJECT` and in every message.

It is **required wherever a path would also be accepted**, which is `bp create`,
`bp add` and `bp rm`, because that is the only place the two could be confused.
The sigil is the whole of what keeps them apart: `@acme/codestyle` is a
project, and bare `acme/codestyle` is a relative path.
Everywhere else it is optional: `bp project show api` and
`bp project show @api` are the same command. bitplane always prints a project
with the sigil, so its output is unambiguous even where its input did not need
to be.

## The default branch is a base, not a destination

[`bp project show`](../reference/project/show.md) reports a `default` branch.
It is easy to read that as *the branch a worktree lands on*. It is not.

The default branch is the **base a new branch is cut from**. `bp create @api -b
feat-login` creates `feat-login` starting at it — and if `feat-login` already
exists, the default branch plays no part at all. bitplane never picks a branch
for your worktree; you name it, every time.

It is resolved from git on every call, never stored: `refs/remotes/origin/HEAD`
first, then the repository's own `HEAD`, and otherwise it is simply unspecified.
Unspecified is not an error by itself — it only becomes one at the moment a
**new** branch actually has to be cut from it.

## `bin/`, and the trust boundary

`<project-dir>/bin` is **prepended to `PATH`** for every script that project
runs, so a project can ship its own executables and have them win.

That directory is **yours to create, and bitplane never writes into it** — same
as `project.toml` itself. This is the whole reason scripts are safe to run
without asking: everything bitplane executes on your behalf is something you
typed into your own data directory. A script declared inside a repository you
cloned would mean `bp project add <url>` runs a stranger's code, which is why
declaring scripts in the repository is out of scope rather than merely
unbuilt.

## Next

- [The member](./member.md) — how a project becomes a worktree in a plane
- [Scripts](./script.md) — what a project can run for you
- [`bp project add`](../reference/project/add.md) ·
  [`bp project adopt`](../reference/project/adopt.md)
