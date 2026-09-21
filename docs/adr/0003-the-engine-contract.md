# ADR-0003: The Engine contract is eighteen coarse actions over a serialisable wire

Status: accepted, amended by [ADR-0004](./0004-create-aborts-rename-and-destroy-converge.md), [ADR-0005](./0005-the-source-repo-is-a-bare-clone-shaped-repo.md), [ADR-0006](./0006-bitplane-owns-a-worktrees-existence-not-its-contents.md), [ADR-0008](./0008-the-plane-file-is-three-keys-and-a-path-keyed-membership-table.md) and implementation ticket 04
Date: 2026-09-20
Ticket: `.alfonz/issues/bitplane-architecture/issues/06-engine-contract-action-set.md`
Sketch: `prototypes/06-engine-contract/` (throwaway; `cargo test` passes)

> **Amended by ADR-0004 in three places.** The decision below stands; these details do not.
>
> 1. **The action set is nineteen, not eighteen.** `plane_repair` joins `Engine`, and `doctor` gains a second sweep (directories holding `.bitplane/` but no `plane.toml`).
> 2. **Rename's primitive is `git worktree repair`, not `git worktree move`.** You cannot `git worktree move` a worktree whose parent directory has already been moved — git looks for it at the old path. `repair` takes every new path in one invocation and is idempotent by construction. The mechanics described under `PlaneRenameRequest` below are wrong; the response shape is unchanged.
> 3. **The envelope `Err` rule is widened** from "failures that stopped the operation from *starting*" to "operations that produced **no durable state**" — covering both "never started" and "started, then fully unwound". A failed `plane_create` now returns `Err(EngineError::CreateAborted { projects, rollback })` and loses its `partial` field. Every other operation is unaffected, and `partial: true` keeps meaning what it says.
>
> Also settled downstream: `Waivers` grows a sixth member, `source_repo_missing`.

> **Amended by ADR-0005 in four places.**
>
> 1. **The action set is seventeen.** `plane_commit` and `plane_push` are scoped
>    out; `plane_repair` stays. Six reads, eleven mutations.
> 2. **The `bp push` blocking question below is void, not answered.** A worktree
>    reads the *source repo's* config, so `origin` is the forge under every clone
>    flag — the described state cannot occur. **`Pushed` and
>    `Pushed::remote.is_bitplane_mirror` are deleted.** The real defect was
>    `git clone --mirror`, now forbidden.
> 3. **`ProjectView.default_branch` is derived, not stored.** `project.toml`
>    carries no such field; the third resolution rung reads
>    `refs/remotes/origin/HEAD`, falling back to the source repo's `HEAD`.
>    **Further amended by implementation ticket 04**: there is no first or
>    second rung any more. `refs/remotes/origin/HEAD` is now the *first*, the
>    resolved value is the **base** a new branch is cut from rather than a
>    branch a worktree lands on, and `create` and `add` both require an explicit
>    branch. See ADR-0005's banner.
> 4. **`BranchIntent::Resolve` combined with `fetch: false` is an unresolved
>    hazard** — it silently creates an unrelated branch against a stale source
>    repo. Flagged in ADR-0005's consequences, not yet ratified.

> **Amended by ADR-0006 in four places.**
>
> 1. **`plane_status` survives and is unchanged.** Recorded explicitly because
>    ADR-0006 forbids bitplane managing worktree state, and the question of
>    whether a `git status` fan-out is "managing" was put and answered: reading
>    is allowed, writing is not.
> 2. **`Waivers` drops to five** — `{uncommitted, untracked, unpushed,
>    locked_worktree, source_repo_missing}`. **`project_in_use` is removed**:
>    removing a project with live worktrees is refused unwaivably, and the error
>    lists the blocking planes by id.
> 3. **`unpushed`'s check is specified**, and it is *not* `git branch -d`. The
>    predicate is *"is the branch tip contained in any `refs/remotes/origin/*`"*.
>    `git branch -d` compares against the branch's upstream, which for a plane
>    branch is `origin/main`, so it falsely refuses a branch already safe on the
>    forge — measured, see ADR-0006.
> 4. **`doctor` gains a fourth sweep**: source-repo branches with no worktree and
>    no plane. It reports them and never deletes them.
>
> Also: the fan-out key stays `ProjectName` — a plane member is always a
> registered project, never a bare path.

