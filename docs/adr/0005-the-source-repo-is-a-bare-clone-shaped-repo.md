# ADR-0005: The source repo is a bare repo with a clone-shaped refspec

Status: accepted, amends [ADR-0003](./0003-the-engine-contract.md), amended by [ADR-0006](./0006-bitplane-owns-a-worktrees-existence-not-its-contents.md) and by implementation ticket 04
Date: 2026-09-21
Ticket: `.alfonz/issues/bitplane-architecture/issues/08-project-schema-and-hooks.md`

> **Amended by implementation ticket 04
> (`.alfonz/issues/bitplane-implementation/issues/04-cli-reference.md`) in three
> places.** The decision below stands. The branch-resolution paragraph stands as
> a *derivation* and no longer as a *fallback*; the rest of this banner is what
> changed.
>
> 1. **The ladder resolves a *base*, not a branch to land on.** The sentence
>    below beginning *"Resolution becomes: the member's branch > the plane-wide
>    `branch` > …"* is retained as the derivation of
>    `ProjectView.default_branch` and **retired as a fallback for a missing
>    `-b`**. `bp create` and `bp add` both require an explicit branch; the
>    resolved value is what a new plane branch is *cut from*.
> 2. **The first two rungs go with it.** The member's branch and the plane-wide
>    `branch` answered *which branch*, which is now always given, and ADR-0006
>    deleted the plane-level branch outright. What survives is
>    `refs/remotes/origin/HEAD` > the source repo's own `HEAD` > unspecified.
> 3. **`BranchUnspecified` splits in two**: `branch_unspecified` is a member
>    given no branch at all, and `base_branch_unresolved` is a member whose
>    ladder bottomed out **when a new branch had to be cut**. An unresolved base
>    is not an error on its own — it only matters when there is nothing to cut
>    from.
>
> The reason is a case this ADR predates. Falling back to the default branch
> works for an **owned** project, whose source repo is bare and occupies
> nothing. It fails for everything else: an **adopted** project's source repo is
> the user's checkout, and so is an **ad-hoc member**'s (a member kind ADR-0008
> added afterwards), so the default branch there is precisely the branch that
> checkout is sitting on — and the fallback resolves straight into the
> `occupied branch` refusal this same ADR specifies, for exactly the members the
> convenience was meant to help. Only one of the three things a plane can hold
> would have been served by it.
>
> The wording is spelled out in `website/docs/reference/refusals-and-waivers.md`
> and `website/docs/reference/project/show.md`.

## Context

Since ticket 03, every document has said a URL project is "cloned as a **bare
mirror**" into `<project-dir>/repo.git`. No document ever said which git command
produces it. The noun was doing the specifying, and `git clone --mirror` is the
command that makes a thing called a mirror.

ADR-0003 handed ticket 08 a *blocking* question built on that noun: a worktree of
a URL project supposedly has `origin` pointing at bitplane's own mirror, so
`bp push` would write into a local directory and the user's work would never
leave the machine. `Pushed::remote.is_bitplane_mirror` was specified to report it.

Both turned out to be wrong, and the truth was worse than the fear. Measured
against a scratch forge:

- **A worktree has no config of its own.** It reads the source repo's, so
  `git remote -v` inside a worktree shows the **forge URL** under every clone
  flag. Push does reach the forge. ADR-0003's blocking question describes a state
  that cannot occur.
- **`--mirror` sets `remote.origin.mirror = true`, which the worktree inherits.**
  A plain `git push` from a plane worktree printed
  `+ b0724e7...44846d8 feat -> feat (forced update)` and
  `- [deleted] other` — it force-pushed over the forge's branch and **deleted a
  branch that existed only on the forge**. `bp push` would have been a
  repo-wiping command.
- **`--mirror` sets `fetch = +refs/*:refs/*`**, so the forge's branches land in
  the same `refs/heads/*` that plane branches live in. Git refuses to update a
  checked-out branch, so `git fetch` exits **128** and fetches *nothing*:
  `fatal: refusing to fetch into branch 'refs/heads/feat' checked out at …`.
  Not from the start — a plane on a new branch fetches fine until the moment
  someone pushes that branch, and from then on `project_fetch` is dead for that
  project, and one offending ref aborts the whole fetch for every other plane.

So the decision was never "how do we fix push"; it was "what is the source repo,
exactly".

## Decision

### `git clone --mirror` is forbidden, and the blocking question is void

Not "avoided" — forbidden, with the reason in `CONTEXT.md`, because the word
"mirror" is what selected it and the word is now retired in favour of **source
repo**.

`Pushed::remote.is_bitplane_mirror` is **deleted rather than implemented**, and
the map's "Where `bp push` actually sends" open item closes as void. There is no
`pushurl`, no second remote in the worktree, and no push-through-the-mirror.

### A bitplane-owned source repo is bare, with a clone-shaped refspec

```sh
git init --bare <project-dir>/repo.git
git -C … remote add origin <url>
git -C … config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
git -C … fetch origin
git -C … remote set-head origin -a
```

