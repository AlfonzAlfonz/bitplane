# ADR-0008: `plane.toml` is three keys, and membership is a path-keyed table

Status: accepted, amends [ADR-0003](./0003-the-engine-contract.md), [ADR-0006](./0006-bitplane-owns-a-worktrees-existence-not-its-contents.md) and [ADR-0007](./0007-project-schema-and-scripts.md)
Date: 2026-09-21
Ticket: `.alfonz/issues/bitplane-architecture/issues/09-plane-file.md`

## Context

Ticket 09 asked two things: what is in `plane.toml`, and how does a process
inside a plane discover which plane it is in. Four ADRs had already eaten most
of it — [ADR-0002](./0002-no-store-the-filesystem-is-the-state.md) made the file
*the* state rather than a cache of it,
[ADR-0004](./0004-create-aborts-rename-and-destroy-converge.md) fixed its write
timing and killed the status field,
[ADR-0006](./0006-bitplane-owns-a-worktrees-existence-not-its-contents.md)
stripped the branch out and made it the membership list, and
[ADR-0003](./0003-the-engine-contract.md) added the constraint that discovery
must be executable server-side.

What was left was the exact schema, the discovery mechanism, and — as it turned
out — a claim in `CONTEXT.md` that measurement showed to be false.

## Decision

### `plane.toml` is `version`, `id`, `[members]`

```toml
version = 1
id = "bp-a3f9c2e1"

[members]
"signageos/codestyle" = "@codestyle"
"projects/bitplane"   = "/Users/alfonz/projects/bitplane"
```

Every other candidate field died for one of two reasons: git or the filesystem
already stores it, or storing it would make a read write.

**`host` is derived, not stored.** ADR-0003 has no host parameter — you choose a
host by choosing which `Engine` you hold — so `PlaneSummary.host` is filled by
the engine that answered. A stored `host = "local"` is tautological when read by
the local engine and false the moment the directory is copied to another
machine.

**`bitplane_version` is struck from `PlaneSummary`.** It has no source and no
consumer. `version = 1` already owns forward compatibility, and keeping a
"written by" field honest means rewriting `plane.toml` on the first command
after every upgrade — a write on a read-ish path, which ADR-0004 names as the
signal that a field belongs in recomputed health instead.

**`created_at` is derived from the plane directory's birth time**, and
`PlaneSummary.created_at` becomes `Option<Timestamp>`. Measured on APFS:

| event | dir btime | `plane.toml` btime |
| --- | --- | --- |
| `mkdir` + first write | 20:04:38 | 20:04:38 |
| atomic-rename rewrite + `worktree add` | 20:04:38 | **20:04:45** |
| `mv` of the plane directory | 20:04:38 | 20:04:45 |

`plane.toml` is disqualified by ADR-0002's own write strategy: temp-file +
atomic rename swaps the inode, so its birth time is the time of the *last* write.
The plane directory's birth time is `mkdir`, which is step 1 of `create`, and it
survives every rewrite, every worktree add and a same-filesystem `mv`. Directory
*mtime* is not a substitute — it moved with the `worktree add`.

It is `Option` because `btime` is unavailable on some filesystems (NFS, older
ext4) and a **cross-filesystem `mv` resets it**. That is acceptable for a field
only `bp list` displays and nothing gates on; the cross-filesystem case is the
same event the `id` self-check already reports as "moved outside bitplane".

**Unrecognised keys are a `ParseError`**, matching ADR-0007's rule for
`project.toml`.

### Membership is keyed by worktree path, and the value names the source

This **reverses ADR-0006's `project -> path`**. The key is the worktree's path
relative to the plane directory root; the value identifies what it is a worktree
*of*.

- **Value starting with `@`** — the rest is a project name.
- **Any other value** — an absolute path to a repo (an **ad-hoc member**, below).

The arms cannot collide: a project name is `[a-z0-9][a-z0-9._-]*` and can
contain neither `@` nor `/`.

