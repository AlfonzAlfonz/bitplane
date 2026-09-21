---
title: bp project rm
---

# `bp project rm`

Unregister a project and delete everything `bp` built for it.

```
bp project rm <project>
```

:::danger[This is not `bp rm`]

[`bp rm`](../plane/rm.md) takes a member out of one plane. **`bp project rm`
deletes the project**: its directory, its `project.toml`, its scripts, and — for
an owned project — the source repo and every object in it.

Both take `@name`, both are destructive, and they rhyme. Project deletion is
therefore **never** abbreviated to a bare verb.

:::

## Arguments
| argument | required | what it is |
| --- | --- | --- |
| `<project>` | yes | The project to remove, as `@name` or `name`. |

## Flags
Only the [global flags](../global-flags.md#global-flags).

There is no `--waive`. The one refusal this command raises cannot be waived.

## What it does
1. Scan the planes directory for planes holding this project. If any do,
   **refuse**.
2. Delete the project directory.

For an **adopted** project, step 2 deletes the project directory only. Your
checkout is untouched — `bp` never owned it.

## `project_in_use` cannot be waived
A waiver says *"I accept losing **this** work, which I am looking at."*
Removing a project whose worktrees are live in some plane damages planes you did
not mention and are not looking at, so there is no moment at which consenting to
it is informed.

This is the only refusal in the tool whose consequences land outside your field
of view, and it is the only unwaivable one of its kind.

An **incomplete** plane blocks removal too, for uniformity. Since
[`bp destroy` never refuses on a latched plane](../plane/destroy.md#a-plane-that-was-never-finished-being-created),
clearing it is one command.

## Output
```
$ bp project rm @codestyle
```
```
removed @codestyle
  directory  ~/.local/share/bitplane/projects/codestyle
  repo       12 MB freed
```

## Examples
### A plane still holds it
Exit `1`. The blocking planes are always listed **by id** — the entire value of
an unwaivable refusal is that the way out is obvious.

```
$ bp project rm @codestyle
```
```json
{"error":"project_in_use","code":1,"message":"@codestyle has worktrees in 2 planes","problems":[{"subject":"bp-a3f9c2e1","message":"holds a worktree of @codestyle"},{"subject":"auth-work","message":"holds a worktree of @codestyle"}],"remedy":"Destroy those planes, or run bp rm @codestyle in each, then try again."}
```

### An incomplete plane holds it
Exit `1`.

```json
{"error":"project_in_use","code":1,"message":"@codestyle has worktrees in 1 plane","problems":[{"subject":"bp-7c1e0d44","message":"holds a worktree of @codestyle; create never completed"}],"remedy":"Nothing in bp-7c1e0d44 is yours; run bp destroy -p bp-7c1e0d44 to clear it, then try again."}
```

### No such project
Exit `2`.

```json
{"error":"project_not_found","code":2,"message":"there is no project called codestyle","problems":[],"remedy":"Run bp project list to see what projects exist."}
```

## Exit codes
| code | when |
| --- | --- |
| `0` | the project is gone |
| `1` | refused because a plane holds it, or the delete failed |
| `2` | bad arguments, or no such project |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted |

## See also

- [`bp rm`](../plane/rm.md) — taking a member out of a plane
- [Refusals and waivers](../refusals-and-waivers.md#project_in_use)