`git clone --bare` is not used because it copies the forge's branches into
`refs/heads/*` before the refspec can be set, polluting the namespace this ADR
exists to keep clean. Measured: after `init --bare` + `fetch`, `refs/heads/*`
holds exactly **0** refs, and exactly **1** after the first `worktree add`.

**The refspec is the load-bearing line, and it is not a default.** It is what
makes all three of the following true at once, and every one of them is false
under `+refs/*:refs/*`:

- `fetch` always succeeds, because it writes only to `refs/remotes/origin/*`,
  which nothing ever checks out. Verified with a plane worktree live.
- `refs/heads/*` is **exactly the set of branches bitplane created** — the
  glossary's **plane branch**. A local branch exists only because a worktree
  needed it, which is what makes that branch the responsibility of the plane
  that owns it.

  > **Weakened by [ADR-0006](./0006-bitplane-owns-a-worktrees-existence-not-its-contents.md)**
  > to *"exactly the branches plane members were created on, **past and
  > present**"*. `plane.toml` no longer records a branch, so `destroy` deletes
  > the branch a worktree is **actually on** at that moment; a user who switches
  > from `feat-a` to `feat-b` leaves `feat-a` behind with no record of it. Still
  > never colliding with the forge, but no longer self-pruning. `doctor` gains a
  > fourth sweep that reports such branches and never deletes them. In an
  > **adopted** project bitplane deletes no branch at all.
- `@{u}` means the forge's view rather than a local copy of it, so "unpushed" is
  honest.

### Bare, not non-bare with a detached HEAD

The near-miss alternative was a normal clone: it makes `add` and `adopt` produce
structurally identical source repos, and `git clone` writes the refspec and
`origin/HEAD` for you, so the load-bearing line above cannot be forgotten. It
was taken seriously and rejected on a measured property.

A non-bare source repo **must have a HEAD**, and a HEAD occupies a branch. Four
ways to avoid that were tried:

| attempt | result |
| --- | --- |
| delete `.git/HEAD` | `fatal: not a git repository` — HEAD's existence is what makes it a repo |
| HEAD → unborn `refs/heads/main` | still `fatal: 'main' is already used by worktree at …`. Occupancy is HEAD's *target*, not whether the ref exists |
| HEAD → `refs/bitplane/void`, outside `refs/heads/` | fetch and status fine, but `git worktree add` dies: `fatal: HEAD not found below refs/heads!` |
| HEAD detached at a sha | works — the only viable non-bare form |

Detaching is reachable but must be spelled `git update-ref --no-deref HEAD <sha>`;
the obvious `git checkout --detach` **materialises the entire working tree**
(measured: the directory grew to full clone size). It then pins that commit
forever, reports `D` on every file to anyone who runs `git status` there, and is
one stray `git checkout main` away from occupying a branch again — at which point
planes on the default branch start failing with an error naming a directory the
user has never heard of.

A bare repo's HEAD is a symref to `refs/heads/main` — textually identical to the
second row above, which failed — yet `git worktree add main` **succeeds**,
because there is no main worktree for it to occupy. It starts unborn and pins
nothing. The property is structural rather than maintained, and that is the whole
argument.

The symmetry the alternative was reaching for is **relocated into the rule, not
the artifact**: an **occupied branch** (checked out in any worktree of a source
repo, including its own) is unusable by a plane. That case is unavoidable for an
adopted project, whose checkout is the user's and is sitting on `main`, so
bitplane needs the refusal regardless. Bare merely guarantees bitplane never
causes one itself.

### `default_branch` is not a field in `project.toml`

ADR-0003 assumed one, as the third rung of branch resolution. It is dropped,
because **git already stores it**, per project, user-settable, and durable:

```
git remote set-head origin develop   → refs/remotes/origin/develop
  after a fetch:                       refs/remotes/origin/develop   (survives)
  after a fetch with new commits:      refs/remotes/origin/develop   (survives)
```

A stored copy would be a second source of truth that goes silently stale when a
forge renames its default branch — ADR-0002's objection to a store, at field
scale. Resolution becomes: the member's branch > the plane-wide `branch` >
`refs/remotes/origin/HEAD` > the source repo's own `HEAD` (the remote-less
adopted case) > `BranchUnspecified`. ADR-0003's third rung survives; it reads git
instead of TOML. (**Amended above**: the first two rungs are retired with the
mandatory branch, and what is left resolves a *base* to cut from rather than a
branch to land on.) `ProjectView.default_branch` stays in the contract as the
*resolved* value.

### `project_fetch` fetches the configured refspec, prunes, and never prunes tags

Not "only what a plane needs": that saves nothing on one round trip and would
make `bp create` on a colleague's new branch need a second fetch.

`--prune` is safe **because of the refspec** — `refs/remotes/origin/*` is a pure
cache of the forge and holds nothing a user owns, so pruning cannot lose work. It
would have been unsafe under `+refs/*:refs/*`.

Tags are left at git's default and **never pruned**: `refs/tags/*` is a namespace
shared with every worktree that bitplane does not own.

### `push.default = current` on bitplane-owned source repos only

A plane branch is created from `origin/main`, so its upstream is `origin/main`
and the names differ. Under git's default `push.default = simple`, a plain
`git push` in a worktree bitplane just created fails:

