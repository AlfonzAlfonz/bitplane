---
title: Global flags and directories
sidebar_label: Global flags
---

# Global flags and directories

Every `bp` command accepts the flags on this page, resolves its directories by
the same precedence, and prints a fan-out the same way. Nothing here is repeated
on the individual command pages; they link back to it.

## Global flags

| flag | value | what it does |
| --- | --- | --- |
| `--planes-dir <path>` | a directory | Where plane directories live for this invocation. |
| `--projects-dir <path>` | a directory | Where project directories live for this invocation. |
| `--config <path>` | a file | The config file to read instead of the default. |
| `--json` | — | Render machine output instead of human output — the response on stdout, a failure on stderr. See [`--json`](#--json) below. |
| `-h`, `--help` | — | Print help for the command and exit `0`. |
| `-V`, `--version` | — | Print `bp <version>` and exit `0`. |

`--help` and `--version` go to **stdout** and exit `0`. They are not errors, even
though they stop the command.

There is **no `--force`**. Consent is granted per reason, one reason at a time —
see [Refusals and waivers](./refusals-and-waivers.md).

There is **no `--host`**. A host is chosen by which engine `bp` is talking to,
and only the local engine exists today; a flag for a choice with one option would
be documenting a feature that is not there.

## The plane is resolved from the current directory

Every command that acts on an **existing** plane takes that plane from the
current directory: `bp` canonicalises the working directory and walks up to the
filesystem root, and the innermost `plane.toml` wins. A plane found outside the
planes directory still resolves — being outside is something `bp list` reports,
not a reason to refuse to find it.

`-p <id>`, `--plane <id>` names a plane explicitly instead.

```
bp status                    # the plane I am standing in
bp status -p auth-work       # the plane called auth-work
```

**A plane is never a positional argument.** Positional slots hold members,
branches, script names and project names — things that vary per command — so a
plane id can never end up in one by accident. This is the same discipline that
makes `bp add`'s branch mandatory and `bp run`'s project mandatory: an implicit
subject inferred from an argument slot is where surprise damage comes from.

A path resolves to a **plane**, never to a member. `bp rm` standing inside
`acme/api` still requires you to name `@api`.

`bp` never reads `BITPLANE_PLANE_DIR` or `BITPLANE_PLANE_ID` back. Those are
written for scripts to consume; a stale one inherited from an outer shell would
make `bp status` report a plane you are not standing in, and since a plane id is
mutable it could resolve to a *different* plane that has since taken the name.

## Directory resolution

Two directories matter, and both resolve by **flag > environment > config >
default**.

| directory | flag | environment | config key | default |
| --- | --- | --- | --- | --- |
| planes | `--planes-dir` | `BITPLANE_PLANES_DIR` | `planes_dir` | `~/planes` |
| projects | `--projects-dir` | `BITPLANE_PROJECTS_DIR` | `projects_dir` | `$XDG_DATA_HOME/bitplane/projects` |

The config file itself resolves by **flag > environment > default**:
`--config`, then `BITPLANE_CONFIG`, then
`$XDG_CONFIG_HOME/bitplane/config.toml`.

`XDG_DATA_HOME` falls back to `~/.local/share` and `XDG_CONFIG_HOME` to
`~/.config`, on macOS as well as Linux — a developer who has set `XDG_DATA_HOME`
means it.

The planes directory is deliberately **not** under XDG. It holds your actual
working trees and is the one path you type daily;
`cd ~/.local/share/bitplane/planes/bp-a3f9c2e1` is hostile.

```toml
# $XDG_CONFIG_HOME/bitplane/config.toml

version = 1
planes_dir = "~/work/planes"
```

A missing config file is not an error. An unknown key in one is
[`parse_error`](./refusals-and-waivers.md#parse_error).

## How a fan-out prints

Every command that touches more than one member prints a header line, then one
indented row per member, in `plane.toml` order:

```
<plane-id>  <plane-directory>

  <member>  <branch>  <outcome>  <path>
```

| column | what it holds |
| --- | --- |
| member | `@name` for a project, the absolute repo path for an ad-hoc member |
| branch | the branch the worktree is on, or `-` where there is none to report |
| outcome | one of the words below |
| path | the worktree's path relative to the plane directory |

The outcome word is one of:

| outcome | meaning |
| --- | --- |
| `created` | the worktree was made by this run |
| `removed` | the worktree was taken away by this run |
| `repaired` | git's record of the worktree was rewritten |
| `renamed` | the member moved with its plane |
| `unchanged` | it was already in the state asked for |
| `already gone` | there was nothing left to remove; the run converged |
| `skipped: <reason>` | not attempted — usually `aborted after an earlier failure` |
| `failed: <message>` | attempted and did not work |

A fan-out **never prints a count in place of the rows**. Four members that
succeeded print four rows, because those rows are the repair instruction when
some of them did not.

## `--json`

`--json` selects the **machine rendering** for everything the command prints: the
response as a single JSON value on stdout, and a failure as one line of JSON on
stderr. Without it, both are rendered for a human. The exit code is unchanged
either way.

**The payload is out of scope for this page.** It is the serialisation of the
core request/response types, and documenting a second copy of it here would
guarantee the two disagree. What is fixed, and is a contract:

- **The stream is decided by what the output is, the rendering by the mode.**
  Results go to stdout, failures to stderr, in both modes. Only the rendering
  changes.
- **stdout carries results only.** No progress, no script output, no warnings,
  and no errors — so `bp list --json | jq` never receives an error object where
  it expected a list of planes. It gets empty input, and the exit code.
- **Failures are always the [error
  envelope](./refusals-and-waivers.md#the-error-envelope)**, built in
  `bitplane-core`. The two renderings carry the same five fields, so nothing is
  reported to a human that a machine cannot read, or the reverse.
- The envelope's `code` field is the number the process exits with.

`--json` is the **only** switch. `bp` does not check whether stderr is a
terminal, so piping or redirecting never changes the bytes you would have seen
on screen — the same discipline that makes every branch explicit and every
waiver named.

## Environment variables

`bp` reads three:

| variable | what it sets |
| --- | --- |
| `BITPLANE_PLANES_DIR` | the planes directory |
| `BITPLANE_PROJECTS_DIR` | the projects directory |
| `BITPLANE_CONFIG` | the config file |

It **writes** eight more into every script it runs, and reads none of them back.
Those are documented on [`bp run`](./plane/run.md#environment).

## See also

- [Member syntax](./member-syntax.md) — how to name a member on the command line
- [Exit codes](./exit-codes.md) — the six codes, and `130`
- [Refusals and waivers](./refusals-and-waivers.md) — every refusal and its exact wording
