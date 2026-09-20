# ADR-0004: `create` aborts and removes; `rename` and `destroy` converge

Status: accepted
Date: 2026-09-20
Ticket: `.alfonz/issues/bitplane-architecture/issues/07-plane-lifecycle-partial-failure.md`

## Context

Creating a plane across four repos is four `git worktree add` calls with no
daemon to finish or undo them. Ctrl-C after the second leaves a half-built
plane, and nothing in the design said what happens next.

Ticket 07 was blocked on 05 and 06 because the answer depends on what there is
to converge. Both have now landed: there is no store, the filesystem is the
state (ADR-0002), and a fan-out returns `Vec<PerProject<T>>` with `Ok` /
`AlreadyDone` / `Skipped` / `Failed` per row (ADR-0003). ADR-0003 explicitly
handed 07 three things: the write-ordering trade-off, the id-claim question, and
partial-result-on-interrupt.

The ticket licensed deciding **per operation**, and that turned out to be the
honest answer — `create` and `rename` get opposite treatments for a reason that
is a property of the operations, not a failure of consistency.

## Decision

### `create` is abort-and-remove, in four steps

```
1. mkdir <plane-dir>                          ← the claim, atomic, EEXIST is fatal
2. write .bitplane/incomplete                 ← the latch
3. write plane.toml                           ← full intent: every member, resolved branch
4. git worktree add, per project              ← the abort window
5. unlink .bitplane/incomplete                ← the point of no return
6. run hooks                                  ← failure is reported, never unwound
```

A failure in step 4 stops scheduling new projects, unwinds every worktree the
run created, and removes the plane directory. `create` is **not** idempotent and
the caller-supplied id claim stays strict: `EEXIST` is `PlaneIdInUse`, returned
immediately, exactly as ADR-0002 said.

**The step ordering is the load-bearing part.** Every expensive,
side-effect-bearing thing happens *outside* the abort window: hooks are step 6,
after the latch clears, and a cold `fetch` writes the bare mirror rather than the
plane, so a retry finds the mirror warm. The window therefore contains **only
cheap, discardable work** — which is what makes throw-away-and-retry a cheap
repair rather than a lossy one. The ticket's stated fear that "a rollback
running after work landed in a worktree destroys that work" does not apply,
because by construction there is no work in there yet.

Because the window holds nothing a user has touched, the abort path passes
`git worktree remove --force` unconditionally. **That force is scoped to the
abort path and must not be "consistency-fixed" onto `destroy`,** where the same
flag would step straight over the uncommitted-work veto.

### The latch is a file, and the ordering makes an uncoverable hole coverable

`<plane-dir>/.bitplane/incomplete` holds the RFC 3339 timestamp of the claim and
nothing else. It is diagnostic only — nothing parses it for control flow, so a
corrupt or empty file still means "incomplete". It exists so `doctor` and
`bp list` can say *"create never completed, started 3 days ago"* rather than
just *"broken"*, which is the difference between a user confidently discarding a
remnant and wondering whether something is still running.

It is a **one-way latch**, not a status field: set once at birth, cleared once,
never set again. That is why it does not fall to ADR-0002's objection to stored
state — a crash before clearing leaves it set, which is the truthful reading. It
can only fail in the safe direction.

Its single job is to separate **"never completed"** from **"completed once, then
something was deleted."** `plane.toml` records full intent from step 3, so a
member with no worktree on disk already marks a plane incomplete; what that
cannot tell you is whether the plane ever worked.

Writing it **before** `plane.toml` leaves a window where the directory holds
`.bitplane/` and no plane file, which ADR-0002's scan rule silently skips. That
is not a new hole: `mkdir` must precede any write into the directory, so a bare
claimed directory was always possible and was invisible to *both* oracles. The
ordering grows that window from one syscall to three — all local, no fork/exec,
no I/O wait between them — and in exchange the directory now carries
**bitplane's own fingerprint**, which a stray user directory never has.

So **`doctor` gains a second sweep**: report directories in the planes directory
containing `.bitplane/` but no `plane.toml`. A case that previously had no cure
at all becomes findable. And `create` hitting `EEXIST` stats the one path it
already knows and classifies it — healthy plane / latched remnant /
claimed-but-no-plane-file — so its `remedy` can say *"`auth-work` exists but was
never completed; `bp destroy auth-work` to clear it"* rather than the useless
"already in use". That is most of what idempotency would have bought, without
the contract change.

