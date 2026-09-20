# bitplane — domain context

bitplane is a **plane** manager: it creates, tracks and tears down named sets of git worktrees that share one lifecycle, so coordinating multi-repo, multi-session work is automatable rather than tedious.

This file is the glossary. Every ADR, issue and piece of code uses these words and no synonyms. Where a term replaces an earlier working term, the retired term is named so it stops leaking back in.

## Planes

**plane**
A named set of `(project, branch)` worktrees sharing one lifecycle, on a single host, in one directory. The core noun. Multi-repo from day one; a single-repo plane is the degenerate case.

A plane holds **at most one worktree per project** — its pairs are unique on *project*, not on `(project, branch)`. Two branches of the same repo is two planes. This is what keeps **plane layout** a pure function of the project source, with no branch component to disambiguate.

Replaces the working term **worktree group**.

> **Why "plane" and not an existing word?** Four shipping tools already name this concept — **task** (`wkt`), **group** (`multree`), **feature**, **label** — and Android's `repo` has called it a **topic** for fifteen years. Adopting any of them would be instantly legible at the cost of colliding with everything: "task" is overloaded past repair (issue trackers, agents, schedulers — this repo's own tickets carry a `Type: task` line), and "group" collides with git's own vocabulary. "plane" is distinctive, fits the project name, and teaches nothing on first read. That last part is the accepted cost: a newcomer must be told what a plane is once. In exchange the word means exactly one thing everywhere it appears.

**host**
Where a plane lives. Always `local` today; later an SSH target or a container. A plane never spans hosts. Each host has exactly one planes directory and exactly one projects directory.

**bitplane keeps no state about a host except on that host.** Asking about a remote host's planes means asking that host; nothing is cached locally, because nothing could keep such a cache true.

**planes directory**
The directory on a host under which plane directories are created. Exactly one per host, defaulting to `~/planes`. It holds the user's working trees, so it is deliberately *not* hidden and not under XDG.

**plane directory**
`<planes-dir>/<plane-id>/` — the one directory holding a single plane's worktrees. Its name **is** the plane's identity: the directory is the key, so renaming a plane is moving it.

`.bitplane/` at its root is **reserved** for bitplane's own per-plane files (hook logs, the lock sentinel). A project whose derived path would start with that segment is refused.

**plane id**
The identifier naming the plane directory, unique per host. **Mutable by design.** It defaults to a generated `bp-<hex8>`, may be supplied at `create`, and can be renamed afterwards — a wall of `bp-a3f9c2e1` directories is impossible to navigate, and renaming is the escape hatch.

It is therefore **not safe to cache**: nothing outside bitplane may store a plane id and expect it to resolve later, and no alias to a former id is kept. A stale reference is simply an error.

Generated ids are random hex, collision-detected by attempting the directory create and retrying — there is no allocator and no coordination. The `bp-` prefix is **reserved** for generated defaults and rejected as a user-chosen id, so "starts with `bp-`" stays a reliable signal that an id was machine-generated. User-chosen ids are lowercase `[a-z0-9][a-z0-9._-]*`, max 64 — lowercase-only so two ids can never collide on a case-insensitive filesystem.

**plane file**
`plane.toml` at the **root of the plane directory**, describing the whole plane and every project in it. One file per plane, not one per worktree.

Replaces the working term **marker**.

The location is load-bearing: the plane directory root sits outside every repo, so the file can never be committed by accident and needs no per-repo ignore rules.

**plane layout**
Worktrees live at `<plane-dir>/<derived-path>/`, where the derived path comes from the project's source:

| source | derived path |
| --- | --- |
| `git@gitlab.com:signageos/codestyle.git` | `signageos/codestyle` |
| `~/projects/bitplane` | `projects/bitplane` |

The path is derived **once, at create time**, never recomputed — so existing planes keep the layout they were built with when the derivation rules change, and a user is free to move worktrees around within a plane.

## Projects

**project**
Exactly one repo, plus its hooks and settings. The user-facing noun for a thing a plane can contain.

Replaces **source repo** as the user-facing term (see below for the surviving internal sense).

