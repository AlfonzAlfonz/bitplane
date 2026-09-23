# bitplane — domain context

bitplane is a **plane** manager: it creates, tracks and tears down named sets of git worktrees that share one lifecycle, so coordinating multi-repo, multi-session work is automatable rather than tedious.

This file is the glossary. Every ADR, issue and piece of code uses these words and no synonyms. Where a term replaces an earlier working term, the retired term is named so it stops leaking back in.

## Planes

**plane**
A named set of `(project, branch)` worktrees sharing one lifecycle, on a single host, in one directory. The core noun. Multi-repo from day one; a single-repo plane is the degenerate case.

A plane holds **at most one worktree per project** — its pairs are unique on *project*, not on `(project, branch)`. Two branches of the same repo is two planes. This is what keeps **plane layout** a pure function of the project source, with no branch component to disambiguate.

Since ADR-0008 keyed the plane file by path, this is an **invariant the parser checks**, not a property of the file format. Lifting it later requires a new layout rule, because two worktrees of one project derive the same path.

**A plane has no branch.** `bp create -b feat-x` applies `feat-x` to every member at that moment and the name is not kept; there is no plane-level branch and nothing derives one. A member's branch is whatever its worktree is on right now, read when asked. **`bp create` and `bp add` both require an explicit branch** — `create` because a member's own default branch is the one an adopted or ad-hoc checkout already occupies, `add` because deriving one from the existing members would make it depend on unrelated work. See ADR-0006 and ADR-0005's banner.

Replaces the working term **worktree group**.

> **Why "plane" and not an existing word?** Four shipping tools already name this concept — **task** (`wkt`), **group** (`multree`), **feature**, **label** — and Android's `repo` has called it a **topic** for fifteen years. Adopting any of them would be instantly legible at the cost of colliding with everything: "task" is overloaded past repair (issue trackers, agents, schedulers — this repo's own tickets carry a `Type: task` line), and "group" collides with git's own vocabulary. "plane" is distinctive, fits the project name, and teaches nothing on first read. That last part is the accepted cost: a newcomer must be told what a plane is once. In exchange the word means exactly one thing everywhere it appears.

**host**
Where a plane lives. Always `local` today; later an SSH target or a container. A plane never spans hosts. Each host has exactly one planes directory and exactly one projects directory.

**bitplane keeps no state about a host except on that host.** Asking about a remote host's planes means asking that host; nothing is cached locally, because nothing could keep such a cache true.

**planes directory**
The directory on a host under which plane directories are created. Exactly one per host, defaulting to `~/planes`. It holds the user's working trees, so it is deliberately *not* hidden and not under XDG.

**plane directory**
`<planes-dir>/<plane-id>/` — the one directory holding a single plane's worktrees. Its name **is** the plane's identity: the directory is the key, so renaming a plane is moving it.

`.bitplane/` at its root is **reserved** for bitplane's own per-plane files (script logs, the lock sentinel, the incomplete marker). A project whose derived path would start with that segment is refused.

**plane id**
The identifier naming the plane directory, unique per host. **Mutable by design.** It defaults to a generated `bp-<hex8>`, may be supplied at `create`, and can be renamed afterwards — a wall of `bp-a3f9c2e1` directories is impossible to navigate, and renaming is the escape hatch.

It is therefore **not safe to cache**: nothing outside bitplane may store a plane id and expect it to resolve later, and no alias to a former id is kept. A stale reference is simply an error.

Generated ids are random hex, collision-detected by attempting the directory create and retrying — there is no allocator and no coordination. The `bp-` prefix is **reserved** for generated defaults and rejected as a user-chosen id, so "starts with `bp-`" stays a reliable signal that an id was machine-generated. User-chosen ids are lowercase `[a-z0-9][a-z0-9._-]*`, max 64 — lowercase-only so two ids can never collide on a case-insensitive filesystem.