### There is no status field, anywhere

The latch is the only piece of lifecycle state on disk, and only `create` ever
writes it. Everything else is recomputed: a member listed in `plane.toml` whose
worktree path does not exist is a `Finding::MemberWorktreeMissing`, one `stat`
per member, so it sits in `HealthCheck::Cheap`. This is ADR-0002's "a read never
writes" and ADR-0003's "staleness is a recomputed field" applied without
exception.

A multi-valued lifecycle enum (`creating` / `ready` / `renaming` / `destroying`)
was rejected on a specific ground: it needs a writer on every transition, and a
crash *between* the transition and the write makes the field actively lie. The
filesystem cannot lie.

Accepted cost: an interrupted `create` is indistinguishable from a user who
`rm -rf`'d a worktree by hand. That is correct rather than a flaw — `plane.toml`
is the desired state and `bp rm @x` is how a member is removed.

### A failed `create` returns `Err`, which generalises ADR-0003's envelope rule

`Err(EngineError::CreateAborted { projects, rollback })` — the per-project rows
ride inside the error, so you still see which project failed and whether its
cleanup worked. `create` loses its `partial` field entirely: it either made a
plane or it did not.

ADR-0003 reserved the envelope `Err` for failures that stopped an operation
*before it started*. That rule is **amended** to: the envelope `Err` is for
operations that produced **no durable state** — covering both "never started"
and "started, then fully unwound". `partial: true` keeps meaning what it says
everywhere else.

If the unwind itself fails, the latch stays set, the failing rows land in
`CreateAborted.rollback`, and the message names the remnant and the destroy
command. Exit code is `1` either way; a script's decision tree is the same.

### `rename` converges, and the primitive is `repair`, not `move`

The ticket and ADR-0003 both described rename as `mv` the plane directory then
one `git worktree move` per project. **That is wrong.** You cannot
`git worktree move` a worktree whose parent directory you have already moved —
git looks for it at the old path.

The verb that fits is **`git worktree repair <path>...`**, which exists
precisely for "a worktree was moved without `git worktree move`", takes every new
path in one invocation from the bare mirror, and is **idempotent by
construction**. It is available since git 2.30; ADR-0001 floors at 2.36. It
writes `worktrees/<name>/gitdir` in the mirror, so it takes the per-source-repo
lock.

Rename is therefore: `mv` (a single atomic `rename(2)` — the ticket's fear of a
plane "findable under neither name" does not arise), then `git worktree repair`
per source repo, then rewrite `plane.toml`'s `id` **last**, so a stale `id` is a
reliable "this rename is unfinished" signal rather than a lie.

An interrupted rename is indistinguishable from a hand-`mv`, and that is the
point: ADR-0002 already named that state `PlaneMovedExternally`, and **the cure
is identical in both cases and idempotent**. No latch is needed.

Rename gets the opposite treatment from `create` not out of inconsistency but
because throwing away is **genuinely unavailable**: the worktrees contain real
work, so there is nothing to discard and unwinding is the same work in the other
direction with no advantage.

### `plane_repair` is a nineteenth action, narrow scope

`git worktree repair` per project, and nothing else. It takes only a `PlaneRef`,
returns the standard `Vec<PerProject<T>>`, and is idempotent because its
primitive is. It refuses on a latched plane — repairing admin entries for a
plane that was never completed is meaningless work on a thing headed for
deletion — with `bp destroy` as the remedy.

It exists because ADR-0003 made `doctor` report-never-repair and left the write
side unnamed, and because the hand-`mv` case is a real user action that ADR-0002
detects and deliberately leaves uncured. Spelling it `rename X X` would have
avoided a nineteenth action at the cost of reading like a puzzle.

Its **wide** scope — "converge the plane to `plane.toml`, creating missing
worktrees" — was designed and then dropped when `create` went strict. It has no
customer once a half-built plane is thrown away rather than finished.

### `destroy` and `remove` converge, with no latch and no incremental writes

Rollback is not available — you cannot un-remove a worktree — so the choice was
converge or fail-and-report, and converge costs nothing. `destroy` removes
worktrees, then deletes `plane.toml`, then `rmdir`s. Re-running re-checks
refusals against what survived, reports the gone ones `AlreadyDone`, and
finishes.

**`plane.toml` is the first thing written and the last thing removed.** The
visibility window strictly contains the existence window, so anything that
exists is visible and every partial state is addressable by a `PlaneRef`.
Rewriting the file as each worktree goes buys nothing but fsyncs on a file about
to be deleted.

