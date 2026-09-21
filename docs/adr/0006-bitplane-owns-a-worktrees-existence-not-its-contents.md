# ADR-0006: bitplane owns a worktree's existence and location, not its contents

Status: accepted, amends [ADR-0003](./0003-the-engine-contract.md), [ADR-0004](./0004-create-aborts-rename-and-destroy-converge.md) and [ADR-0005](./0005-the-source-repo-is-a-bare-clone-shaped-repo.md)
Date: 2026-09-21
Ticket: none — a standing-principle session, feeding tickets 09 and 10

## Context

ADR-0002 settled that bitplane keeps **no store**, and ADR-0004 settled that
there is **no status field, anywhere**. Both are about *storage*. Neither says
what bitplane may legitimately *know* about a worktree once it exists.

That gap left four places where the tool still formed opinions about the user's
git state:

- `plane_status` shells `git status` per worktree.
- `destroy` and `remove` refuse on `uncommitted` / `untracked` / `unpushed`.
- Ticket 10 declares *"uncommitted or unpushed work vetoes reaping"* a **hard
  invariant, not up for negotiation**, and proposes staleness signals including
  "branch merged into its base" and "branch deleted on the remote".
- ADR-0005 says a plane branch *"belongs to the plane that owns it"*, which
  implies bitplane deletes it.

And one place where it stored an opinion: ADR-0004 specified `plane.toml`'s
contents at claim time as *"full intent — every member with its resolved branch
name"*. A user who runs `git switch` in a worktree makes that file wrong, with
nothing able to notice.

The question is where the line falls between "does not store state", "does not
write state", and "does not look at state" — three readings of the same
sentence that produce very different tools.

## Decision

### The rule: existence and location are bitplane's, contents and HEAD are the user's

bitplane creates a worktree, moves it, repairs its administrative entry, and
removes it. It never commits, pushes, stashes, checks out or resets. Between
`worktree add` and `worktree remove` the directory is the user's, and everything
they do inside it — switching branches, committing, pushing, rebasing,
abandoning — is ordinary use rather than drift.

**Reading is allowed; writing is not.** This is the middle reading, chosen
deliberately over the strict one. A tool forbidden to *look* could not tell you
the state of six worktrees in one command, which is most of what a multi-repo
plane manager is for, and looking stores nothing and can never go stale: it is
git's answer, rendered.

The rule is stated as ownership rather than as a list of permitted commands
because it decides cases the list would not. It is what makes `worktree repair`
obviously in scope and `git stash` obviously out, without appealing to taste.

### `plane_status` survives, unchanged

It is a pure read: it shells out, renders, and stores nothing. ADR-0003's note
that it is *"a read that makes git write"* (the index refresh) still stands and
is still safe — `index.lock` is per-worktree.

### `plane.toml` holds no branches; it is the membership list

A member entry is **a project name mapped to a path relative to the plane
directory root**. That is the whole per-member schema.

`plane.toml` therefore stops being "the desired state" and becomes the
**membership list**: it answers *which projects are in this plane, and where*,
and nothing about them. A member's branch is read from the worktree's `HEAD`
when asked, so what bitplane reports is always true by construction rather than
true until someone runs `git switch`.

Three consequences fall out of the shape itself:

- **Keying by project makes "a plane holds at most one worktree per project" a
  property of the file format**, not an invariant anything has to check.
- **The branch is cheap to read.** A worktree's `.git` file points at
  `<source-repo>/worktrees/<name>/`, whose `HEAD` is a one-line file — no git
  process. So the live branch belongs in `HealthCheck::Cheap`, and `bp list`
  shows real branches without spawning anything.
- **Paths, not a formula.** Ticket 09 left "does it record the subdirectory as a
  path or as a formula?" deliberately open, with the counter-argument that
  `git worktree list --porcelain` already knows every path. That oracle requires
  knowing which source repo to ask, and with the branch gone this dictionary is
  the only thing mapping a member to its directory. It is now load-bearing, and
  the answer is **paths**, relative to the plane root so that an external `mv`
  and a `bp rename` both leave them correct.

### A plane has no branch

*"One shared branch name across the plane by default"* is retired as a property
and survives only as a **create-time convenience**: `bp create -b feat-x` applies
`feat-x` to every member at that moment, and that is the end of its life. There
is no plane-level branch field and nothing derives one.

`bp add @project` to an existing plane therefore **requires an explicit branch**.
Deriving one from the existing members was considered and rejected: it would
make `add`'s behaviour depend on what the user did in an unrelated worktree last
week, which is worse than asking.

### `destroy` and `remove` still refuse on uncommitted work

