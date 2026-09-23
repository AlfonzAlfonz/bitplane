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
| `--name <name>` | the URL's path, after the host | The project's name, which is also its directory name. |

Plus the [global flags](../global-flags.md#global-flags).

A name is a `/`-separated path of segments, each lowercase
`[a-z0-9][a-z0-9._-]*`, unique per host.

**The default is the whole path the forge gives the repo.**
`git@gitlab.com:acme/platform/tooling/codestyle.git` registers as
`@acme/platform/tooling/codestyle` — the transport and a trailing `.git` are
stripped, and the host is not part of the name. A forge path is a namespace:
stable, globally unique, and agreed on by everyone who clones the repo, which
is the thing whose absence used to make `acme/platform/codestyle` and
`acme/infra/codestyle` collide on `codestyle`. Importing a whole group tree
therefore needs **no `--name` at all**.

A URL that names no host — a bare local path, which `git` will happily fetch
from — keeps its **last segment**, exactly as
[`bp project adopt`](./adopt.md) does. It is the same rule, not an exception to
it: a forge path is a namespace, and a filesystem path is an accident of where
a directory happens to sit.

A derived name is **lowercased silently**, which is a normalisation rather than
a guess — two names differing only in case are one directory on a
case-insensitive filesystem. `--name` is never rewritten.

`repo.git` and `bin` may not be used as a segment: `bp` puts a directory of
each name *inside* a project directory, and the walk that finds projects skips
both by name, so a project called `@acme/bin` would be registrable once and
invisible thereafter.

A name may not **nest** inside another project, or contain one — see
[below](#the-name-would-nest-inside-another-project).

## What it builds

```
<projects-dir>/acme/platform/tooling/codestyle/
  project.toml
  repo.git/          the source repo: bare
  bin/               yours to create; never written to by bp
```

Because the name is a path, the project directory is **nested**, and `bp`
creates whatever intermediate directories it needs on the way down. Those hold
no `project.toml`, so they are **not projects** — nothing lists them, nothing
can name them, and [`bp project rm`](./rm.md) prunes them again when the last
project under them goes.

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
@acme/codestyle  owned
  source     git@gitlab.com:acme/codestyle.git
  directory  ~/.local/share/bitplane/projects/acme/codestyle
  default    main
```

## Examples

### A nested group becomes a nested name

No flags, however deep the group tree goes.

```
$ bp project add git@gitlab.com:acme/platform/tooling/codestyle.git
```
```
@acme/platform/tooling/codestyle  owned
  source     git@gitlab.com:acme/platform/tooling/codestyle.git
  directory  ~/.local/share/bitplane/projects/acme/platform/tooling/codestyle
  default    main
```

This is what makes a bulk import a shell loop rather than a feature: `bp`
speaks the git protocol and no forge API, so there is no `--group` flag and
there never will be. Piping your forge's own CLI into `bp project add` is the
supported way, and it now needs nothing per repo.

### The repo is already registered

Exit `2`. The name is taken by a project with the **same** source URL, which is
not a collision at all — it is the same repo, already here.

```
$ bp project add git@gitlab.com:acme/codestyle.git
```
```
error[project_name_taken]: acme/codestyle is already registered from that url
```

No remedy, because there is nothing to do. Offering you `acme/codestyle-2`
would propose a second copy of a repo you already have.

### The name is taken by a different repo

Exit `2`. The host is not part of the name, so the same path on two forges
collides. Refused with a suggestion, never silently disambiguated — a generated
default that renames itself behind your back is worse than an error naming the
conflict.

```
$ bp project add git@github.com:acme/codestyle.git
```
```
error[project_name_taken]: acme/codestyle is already a project

remedy: acme/codestyle-2 is free; re-run with --name acme/codestyle-2.
```

The ordinal goes on the **last segment**, never on the whole name — suffixing
the whole name would propose a project nested under the one in the way, which
is refused. Two commands is correct: you supplied new information in between,
so it is a second intent rather than a retry.

### The name would nest inside another project

Exit `2`, and **unwaivable**. `@acme` and `@acme/codestyle` cannot both exist:
the second would live inside the first's project directory, where
[`bp project rm @acme`](./rm.md) would take it along without mentioning it and
the registry walk, which stops at the first `project.toml`, would never see it.

```
$ bp project add git@gitlab.com:acme/codestyle.git
```
```
error[project_name_nests]: acme/codestyle would sit inside the project acme

remedy: Choose a name outside acme, or rename acme first.
```

The refusal runs in **both directions**: registering a project that would
*contain* an existing one is the same error, naming the same pair the other way
round.

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

Exit `2`, naming the segment that is not one. Capitals are folded away first,
so what reaches this is a URL the charset genuinely cannot take.

```
$ bp project add git@gitlab.com:acme/code+style.git
```
```
error[derived_name_invalid]: code+style is not a name bitplane can derive a project from

remedy: Re-run with --name <name>, using lowercase letters, digits and . _ - in each / -separated segment.
```

**No suggestion.** `bp` used to offer the nearest name the charset would take,
folding everything else to `-`; that was worth it when the alternative was no
name at all. Now that the default is the forge's own path, a URL that cannot
produce a name is a URL worth looking at rather than papering over.

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
| `2` | bad arguments, or a name that is taken, nesting, invalid or reserved |
| `4` | git is missing, unusable or older than 2.36 |
| `5` | a lock could not be taken in time |
| `130` | interrupted |

## See also

- [`bp project adopt`](./adopt.md) — register a checkout you already have
- [`bp project fetch`](./fetch.md) · [`bp project show`](./show.md)