```
fatal: The upstream branch of your current branch does not match
the name of your current branch.
exit 128
```

bitplane would be handing the user a worktree where the most ordinary git command
fails on a config line bitplane chose. An **adopted** project's config is never
touched — bitplane does not own that repo.

### A failed `project_add` keeps the objects and unwinds only the registration

ADR-0004 recorded that `project_add` follows `plane_create`'s abort-and-remove
shape but that the justification does not transfer, and left the call here. It is
taken: **discard the registration, keep the object store.**

`plane_create`'s window is cheap because its only expensive step writes the
source repo, which is *outside* the window. For `project_add` the cold fetch **is**
the window. And `init --bare` + `fetch` is resumable in a way `git clone` is not:
a partial object store is reused by the next fetch.

`project.toml` is never written, so by the existing rule the directory simply is
not a project. `Err(EngineError::ProjectAddAborted { kept })`, with the remedy
naming the path and saying the retry will reuse it. **`doctor` gains a third
sweep**: project directories holding `repo.git` and no `project.toml`, mirroring
ADR-0004's plane-side sweep.

### `plane_commit` and `plane_push` leave the action set

Scoped out as nice-to-have. The action set is **seventeen**: six reads
(`plane_list`, `plane_show`, `plane_status`, `project_list`, `project_show`,
`doctor`) and eleven mutations (`plane_create`, `plane_rename`, `plane_destroy`,
`plane_add`, `plane_remove`, `plane_repair`, `project_add`, `project_adopt`,
`project_rename`, `project_remove`, `project_fetch`).

Note that this removes the *verbs*, not the concerns: the `unpushed` waiver still
gates `destroy`, and `push.default` above matters **more** once the user is the
one running `git push`.

## Consequences

- **ADR-0003 is amended in four places.** The action set is seventeen;
  `Pushed` and `remote.is_bitplane_mirror` are deleted; `ProjectView` loses
  nothing but its `default_branch` becomes derived; and see the open constraint
  below.
- **An open constraint is handed back to the contract, not resolved here.**
  `BranchIntent::Resolve` creates a branch when it does not resolve, and
  `plane_add` takes `fetch: bool`. Together, `bp add @foo:colleagues-branch`
  with `fetch: false` against a stale source repo silently creates a *new,
  unrelated* branch of that name off the default branch; the user discovers it at
  push time, having already committed. The recommendation on the table is that
  `Resolve` requires a fetch in the same operation and errors naming the
  staleness otherwise, with `RequireNew` as the way to ask for a new branch
  deliberately. ~~**Not ratified.**~~ **Ratified by
  [ADR-0007](./0007-project-schema-and-scripts.md)**: refused at request
  validation, spelled `branch_intent_requires_fetch` on the CLI.
- **Ticket 10 gains a second census oracle.** `refs/heads/*` in a bitplane-owned
  source repo is exactly the set of plane branches, so "which branches does
  bitplane own here" is one ref walk with no git-worktree parsing, alongside
  `git worktree list`.
- **Adopted projects keep none of these guarantees.** The user's checkout has the
  user's branches in `refs/heads/*`, whatever refspec they set, and an occupied
  branch. Only the ownership rule survives there, and bitplane edits no config in
  a repo it did not create.
- **`CONTEXT.md` gains `plane branch` and `occupied branch`**, and retires
  **bare mirror** into **source repo** with `--mirror` recorded as forbidden.
- **The map's "Where `bp push` actually sends" item closes as void**, not
  answered.
- Nothing here needs a new crate; it is all `git` invocations.

## Alternatives considered

**`git clone --mirror`, as every prior document implied.** Rejected on measured
harm: it force-pushes and deletes refs on the forge through an ordinary
`git push`, and it breaks `project_fetch` permanently for any project whose plane
branch also exists upstream. It is a *replication* tool — its intended use is
`clone --mirror`, change the URL, `push --mirror` to move a repo between forges —
and `mirror = true` exists precisely so the copy can be re-published verbatim.
That is the right behaviour for a migration and the wrong one under a user's
worktree.

**A non-bare source repo with a detached HEAD.** The strongest alternative, and
the one preferred for most of the discussion. Its real advantage is that `clone`
writes the refspec and `origin/HEAD` for you, so the load-bearing config cannot
be forgotten; the cost of rejecting it is that `project_add` must set them
explicitly. Rejected because the no-occupying-HEAD property is *maintained*
rather than structural — see the table above — and because the same benefit was
available by writing one config line deliberately.

**No HEAD at all.** Not an option: measured four ways, above.

**`git clone --bare` then fixing up the refspec.** Rejected: the clone copies the
forge's branches into `refs/heads/*` first, so the namespace must then be emptied
by hand, and a missed ref is indistinguishable from a plane branch forever after.

**`default_branch` as a stored field in `project.toml`.** Rejected as a cache
with no invalidation. What survives of the idea is that a user *can* choose a
default — via `git remote set-head origin <branch>`, which is durable across
fetches and is git's own vocabulary.

**Fetching only the branches some plane needs.** Rejected: identical cost on one
round trip, and it turns a colleague's new branch into a second fetch.
