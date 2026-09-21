---
title: bp project adopt
---

# `bp project adopt`

Register a checkout you already have, in place.

```
bp project adopt <path> [--name <name>]
```

This makes an **adopted** project. `bp` never moves, copies or converts it:
every path outside `bp` pointing at that repo must keep working.

Adoption applies to projects only. There is no plane-level adoption.

## What `bp` will not do to an adopted repo
This is the sharpest asymmetry in the design, and it is deliberate: **`bp` owns
the refs and the config in a repo it built, and owns nothing in a repo it merely
pointed at.**

| | owned | adopted |
| --- | --- | --- |
| writes `push.default` and the refspec | yes | **no** |
| deletes the member's branch on `destroy` | yes | **no** |
| has something to fetch | yes | **no** |
| occupies a branch | no, it is bare | **yes**, whatever your working tree is on |
| `refs/heads/*` is exactly the plane branches | yes | **no**, they are your branches |

With nothing stored, `bp` **cannot tell** whether a branch in your repo is one
it created or one you have had for two years. So it touches none of them.

## Arguments
| argument | required | what it is |
| --- | --- | --- |
| `<path>` | yes | A git repository. Relative, absolute or `~`-prefixed; `bp` canonicalises it. |

The canonical absolute path is what gets recorded. There is no `~` expansion in
the stored value: "whose home" is unanswerable when a project is read over SSH
later.

## Flags
| flag | default | what it does |
| --- | --- | --- |
| `--name <name>` | the last segment of the path | The project's name, which is also its directory name. |

Plus the [global flags](../global-flags.md#global-flags).

## What it builds
```
<projects-dir>/<name>/
  project.toml       source type = "adopted", path = <your checkout>
  bin/               yours to create
```

No `repo.git`. The source repo **is** your checkout, and nothing is written
inside it.

An adopted project's source is always a `path` and an owned project's always a
`url`. That 1:1 correspondence is a rule, not a coincidence: a `type` that
disagrees with its payload is a
[`parse_error`](../refusals-and-waivers.md#parse_error).

## Output
```
$ bp project adopt ~/projects/bitplane
```
```
@bitplane  adopted
  source     /Users/alfonz/projects/bitplane
  directory  ~/.local/share/bitplane/projects/bitplane
  default    main
```

## Examples
### Your checkout is sitting on the branch a plane wants
Exit `1`, at [`bp create`](../plane/create.md) time rather than here. Git
refuses `worktree add` on a branch checked out in any worktree of the same repo,
and `bp` reports it in its own words rather than passing git's message through.

```json
{"error":"branch_occupied","code":1,"message":"main is already checked out in /Users/alfonz/projects/bitplane","problems":[{"subject":"/Users/alfonz/projects/bitplane","message":"its working tree is on main"}],"remedy":"Check out a different branch there, or give this member a branch no worktree holds."}
```

This cannot happen for an owned project: a bare repo has no working tree, so it
occupies nothing.

### The path is not a git repository
Exit `2`.

```json
{"error":"bad_usage","code":2,"message":"/Users/alfonz/notes is not a git repository","problems":[],"remedy":"Point at a directory git recognises, or create one with git init."}
```

### The name is taken
Exit `2`, [exactly as for `bp project add`](./add.md#the-default-name-is-taken).

## Exit codes
| code | when |
| --- | --- |
| `0` | the project is registered |
| `1` | the write failed |
| `2` | bad arguments, a taken or invalid name, a reserved path, or a path that is not a repo |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |

## See also

- [`bp project add`](./add.md) — register from a URL, building the repo
- [Member syntax](../member-syntax.md#paths) — using a repo in a plane **without** registering it
