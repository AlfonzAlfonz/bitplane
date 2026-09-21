---
title: bp project fetch
---

# `bp project fetch`

Bring an owned project's source repo up to date with its forge.

```
bp project fetch [<project>...]
```

## Arguments

| argument | required | what it is |
| --- | --- | --- |
| `<project>...` | no | Which projects to fetch, as `@name` or `name`. **With none named, every registered project is taken in turn.** |

An **adopted** project has nothing to fetch — its source repo is your checkout,
and `bp` writes nothing there. Naming one is not an error; it reports `nothing
to fetch` and moves on.

## Flags

Only the [global flags](../global-flags.md#global-flags).

## What it fetches

```sh
git -C repo.git fetch --prune origin
```

**The configured refspec, in full** — not "only what some plane needs". Fetching
selectively saves nothing on one round trip and would turn a colleague's new
branch into a second fetch.

`--prune` is safe **because of the refspec** that
[`bp project add`](./add.md#what-it-builds) set:
`refs/remotes/origin/*` is a pure cache of the forge and holds nothing you own,
so pruning cannot lose work.

**Tags are never pruned.** `refs/tags/*` is a namespace shared with every
worktree, and `bp` does not own it.

A fetch **succeeds while a plane worktree on a branch of the same name is
live**, because it writes only to `refs/remotes/origin/*`, which nothing ever
checks out. That is the whole reason the refspec is not left at a default.

`bp` takes the project's lock while fetching — one sentinel per project
directory guards both `project.toml` and every git command that writes to the
source repo. Fetches of **different** projects never contend.

## Output

One row per project.

```
$ bp project fetch
```
```
  @api        fetched   3 refs updated
  @codestyle  fetched   up to date
  @bitplane   -         nothing to fetch (adopted)
```

## Examples

### One project

Exit `0`.

```
$ bp project fetch @api
```
```
  @api  fetched  3 refs updated
```

### The forge is unreachable

Exit `1`. Other projects still get their turn — a fan-out row is data, not an
error.

```
$ bp project fetch
```
```
  @api        failed: could not fetch origin: Connection refused
  @codestyle  fetched   up to date
  @bitplane   -         nothing to fetch (adopted)
```
```json
{"error":"fetch_failed","code":1,"message":"1 of 2 fetchable projects could not be fetched","problems":[{"subject":"@api","message":"could not fetch origin: Connection refused"}],"remedy":null}
```

The count is over the projects there was something to fetch **for**. An adopted
project is a row, never a denominator.

`bp` never handles a credential. It shells out to git and lets your credential
helpers and `ssh-agent` do the work, so an authentication failure here is the
same one `git fetch` would give you by hand.

### No such project

Exit `2`.

```json
{"error":"project_not_found","code":2,"message":"there is no project called codestyle","problems":[],"remedy":"Run bp project list to see what projects exist."}
```

## Exit codes

| code | when |
| --- | --- |
| `0` | every named project is up to date |
| `1` | at least one fetch failed |
| `2` | bad arguments, or no such project |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted |

## See also

- [`bp project add`](./add.md#what-it-builds) — the refspec that makes this safe
- [`bp create`](../plane/create.md#branch-intent) — which fetches before resolving a branch
