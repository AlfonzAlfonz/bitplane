# ADR-0002: There is no store — the filesystem is the state

Status: accepted
Date: 2026-09-20
Ticket: `.alfonz/issues/bitplane-architecture/issues/05-state-store.md`

## Context

Ticket 05 was written to decide the format, location and concurrency of bitplane's **state store**. By the time it was unblocked, ticket 03 had removed the store's reason to exist without anyone noticing.

Ticket 03 settled that the **projects directory is the registry** — one project directory per project, each containing `project.toml`, so "a project is exactly a directory containing `project.toml`". It also settled that every plane directory carries a **plane file** (`plane.toml`) at its root describing the whole plane, and that there is **exactly one planes directory and one projects directory per host**.

Those two directory walks already answer every question a store would answer: what planes exist, what projects exist, what is in each. A separate store would hold a second copy of facts the filesystem already holds authoritatively — needing synchronisation, invalidation, and a coherence story — and would buy query speed on a dataset of perhaps 10–50 planes, against an SSH floor of 43 ms (local) to 1.2 s (real internet) that bitplane pays anyway.

ADR-0001 deliberately left `rusqlite` with the `bundled` feature alive for this ticket. This ADR declines it.

## Decision

### There is no store

**The filesystem is the state.** `bp list` reads the planes directory and parses each `plane.toml`. `bp project list` reads the projects directory. There is no index, no cache, no database, and no rebuild-by-scanning problem — because scanning *is* the read path.

**"Store" is retired as a term**, alongside "registry" and "marker" (ticket 03).

This dissolves the ticket's hardest stated problem for free. The plane id is mutable, and the ticket worried about a store row whose key moves during a rename. With no store there is no row: the **directory name is the key**, and renaming a plane is `mv`. What remains of rename is git's admin entries, which is ticket 07's problem.

`rusqlite bundled` is rejected. It would have cost a C cross-toolchain for `aarch64-unknown-linux-musl`, and it reintroduces exactly the coherence problem the atomic-rename design was built to avoid.

### Locations

| what | where |
| --- | --- |
| config | `$XDG_CONFIG_HOME/bitplane/config.toml` (fallback `~/.config`) |
| projects directory | `$XDG_DATA_HOME/bitplane/projects/` (fallback `~/.local/share`) |
| planes directory | `~/planes/` |

XDG is honoured on macOS too, rather than `~/Library/Application Support` — a developer who has set `XDG_DATA_HOME` means it.

Config and projects are split because they are different kinds of thing: a project directory holds a bare mirror, potentially gigabytes of packfiles, and must not land in a directory people sync to a dotfiles repo.

The planes directory is **not** under XDG. It holds the user's actual working trees; it is the one path a human types daily, and `cd ~/.local/share/bitplane/planes/bp-a3f9c2e1` is hostile. Overridable by config and `$BITPLANE_PLANES_DIR`, precedence **flag > env > config > default**.

### Format

**TOML**, with `version = 1` in every file from the first commit. An unknown major version is **refused, not guessed at**.

Hand-editing is **tolerated but not documented**. It is the escape hatch that makes a plain-text state worth having and forbidding it would be unenforceable — but documenting it would make the schema a public contract before the schema is known.

Because editing is tolerated, **`toml_edit` is used for read-modify-write** of `plane.toml` and `project.toml`, not serde round-tripping. The default `toml` crate serialises from a data structure and so discards comments, key ordering and formatting; a user's `# do not reap, long-running migration` would vanish on the next `bp add`. A plain-text state whose only advantage over sqlite is that humans can read and edit it must not silently eat the edits. `serde` derive still covers the `Engine` request/response types, which are a wire format with no file to preserve.

### Multi-host

**bitplane keeps no state about a host except on that host.** `bp --host devbox list` opens SSH, runs the servant, and returns what that host says; the local machine keeps no record of remote planes.

