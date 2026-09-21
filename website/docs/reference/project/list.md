---
title: bp project list
---

# `bp project list`

Every project registered on this host.

```
bp project list
```

A **read**: no lock, no file written, nothing repaired.

## Arguments
None. `list` reads the
[projects directory](../global-flags.md#directory-resolution).

## Flags
Only the [global flags](../global-flags.md#global-flags).

## What counts as a project
**A project is exactly a directory containing `project.toml`.** That is what
makes the set of registered projects self-describing: there is no registry file,
no index and no database — the projects directory *is* the registry.

A directory without a `project.toml` is not a project and is skipped. That
includes the leftovers of an interrupted
[`bp project add`](./add.md#the-fetch-fails), which hold `repo.git` and nothing
else; [`bp doctor`](../plane/doctor.md#the-four-sweeps) is what reports those.

A `project.toml` that does not parse is listed as a row in an error state naming
the file and the error, and the scan continues.

## Output
```
$ bp project list
```
```
@api        owned    git@gitlab.com:signageos/api.git
@codestyle  owned    git@gitlab.com:signageos/codestyle.git
@bitplane   adopted  /Users/alfonz/projects/bitplane
```

Whether an owned project's source repo is actually present locally is answered
by a `stat`, never by a stored field:

```
@api        owned    git@gitlab.com:signageos/api.git         repo.git missing
```

## Examples
### Nothing is registered
Exit `0`, with nothing on stdout. An empty registry is not an error, and neither
is a projects directory that does not exist yet.

### A project file will not parse
Exit `3`.

```
$ bp project list
```
```
@api        owned    git@gitlab.com:signageos/api.git
@codestyle  project.toml: unknown key "default_branch" at line 3
@bitplane   adopted  /Users/alfonz/projects/bitplane
```

Nothing is auto-repaired: a file `bp` cannot read is a file it has no business
rewriting.

## Exit codes
| code | when |
| --- | --- |
| `0` | every project was read |
| `2` | bad arguments |
| `3` | at least one project could not be read |
| `4` | git is missing, unusable or older than 2.36 |

## See also

- [`bp project show`](./show.md) — one project, in detail
- [`bp list`](../plane/list.md) — planes rather than projects
