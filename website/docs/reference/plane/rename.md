---
title: bp rename
---

# `bp rename`

Give a plane a different id, which means moving its directory.

```
bp rename <new-id> [-p <plane>]
```

A plane id is **mutable by design**. A wall of `bp-a3f9c2e1` directories is
impossible to navigate, and renaming is the escape hatch — which is also why
nothing outside `bp` may store a plane id and expect it to resolve later.

`rename` **converges**: interrupt it and run it again, and it finishes.

## Arguments
| argument | required | what it is |
| --- | --- | --- |
| `<new-id>` | yes | The plane's new id, which becomes its directory name. |

The new id is lowercase `[a-z0-9][a-z0-9._-]*`, 64 characters at most, and must
not start with the reserved `bp-` prefix. A new id that is already a plane is
[`plane_id_in_use`](../refusals-and-waivers.md#usage-failures).

## Flags
| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane to rename. |

Plus the [global flags](../global-flags.md#global-flags).

## What it does
1. `mv` the plane directory — a single atomic `rename(2)`, so the plane is never
   findable under neither name.
2. `git worktree repair` per source repo, taking every new path in one
   invocation.
3. Rewrite `plane.toml`'s `id` **last**.

The primitive is `git worktree repair`, **not** `git worktree move`: you cannot
move a worktree whose parent directory has already moved, because git looks for
it at the old path. `repair` exists precisely for "a worktree was moved without
`git worktree move`" and is idempotent by construction.

The `id` is written last so a `plane.toml` whose `id` disagrees with its
directory name is a reliable **"this rename is unfinished"** signal rather than
a lie. That is the same state a hand-`mv` leaves, and the cure is identical in
both cases: run [`bp repair`](./repair.md), or run `bp rename` again.

:::note[If you are standing in the plane]

The directory moves out from under your shell. `cd` to the new path afterwards;
`bp` prints it.

:::

## Output
```
$ bp rename auth-work
```
```
auth-work  ~/planes/auth-work

  @api  feat-login  renamed  signageos/api
  @web  feat-login  renamed  signageos/web

bp-a3f9c2e1 is now auth-work
```

## Examples
### The plane was never finished being created
Exit `1`. Renaming a plane headed for deletion is meaningless work.

```json
{"error":"plane_incomplete","code":1,"message":"bp-a3f9c2e1 was never finished being created","problems":[],"remedy":"Nothing in it is yours; run bp destroy -p bp-a3f9c2e1 to clear it."}
```

### The new id is taken
Exit `2`.

```json
{"error":"plane_id_in_use","code":2,"message":"auth-work is already a plane","problems":[],"remedy":"Choose another id, or destroy the existing plane first."}
```

## Exit codes
| code | when |
| --- | --- |
| `0` | the plane is at its new id |
| `1` | the plane is incomplete, or a repair failed |
| `2` | bad arguments, an invalid or reserved id, a taken id, or no plane found |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted — re-run to converge |

## See also

- [`bp repair`](./repair.md) — for a move `bp` did not perform
- [`bp project rename`](../project/rename.md) — renaming a **project**, which sweeps every plane holding it
