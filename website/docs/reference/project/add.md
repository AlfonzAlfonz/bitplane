---
title: bp project add
---

# `bp project add`

:::implemented

Everything on this page works as described.

:::

Register a project from a URL, building the source repo `bp` will own.

```
bp project add <url> [--name <name>]
```

This makes an **owned** project: `bp` built the repo, so `bp` owns its branch
namespace, its config, and the branches `destroy` cleans up. Pointing at a
checkout you already have is [`bp project adopt`](./adopt.md) instead.

## Arguments

| argument | required | what it is |
| --- | --- | --- |
| `<url>` | yes | Anything `git` can fetch from: `git@gitlab.com:acme/codestyle.git`, `https://…`, `ssh://…`. |

`bp` speaks the git protocol and no forge API. It never reads, stores, prompts
for or forwards a credential — your credential helpers and `ssh-agent` do the
work. **A private repo you cannot `git clone` by hand is one `bp` cannot use.**

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `--name <name>` | the last segment of the URL's path | The project's name, which is also its directory name. |

Plus the [global flags](../global-flags.md#global-flags).

A name is lowercase `[a-z0-9][a-z0-9._-]*`, flat, and unique per host.

## What it builds

```
<projects-dir>/<name>/
  project.toml
  repo.git/          the source repo: bare
  bin/               yours to create; never written to by bp
```

The source repo is built by hand rather than cloned:

```sh
git init --bare repo.git
git -C repo.git remote add origin <url>
git -C repo.git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
git -C repo.git fetch origin
git -C repo.git remote set-head origin -a
git -C repo.git config push.default current
```

**The refspec is the load-bearing line, and it is not a default.** It is what
makes three things true at once:

- **`fetch` always succeeds**, because it writes only to
  `refs/remotes/origin/*`, which nothing ever checks out.
- **`refs/heads/*` holds exactly the branches plane members were created on** —
  the forge's branches never touch that namespace. That is what makes a branch
  the responsibility of the plane that owns it.
- **`@{u}` means the forge's view** rather than a local copy of it, so
  "unpushed" is honest.

Neither `git clone --mirror` nor `git clone --bare` is used. `--mirror` sets
`remote.origin.mirror = true`, which a worktree inherits, turning an ordinary
`git push` into a force-push-everything-and-delete-the-rest against your forge;
`--bare` copies the forge's branches into `refs/heads/*` before the refspec can
be set. After the first fetch `refs/heads/*` holds **zero** refs, and exactly
one after the first worktree.

`push.default = current` is set because a plane branch is created off
`origin/main` and keeps `origin/main` as its upstream, so git's default
`push.default = simple` would make a plain `git push` fail in a worktree `bp`
just handed you. It is set on **owned** source repos only.

## Output

```
$ bp project add git@gitlab.com:acme/codestyle.git
```
```
@codestyle  owned
  source     git@gitlab.com:acme/codestyle.git
  directory  ~/.local/share/bitplane/projects/codestyle
  default    main
```

## Examples

### The default name is taken

Exit `2`. Refused with a suggestion, never silently disambiguated — a generated
default that renames itself behind your back is worse than an error naming the
conflict.

```
$ bp project add git@gitlab.com:acme/codestyle.git
```
```
error[project_name_taken]: codestyle is already a project

remedy: acme-codestyle is free; re-run with --name acme-codestyle.
```

Two commands, and that is correct: you supplied new information in between, so
it is a second intent rather than a retry.

### The fetch fails

Exit `1`. The **registration** is unwound; the **objects are kept**.

```
error[project_add_aborted]: @codestyle was not registered

  origin  could not fetch: Connection refused

remedy: The objects fetched so far were kept at ~/.local/share/bitplane/projects/codestyle/repo.git; re-running bp project add will reuse them.
```

A cold fetch of a large repo is expensive to discard, and `init --bare` +
`fetch` is resumable in a way `clone` is not: a partial object store is reused
by the next fetch. Because `project.toml` was never written, **the leftover
directory simply is not a project** — nothing lists it, and
[`bp doctor`](../plane/doctor.md#the-four-sweeps) reports it.

### The URL derives a name the charset will not take

Exit `2`. A forge that names a repo `MyProject` has not told `bp` what the
project should be called.

```
$ bp project add git@gitlab.com:acme/MyProject.git
```
```
error[derived_name_invalid]: MyProject is not a name bitplane can derive a project from

remedy: Re-run with --name myproject.
```

The suggestion is the nearest name the charset takes, and it is **offered, never
applied** — same reason a taken default is refused rather than disambiguated.

### The repo would land on a reserved path

Exit `2`.

```
error[reserved_path_segment]: a worktree of this repo would land at .bitplane/tools, which is reserved

remedy: Move the repository out of a directory called .bitplane.
```

## Exit codes

| code | when |
| --- | --- |
| `0` | the project is registered |
| `1` | the fetch or the write failed; nothing is registered |
| `2` | bad arguments, a taken name, an invalid name, or a reserved path |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted |

## See also

- [`bp project adopt`](./adopt.md) — register a checkout you already have
- [`bp project fetch`](./fetch.md) · [`bp project show`](./show.md)