A project that bundles several repos is *not* this concept — bundling belongs to plane templates, still unspecified.

**project source**
Either a git URL or a local path.

**project name**
User-supplied, defaulting to the last segment of the source path (`codestyle`). Flat, unique per host, renameable.

When the default collides with an existing project the operation is **refused with a suggested alternative** (`signageos-codestyle`), never silently disambiguated — a generated default that renames itself behind the user's back is worse than an error naming the conflict.

Renaming a project moves its project directory and rewrites its entry; existing planes keep the subdirectories they were built with.

**`@` sigil**
How a project is referenced: `@codestyle`. Syntax only — not part of the name, never on disk, never in the store, never in a hook environment variable. Required wherever a path would also be accepted (that being the only real ambiguity), optional elsewhere, and always used when bitplane prints a project.

**projects directory**
The directory on a host holding one project directory per project. Exactly one per host, under `$XDG_DATA_HOME/bitplane/projects/` — application-owned data, not configuration, because it holds bare mirrors. **This is the registry** — there is no separate registry file, and the term **registry** is retired.

**project directory**
`<projects-dir>/<project-name>/`, containing `project.toml` and, for a URL-sourced project, the bare mirror at `repo.git`. A project is exactly "a directory containing `project.toml`", which makes the set of known projects self-describing. The mirror is a named child rather than the directory itself, so git commands run while sitting in a project directory do not silently operate on the mirror.

**source repo**
Internal term only: the local git repo a worktree is derived from. That is the bare mirror for a URL project, and the user's own checkout for a local-path project. The per-source-repo git lock attaches to this noun.

**add** vs **adopt**
`add` takes a URL and clones a bare mirror into the project directory. **adopt** takes an existing local checkout and creates a project directory whose source points at it **in place** — bitplane never moves, copies or converts a checkout it did not create, because every path outside bitplane pointing at that repo must keep working.

Adoption applies to projects only. There is no plane-level adoption.

**hook**
A user-declared command bitplane runs at a defined point in a plane's lifecycle. Declared on a project.

## State

**state**
Everything bitplane knows, which is exactly what is on disk: the planes directory, the projects directory, and the files in them. There is no database, no index and no cache.

The term **store** is retired. It described a thing that was never built — once the projects directory became the registry and every plane carried a plane file, a store would only have held a second copy of facts the filesystem already holds. See ADR-0002.

Two rules follow, and they are vocabulary rather than implementation:

- **A plane is exactly a directory containing a plane file**, the mirror of "a project is exactly a directory containing `project.toml`". Anything else in the planes directory is not a plane and is ignored.
- **A read never writes.** `list`, `show` and discovery take no locks and create no files, which is what makes them safe to run from a hook or in a loop.

**sentinel**
The file a lock is taken on — never the file being protected, because writing by atomic rename replaces the inode and would leave the lock guarding a file that no longer exists. One per plane directory and one per project directory.

**per-source-repo lock**
The mutex serialising every git command that *writes* to a source repo — `worktree add`, `worktree remove`, and `fetch`. It attaches to the source repo, so operations on different projects never contend. Reads take no lock.

## Contract

**`bp`**
The command bitplane installs on `PATH`. `bitplane` is the crate, repo and package name; `bp` is what a user types. Chosen to rhyme with the reserved `bp-` plane-id prefix — `bp create` yields `bp-a3f9c2e1`. See ADR-0001.

**Engine**
The request/response interface every mutation passes through; the machine contract the CLI renders.

**reap**
Destroying a plane judged to be garbage.

## Boundary rules, as vocabulary

**git protocol yes, forge API no.**
bitplane speaks git. It does not create pull requests, merge, review, or read CI status.

**git's credentials, never ours.**
bitplane shells out to git and lets git's credential helpers and `ssh-agent` do the work. It never reads, stores, prompts for or forwards a credential. A private repo you cannot `git clone` by hand is one bitplane cannot use.

This is the auth-shaped restatement of the rule above, and it is what keeps URL-sourced projects from dragging a secrets story into the tool.