A local aggregate would be a cache of a machine you are not talking to, wrong the moment someone runs `bp` directly on the remote — which they will, because that is where they are working. Aggregated listing, if ever wanted, is a parallel fan-out over SSH with rows labelled by host, which is honest about its cost.

### Writing: atomic rename, done properly

Every file is written to a temp file **in the target's own directory** — never `$TMPDIR`, because `rename(2)` across filesystems fails `EXDEV` and `~/planes` on a separate volume is ordinary — then `fsync`'d, then renamed, then the parent directory is `fsync`'d.

The `fsync` is load-bearing and not optional. `rename(2)` is atomic with respect to other *processes*, but says nothing about power loss: without the fsync you can reboot into a `plane.toml` that is present and zero bytes, which with no store means a plane full of real worktrees that `bp` can no longer see.

**`F_FULLFSYNC` is not used on macOS.** It forces a full drive cache flush at tens of milliseconds; ordinary `fsync` covers process and kernel crashes, and the residual power-loss window is one a developer tool accepts.

### Locking

**`flock(2)`** via a pure-Rust crate (`fs4` / `fd-lock`). Chosen over an `O_EXCL` pid-file specifically because the kernel releases it when the fd closes, **including on `SIGKILL`** — so the ticket's "what happens to a stale lock left by a killed process" question does not arise. No liveness probing, no pid-reuse handling, no `--force-unlock`.

Consequence: **a projects directory on NFS is unsupported**, since `flock` over NFS is unreliable. Acceptable — it also holds bare mirrors nobody wants on NFS.

**Lock a sentinel, never the data file.** This is the subtle part and must not be "simplified" away. Atomic rename *replaces the inode*: a process holding an `flock` on `plane.toml` is holding it on an inode that the writer has already swapped out, so a second writer wakes holding a lock on a file that no longer exists and clobbers the first. The locks are therefore:

| sentinel | protects |
| --- | --- |
| `<plane-dir>/.bitplane/lock` | read-modify-write of that plane's `plane.toml` |
| `<project-dir>/.bitplane-lock` | that project's `project.toml`, **and** every git command that writes to its source repo |

One sentinel per project directory, not two — the `project.toml` write lock and the per-source-repo git lock protect the same object.

The plane lock is **not** the global lock ticket 03 forbade: it is per-plane, so operations on different planes still never contend, which was the point of that constraint. It exists because atomic rename prevents torn reads but not **lost updates** — concurrent `bp add @a:x` and `bp add @b:y` on one plane would otherwise silently drop one project while leaving its worktree on disk, orphaned and invisible.

**Acquisition: block with a timeout.** Try non-blocking; on failure print `waiting for @codestyle…` and block up to 120 s (a cold fetch of a large mirror genuinely takes that long), then fail naming the project. The common case stays silent. The failure names the project, not a pid — `flock` does not identify its holder, and that is fine.

**Ordering: plane sentinel → project sentinel, never the reverse; and a thread holds at most one project lock at a time.** Two rules, both auditable by reading the code, and together they make a deadlock cycle unconstructible. The timeout is the backstop if the invariant is ever broken: a named failure rather than a hang.

### Scan semantics

- **A plane is exactly a directory containing `plane.toml`** — the mirror of ticket 03's rule for projects. A directory without one is silently skipped, so an unrelated directory dropped in the planes directory is ignored rather than being an error.
- **Per-entry error isolation.** A `plane.toml` that does not parse is listed as a row in an error state naming the file and the error; the scan continues. Never abort the listing, and never auto-repair — a file bitplane cannot read is a file it has no business rewriting.
- **A read never writes.** `list`, `show` and discovery take no locks and create no files. This is what makes them safe from a hook, from a prompt, or in a loop, and it is the concurrency-shaped restatement of ticket 10's "nothing is deleted as a side effect of a read."

### Identity

**The directory name wins.** `plane.toml` carries `id` as a **self-check**, not as the authority.