No `destroying` latch, despite a real argument for one — a half-destroyed plane
genuinely *is* discardable, since refusals were already checked. It was rejected
because waivers are granted per invocation, and a latch that carries waiver
authority into a later `destroy` is a trapdoor.

`destroy` on a **latched** plane never refuses: no hook has run, so no untracked
files; the branches are seconds old, so nothing is uncommitted or unpushed. Every
reason in `Waivers` is structurally impossible.

Accepted cost: bitplane cannot tell a half-destroyed plane from a
half-built-then-abandoned one. Both are `Broken`, and the user's move is the
same in both.

### `add` unwinds only its own rows, and must never borrow the latch

`plane_add` cannot abort-and-remove — the plane holds other members full of
work. On failure it force-removes the worktrees *it* created, rewrites
`plane.toml` dropping *its own* entries, and returns
`Err(EngineError::AddAborted { … })`. The plane returns to its prior state.

**`add` must not set `.bitplane/incomplete`.** The latch means "this plane was
never completed, nothing in it is yours", which is what licenses a refusal-free
`destroy`. An interrupted `bp add` that had set the latch would leave a plane
containing real work flagged as free to discard without a single refusal check.
Only `create` is ever entitled to say a plane was never completed.

If the unwind itself dies, `plane.toml` lists the member with no worktree →
`MemberWorktreeMissing`, `Broken`. The repair is `bp rm @c`, so **`plane_remove`
must tolerate a member whose worktree is already gone** — report `AlreadyDone`
and drop the entry. The naive implementation would fail on `git worktree remove`
of a path that is not there.

Re-running `bp add @c:feat` against a listed-but-missing `@c` is refused with
`DuplicateProjectInPlane`. `bp rm @c` then `bp add @c:feat` is the fix: two
commands, consistent with strict `create`, and no adoption magic anywhere in the
tool.

### Fan-out stops scheduling on first failure

For `create` and `add`, continuing to build worktrees that are about to be
unwound is pure waste. `SkipReason::AbortedAfterEarlierFailure` therefore applies
to this family, as the ticket-06 prototype already assumed.

The fan-out is parallel (ADR-0002: per-source-repo locks can never contend within
one plane), so "stop" means stop *scheduling*; in-flight `worktree add` calls
still land and are then unwound.

`commit` and `push` converge by nature — nothing to unwind — and return `Ok` with
`partial: true`. They are also where `AbortedAfterEarlierFailure` genuinely earns
its keep: a `commit`-then-`push` chain whose commit failed must not push, unlike
the independent rows in `create`.

### Hooks run after the point of no return, and there is no re-run verb

A hook failure never unwinds anything: by step 6 the plane is complete and
usable. It is reported with its log path and exits `1` — `bp create` returning
`0` would hide a real failure.

No `plane_hooks` action. The plane works; inventing a verb expands 07's surface
into ticket 08's territory, and nothing here forecloses 08 adding one.

`HookOutcome.finished_at` stays non-optional, so a hook killed mid-run leaves
**no record at all**. That is deliberate: "started but never finished" is not
representable, and it does not need to be.

### A plane whose source repo has disappeared

`Finding::SourceRepoMissing { project }` at `HealthLevel::Broken`, detected in
the `Cheap` tier — "does `<projects-dir>/<name>/` exist" is one `stat`, no git
spawn, so `bp list` can carry it. Not drift: nothing heals it by itself.

The consequence is sharper than the representation. `destroy` then cannot use
`git worktree remove` and **cannot check for uncommitted work at all**, so it
needs a sixth waiver, `source_repo_missing`, with
`Refusal::SourceRepoMissing { project, expected: PathBuf }`.

It is different in kind from the existing five. `uncommitted` and friends mean
*"I know there is work here and I accept losing it"*; this one means *"I cannot
check at all."* Folding it into `uncommitted` would let a waiver granted for a
known, inspected diff silently authorise deleting a directory nobody looked at.
Naming `expected` also gives the better remedy: remount the volume or restore the
mirror, then destroy properly. Same treatment for a local-path project whose
checkout is gone — bitplane never assumed it owned that repo.

### Interrupt: what Ctrl-C actually does

bitplane cannot protect its children. SIGINT reaches the whole process group, so
an in-flight `git worktree add` gets it too and may leave a half-written
worktree. All bitplane controls is what it records and prints.

