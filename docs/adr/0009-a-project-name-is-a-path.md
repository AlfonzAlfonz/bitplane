# ADR-0009: A project name is a path

Status: accepted, amends [ADR-0007](./0007-project-schema-and-scripts.md) and [ADR-0008](./0008-the-plane-file-is-three-keys-and-a-path-keyed-membership-table.md)
Date: 2026-09-23
Ticket: `.alfonz/issues/nested-project-names/issues/01-a-project-name-is-a-path.md`

## Context

[ADR-0007](./0007-project-schema-and-scripts.md) gave a project a `name` field
and left it flat: lowercase `[a-z0-9][a-z0-9._-]*`, one token, unique per host,
defaulting to the last segment of the source. That held for as long as the
projects a user registered came from one namespace each. It does not hold for a
GitLab instance, where a repo's identity **is** its group path and the last
segment is the least distinctive part of it.

Three pains follow, and they are one pain wearing three coats — a flat name
cannot hold a group path:

- **Collisions.** `acme/platform/codestyle` and `acme/infra/codestyle` both
  derive the default name `codestyle`. The second `bp project add` is refused
  and has to be given a `--name`, even though the two repos were never
  ambiguous on the forge.
- **Legibility.** The escape hatch out of that refusal is the suggested
  alternative, and what it suggests is `acme-platform-tooling-codestyle` — a
  name nobody can read, type or guess, that exists only because the separator
  the user already knows was unavailable.
- **Bulk.** Importing a group tree means one `--name` per repo, chosen by hand,
  one at a time. The larger the tree the less the tool helps, which is backwards.

The pressure this puts on the tool's boundaries is worth naming, because the
obvious relief is out of bounds. *git protocol yes, forge API no* is a boundary
rule, and a `--group` flag that imports every repo under a GitLab group is
exactly what it forbids: enumerating a group is a forge API call, not a git
operation. Held to that rule, the bulk pain has no flag-shaped answer — so it
gets a naming-shaped one, and the rule survives having been leaned on rather
than being quietly filed down.

A second thing this surfaced. `CONTEXT.md` justified the two arms of
`MemberRef` — project versus path — with the claim that *a project name
contains neither `@` nor `/`*, and
[ADR-0008](./0008-the-plane-file-is-three-keys-and-a-path-keyed-membership-table.md)
repeated it when it admitted the sigil onto disk. The conclusion is right and
the reason is not; this ADR rewrites the reason before the charset change makes
it false.

## Decision

### A name is `/`-separated segments, each on today's charset

Each segment is `[a-z0-9][a-z0-9._-]*` — the charset ADR-0007 already settled,
unchanged — joined by single forward slashes:

```
acme/platform/tooling/codestyle
```

Refused: a leading or trailing `/`, an empty segment (`//`), and a `.` or `..`
segment. Those fall out of the charset rather than needing rules of their own —
a segment cannot start with `.` — but they are stated because a name is now a
path on disk and a path that can say `..` is a path that can escape.

**There is no depth cap.** GitLab nests as deeply as its groups do, and
refusing an import because a group is four levels down is the exact pain being
fixed; a cap would simply move the refusal rather than remove it. Total length
is bounded at **128** characters, which is a filesystem bound, not a modelling
opinion.

**`repo.git` and `bin` are forbidden as any segment.** Both are legal under the
segment charset, and both name a directory bitplane itself puts inside a
project directory — the bare source repo and the `PATH`-prepended executables
directory. The walk below **skips both by name**, because it must: an
interrupted `add` leaves a `repo.git` with no `project.toml` behind it. So a
project named `@acme/bin` would sit at a path the walk refuses to enter and
would be **invisible to every listing** — registrable once and never seen
again. The reservation is what makes the skip list safe; without it the skip
list is the thing that loses projects.

Note that the nesting rule below does *not* already cover this. It would refuse
`@acme/bin` only while `@acme` is itself a registered project; `@acme/bin` with
no `@acme` registered is nesting-legal and still invisible.

`.bitplane` needs no rule of its own: a segment cannot start with `.`.

### The sigil, not the charset, keeps the two member arms apart

`MemberRef::parse` is **untouched**, and the property it relies on is restated:
`@` is **required wherever a path is also accepted**. `@acme/codestyle` is a
project; bare `acme/codestyle` is still read as a relative path, exactly as it
was before names could contain a slash.

This was always the real rule. The charset argument — *a project name contains
no `/`, so the arms cannot collide* — was a second, accidental guarantee that
happened to hold, and stating it alongside the sigil made it look load-bearing.
It never was: the sigil is required in precisely the position where the
ambiguity exists, which is the whole reason it is required there. ADR-0008's
sentence and `CONTEXT.md`'s entry are corrected to say so.

### An owned project takes the full forge path after the host

`git@gitlab.com:acme/platform/tooling/codestyle.git` registers as
`@acme/platform/tooling/codestyle`. The transport and a trailing `.git` are
stripped; **the host is not part of the name.**

Leaving the host out means two forges serving the same path collide. That is
accepted, and it falls to the ordinary `project_name_taken` refusal — a user
mirroring one path across two forges is rare, and the alternative is a name
beginning `gitlab.com/` for everybody else.