A user who runs `mv ~/planes/bp-a3f9c2e1 ~/planes/auth-work` outside bitplane leaves the directory saying one thing and the file another — and, more painfully, leaves every git admin entry pointing at the old path, so all the worktrees are now `prunable`. The mismatch is worth keeping precisely because it is a positive signal: *this plane was moved outside bitplane*. Dropping `id` from the file would make that indistinguishable from a healthy plane, and would also make a copied plane directory unrecognisable as a copy.

**A plane directory is claimed by `mkdir(2)`, which is atomic and fails `EEXIST`.** Never probe first — a `exists()?`-then-create is a TOCTOU that two concurrent `bp create`s both pass. `EEXIST` on a generated id means retry with fresh hex, bounded at 5 attempts (needing 6 means something other than collision is wrong); `EEXIST` on a user-supplied id is a typed "plane id already in use" error, returned immediately, never retried.

Note the claim and the visibility are deliberately different instants: `mkdir` claims the directory before `plane.toml` exists.

### Hook results

The **outcome** (`name`, exit code, started/finished) goes in `plane.toml`. The **output** goes to `<plane-dir>/.bitplane/logs/<hook>-<timestamp>.log`.

A hook is a user command and its stdout can be a multi-megabyte `pnpm install` log; that cannot live in a file whose whole value is being small and hand-readable. `.bitplane/` uses the same trick as the plane file — the plane directory root is outside every repo, so nothing there can be committed by accident. Retention on those logs is ticket 10's.

`.bitplane` is therefore a **reserved path segment at the plane directory root**: a project whose derived path would start with it is refused at `add`/`adopt` time with a suggested alternative, the same treatment as a colliding project name.

### `git worktree lock` is not a mutex — tested and rejected

Ticket 01 recommended adopting `git worktree lock` as a git-native liveness mutex. Measured against git 2.50.1, it fails on both counts:

1. **It does not apply to a bare mirror at all.** `git worktree lock` on a bare repo or on any main worktree is a deterministic `fatal: The main working tree cannot be locked or unlocked` (exit 128). It only ever locks *linked* worktrees. There is no repo-level lock verb in git.
2. **It is not atomic even on a linked worktree.** It is a check-then-write TOCTOU over `worktrees/<name>/locked`. Barrier-synced: two concurrent `lock` calls **both succeeded in 20 of 50 rounds**; at 8-way, 16 of 30 rounds had 2–4 simultaneous winners. And it has no liveness whatsoever — a `SIGKILL`'d owner leaves the file held forever, clearable only by `git worktree unlock`.

It remains useful for what it actually is: a **persistent advisory marker** that blocks `git worktree remove` and is skipped by `git worktree prune`, surfaced as the `locked` field in `--porcelain`. It is not a concurrency primitive.

### What actually races, and what the lock is for

The per-source-repo lock survives, but its justification is not the one the ticket assumed. `index.lock` is **per-worktree** (`worktrees/<name>/index`), never shared — pre-creating one in a bare root does not affect `worktree add` at all. `worktrees.lock` does not exist; the string is not in the git binary.

Measured, barrier-synced, against one bare mirror:

| race | result |
| --- | --- |
| 2× `worktree add -b <different>` | **30/30 fail** — `config.lock`, from the upstream-tracking config write |
| same, with `--no-track` | 0/30 |
| 2× `worktree add -b <same name>` | 30/30 one side fails: `cannot lock ref: reference already exists` |
| 2× `worktree add <path> <same existing branch>` | **25/25 both succeed** — same branch checked out twice |
| `fetch` ‖ `worktree add --detach` | 0/60 |
| `fetch` ‖ `worktree add -b <new>` | 1/60 — `Failed to resolve 'HEAD' as a valid revision` |
| `fetch` ‖ `fetch` | **40/40 fail** — `cannot lock ref: is at X but expected Y` |

So the lock covers **`fetch` as well as `worktree add` and `worktree remove`** — fetch-vs-fetch fails 100% of the time — and it is needed because git's own "already used by worktree" guard is non-atomic: serially git correctly refuses, concurrently both sides win and the same branch ends up checked out twice. Reads (`worktree list`) take no lock.