**plane file**
`plane.toml` at the **root of the plane directory**. One file per plane, not one per worktree.

Exactly three keys: `version`, `id`, and `[members]`. It is the **membership list**, not the desired state — it answers *which worktrees are in this plane, and what each is a worktree of*. It carries **no branch** (a member's branch is read from the worktree's `HEAD` when asked), no status, no host, no timestamp: every other candidate was either already stored by git or the filesystem, or would have made a read write. See ADR-0006 and ADR-0008.

`[members]` is keyed by the **worktree's path relative to the plane directory root**. The value names what it is a worktree *of*: `@name` for a **project**, any other value being an absolute path to an **ad-hoc member**'s repo.

Replaces the working term **marker**.

The location is load-bearing: the plane directory root sits outside every repo, so the file can never be committed by accident and needs no per-repo ignore rules.

**member**
One worktree in a plane, together with what it is a worktree of. Identified by its **`MemberRef`** — a project (`@codestyle`) or an ad-hoc repo path — which is unique within a plane and is what keys every plane fan-out. `ProjectName` keys the `project_*` actions only.

A member is *usually* a project, but need not be: see **ad-hoc member**.

**ad-hoc member**
A member whose worktree comes from a repo that is not a registered project — `bp create ~/projects/bitplane`. **Adopted-shaped, minus the registration**: bitplane never deletes a branch in it, has nothing to fetch, and must report the branch its working tree occupies as a refusal in bitplane's own words.

It is a **member kind, not a project kind**. It has no `project.toml`, so it has no name, no `bin/` and **no scripts** — it behaves exactly like a project with an empty `[scripts]`, silently. `bp run` against one is an error, because `bp run` takes a project.

_Avoid_: unregistered member (the term **registry** is retired), bare member (collides with the bare source repo), non-adopted project (it is not a project).

**plane layout**
Worktrees live at `<plane-dir>/<derived-path>/`, where the derived path comes from the project's source:

| source | derived path |
| --- | --- |
| `git@gitlab.com:acme/codestyle.git` | `acme/codestyle` |
| `~/projects/bitplane` | `projects/bitplane` |

The path is derived **once, at create time**, never recomputed — so existing planes keep the layout they were built with when the derivation rules change.

**Moving a worktree within a plane requires `bp repair`.** A plain `mv` makes git report the worktree at its old path and mark it `prunable`, which is a reap staleness signal — so the move is not a free act the user may perform unobserved. `bp repair` scans the plane, runs `git worktree repair`, and rewrites the plane file's keys. Measured in ADR-0008.

## Projects

**project**
Exactly one repo, plus its scripts and settings. The user-facing noun for a thing a plane can contain.

Replaces **source repo** as the user-facing term (see below for the surviving internal sense).

A project that bundles several repos is *not* this concept — bundling belongs to plane templates, still unspecified.

**project source**
Either a git URL or a local path — and which one it is *is* the project's kind, so the two are never named separately. See **owned project** / **adopted project**.

**owned project**
A project whose source repo bitplane built and owns: source `type = "owned"`, always a `url`, always a bare repo at `<project-dir>/repo.git`.

Ownership — not "it came from a URL" — is what every rule in the design branches on: the **plane branch** invariant, `push.default = current`, `destroy` deleting the member's branch, occupying no branch, and having something to fetch. A reader who knows a project is owned predicts all five.

_Avoid_: URL project, URL-sourced project.

**adopted project**
A project whose source repo is the user's own checkout, pointed at in place: source `type = "adopted"`, always a `path`. bitplane never moves, copies or converts it, never deletes a branch in it, and cannot tell its own branches from the user's there.

_Avoid_: local project, local-path project.

The two are in **1:1 correspondence**: an owned project's source is always a `url`, an adopted project's always a `path`. This holds because `adopt` never converts a checkout and `add` always builds a bare repo — so it is a rule that a future "adopt an existing bare repo" would break, not a coincidence.