**Keys** are quoted, unique, forward-slash separated, and rejected if absolute,
containing `..`, starting with `./`, or beginning with `.bitplane` — a `..` in a
hand-edited file turns `destroy`'s per-member removal into a write outside the
plane directory. Keys must be quoted for a second reason: project names permit
`.`, and an unquoted TOML key containing a dot is a *dotted key*, so a member
named `foo.bar` would silently parse as a nested table. A nested table under
`[members]` is a `ParseError`.

**Values of the path arm are absolute and stored literally.** No `~` expansion
in a data file — "whose home" is unanswerable when a plane is read over SSH.

The reversal costs the property CONTEXT.md leaned on: keying by project made
*"at most one worktree per project"* a property of the file format. It is now an
**invariant the parser checks** — two members with the same `MemberRef` is a
`ParseError`, covering both arms with one rule. The cost is one comparison over
a handful of entries; it is bought deliberately, so that multi-worktree-per-project
can arrive later without a format migration. The asymmetry is the justification:
**the file is expensive to migrate and the contract is not.** `plane.toml` sits
on disk in every plane a user owns; `PerProject<T>` is a compiled type.

Lifting the restriction later has a named admission price, recorded so the
future feature answers it up front: **plane layout stops being a pure function
of the project source** — two members of `codestyle` both derive
`signageos/codestyle`, and the disambiguator that used to exist was the branch,
which ADR-0006 removed on purpose.

### The `@` sigil is admitted onto disk, in exactly one place

CONTEXT.md stated the sigil was *"never on disk, never in the store, never in a
`BITPLANE_*` environment variable"*. That absolute is amended to **"never on
disk except as the member-kind tag in `plane.toml`"**.

The amendment is justified by the sigil's own written criterion — *"required
wherever a path would also be accepted (that being the only real ambiguity)"* —
and the member value is now the only place on disk where a path would also be
accepted. It is an explicit tag, not the charset-sniffing alternative
(`does it contain a slash?`) that the same question got in ADR-0007 and lost.

One guard, because sigil leakage is the predictable failure: the parser strips
`@` **once, at the read boundary**, and a `ProjectName` never carries the sigil
in memory, in a comparison, or in any other file. `BITPLANE_PROJECT` stays the
bare name.

### An **ad-hoc member** is adopted-shaped, minus the registration

`bp create` and `bp add` accept an absolute path to a repo that is not a
registered project. This **amends ADR-0003's** *"a plane member is always a
registered project, never a bare path"*.

It is a **member kind, not a project kind**. It cannot be a project: CONTEXT.md
defines a project as *exactly a directory containing `project.toml`*, and an
ad-hoc member has none — no project directory, no name, no scripts, no `bin/`.
Keeping the definition intact is what keeps the projects directory
self-describing.

Every **adopted**-project rule transfers, because the situation is structurally
identical minus the registration: bitplane never deletes a branch in it, so
`destroy` skips branch deletion (ADR-0006); there is nothing to fetch; and it
**occupies** whatever branch its working tree sits on, which is a refusal
bitplane reports in its own words. Layout needs no new rule — CONTEXT.md's
derivation table already maps `~/projects/bitplane` → `projects/bitplane`.

**Scripts: an ad-hoc member behaves exactly like a project with no scripts
defined.** Nothing runs, nothing is reported, and it is not a special case in
the script path at all. Accepted cost, recorded once: a user expecting a
`pre_worktree_remove` to stop their containers gets no indication that none was
bound — the same silence any project with an empty `[scripts]` gives. `bp run`
against an ad-hoc member is a plain error, since `bp run` takes a project.

`ad-hoc` was chosen over three alternatives. **`unregistered` is unavailable** —
CONTEXT.md retired the word "registry", so the term would revive it. **`bare`**
collides with the bare source repo. **`anonymous`** is accurate but names only
the absence; `ad-hoc` names the trade — convenience now, no setup, no scripts.

