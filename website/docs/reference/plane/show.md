---
title: bp show
---

# `bp show`

Everything about one plane: its members, where they are, what branch each is on,
and any finding against it.

```
bp show [-p <plane>] [--health none|cheap|full]
```

Like [`bp list`](./list.md), `show` is a **read**: no lock, no file written,
nothing repaired.

## Arguments

None. The plane comes from the current directory, or from `--plane`.

### How the current directory resolves

`bp` canonicalises the working directory — following symlinks, so a worktree
reached through one resolves to its real plane — then walks up to the filesystem
root. The **innermost** `plane.toml` wins, which is the only answer that can be
right for nested planes.

The walk runs in the engine rather than in the CLI, so it stays executable
against a remote host later.

**A path resolves to a plane, never to a member.** Standing in
`acme/api` and running `bp show` describes the whole plane, not `@api`.
Identifying the member would be free and is declined anyway: a ref that
sometimes carries a member is an implicit subject only some commands honour, and
you cannot predict which.

There is **no planes-directory constraint**. A plane outside the configured
planes directory still resolves; being outside is a finding, not a reason to
fail to find it.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane to show. |
| `--health <level>` | `cheap` | `none`, `cheap` or `full`. See [health levels](./list.md#health-levels). |

Plus the [global flags](../global-flags.md#global-flags).

## Output

```
$ bp show
```
```
bp-a3f9c2e1
  directory  ~/planes/bp-a3f9c2e1
  created    2026-09-21 14:03
  members    3

  @api                             feat-login  acme/api
  @web                             feat-login  acme/web
  /Users/alfonz/projects/bitplane  main        projects/bitplane
```

Each member's branch is read from its worktree's `HEAD` at the moment you ask,
so it is **true by construction**. `plane.toml` records no branch: a plane has
no branch, and nothing derives one.

A member whose `HEAD` is detached shows `(detached)` in place of a branch.

## Examples

### There is no plane here

Exit `2`.

```
$ bp show
```
```
error[plane_not_found]: no plane contains /Users/alfonz/src

remedy: cd into a plane, or name one with --plane.
```

### The plane has a finding

Exit `3`.

```
$ bp show -p auth-work
```
```
auth-work
  directory  ~/planes/auth-work
  created    2026-09-18 09:41
  members    2
  health     broken

  @api  feat-login  acme/api
  @web  -           acme/web   worktree missing

findings
  @web  the worktree at acme/web is not there
        bp rm @web drops it; bp add @web:<branch> puts it back
```

Every finding names the command that fixes it. `show` reports and never repairs.

### The plane was moved outside `bp`

Exit `3`. The directory name is the plane's identity; `plane.toml`'s `id` is a
self-check, and a disagreement is a positive signal rather than a problem to
paper over.

```
$ bp show -p auth-work
```
```
auth-work
  directory  ~/planes/auth-work
  members    2
  health     broken

findings
  plane.toml says bp-a3f9c2e1; this directory is auth-work
  the plane was moved outside bp, or a rename was interrupted
  bp repair reconnects the worktrees and settles the id
```

## Exit codes

| code | when |
| --- | --- |
| `0` | the plane is healthy |
| `1` | `plane.toml` could not be parsed |
| `2` | bad arguments, or no plane found |
| `3` | the plane reported a finding |
| `4` | git is missing, unusable or older than 2.36 |

## See also

- [`bp list`](./list.md) — every plane at once
- [`bp status`](./status.md) — what git says about each member
- [`bp repair`](./repair.md) — the cure for most findings