Rejected names, so they stop coming back: **remote** collides with git's own `remote` and with **host** (`bp --host devbox` makes "remote project" read as *a project on another machine*); **native** collides with ADR-0001's "no native dependencies"; **cloned** is false, since ADR-0005 builds the repo with `git init --bare` rather than `git clone`.

**project name**
A `/`-separated **path of segments**, each lowercase `[a-z0-9][a-z0-9._-]*`, unique per host and renameable: `@acme/platform/tooling/codestyle`. No leading or trailing `/`, no empty segment, no `.` or `..`. **Depth is uncapped** — a forge nests as deeply as its groups do — and total length is bounded at 128.

**`repo.git` and `bin` are reserved as any segment.** Both are legal under the segment charset and both name a directory bitplane puts *inside* a project directory, and the walk that finds projects skips both by name — so a project called `@acme/bin` would sit at a path nothing ever enters, registrable once and invisible thereafter. The reservation is what makes that skip safe. `.bitplane` needs no rule: a segment cannot start with `.`.

Two derivation rules, which are one rule — *use the namespace when there is one*:

- An **owned project** takes the **full forge path after the host**, minus the transport and a trailing `.git`: `git@gitlab.com:acme/platform/tooling/codestyle.git` becomes `@acme/platform/tooling/codestyle`. A forge path is a namespace — stable, globally unique, and the thing whose absence made flat names collide. The host is not part of the name, so two forges sharing a path collide and fall to the ordinary refusal.
- An **adopted project** keeps the **last segment**. A filesystem path is an accident of where a home directory sits; `@users/alfonz/projects/bitplane` names the machine, not the project. So does an **owned project whose url names no host** — a bare local path, or a `file://` url — since that is the same accident wearing a git url's clothes.

A **derived** name is lowercased silently — a *normalisation*, not a guess, since two names differing only in case are the same directory on a case-insensitive filesystem. It never applies to `--name`, which is the user's stated intent, and parsing stays strict everywhere else, so a capital in a hand-edited file is a parse error and never a silent rewrite. A derived name still invalid after lowercasing fails naming the bad segment, with no suggestion.

When a name is already taken the operation is **refused**, never silently disambiguated — a generated default that renames itself behind the user's back is worse than an error naming the conflict. Same source url as the registered project means "already registered", with nothing to fix; a different url is offered the next free alternative, suffixed on the **last segment** (`acme/api-2`), never on the whole name, which would propose a nested project.

**A name may not nest inside another.** `@acme` and `@acme/codestyle` cannot both exist: the second would live inside the first's project directory, where removing `@acme` would take it along unmentioned and the registry walk would never see it. Refused unwaivably, in both directions.

Renaming a project moves its project directory and rewrites its entry; existing planes keep the subdirectories they were built with. See ADR-0009.

**`@` sigil**
How a project is referenced: `@acme/codestyle`. Syntax only — not part of the name, never in a `BITPLANE_*` environment variable, and **never on disk except as the member-kind tag in the plane file**, where it is the one place a project name and a path are both accepted. The parser strips it once, at the read boundary; a project name never carries the sigil in memory or in any other file. Required wherever a path would also be accepted (that being the only real ambiguity), optional elsewhere, and always used when bitplane prints a project.

**The sigil, not the charset, is what keeps the two member arms apart.** This entry used to add that a project name contains neither `@` nor `/`, and the member arms leaned on it. A name is a path of segments now, so the charset guarantees nothing — but nothing is lost, because the sigil was always doing the work: it is required in exactly the position where the ambiguity exists. `@acme/codestyle` is a project, bare `acme/codestyle` is a relative path, and `MemberRef` parsing is unchanged. See ADR-0009.

**projects directory**
The directory on a host holding one project directory per project. Exactly one per host, under `$XDG_DATA_HOME/bitplane/projects/` — application-owned data, not configuration, because it holds cloned repositories. **This is the registry** — there is no separate registry file, and the term **registry** is retired.