### An adopted project keeps the last segment

`bp project adopt ~/projects/bitplane` still registers `@bitplane`.

So does an **owned** project whose url names no host. A bare local path is a
git url — `bp project add /srv/git/acme/codestyle.git` works, and it is what
every fixture in the test suite uses — and `file:///srv/…` is one too. Neither
has a host to strip, and reading "the full path after the host" literally there
would register `@srv/git/acme/codestyle`. The rule below is what decides it,
not the url's shape: there is no namespace in a local path, so there is nothing
to keep.

The asymmetry is deliberate and it is not a convenience. **A forge path is a
namespace**: stable, globally unique, agreed on by everyone who clones the
repo, and the very thing whose absence made the name collide. **A filesystem
path is an accident** of where a home directory happens to sit —
`@users/alfonz/projects/bitplane` names the machine, not the project, and would
differ between two people working on the same checkout. Taking the full path in
one case and the last segment in the other is one rule applied twice: *use the
namespace when there is one.*

### A derived name is lowercased silently

Uppercase in a **derived** name is folded to lowercase with no message.

This is a **normalisation, not a guess**. Two names differing only in case are
the same directory on a case-insensitive filesystem, so `Codestyle` and
`codestyle` are not two names bitplane could honour anyway — it is the same
argument that already makes a plane id lowercase-only, applied where the same
fact applies. A guess would be bitplane choosing between things the user might
have meant; there is nothing here to choose between.

It applies to the **URL-derived default only**. `--name` is the user's stated
intent and is never rewritten, and `ProjectName::parse` stays strict
everywhere else — a hand-edited `project.toml` or `plane.toml` carrying a
capital is still a parse error, never a silent rewrite of a file the user
wrote.

**A derived name still invalid after lowercasing just fails.** ADR-0007's
fold-everything-to-`-` suggestion is deleted along with the flat names that
motivated it: `derived_name_invalid` names the offending segment and stops.
Suggesting a mangled name was worth it when the alternative was no name at all;
now that the default is the forge's own path, a URL that cannot produce a valid
name is a URL worth looking at rather than papering over.

### `qualified_name` is deleted, and the ladder suffixes the last segment

`ProjectSource::qualified_name` existed to answer "what would this project be
called if the parent segment were part of the name?" The parent segment **is**
part of the name now, so there is nothing left to qualify with.

The alternatives ladder survives for a taken name, and it suffixes the **last
segment**: `acme/api` yields `acme/api-2`, never `acme/api/2` or `acme-api-2`.
Suffixing the whole name would propose a *nested* project, which is refused a
paragraph below.

The ladder skips a candidate that is **taken or that would nest**, not merely
one that is taken. A suggestion the user cannot act on is worse than no
suggestion: the whole value of the remedy line is that the command it prints
runs.

**A taken derived name usually means the repo is already registered.** Before
choosing a message, compare the registered project's source: the **same** url
is reported as already registered, with no suggestion and nothing for the user
to do; a **different** url falls to the ladder as before. The old behaviour —
offering `codestyle-2` to somebody who has just re-run the same `project add`
— proposed a second copy of a repo they already had.

### On disk, a name is a nested directory

`<projects-dir>/acme/platform/tooling/codestyle/`. The directory name **is** the
name, as it always was; there is still no registry file.

The walk that finds projects becomes recursive, with three guards:

- **Stop descending at the first directory holding a `project.toml`.** A
  project's own contents are not more projects.
- **Never descend into `.bitplane`, `repo.git` or `bin`**, or any dot-directory.
  This is load-bearing rather than tidy: an interrupted `project add` leaves a
  bare `repo.git` behind with **no** `project.toml`, so a walk that only stops
  at registrations would descend into a git object store on every
  `bp project list`.
- **A hard depth cap of 16**, as a runaway guard against a symlink loop or a
  pathological tree. It bounds the *walk*, not a name.

The last guard and the uncapped name are in tension, and the tension is
accepted rather than resolved: a name of **seventeen or more segments**
registers — nothing at parse time refuses it — and then sits below the walk's
floor, where `bp project list` cannot see it and the taken and nesting checks,
which read the walk, cannot find it. It is reachable, because sixteen
two-character segments fit inside the 128-character bound.

It is left because both halves are right on their own terms. Capping names is
the pain this ADR exists to remove, and an uncapped walk is a `readdir` loop a
symlink can turn infinite. Sixteen is far above any group tree a forge
realistically produces, so the case is pathological rather than merely
unusual — but it is a **known hole, not a proof**, and if a real name ever
reaches it the fix is to refuse at parse time what the walk cannot reach, so
that the two bounds are one number in one place.

**Intermediate directories are not projects.** `add` creates them, they hold no
`project.toml`, and nothing lists them. `rm` and `rename` prune them bottom-up
while they are empty, stopping at the first non-empty directory and never
removing the projects directory itself — so registering and removing one deep
project leaves the tree as it found it, while a sibling project keeps its
parents alive.

### Two new refusals, and two changed ones

