---
title: bp run
---

# `bp run`

Run a project's declared scripts, by name, on demand.

```
bp run <project> <script>... [-p <plane>]
```

A **script** is a named command declared in a project's `project.toml`. It may
be bound to a lifecycle point, to both, or to neither — and either way it can be
run by hand. `bp run @api install` reads like `npm run install` on purpose.

## Why this exists
Re-running a failed script by hand means reconstructing eight environment
variables, the project's `bin/` entry on `PATH`, and the right working
directory. That is not something you will get right, and getting it wrong
silently is worse than not retrying at all.

## Arguments
| argument | required | what it is |
| --- | --- | --- |
| `<project>` | yes | The project whose scripts to run, as `@name` or `name`. Must be a member of the plane. |
| `<script>...` | yes, one or more | Script names, run in **the order you gave them**. |

**The project is mandatory.** Nobody gets a six-repo script run by typing
nothing. Both arguments are required or it is exit `2`.

`bp run` takes **script names only, never a point name**. Points are bindings,
not addressable things, so the two never share a namespace.

An [ad-hoc member](../member-syntax.md#paths) has no `project.toml` and
therefore no scripts. `bp run` against one is an error, because `bp run` takes a
project.

## Flags
| flag | default | what it does |
| --- | --- | --- |
| `-p`, `--plane <id>` | the plane containing the current directory | Which plane's worktree to run in. |

Plus the [global flags](../global-flags.md#global-flags).

## Execution
Scripts run **sequentially**, in the order named on the command line, and stop
at the first non-zero exit.

- **The working directory is the worktree.**
- **`<project-dir>/bin` is prepended to `PATH`**, so a project can ship its own
  executables and have them win. `bp` never writes into that directory.
- **Your environment is inherited in full** — a sanitised one would break
  `SSH_AUTH_SOCK`, `ssh-agent` and every credential helper.
- **There is no timeout.** A real `pnpm i && pnpm build` outlives any default
  worth shipping, and a build killed at ten minutes is a worse failure than a
  hang you can see.

### Output
stdout and stderr are **merged and streamed live to `bp`'s stderr**, and tee'd
to a log at
`<plane-dir>/.bitplane/logs/<YYYYMMDDTHHMMSSZ>-<project>-<script>.log`.

Merged onto **stderr**, never stdout: stdout is the machine contract, and a
`pnpm` banner inside the JSON is a broken contract.

Sequential execution is what keeps live output legible — exactly one script
writes at a time, so there is no interleaving and no per-line prefixing.

`bp` never cleans these logs up. [`bp destroy`](./destroy.md) removes them with
the plane directory.

### Environment
Eight variables, on top of everything you already had:

| variable | value |
| --- | --- |
| `BITPLANE_SCRIPT` | the script's name |
| `BITPLANE_PROJECT` | the **bare** project name; the `@` sigil never appears here |
| `BITPLANE_PROJECT_DIR` | `<projects-dir>/<name>/` |
| `BITPLANE_SOURCE_REPO` | `repo.git` when the project is owned, your checkout when it is adopted |
| `BITPLANE_WORKTREE` | the worktree path — the same as the working directory |
| `BITPLANE_BRANCH` | the **live** `HEAD`. **Unset** when detached |
| `BITPLANE_PLANE_DIR` | the plane directory |
| `BITPLANE_PLANE_ID` | the plane id |

`BITPLANE_BRANCH` is unset rather than set to a sha when `HEAD` is detached, so
a script doing `git switch "$BITPLANE_BRANCH"` fails loudly instead of detaching
again.

**Nothing in that set is stable.** A plane id is mutable by design and the
directory moves with it, so a script must persist none of it. `bp` never reads
these back either — see [the plane
argument](../global-flags.md#the-plane-is-resolved-from-the-current-directory).

## Idempotency
`bp run` is **not idempotent and does not pretend to be**. Re-running
`ln -s` fails the second time. That failure is yours to own, exactly as the
first one was.

## Examples
### Running a script
Exit `0`.

```
$ bp run @api install
```
```
@api  install  ok  4.2s
```

The script's own output went to stderr as it ran, and to
`~/planes/bp-a3f9c2e1/.bitplane/logs/20260921T140312Z-api-install.log`.

### Several scripts, in the order you named them
```
$ bp run @api link-alfonz install
```
```
@api  link-alfonz  ok  0.0s
@api  install      ok  4.2s
```

Request order, not declaration order — unlike a lifecycle point, you named
these, so that is the order you meant.

### A script fails
Exit `1`. Later scripts are not run.

```json
{"error":"script_failed","code":1,"message":"install exited 1 in @api","problems":[{"subject":"@api","message":"install exited 1; see ~/planes/bp-a3f9c2e1/.bitplane/logs/20260921T140312Z-api-install.log"}],"remedy":"See the log, fix the cause, then run bp run @api install."}
```

### No such script
Exit `2`.

```json
{"error":"script_not_found","code":2,"message":"@api has no script called intall","problems":[],"remedy":"Run bp project show @api to see the scripts it declares."}
```

### The project is not in this plane
Exit `2`.

```json
{"error":"project_not_in_plane","code":2,"message":"@web is not a member of bp-a3f9c2e1","problems":[],"remedy":"Run bp show to see the members of this plane."}
```

## Exit codes
| code | when |
| --- | --- |
| `0` | every named script exited `0` |
| `1` | a script exited non-zero, or the plane is incomplete |
| `2` | bad arguments, an unknown script, or a project that is not a member |
| `4` | git is missing, unusable or older than 2.36 |
| `130` | interrupted |

`bp run` refuses on a plane that was never finished being created, with
[`plane_incomplete`](../refusals-and-waivers.md#plane_incomplete).

## See also

- [`bp project show`](../project/show.md) — what scripts a project declares, and what runs at each point
- [`bp create`](./create.md) · [`bp destroy`](./destroy.md) — the two points scripts fire at