**project directory**
`<projects-dir>/<project-name>/`, containing `project.toml` and, for an owned project, the source repo at `repo.git`. Since a name is a path, this is a **nested** directory — `@acme/platform/tooling/codestyle` lives at `<projects-dir>/acme/platform/tooling/codestyle/` — and the directory name is still the name.

**Intermediate directories are not projects.** They are created by `add`, they hold no `project.toml`, and nothing lists them. `rm` and `rename` prune them bottom-up while empty, stopping at the first non-empty directory and never removing the projects directory itself. The walk that finds projects is therefore recursive, and stops at the first directory holding a `project.toml`, never descending into `.bitplane`, `repo.git`, `bin` or any dot-directory — an interrupted `add` leaves a bare `repo.git` with no `project.toml`, so without that skip every listing would walk a git object store. A depth cap of 16 bounds the walk, not a name.

`bin/` under it is **prepended to `PATH`** for every script that project runs, so a project can ship its own executables and have them win. It holds executables; a **script** is a declared entry in `project.toml`. The directory is the user's to create and bitplane never writes to it — which is what keeps the shell-alias trust posture true. A project is exactly "a directory containing `project.toml`", which makes the set of known projects self-describing. It is a named child rather than the directory itself, so git commands run while sitting in a project directory do not silently operate on it.

**source repo**
Internal term only: the local git repo a worktree is derived from — the bare repo at `<project-dir>/repo.git` for an **owned project**, the user's own checkout for an **adopted project**. The per-source-repo git lock attaches to this noun.

The working term **bare mirror** is retired, and `git clone --mirror` is **forbidden**. A worktree has no config of its own — it reads the source repo's — so `--mirror`'s `remote.origin.mirror = true` turns an ordinary `git push` into a force-push-everything-and-delete-the-rest against the forge, and its `+refs/*:refs/*` makes `fetch` fail outright once any plane branch also exists on the forge.

**plane branch**
A branch in a source repo's `refs/heads/*`. For a bitplane-owned source repo these are **exactly** the branches plane members were created on, **past and present** — the forge's branches live in `refs/remotes/origin/*` and never touch that namespace.

"Past and present" rather than "one per plane member": nothing records the branch a worktree was created on, so `destroy` deletes the branch the worktree is **actually on**, and a branch the user switched away from is left behind with no record of it. The namespace is therefore clean but not self-pruning; `doctor` reports branches with no worktree and no plane, and never deletes them.

This is what makes a branch the responsibility of the plane that owns it, and it is secured by one line of config on the source repo: `remote.origin.fetch = +refs/heads/*:refs/remotes/origin/*`. That line is load-bearing, not a default.

An **adopted** project's source repo is the user's own checkout, so its `refs/heads/*` are the user's branches and this invariant does not hold. Only the ownership rule below survives there.

**base branch**
The commit a new **plane branch** is cut from, resolved per project by `refs/remotes/origin/HEAD` > the source repo's own `HEAD` > unspecified. Never stored: a forge that renames its default branch would make a stored copy silently wrong.

An unresolved base is not an error by itself — it is reported as unspecified, and refused (`base_branch_unresolved`) only at the moment a **new** branch has to be cut from it.

It is a **base, not a default to land on**. bitplane never picks a branch for a worktree — every member's branch is given on the command line — so the base is used only when the branch asked for does not yet exist and has to be created. Amends ADR-0005's original resolution ladder, which read as a fallback for an unnamed branch.

**occupied branch**
A branch checked out in *any* worktree of a source repo, including the source repo's own. Git refuses `worktree add` on one, so a plane cannot use it. A bitplane-owned source repo is bare and therefore occupies nothing; an adopted project's checkout occupies whatever the user is sitting on, which is a refusal bitplane must report in its own words rather than passing git's message through.