### A plane fan-out is keyed by `MemberRef`, not `ProjectName`

ADR-0003 keys every `Vec<PerProject<T>>` by `ProjectName`. An ad-hoc member has
none, so **plane** fan-outs are re-keyed to `MemberRef` — the union already in
the file, `{ Project(ProjectName), Repo(PathBuf) }`, rendered on the wire and on
screen as the string the file stores (`@codestyle`, `/Users/…`). `ProjectName`
survives as the key for every `project_*` action.

Keying by the **worktree path** was rejected despite being shorter and already
unique: it is the field `bp repair` rewrites when a user moves a worktree, so
the same command would label the same member differently before and after a
move — destroying the cross-run correlation that ADR-0003 built the keyed-vector
shape to provide.

### Discovery is a walk to the root, performed by the Engine

`PlaneRef::ContainingPath` is resolved by **canonicalising** the given path and
walking up to the filesystem root; the **innermost** `plane.toml` wins. It runs
in the Engine, not the CLI, which is ADR-0003's constraint — a client cannot
walk a remote filesystem.

**There is no planes-directory constraint.** A `plane.toml` found outside the
configured planes directory still resolves; being outside is a *finding*, not a
resolution failure. Refusing it would make `bp repair` impossible to aim at a
plane from inside it, which is the one case the `id` self-check exists to
diagnose.

Canonicalisation comes first so a worktree reached through a symlink resolves to
its real plane. Nested planes resolve to the innermost, which is the only answer
that can be right.

**A path resolves to a plane, never to a member.** Identifying the member would
be free — the membership table is now literally a path-keyed map — and it is
declined anyway. ADR-0007 made `bp run`'s project argument mandatory and
ADR-0006 made `bp add`'s branch mandatory, both because an implicit subject
inferred from context is where surprise damage comes from. A ref that sometimes
carries a member is a second implicit subject only some actions honour, and a
user cannot predict which.

**bitplane never reads its own injected environment variables back.**
`BITPLANE_PLANE_DIR` and `BITPLANE_PLANE_ID` are for scripts and agents to
consume; `bp` always resolves from cwd, with `--plane` as the explicit escape.
Reading them back means a stale value inherited from an outer shell makes
`bp status` report a plane you are not standing in — and since the plane id is
**mutable by design**, a stale value can resolve to a *different* plane that has
since taken the name. Every script already runs with cwd inside its worktree, so
the walk answers identically and cannot go stale.

This is distinct from `BITPLANE_PLANES_DIR` (ADR-0002), which bitplane does
read: that is **configuration**, not identity.

## Consequences

- **A worktree cannot be moved within a plane without a repair, and the freedom
  CONTEXT.md granted is retracted.** Measured against a real bare repo and
  worktree: after a plain `mv` within the plane,
  `git worktree list --porcelain` reports the **old** path and marks the
  worktree `prunable — gitdir file points to non-existent location`. The
  worktree's own `.git` file survives and `git status` inside it still works;
  `git worktree repair` run **from inside** the moved directory fixes git's
  record and clears `prunable`. So the freedom was never real — git revokes it,
  not `plane.toml` — and worse, `prunable` is a **reap staleness signal**, so a
  user who moves a worktree creates something reaping treats as garbage at a
  path where reap cannot find their work to veto on.
- **`plane_repair` gains the filesystem scan**, and that is what replaces the
  retracted freedom. It walks the plane for `.git` files, runs
  `git worktree repair` on each, and rewrites `plane.toml`'s keys to match.
  Moving a worktree becomes an explicit, supported operation instead of a silent
  hazard. The scan is affordable there because `repair` is an explicit request;
  it is **not** affordable on the read path, which is why membership stays
  stored rather than derived by containment.