> **Amended by ADR-0008 in four places.**
>
> 1. **A plane member is no longer always a registered project.** An **ad-hoc
>    member** is an absolute path to an unregistered repo, accepted by
>    `plane_create` and `plane_add`. The sentence "the fan-out key stays
>    `ProjectName` — a plane member is always a registered project, never a bare
>    path" is superseded.
> 2. **Plane fan-outs are keyed by `MemberRef`**, the union
>    `{ Project(ProjectName), Repo(PathBuf) }`, rendered as the string the plane
>    file stores. `ProjectName` still keys every `project_*` action.
> 3. **`PerPlane<T>` keyed by `PlaneId` joins the contract**, for
>    `project_rename`'s sweep across every plane holding the project.
> 4. **`PlaneSummary` changes**: `bitplane_version` is **struck** (no source, no
>    consumer), `host` is filled by the answering engine rather than read from
>    disk, and `created_at` becomes `Option<Timestamp>` derived from the plane
>    directory's birth time.
>
> Also settled downstream: ticket 09's `PlaneRef::ContainingPath` resolves by
> canonicalising and walking to the filesystem root, innermost `plane.toml`
> wins, with no planes-directory constraint — and it resolves to a **plane
> only**, never to a member.

## Context

"Machine contract first; the human CLI renders it" has been settled since
charting, and ADR-0001 gave it a structural form — `bitplane-core` owns the
`Engine` trait and its types, `bitplane-cli` renders them. What the contract
*was* had never been written down.

It could not be deferred further. Ticket 07 (plane lifecycle: partial failure
and repair) was explicitly blocked on it, because whether convergence is
affordable depends entirely on what there is to converge — and that is a
property of the response types, not of the implementation.

Three rules were already binding: one request equals one user intent (coarse
granularity, so a remote transport is not a latency disaster); single-value
responses, with streamed progress deferred; and everything serialisable in both
directions, even though the only transport today is in-process.

## Decision

### Eighteen actions, split into two traits

| | read | mutate |
| --- | --- | --- |
| **plane** | `plane_list`, `plane_show`, `plane_status` | `plane_create`, `plane_rename`, `plane_destroy`, `plane_add`, `plane_remove`, `plane_commit`, `plane_push` |
| **project** | `project_list`, `project_show` | `project_add`, `project_adopt`, `project_rename`, `project_remove`, `project_fetch` |
| **host** | `doctor` | — |

`Reader` holds the six reads; `Engine: Reader` adds the twelve mutations. The
split exists so ADR-0002's **"a read never writes"** is checkable by reading the
trait rather than by auditing every body. Sync throughout, per ADR-0001.

The traits are the ergonomic face. A `Request` / `Response` enum pair plus
`dispatch()` is what crosses a transport; `dispatch` is a `match`, so an action
added to the trait and forgotten on the wire fails to compile.

**No `host` parameter.** The host is chosen by picking which `Engine` you hold —
`LocalEngine` now, `SshEngine` later.

### A fan-out response is a vector of keyed per-item outcomes, never a scalar

Every long operation returns `Vec<PerProject<T>>` keyed by `ProjectName`, each
row being `Ok` / `AlreadyDone` / `Skipped(SkipReason)` / `Failed(EngineError)`.
The envelope `Result` is reserved for failures that stopped the operation from
**starting**: plane not found, malformed request, a lock timeout on the plane.
Once the fan-out has begun, a per-project failure is **data, not an error**.

This is the load-bearing decision. It is what makes convergence affordable for
ticket 07 with no store behind it: a re-run of the identical request produces a
comparable vector, and every row already says which of the states it is in.