**add** vs **adopt**
`add` takes a URL and creates a bare source repo in the project directory. **adopt** takes an existing local checkout and creates a project directory whose source points at it **in place** — bitplane never moves, copies or converts a checkout it did not create, because every path outside bitplane pointing at that repo must keep working.

Adoption applies to projects only. There is no plane-level adoption.

**script**
A **named** user-declared command, declared on a project, bound to zero or more **script points** and always runnable on demand with `bp run @project <name>`.

Declared as `[scripts.<name>]` in `project.toml` with either `argv` or `shell`, plus a boolean toggle per point it is bound to. **Order is TOML declaration order** — scripts run sequentially, so that order is semantic, and bitplane never rewrites the table.

`package.json`'s word for the same concept, so `bp run @api build` reads like `npm run build`. Distinct from the executables in `<project-dir>/bin/`, which are files on `PATH`, not declarations.

Replaces the working term **hook**, which stopped fitting once a script could be invoked by name rather than only fired at a point. _Avoid_: hook, action (the Engine's verbs are actions), task, command.

**script point**
A moment in a worktree's lifecycle at which bound scripts fire. Exactly two: `post_worktree_create` (per member, on `create` and `add`) and `pre_worktree_remove` (on `destroy` and `remove`).

Named for the **worktree**, not the plane, because scripts are per-project. A point is a binding, not something you can invoke: `bp run` takes script names only.

**Pre blocks, post does not.** A non-zero `pre_worktree_remove` aborts the pass and removes nothing; a failed `post_worktree_create` is reported and unwinds nothing. `--no-scripts` is always available, so a `project.toml` can never make a plane undestroyable.

## State

**state**
Everything bitplane knows, which is exactly what is on disk: the planes directory, the projects directory, and the files in them. There is no database, no index and no cache.

The term **store** is retired. It described a thing that was never built — once the projects directory became the registry and every plane carried a plane file, a store would only have held a second copy of facts the filesystem already holds. See ADR-0002.

Two rules follow, and they are vocabulary rather than implementation:

- **A plane is exactly a directory containing a plane file**, the mirror of "a project is exactly a directory containing `project.toml`". Anything else in the planes directory is not a plane and is ignored.
- **A read never writes.** `list`, `show` and discovery take no locks and create no files, which is what makes them safe to run from a script or in a loop.

**sentinel**
The file a lock is taken on — never the file being protected, because writing by atomic rename replaces the inode and would leave the lock guarding a file that no longer exists. One per plane directory and one per project directory.

**incomplete marker**
`<plane-dir>/.bitplane/incomplete` — present exactly while a plane has been claimed but never finished being created. A **one-way latch**: written at the claim, removed when the last worktree lands, never written again.

It is not a status field. It says one thing no other signal can: that the plane **never worked**, as opposed to having worked and since lost a worktree. That is what makes a latched plane safe to discard without refusal checks — nothing in it was ever the user's.

Only `create` may write it. An operation on a plane that already holds the user's work must never set it.

**abort window**
The span of a `create` between the claim and the last worktree landing, in which everything done can be discarded losing nothing. Scripts and fetches are deliberately kept outside it, which is what makes "throw it away and retry" a cheap repair rather than a lossy one.

**repair**
Reconnecting a plane's worktrees to their source repos after a directory moved — the plane directory (whether bitplane moved it in a `rename` or a user did with `mv`), a worktree within the plane, or the project directory behind it. Distinct from **reap**: repair fixes a plane, reaping destroys one.

It is the one operation licensed to **scan the plane directory** for worktrees, because it is an explicit request rather than a read. That is what keeps `list` and `show` to one `readdir` and one small file per plane.

**per-source-repo lock**
The mutex serialising every git command that *writes* to a source repo — `worktree add`, `worktree remove`, and `fetch`. It attaches to the source repo, so operations on different projects never contend. Reads take no lock.

## Contract

**`bp`**
The command bitplane installs on `PATH`. `bitplane` is the crate, repo and package name; `bp` is what a user types. Chosen to rhyme with the reserved `bp-` plane-id prefix — `bp create` yields `bp-a3f9c2e1`. See ADR-0001.

**Engine**
The request/response interface every mutation passes through; the machine contract the CLI renders.

**error envelope**
The five fields every failure is reported as — `error`, `code`, `message`, `problems`, `remedy` — built in `bitplane-core` so every surface reports a failure with the same tag, the same sentence and the same remedy.

It is **the structure, never a serialisation of it.** The envelope has two **renderings**: human by default, JSON under `--json`. Both go to stderr, because the *stream* is decided by what the output is (results on stdout, failures on stderr) and the *rendering* by the mode. Every line of the human rendering maps to exactly one field, which is what keeps them from drifting apart.

The distinction is load-bearing because conflating the two is what let "always JSON on stderr" survive unargued through ADR-0003 while ADR-0007 was independently making stderr the human stream. See ADR-0003's ticket-04 banner.

`error` is the **stable machine tag** and the one thing a script matches on; it is bracketed in the human rendering (`error[refused]:`) rather than hidden, because it is also what a user searches for. `code` is the exit status and is not printed.

_Avoid_: "the JSON error", "the error JSON" — both name a rendering as though it were the contract.

**reap**
Destroying a plane judged to be garbage.

The veto stands — uncommitted or unpushed work overrides every staleness signal — but the signals are only things bitplane owns: worktree directory missing, git's own `prunable`, source repo gone, `create` never completed, plane untouched for N days. **"Branch merged into its base" and "branch deleted on the remote" are not staleness signals**; they are opinions about the user's git workflow.

**waiver** vs **refusal**
A **waiver** says *"I accept losing **this** work, which I am looking at"* — `{uncommitted, untracked, unpushed, locked_worktree, source_repo_missing}`, granted per reason and per invocation, never one blanket `--force`.

A **refusal** is what is left when that sentence cannot honestly be said. Removing a project whose worktrees are live in some plane damages planes the user did not mention and is not looking at, so no consent given in that moment is informed: it is refused unwaivably, and the error lists the blocking plane ids. `project_in_use` is **not** a waiver.

**unpushed**
*The branch tip is not contained in any `refs/remotes/origin/*`.* Deliberately **not** `git branch -d`'s check, which compares against the branch's upstream — for a plane branch that is `origin/main`, so `-d` falsely refuses a branch already safe on the forge. Measured in ADR-0006.

**prunable**
Git's own word, and a **worktree** property: the worktree's directory is gone. Never a branch predicate.

## Boundary rules, as vocabulary

**bitplane owns a worktree's existence and location; the user owns its contents and its HEAD.**
bitplane creates a worktree, moves it, repairs its administrative entry and removes it. It never commits, pushes, stashes, checks out or resets. Between `worktree add` and `worktree remove` the directory is the user's, and switching branches, committing, pushing, rebasing or abandoning it is **ordinary use, not drift**.

**Reading is allowed; writing is not.** `plane_status` shelling `git status` across every member stores nothing and cannot go stale — it is git's answer, rendered. The objection to tracking state is about *storage and judgement*, and reading is neither. See ADR-0006.

Stated as ownership rather than as a list of permitted commands because it decides cases a list would not: it is what makes `git worktree repair` obviously in scope and `git stash` obviously out.

**git protocol yes, forge API no.**
bitplane speaks git. It does not create pull requests, merge, review, or read CI status.

**git's credentials, never ours.**
bitplane shells out to git and lets git's credential helpers and `ssh-agent` do the work. It never reads, stores, prompts for or forwards a credential. A private repo you cannot `git clone` by hand is one bitplane cannot use.

This is the auth-shaped restatement of the rule above, and it is what keeps owned projects from dragging a secrets story into the tool.