First SIGINT stops scheduling new projects, waits for in-flight children, runs
the abort path, prints what it has on stdout, and exits **`130`** (128+SIGINT).
Second SIGINT exits immediately. The partial vector is not a courtesy — it *is*
the repair instruction, which is the whole reason ADR-0003 forbade scalar counts.

## Consequences

- **ADR-0003's envelope rule is amended** (see above) and its rename mechanics
  are **corrected**: `git worktree repair`, not `git worktree move`.
- **The action set is nineteen**, not eighteen: `plane_repair` joins `Engine`.
  `doctor` gains a second sweep and stays a `Reader`.
- **`Waivers` grows a sixth member**, `source_repo_missing`.
- **Ticket 09 inherits `plane.toml`'s contents at claim time**: full intent —
  every member with its *resolved* branch name — written once, before any git
  work. No status field, and no `created_branch` in the file (that is a response
  field, known only after the fact). The happy path writes `plane.toml` exactly
  twice: once at step 3, and never again unless membership changes.
- **Ticket 10 is unblocked** and inherits three findings to place in its tiers —
  `MemberWorktreeMissing`, `CreateNeverCompleted`, `SourceRepoMissing`, all
  `Cheap` — plus the rule that a latched plane is safe to reap without refusal
  checks, which is the one case where reaping needs no veto.
- **Ticket 08 inherits the pattern, not the mechanism.** Project creation follows
  the same abort-and-remove shape. But a failed clone of a 2 GB mirror is
  expensive to discard, unlike anything in a plane's abort window, so 08 may well
  want the mirror kept and only the *registration* unwound. That is a real
  deviation and explicitly 08's call, not an inherited assumption.
- **`.bitplane/` now holds three things**: the lock sentinel, `logs/`, and
  `incomplete`. Its reserved-path-segment rule (ADR-0002) covers all of them.
- **The map's "Plane rename" open item is closed.**

## Alternatives considered

**Converge `create`, with an idempotent caller-supplied id claim.** The original
recommendation, and rejected on merits. It would have made "run it again" the
repair — attractive for a machine-contract-first tool with retrying agents as
first-class consumers, and `Outcome::AlreadyDone` was already in the enum for it.
Three things sank it. It changes `create`'s contract rather than its
implementation; it needs a compatibility rule to stop `create --id auth-work`
silently adopting an unrelated healthy plane, and every candidate rule (exact
match / superset / declarative) trades safety against usefulness with no clean
answer; and it buys little, because the abort window holds nothing expensive, so
discarding and retrying costs a few `worktree add` calls against a warm mirror.
Strictness plus a self-explaining `PlaneIdInUse` remedy gets the same user
outcome with a smaller contract.

**Writing `plane.toml` last.** Rejected, and it is the worst option in the
design. An interrupted `create` would leave real worktrees and real git admin
entries that the plane scan **structurally cannot see** (ADR-0002's scan rule),
and — interrupted before the first `worktree add` — nothing pointing at the
directory either, so `doctor`'s source-repo sweep could not find it. Invisible to
both oracles, with the id **permanently burned**: `create --id auth-work` would
fail `EEXIST` forever with nothing able to explain why, clearable only by `rm -rf`
by hand.

**The latch as a key in `plane.toml` (`incomplete = true`).** Genuinely close.
One file and one truth, written in a single atomic rename, so the latch could
never be absent while the plane file was present. Rejected for the separate file
because clearing it becomes an `unlink` rather than a second `toml_edit`
read-modify-write, and because the window it opens turns out to be a pre-existing
hole made *smaller* rather than a new one (see above).

**A `destroying` latch, and letting `add` set the `incomplete` latch.** Both
rejected, and for the same underlying reason: the latch's meaning is
load-bearing. It licenses a refusal-free `destroy`, so any operation that sets it
on a plane containing real work turns a safety property into a hazard.

**`git worktree lock` as the incomplete marker.** Not pursued. ADR-0002 retained
it as an advisory marker and ticket 10 has a genuine claim on it for
protect-from-reaping, which is the opposite meaning. Overloading one git-visible
flag with "do not reap this" and "this is garbage" would be actively dangerous.

**A blanket `--force` on the abort path, reused by `destroy`.** Rejected by
scoping. `--force` is correct in the abort window *because* the window contains
no user work by construction, and that justification does not transfer one line
further.
