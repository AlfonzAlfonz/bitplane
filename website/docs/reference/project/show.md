---
title: bp project show
---

# `bp project show`

Everything about one project: its kind, its source, the branch a new worktree
would land on, when it was last fetched, and every script it declares.

```
bp project show <project>
```

A **read**: no lock, no file written, nothing repaired.

## Arguments

| argument | required | what it is |
| --- | --- | --- |
| `<project>` | yes | `@name` or `name`. The sigil is optional here, because no path is accepted in this slot. |

## Flags

Only the [global flags](../global-flags.md#global-flags).

## Output

```
$ bp project show @codestyle
```
```
@codestyle  owned
  source     git@gitlab.com:acme/codestyle.git
  directory  ~/.local/share/bitplane/projects/codestyle
  repo       ~/.local/share/bitplane/projects/codestyle/repo.git
  default    main
  fetched    2026-09-21 09:14

scripts
  link-alfonz  ln -s ../../.alfonz .alfonz    post_worktree_create
  install      pnpm i && pnpm build           post_worktree_create
  stop-stack   docker compose down            pre_worktree_remove
  reset-db     pnpm db:reset && seed-fixtures

post_worktree_create   link-alfonz, install
pre_worktree_remove    stop-stack
```

### The resolved per-point sequence

The last two lines are the point of this command. Scripts declare their bindings
as toggles on the script, so *"what runs at `post_worktree_create`, in what
order"* is not answerable from any single place in `project.toml` — you would
read every table, in order. `bp project show` answers it.

**Order is declaration order**, and it is semantic: scripts run sequentially, so
`link-alfonz` landing before `install` is what lets the build read `.alfonz`.
`bp` never rewrites the `[scripts]` table, so the order cannot change behind
your back.

A script bound to no point — `reset-db` above — is legal and useful. It is
runnable with [`bp run`](../plane/run.md) like any other.

### The default branch is read from git, every time

`default` is the **base** a new plane branch is cut from: `bp create @api -b
feat-login` creates `feat-login` starting at it. It is not a branch a worktree
lands on — [`bp create`](../plane/create.md) and [`bp add`](../plane/add.md)
both make you name the branch you want.

It is **not a stored field**; it is resolved on every call, by this ladder:

1. `refs/remotes/origin/HEAD` — settable with `git remote set-head origin <branch>`, durable across fetches
2. the source repo's own `HEAD`, for an adopted project with no remote
3. otherwise, unspecified — and a command that would have to cut a **new** branch here is [`base_branch_unresolved`](../refusals-and-waivers.md#usage-failures)

A stored copy would be a second source of truth that goes silently stale when a
forge renames its default branch.

### The last fetch is derived, not recorded

`fetched` is the mtime of `repo.git/FETCH_HEAD`, which git writes on every
fetch. Recording it in `project.toml` would make every fetch do a
read-modify-write under the project lock, for a field nothing gates on.

## Examples

### An adopted project

Exit `0`. There is nothing to fetch, and `bp` says so rather than showing an
empty field.

```
$ bp project show @bitplane
```
```
@bitplane  adopted
  source     /Users/alfonz/projects/bitplane
  directory  ~/.local/share/bitplane/projects/bitplane
  repo       /Users/alfonz/projects/bitplane  (yours; bp writes nothing here)
  default    main
  fetched    nothing to fetch

scripts
  none declared
```

### A bad key in `project.toml`

Exit `1`. A misspelled toggle in a hand-written file fails loudly here and at
[`bp doctor`](../plane/doctor.md), not only when a plane is created and your
worktree comes out subtly wrong.

```json
{"error":"parse_error","code":1,"message":"~/.local/share/bitplane/projects/api/project.toml: unknown key \"post_worktree_created\" in [scripts.install]","problems":[],"remedy":"Legal keys are argv, shell, post_worktree_create and pre_worktree_remove."}
```

### No such project

Exit `2`.

```json
{"error":"project_not_found","code":2,"message":"there is no project called codestyle","problems":[],"remedy":"Run bp project list to see what projects exist."}
```

## Exit codes

| code | when |
| --- | --- |
| `0` | the project was read |
| `1` | `project.toml` could not be parsed |
| `2` | bad arguments, or no such project |
| `4` | git is missing, unusable or older than 2.36 |

## See also

- [`bp run`](../plane/run.md) — running one of these scripts on demand
- [`bp project list`](./list.md)