It is also, for free, the streaming hedge — see below.

### `PlaneRef` is `Id` or `ContainingPath`

A client cannot walk a *remote* filesystem up to `plane.toml`. Without the path
variant, every in-plane command (`bp status`, `bp add`, `bp commit`) would be
resolve-then-act — two round trips, the precise failure mode coarse granularity
exists to prevent. Ticket 09's discovery mechanism therefore acquires a
constraint it did not know it had: whatever it picks must be executable
server-side.

### Ambiguity is resolved by an intent flag, never by a lookup round trip

`create` and `add` carry `BranchIntent::{Resolve, RequireExisting, RequireNew}`.
`Resolve` — the default — checks the branch out if it resolves after the fetch
and creates it otherwise, and the response reports `created_branch: bool`. The
ambiguity costs one field instead of a "does this branch exist?" call.

### Refusals are collected before anything is touched, and waived per reason

`destroy` and `remove` are check-all-then-act: every refusal across every project
returns in one `EngineError::Refused`, because a half-destroyed plane is worse
than a refused one. Waivers are per-reason — `{uncommitted, untracked, unpushed,
locked_worktree, project_in_use}` — adopting Orca's pattern (ticket 01) rather
than a blanket `--force`. The response echoes `waived`, so a forced destruction
is visible in a transcript.

### There is no reconcile action: staleness is a field on a read

ADR-0002 forbids a read writing, so a staleness mark cannot be stored; it is
recomputed. `PlaneHealth { level, findings }` rides on `plane_show` and
`plane_list`, with `HealthCheck::{None, Cheap, Full}` controlling the cost —
`Cheap` is filesystem-only, `Full` shells out for git's `prunable`. `bp list`
runs constantly and must not spawn git per project.

