---
title: Member syntax
---

# Member syntax

A **member** is one worktree in a plane, together with what it is a worktree of.
`bp create` and `bp add` take members; `bp rm` takes members that are already in
the plane. This page is how you write one.

## The two forms

| form | example | what it is |
| --- | --- | --- |
| `@<name>` | `@codestyle` | a **registered project** — see [`bp project add`](./project/add.md) |
| a path | `~/projects/bitplane` | an **ad-hoc member**: a repo that is not a registered project |

```
bp create @api @web -b feat-login          # two registered projects
bp create ~/projects/bitplane -b feat-x    # one ad-hoc member
bp create @api ~/projects/bitplane -b feat-x   # both, in one plane
```

The two cannot be confused. A project name is `[a-z0-9][a-z0-9._-]*`, so it can
contain neither `@` nor `/`, and anything that is not a project name is read as
a path.

### The `@` sigil

The `@` is **syntax, not part of the name**. `bp` strips it once, when it reads
your command line, and the bare name is what appears in `BITPLANE_PROJECT`, in
`project.toml` and in every message.

It is **required wherever a path would also be accepted** — that is, on
`bp create`, `bp add` and `bp rm` — and **optional everywhere else**:
`bp project show codestyle` and `bp project show @codestyle` are the same
command. `bp` always prints a project with the sigil.

### Paths

A path member may be relative, absolute, or start with `~`. `bp` canonicalises it
— following symlinks — before doing anything with it, and the **absolute**
result is what the plane records and what identifies the member afterwards.

The path must be a git repository. It is treated exactly like an **adopted**
project minus the registration: `bp` never moves or converts it, never deletes a
branch in it, has nothing to fetch for it, and it has no `project.toml`, so it
has **no scripts**. [`bp run`](./plane/run.md) against one is an error.

## The branch suffix

A member may carry its branch, separated by a colon:

```
@api:feat-login
~/projects/bitplane:feat-x
```

The branch is everything after the **last** colon. Git forbids a colon in a ref
name, so whatever follows that colon is unambiguously a branch — but what
*precedes* it is not always unambiguously a whole path. See
[the one path a suffix cannot express](#the-one-path-a-suffix-cannot-express)
below.

A member with a branch suffix overrides `-b` for that member alone:

```
bp create @api @web:hotfix -b feat-login
# @api  -> feat-login

# @web  -> hotfix

```

**`bp add` requires a branch for every member**, either as a suffix or via `-b`.
A plane has no branch, and nothing is derived from what the other members happen
to be on — that would make `bp add`'s behaviour depend on what you did in an
unrelated worktree last week.

`bp create` requires one too. A member's own default branch is not a fallback:
for an adopted project or an ad-hoc member that is the branch the user's
checkout occupies, so falling back to it would refuse. With neither `-b` nor a
suffix the request is
[`branch_unspecified`](./refusals-and-waivers.md#usage-failures).

### The one path a suffix cannot express

A path whose **last segment contains a colon** cannot carry a branch suffix,
because the split cannot tell the two apart. `bp` refuses it rather than
guessing:

```
$ bp create ~/projects/weird:name -b feat-x
```
```
error[member_path_ambiguous]: ~/projects/weird:name could be a path or a member with a branch suffix

remedy: Register it with `bp project adopt` and use its @name instead.
```

`bp project adopt` takes a path and no branch, so it has no such ambiguity;
adopting the repo once makes it addressable as `@name` forever after.

## What a member is not

- **Not a worktree path.** `bp rm acme/api` does not work; `bp rm @api`
  does. The path is where the worktree happens to live, and
  [`bp repair`](./plane/repair.md) is allowed to change it.
- **Not unique per `(project, branch)`.** A plane holds **at most one worktree
  per project**. Two branches of one repo is two planes. A second member
  resolving to a project the plane already holds is
  [`duplicate_member`](./refusals-and-waivers.md#usage-failures).
- **Not a plane.** See [the plane argument](./global-flags.md#the-plane-is-resolved-from-the-current-directory):
  a plane is never a positional.

## Where a member's worktree lands

The path inside the plane directory is derived from the member's source, once,
at the moment the member is created — never recomputed, so an existing plane
keeps the layout it was built with.

| source | derived path |
| --- | --- |
| `git@gitlab.com:acme/codestyle.git` | `acme/codestyle` |
| `~/projects/bitplane` | `projects/bitplane` |

A member whose derived path would begin with `.bitplane` is refused — that
segment is reserved for `bp`'s own per-plane files. See
[`reserved_path_segment`](./refusals-and-waivers.md#usage-failures).

## See also

- [Global flags](./global-flags.md) — how a fan-out prints these members back
- [`bp create`](./plane/create.md) · [`bp add`](./plane/add.md) · [`bp rm`](./plane/rm.md)