**`--no-track` is not used**, despite being what makes concurrent `worktree add -b` race-free (30/30 failures → 0/30). The concurrency motivation disappears once the lock exists, and deliberately creating upstream-less branches would hand ticket 10 a worse problem than the one it already has — tracking branches make "unpushed work vetoes reaping" trivially computable via `@{u}`.

### A plane holds at most one worktree per project

Surfaced by the lock question and settled here. `CONTEXT.md` derives a worktree's path from the **project source alone** (`git@gitlab.com:signageos/codestyle.git` → `signageos/codestyle`), with no branch component — so `@codestyle:main` and `@codestyle:review-x` in one plane derive the same path and collide.

Rather than complicate the derivation, the constraint is made explicit: **`(project, branch)` pairs in a plane are unique on *project***, and `create`/`add` reject a duplicate project with a typed error. This keeps layout derivation a pure function of the source, which is what makes ticket 03's "derived once at create time, never recomputed" safe. Reviewing two branches side by side is naturally two planes, which is the tool's whole point.

Side effect: the per-source-repo lock can never contend *within* a single plane's fan-out, so `std::thread` per-project parallelism is always genuinely parallel.

## Consequences

- **Ticket 05's own subject does not exist.** The ticket should be read as "on-disk state: layout, locking and concurrency".
- The map's deferred **"store schema migration and versioning"** item stays answerable: `version = 1` reserves the field at zero cost, and refuse-on-unknown-major is the minimum viable half.
- **Ticket 07** inherits the write-ordering argument: writing `plane.toml` *last* leaves an interrupted `create` as a directory full of real worktrees that the plane scan structurally cannot see, findable only by a source-repo-side `worktree list --porcelain` sweep. That is an argument for converge, but 07 makes the call. Rename authority (directory name wins, file `id` is a self-check) is settled here and not reopened there.
- **Ticket 09** inherits: `plane.toml` carries `version = 1` and `id`-as-self-check, the scan contract, and per-hook outcomes; its open question about store-vs-file disagreement collapses to directory-name-vs-file-content.
- **Ticket 10** inherits computable "unpushed" (branches have upstreams) and owns retention for `<plane-dir>/.bitplane/logs/`.
- **Ticket 06** inherits: `--id` collision is a typed error; duplicate project in a plane is a typed error; `Engine` types keep serde derive.
- **Ticket 01's `git worktree lock` recommendation is withdrawn** as a mutex and retained as an advisory marker.
- Crates added: `toml_edit`, an `flock` wrapper (`fs4` or `fd-lock`), a directories crate. All pure Rust — no vendored C, no cross-toolchain cost, consistent with ADR-0001's linking constraint.

## Alternatives considered

**`rusqlite` with `bundled`.** Rejected. ADR-0001 kept it alive for this ticket, and it would have been the right answer if bitplane had a query problem or a scale problem. It has neither: the biggest read is 50 small TOML files, sub-millisecond against an SSH floor three orders of magnitude larger. It costs a C cross-toolchain for `aarch64-unknown-linux-musl`, and — the real objection — it makes the database and the filesystem two sources of truth that must be reconciled after every crash, which is precisely what the no-store design gets for free.

**A cache over the filesystem, rebuildable by scanning.** Rejected as sqlite's costs deferred rather than avoided. It has the same read path as no-store plus an invalidation story, in exchange for a speedup nobody has measured a need for.

**`O_EXCL` pid-file locking.** Rejected. Survives process death as a stale file, so it needs liveness probing, pid-reuse handling and a force-unlock escape hatch — all machinery `flock` provides for free by being kernel-held.

**A local aggregate of remote hosts' planes.** Rejected; see Multi-host.

**Putting everything under one directory** (`$XDG_DATA_HOME/bitplane/`, planes included). Considered for simplicity and rejected: changing the planes directory default is free today and expensive once there are users, since it means moving every worktree and repairing every git admin entry.