**`project_name_nests`** — `@acme` and `@acme/codestyle` cannot both exist. The
second would live *inside* the first's project directory, where
`bp project rm @acme` would take it along without mentioning it and the
registry walk, stopping at the first `project.toml`, would never see it.
Unwaivable — the damage lands on a project the user is not looking at, which is
`project_in_use`'s test — raised at `add`, `adopt` and `rename`, and it names
the blocker in **both** directions: registering a parent of an existing project
and registering a child of one are the same refusal.

At `rename` the check **excludes the project being renamed**, which is not a
special case so much as the absence of one: a project cannot nest inside
itself, and `bp project rename @acme acme/core` is a legal move rather than a
collision. It is the one move whose `mv` cannot be a single call — a directory
cannot be moved inside itself — so it goes via a temporary sibling under the
projects directory, which changes the number of syscalls and nothing else about
the converging order.

**A reserved-segment refusal** — `reserved_name_segment` — for `repo.git` and
`bin`, per the charset above.

**`project_name_taken` gains the same-url case**, per the ladder above.

**`derived_name_invalid` loses its suggestion.**

### What this does not change

Stated because each was argued and kept, not because it was overlooked:

- **Plane layout.** A member's worktree path is still the last two segments of
  the **source**, derived once at create time and never recomputed. Names do not
  enter it.
- **`member_path_collision`** works exactly as it does.
- **`plane.toml` needs no version bump.** Nothing about the file's shape
  changes; a longer string in the member value is still a string.
- **`bp project list` stays flat** — sorted, full names, `--json` unchanged.
- **No `doctor` check.** Nothing on disk becomes wrong, because a flat name is
  still a valid name. There is no migration and nothing to repair.

## Consequences

- **`bp project add <url>` names differently than it did.** This is the breaking
  change and it is user-visible on the first command: a script calling
  `bp run @codestyle` against a project registered from
  `git@gitlab.com:acme/codestyle.git` now gets `project_not_found`, because the
  project is `@acme/codestyle`. Nothing **on disk** breaks — an existing flat
  registration keeps its name forever — so the breakage is confined to names
  derived from here on, and `bp project rename` is the fix for a user who wants
  the old one back.
- **`member_path_collision` becomes reachable.** Two projects sharing a parent
  segment — `acme/platform/tooling/codestyle` and
  `acme/infra/tooling/codestyle` — both derive the worktree path
  `tooling/codestyle`, and cannot be members of the same plane. The refusal is
  correct: two worktrees cannot occupy one directory. It is also **terminal** —
  there is no flag, no waiver and no disambiguator, and the only remedy is two
  planes. Before this ADR the case existed but was hard to reach, because two
  projects with the same parent segment could not both be registered under
  their default names in the first place. This is the one thing name-derived
  layout would fix, and it is rejected below with that cost accepted.
- **`BITPLANE_PROJECT` can now contain `/`.** It is still the bare name with no
  sigil, as ADR-0007 set it; it is no longer a single path segment, so a script
  interpolating it into a path builds a deeper path than it used to, and one
  using it as a filename needs to stop.
- **The forge-API boundary held under load.** It was pushed on by a real,
  repeated pain and the answer was to make the pain unnecessary rather than to
  carve an exception. A shell loop over `glab repo list` piped into
  `bp project add` now works with no `--name` at all, which is the integration
  the user wanted, written where forge-specific knowledge belongs — in the
  user's shell, not in bitplane.

## Alternatives considered

**Half-nested naming** — last segment by default, falling back to the full path
only on a collision. Rejected: it produces a registry that is half flat and half
nested, where a name's shape depends on the order things were imported in, and
where the same repo is `@codestyle` on one machine and
`@acme/platform/codestyle` on another. Worse than either end state, and it keeps
the legibility pain for exactly the projects that collide.

**A `--group` bulk-import flag.** The most directly requested option and the
one the boundary rule forbids: enumerating a group's repos requires the GitLab
API, and *git protocol yes, forge API no*. It would also be one flag per forge
forever. The full-path default is what makes it unnecessary rather than merely
disallowed — the loop over `glab repo list` was already possible and was only
unpleasant because of the `--name` per repo.

**Prefix selectors** (`bp project fetch @acme/...`, `bp project list @acme/`).
Deferred, not rejected on the merits — they are a genuinely useful thing a tree
makes possible. Deferred because nothing here needs them and because a syntax
chosen now would be chosen without the usage that should decide it. **No syntax
is reserved**, so this is a free future decision.

**Tree-rendered listings.** `bp project list` drawing a tree instead of a sorted
list of full names. Rejected: sorted full names already group visually, because
sorting a list of paths *is* a tree traversal, and they stay greppable, which a
tree with elided parent segments does not.

**Name-derived plane layout** — a member's worktree path taken from the project
*name* instead of the project *source*. Attractive because it would dissolve
`member_path_collision` between two registered projects by construction: names
are unique per host, so derived paths would be too. Rejected as a second change
riding on this one — it changes the layout table in `CONTEXT.md`, the member
syntax reference, and ADR-0008's worked example, and it changes them for **every
member**, including every plane already on disk. If the collision consequence
above turns out to bite in practice, this is the change to reopen, on its own
ticket, with its own migration answer.