Drift is **a state the tool reports**, not an error (jj's posture, ticket 01),
and it maps to wkt's exit code 3.

### `doctor` is in the action set, and it is a read

Two jobs nothing else can do:

1. **The source-repo sweep.** `git worktree list --porcelain -z` against each
   source repo finds worktrees the plane scan structurally cannot see — the
   invisible-garbage case ADR-0002 flagged for an interrupted `create`. This is
   its only oracle. Off by default; it is O(projects) git invocations.
2. **`footprint` is the uninstall answer** (`wkt doctor --all`): every path
   bitplane owns, plus every `worktrees/<name>/` admin entry it wrote into a repo
   it does not own — the only mark it leaves outside its own directories.

It reports and never repairs, which is why it lives in `Reader`.

### Typed exit codes: wkt's, plus one

`0` ok · `1` typed failure · `2` usage or already-exists · `3` drift ·
`4` prerequisite missing (git < 2.36) · **`5` busy**.

`5` is bitplane's addition, for `LockTimeout`. A lock timeout is neither a usage
error nor a failure of the operation — it is "try again", and a script needs to
tell that apart to know whether retrying is sane. `ErrorEnvelope { error, code,
message, problems, remedy }` is constructed in `bitplane-core`, so the JSON
written to stderr (stdout staying clean for results) is byte-identical across the
CLI, a future MCP surface, and the SSH servant.

### The streaming hedge is cheap, and it is the response shape

Yes, there is a cheap hedge, and taking it cost nothing.

Because every long operation returns a vector of outcomes keyed by a stable id,
**a progress stream is that vector arriving early**. The items already exist and
already have keys, so a `ProgressEvent` can later be defined as a partial row
without inventing anything.

The sink half is free for a reason specific to this tool: bitplane is ephemeral
and runs one operation per process, so the sink belongs on the Engine rather than
on each call — `fn set_progress(&mut self, sink: Box<dyn ProgressSink>) {}` with
a default no-op body, added later, changing no request or response type.
**ADR-0001's sync decision is what makes this trivial**: a sink is a callback on
the worker thread, with no channel, no runtime and no `async` colouring.

The standing discipline is therefore one line: **never return a scalar count from
a fan-out.**

Named costs of deferring, so they stay noticed:

- **Interleaving is lost in the meantime**, and that cost is paid now, not later:
  four parallel fetches cannot show progress, so the user waits on the slowest
  clone with nothing on screen.
- **The framing lives in the transport**, not in `Engine` — the SSH servant needs
  a stdout protocol carrying N events then one result. It does not exist yet, so
  nothing constrains it.
- **Partial-result-on-interrupt** is ticket 07's question either way.

## Consequences

- **Ticket 07 is unblocked, with the trade-off runnable rather than arguable.**
  The prototype has a `plane.toml` first/last switch and an interrupt control.
  Written *last*, an interrupted `create` leaves real worktrees and `plane_list`
  returns zero planes — verified. Written *first*, the plane is
  visible-but-incomplete and needs a status field. Additionally: re-running the
  identical `create` today returns `PlaneIdInUse`, because the directory is
  claimed by atomic `mkdir`. **Converge therefore requires 07 to make the id
  claim idempotent for a caller-supplied id.** The response shape is ready for
  either answer; the id claim is not.
- **Ticket 08 inherits a blocking question it did not have.** A worktree of a URL
  project has `origin` pointing at bitplane's bare mirror, not at the forge, so a
  naive `bp push` writes into a local directory and the work never leaves the
  machine. An adopted local project pushes correctly, so the same command behaves
  differently depending on how the project was registered. The contract makes it
  unambiguous (`Pushed::remote.is_bitplane_mirror`); the mechanism — a `pushurl`
  on the mirror, a second remote in the worktree, or push-through-the-mirror — is
  `project.toml`'s problem.
- **Ticket 09 inherits a server-side constraint on plane discovery** (see
  `PlaneRef`), and learns that the contract needs only to *return* a worktree
  path, which leaves its "paths or formula?" question open.
- **Ticket 10 inherits `HealthCheck`'s cost tiers** as the shape of "how cheap
  must the detection pass be", and `Waivers` as the form its refusals take.
- **`plane_status` is a read that makes git write.** `git status` refreshes the
  per-worktree index. It stays in `Reader` because the invariant is about
  bitplane's state, and `index.lock` is per-worktree and never shared (ADR-0002,
  measured). Recorded so it is not rediscovered as a bug.
- **Two round trips are sometimes correct.** `project add <url>` colliding on a
  default name, then re-running with `--name <suggestion>`, is two requests — but
  the caller supplied new information in between, so it is a second intent. The
  rule is one request per intent, not one request per command line.
- Crates added to `bitplane-core`: `serde` with `derive`. Nothing else; the
  contract has no chrono dependency (timestamps are RFC 3339 strings on the wire).

## Alternatives considered

**One trait instead of `Reader` + `Engine`.** Rejected. In Rust both reads and
mutations take `&self` — the state is the filesystem — so `&self`/`&mut self`
cannot express the distinction, and "a read never writes" would have been a
comment. The split costs one line and makes it structural.

**A progress sink parameter on every mutating call, added now.** Rejected as the
expensive version of a free hedge. It would grow eighteen signatures today to buy
nothing that putting the sink on the Engine does not buy later, and bitplane runs
one operation per process, so per-call granularity has no user.

**Letting a fan-out failure fail the whole envelope.** Rejected. It would make
`create` all-or-nothing at the type level and force ticket 07 toward rollback by
construction, which is exactly the decision 07 is supposed to make on merits.

**A separate `reconcile` / `detect-stale` action.** Rejected; with no store there
is nothing to reconcile *to*. Staleness is recomputed on every read, so it is a
field on `show` and `list`. What survives of the idea is `doctor`, which does the
one thing a read of the planes directory cannot: sweep the source repos.

**A single blanket `--force`.** Rejected in favour of per-reason waivers, which
fit the "uncommitted work is a veto" invariant far better — you cannot
accidentally override the refusal you did not mean to.