- **Membership by containment was rejected, and the reason is worth keeping.**
  Deriving members by scanning for `.git` files would make
  `Finding::MemberWorktreeMissing` **unrepresentable** — a missing worktree
  would simply not be a member, so a plane that lost three of six worktrees
  would look healthy, and ADR-0004's died-unwind signal would have nothing to
  fire on. It also inverts ADR-0006's own boundary: bitplane owns a worktree's
  **existence and location**, so membership is the record of precisely the thing
  bitplane owns, and deriving it from a user-owned `.git` file hands that record
  to the user.
- **`plane_remove` rewrites `plane.toml` last**, mirroring ADR-0004's rule that
  `destroy` deletes it last. A crash between the two then leaves a *listed*
  member with no worktree — visible as `MemberWorktreeMissing`, converging on
  re-run — rather than an unlisted worktree nothing can see. With that ordering,
  **a directory in a plane that no member names is silently ignored, not a
  finding**: it is almost always the user's own scratch space, and ADR-0006
  gives them the plane directory's contents.
- **`project_rename` becomes a multi-plane mutation.** Moving the project
  directory breaks every member worktree's gitdir pointer, so the sweep is
  already O(planes) regardless of the key rewrite. Order: acquire every holding
  plane's lock, then the project lock (**plane→project**, per ADR-0002 — the
  naive implementation inverts this) → `mv` the project directory →
  `git worktree repair` per plane → rewrite each `plane.toml` value → rewrite
  `project.toml`'s `name` **last**, preserving ADR-0007's interrupted-rename
  detection so a re-run converges. Finding the holders means scanning the planes
  directory, since nothing maps project → planes. The contract gains
  `PerPlane<T>` keyed by `PlaneId`.
- **`doctor`'s source-repo sweep extends to ad-hoc repos.** The sweep runs
  `git worktree list --porcelain` against each *registered project's* source
  repo, so an ad-hoc member's repo would never be swept and its orphaned
  worktrees would be invisible to every check bitplane has. The repo paths are
  in the plane files `doctor` already reads, so the extra cost is bounded by the
  number of distinct ad-hoc repos.
- **ADR-0007's log tee stands.** It was reopened in session and kept:
  `<plane-dir>/.bitplane/logs/` remains the durable record of script output.
  What does *not* happen is any script outcome landing in `plane.toml` — an
  earlier ticket-05 decision, now reversed. Storing outcomes would make `create`
  write the file a third time and would store a judgement that goes stale the
  moment the user re-runs `bp run` and fixes the thing.
- **The write count is unchanged at two for the happy path** — `create` and
  `rename` — plus once per membership change. Nothing decided here writes on a
  read.

## Alternatives considered

**Membership derived by containment (no `[members]` at all).** The most
attractive rejected option, and the reason is in the consequences above:
`MemberWorktreeMissing` becomes unrepresentable, `bp list` acquires a recursive
walk of every plane directory on the hot path (needing a depth cap, which is a
heuristic under the enumeration of core state), and resolving an **adopted**
project's gitdir back to a project requires scanning every `project.toml`.

**`project -> path` (ADR-0006's direction), kept.** Rejected in favour of the
reversal above, accepting the loss of the format-level uniqueness property, to
avoid a `version = 2` when multi-worktree-per-project arrives.

**Names only — `[members]` as a set of project names, location derived.**
Attractive because the name tells you which source repo to ask, so
`git worktree list --porcelain` finally works as an oracle. Killed by the same
measurement: after a bare `mv`, porcelain returns the **old** path, so location
fails exactly when it is needed.

**A tagged sub-table per member** (`[members."path"]` with `project = …` or
`repo = …`). Rejected once it was clear the *key* is the multi-worktree
disambiguator, so no per-member field is needed to hold one. The `@` sigil is an
explicit tag in one line rather than three.

**Reserving the path arm (format defines it, parser refuses it).** Recommended
twice and rejected twice: ad-hoc members are wanted live, with the script
contract answered by "behaves like a project with no scripts".

**Env-var-first discovery, or env-var fallback.** Rejected — see the
mutable-id staleness argument above.