"Do not manage the user's state" and "do not check before deleting the user's
uncommitted work" are different claims, and only the first is the rule here. A
refusal manages nothing — it declines to act and hands control back, which is
the same posture as leaving everything else alone.

`plane_remove` — one member out of a live plane — carries **identical** rules to
`plane_destroy`. Removing a member destroys exactly as much work as destroying a
one-member plane, so a lighter rule there would be a hole in this one.

### The unpushed check is `--contains` against `refs/remotes/origin/*`, never `git branch -d`

Measured against a bitplane-shaped source repo (bare, `remote.origin.fetch =
+refs/heads/*:refs/remotes/origin/*`, worktree added with `-b feat origin/main`):

| branch state | work safe on the forge? | `git branch -d` |
| --- | --- | --- |
| committed, never pushed | no | refuses — correct |
| pushed, upstream still `origin/main` | **yes** | **refuses** — wrong |
| pushed with `-u`, upstream `origin/feat` | yes | deletes, with a warning |

Row two is the defect. A plane branch is created off `origin/main` and keeps
`branch.feat.merge = refs/heads/main`, so `git branch -d` compares against the
wrong ref and reports *"the branch 'feat' is not fully merged"* while the commits
sit on the forge at `origin/feat`. Only an explicit `push -u` moves the upstream
and makes `-d` correct, and bitplane does not control how the user pushes.

`git branch -r --contains feat` printed `origin/feat` correctly in **both**
pushed rows. So the predicate is *"is the branch tip contained in any
`refs/remotes/origin/*`"* — the tip being reachable implies every commit below it
is too.

**Stale remote-tracking refs are safe in the only direction that matters.** A
local push updates `origin/feat`, so the common case is current; a push from
another machine leaves ours behind, which produces a false *refusal* the user
waives. There is no staleness that makes `destroy` permissive, so `destroy`
performs no implicit fetch.

### `destroy` deletes the live branch, gated by the check it already runs

On `destroy` and `remove`, bitplane deletes **the branch the worktree is
actually on at that moment**. Nothing records the branch it was created on, so
no other answer is available.

This needs **no new waiver and no second gate**. If the operation already
refuses when a member's branch has commits not on the forge, then any plane that
passes that gate has branches that are by definition safe to delete.
`bp destroy` refuses, names the project and branch, and `--waive unpushed`
proceeds and deletes.

**Accepted hole, stated rather than hidden:** a user who creates on `feat-a`,
switches to `feat-b` and destroys leaves `feat-a` in `refs/heads/*` forever, with
no record in bitplane that it ever existed. The alternative — storing the
created branch purely so `destroy` can clean it up — resurrects the stored
branch this ADR removes. `doctor` gains a **fourth sweep** reporting source-repo
branches with no worktree and no plane, which turns a silent leak into a listed
one without bitplane deleting anything on its own judgement.

### Branches in an adopted project's source repo are never deleted

ADR-0005 already establishes that an adopted project's `refs/heads/*` are the
user's branches and the plane-branch invariant does not hold there. With nothing
stored, bitplane **cannot tell** whether a branch in an adopted repo is one it
created or one the user has had for two years.

So for an adopted project: remove the worktree, touch no ref. The refusal check
still runs — it is about not destroying work — but the deletion step is skipped
and the response says so per project.

This is the sharpest asymmetry in the design and it is deliberate: bitplane owns
the refs in a repo it built and owns nothing in a repo it merely pointed at.

### Reaping keeps its veto and loses its opinions

The veto stands — uncommitted or unpushed work overrides every staleness signal
— and given reads are allowed it is cheap and honest.

What is removed is the rest of ticket 10's proposed signal list. **"Branch merged
into its base" and "branch deleted on the remote" are dropped as reap signals.**
They are bitplane forming a judgement about the user's git workflow, which is
exactly what this rule forbids, and they are the two signals most likely to be
wrong for a user whose team squash-merges or whose forge deletes branches on
merge.

Staleness signals are narrowed to things bitplane owns: worktree directory
missing, git's own `prunable`, source repo gone, `create` never completed, and
plane untouched for N days.

`prunable` stays a **worktree** property — git's word for "the directory is
gone". It is not a branch predicate, and using it as one would collide with the
orphan oracle `doctor` already depends on.

### `project_in_use` becomes a hard refusal, and `Waivers` drops to five

Removing a project whose worktrees are live in some plane is **refused, not
waivable**. `Waivers` becomes `{uncommitted, untracked, unpushed,
locked_worktree, source_repo_missing}`.

The principle this draws out is the only clean line between a waiver and a
refusal in the design: **a waiver says "I accept losing *this* work, which I am
looking at."** The other five are all of that shape. `project_in_use` is not — it
damages planes the user did not mention and is not looking at, so there is no
moment at which consenting to it is informed.

An **incomplete (latched) plane blocks too**, for uniformity; since `destroy` on
a latched plane never refuses, clearing it is one command. The error lists the
blocking planes **by id**, because the entire value of an unwaivable refusal is
that the way out is obvious.

Side effect worth noting: this removes the main way `source_repo_missing` could
arise. bitplane can no longer create the condition, and the waiver survives only
for a user who deleted a repo by hand.

### Plane verbs are bare; project verbs are always prefixed

`bp create`, `bp add`, `bp rm`, `bp destroy` operate on planes. `bp project add`,
`bp project rm`, `bp project rename` operate on projects, and project deletion is
**never** abbreviated to `bp rm`.

Recorded as a decision rather than left to a CLI ticket because of how it was
found: during this session `bp rm @foo` was read as *"delete project foo"* when
it meant *"take this project's worktree out of this plane"*. Both take `@name`,
both are destructive, and they rhyme. A reader misreading it is the only evidence
that kind of ambiguity ever produces.

## Consequences

- **ADR-0004's `plane.toml` line is superseded.** *"Full intent — every member
  with its resolved branch name"* becomes *"membership — every member with its
  path"*. The `create` step order is unaffected: membership is still written
  before any git work, still exactly twice on the happy path.
- **ADR-0003 is amended in four places**: `plane_status` explicitly survives;
  `Waivers` drops to five; `unpushed`'s implementation is specified as
  `--contains` against `refs/remotes/origin/*`; `doctor` gains a fourth sweep.
- **ADR-0005's plane-branch invariant weakens** from *"exactly the branches
  bitplane created for worktrees"* to *"exactly the branches plane members were
  created on, past and present"*. Still clean, still never colliding with the
  forge, but no longer self-pruning.
- **Ticket 09 is partly pre-settled**: `plane.toml` records **paths**, relative
  to the plane root, keyed by project name, with no branch. Its remaining
  questions — the id/rename disagreement and the discovery mechanism — are
  untouched.
- **Ticket 10's "hard invariant" is narrowed, not overturned.** The veto stands;
  two of its proposed signals do not.
- **`bp add` to an existing plane grows a mandatory branch argument.**
- **What is given up**: bitplane can no longer report what a member was
  *supposed* to be — there is no record of intent to compare reality against.
  Traded for the property that nothing bitplane says about a worktree can ever be
  stale.

## Alternatives considered

**The strict reading: bitplane never looks at worktree state at all.** Rejected.
It deletes `plane_status`, and with it the one-command answer for a six-worktree
plane that is most of the tool's value. Looking stores nothing and cannot go
stale, so it violates neither ADR-0002 nor the rule above; the objection to
tracking state is about *storage and judgement*, and reading is neither.

**Keeping the branch in `plane.toml` as a record of intent, never rewritten.**
Rejected, and it was the leading alternative. It is defensible — the field would
be honestly documented as "what was asked for", not "what is" — but it produces
a file in which one field is present-tense and another is historical, and every
consumer has to know which is which. Deleting it makes the file uniformly
descriptive.

**Rewriting the branch in `plane.toml` to match reality on the next command.**
Rejected on arrival: it makes a read a write, which ADR-0002 forbids outright.

**A plane-level `branch` field as the default for later `add`s.** Rejected — it
is the stored branch returning through a side door, stale in exactly the same
way, just less often.

**Deriving `bp add`'s default branch from the existing members' HEADs.**
Rejected. When members agree it works and when they disagree there is no answer,
so the behaviour depends on unrelated work in another worktree. Asking is worse
ergonomics and better semantics.

**Deleting the branch on `destroy` only if it merges cleanly into
`origin/HEAD`.** Rejected — it is the "merged into base" judgement removed from
reaping, reintroduced at a different moment. The `--contains` check answers the
question that actually matters (*is this work recoverable?*) without an opinion
about workflow.

**Leaving every branch behind on `destroy`, deleting none.** Rejected once the
refusal gate was in place. With the `unpushed` check already run, the branches
that survive it are provably safe to delete, so keeping them would be hoarding
refs for no benefit — and the one real accumulation case (a switched-away branch)
is left behind anyway.

**A seventh `Waivers` member for branch deletion.** Rejected as a second gate on
the same predicate. One check, one refusal, one waiver.

**Allowing `project remove` with a `project_in_use` waiver, as today.** Rejected
— see the waiver/refusal line above. It is the one refusal in the set whose
consequences land outside the user's field of view.
