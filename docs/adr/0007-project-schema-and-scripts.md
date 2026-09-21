# ADR-0007: `project.toml` is four fields, and hooks are named scripts

Status: accepted
Date: 2026-09-21
Ticket: `.alfonz/issues/bitplane-architecture/issues/08-project-schema-and-hooks.md`

## Context

[ADR-0005](./0005-the-source-repo-is-a-bare-clone-shaped-repo.md) resolved half
of ticket 08 — the source repo is bare, built clone-shaped — and left two things
open: what is actually *in* a `project.toml` now that `default_branch` is gone,
and all of hooks. This closes both.

Three things were settled in ADR-0005's session and are carried here rather than
re-argued: `project.toml` is **flat, with no per-host table** (a URL project is
portable by luck, a local one is host-bound by nature, and "is it cloned here?"
is a `stat`, never a stored field); hooks are **per-project**, with the plane
directory in the environment, so no plane-level config file is needed and ticket
09 stays closed; and **`.worktreeinclude` is not built in**, because silently
copying a gitignored `.env` into a new worktree is bitplane acting unbidden and
`.env` is exactly where a surprise copy is a security event.

The word "hook" did not survive the session. Once a user-declared command could
be invoked by name on demand — not only fired at a lifecycle point — the
glossary's definition ("a user-declared command bitplane runs **at a defined
point** in a plane's lifecycle") stopped describing the thing.

## Decision

### `project.toml` is `version`, `name`, `source`, `[scripts]`

```toml
version = 1
name = "codestyle"

[source]
type = "owned"
url = "git@gitlab.com:signageos/codestyle.git"

[scripts.link-alfonz]
argv = ["ln", "-s", "../../.alfonz", ".alfonz"]
post_worktree_create = true

[scripts.install]
shell = "pnpm i && pnpm build"
post_worktree_create = true

[scripts.stop-stack]
shell = "docker compose down --remove-orphans"
pre_worktree_remove = true

[scripts.reset-db]
shell = "pnpm db:reset && seed-fixtures"
```

**`name` is an integrity check only.** The directory name *is* the name
(CONTEXT.md), and on conflict the directory wins; the field exists so a project
directory moved or renamed outside bitplane is detectable, and so an interrupted
`project rename` is distinguishable from a completed one. `project rename`
therefore writes it **last**, exactly as `plane rename` writes `plane.toml`'s
`id` last (ADR-0004).

This is *not* the symmetry argument, which would have proved too much.
`default_branch` was rejected in ADR-0005 because git already stores it durably
and updates it itself; a directory name stores the project name but cannot tell
you it was renamed out from under you. That gap is the whole justification.

**`fetched_at` is derived, not stored.** `MirrorState::Present { fetched_at }`
is fed by the mtime of `<project-dir>/repo.git/FETCH_HEAD`, which git writes on
every fetch. Storing it would repeat ADR-0005's `default_branch` mistake and
would make **every fetch do a `toml_edit` read-modify-write under the project
lock** for a field nothing gates on.

**Nothing else.** Tags and grouping are a query feature with no query surface —
a future feature, deliberately out of scope now. A record of which derivation
produced the default name only serves a rename bitplane refuses to do
automatically.

### Projects are **owned** or **adopted**, and that is one word, not five

The design had drifted to five names for two kinds: *URL project*,
*URL-sourced project*, *local project*, *local-path project*, *adopted project*,
plus *bitplane-owned source repo*. The glossary exists to stop precisely this.

The pair is named after **ownership**, not after the source, because every
behavioural difference in the design branches on ownership and is already
written that way: the plane-branch invariant holds only for a bitplane-owned
source repo; `push.default = current` is set on owned repos only; `destroy`
deletes the member's branch and is skipped entirely for adopted projects; a bare
repo occupies no branch while an adopted checkout occupies whatever the user is
sitting on; `project_fetch` and `MirrorState` have nothing to do for an adopted
project. A reader told *"adopted means the user owns the repo"* predicts all
five. A reader told *"local means a local path"* predicts none of them.

`ProjectSource` is therefore `#[serde(tag = "type")]` with variants
`Owned { url }` and `Adopted { path }`, replacing `Url` and `LocalPath`. The
discriminator names the **project kind** rather than the payload shape, so one
word serves the glossary, the prose, the TOML and the JSON.

**The 1:1 correspondence is a rule, and is written down**: an owned project's
source is always a `url`, an adopted project's always a `path`. It holds because
`adopt` never converts a checkout and `add` always builds a bare repo. It is
recorded because it is exactly what a future "adopt an existing bare repo" would
break, and that feature should have to re-argue it rather than inherit the pass.
A `type` and payload that disagree is a `ParseError`.

Three candidate names were rejected on collisions, not on taste. **`remote`**
collides twice and fatally: git's own `remote` is everywhere in this design
(`remote.origin.fetch`, `refs/remotes/origin/*`, `git remote set-head`), and
bitplane's **host** concept already owns the word — `bp --host devbox` is in the
map, so "remote project" reads as *a project on another machine*, a different
axis entirely. **`native`** collides with ADR-0001's "no native dependencies"
and "no native Windows" — a natural reach, since the counterpart to an adopted
child is a natural one, but the word is taken. **`cloned`** is a trap: ADR-0005
deliberately does *not* clone, building the repo with `git init --bare` +
`remote add` + `fetch` because `clone --bare` pollutes `refs/heads/*`.

### A **script** is a named user-declared command, and "hook" is retired

A script is declared on a project, may be bound to zero or more lifecycle
points, and is always runnable on demand. `hook` leaves the vocabulary the way
`store`, `registry`, `marker` and `bare mirror` did.

`script` is `package.json`'s word for this exact concept and `bp run @api build`
mirrors `npm run build`, so the model is already in the reader's head. It does
**not** collide with `<project-dir>/bin/` below: `bin/` holds executables on
`PATH`; a script is a declared entry in `project.toml`. One glossary line
separates them.

The alternative that got furthest was **`action`**, and it fails on the one word
this project cannot spare: the Engine's verbs are *actions* throughout — ticket
06 is titled "the action set", ADR-0003 settles "eighteen actions", ADR-0004
says nineteen, ADR-0005 says seventeen. `plane_actions` would read as *an Engine
action that runs Engine actions*.

**The declaration form is a named table with boolean point toggles**, not a list
of names under each point:

- **Each script has a user-chosen name by construction**, which makes
  `ScriptOutcome.name` honest. The list-per-point form needed a positional
  fiction (`post_worktree_create.1`) to fill a field the contract already
  declared non-optional.
- A script bound to no point is **legal and useful** — `reset-db` above — and
  the toggle form expresses that by omission rather than by absence from a list.
- Each entry is `argv = [...]` **or** `shell = "..."`. argv by default avoids
  inventing quoting rules; the shell form exists because the real examples
  (`ln -s … .alfonz`, `pnpm i && pnpm build`) want a shell and pretending
  otherwise just makes everyone write `["sh","-c",…]`. A bare-string shorthand
  is rejected: a list of strings reads like argv and would mean shell.
- Use **single-quoted TOML** for any `shell` value containing `$VAR`, so TOML's
  escaping never fights the shell's. It is the first thing people get wrong.

**Execution order is TOML declaration order.** Order is semantic, not cosmetic —
Q5's sequential execution means `link-alfonz` must land before `install` if the
build reads `.alfonz`. Declaration order is what someone hand-writing a TOML
file expects, and bitplane never rewrites this table: `toml_edit` touches `name`
and `source`, never `[scripts]`, so the order cannot change behind the user's
back.

**Accepted cost, recorded because the rejected form avoided it**: *"what runs at
`post_worktree_create`, in what order"* is not answerable from any single place
in the file — you read every table, in order. `bp project show` therefore prints
the **resolved per-point sequence**, which turns a config-reading problem into a
command.

### Two script points, named for the worktree

`post_worktree_create` fires per member on both `create` and `add`;
`pre_worktree_remove` fires on both `destroy` and `remove`. Both are the
identical situation seen twice, and the points are named for the **worktree**
rather than the plane because scripts are per-project.

**No rename point.** Rename changes a path, not content, `git worktree repair`
already fixes what breaks, and a script there would run inside a plane holding
real work with no unwind story.

**No project-lifecycle points** (`post_project_add`, `post_fetch`), and on a
sharper ground than taste: they would run before any worktree exists, so
`BITPLANE_WORKTREE` and `BITPLANE_PLANE_DIR` are undefined. That is a *different*
contract wearing the same name.

**`pre_worktree_remove` is skipped entirely on a latched plane.** ADR-0004's
latch means *"this plane was never completed, nothing in it is yours"*, which is
what licenses a refusal-free `destroy`. No `post_worktree_create` ever ran
there, so firing its counterpart would hand a script a worktree it never set up
— and a blocking script (below) would break the refusal-free guarantee outright.

**`PlaneRemoveRequest` gains `run_scripts: bool`.** `PlaneCreateRequest`,
`PlaneAddRequest` and `PlaneDestroyRequest` all carried the flag and
`plane_remove` did not; ADR-0006 made `plane_remove` *identical* to `destroy`,
so the omission was an oversight, not a decision.

### Pre-scripts block; post-scripts do not

ADR-0004's "a script failure never unwinds anything" was written for
post-create, where by construction there is nothing left to stop. A *pre* script
runs before a destructive act, so it gets the opposite treatment — and the rule
is symmetric and easy to hold: **pre blocks and short-circuits, post does
neither.**

Mechanically, for `destroy` and `remove`:

1. Every refusal across every project is collected first and returned as one
   `EngineError::Refused`, exactly as before. Scripts **cannot** join that pass:
   running `docker compose down` and *then* refusing the destroy over
   uncommitted work leaves the container stopped and the plane intact.
2. Once refusals pass, `pre_worktree_remove` runs as **one sequential pass over
   every project**, aborting on the first non-zero exit.
3. On a block, **nothing is removed** —
   `Err(EngineError::ScriptBlocked { outcomes })`, exit `1`.

The envelope `Err` is correct under ADR-0004's amended rule: the operation
produced **no durable state**, because no worktree was removed and `plane.toml`
still stands.

It is a **new `EngineError` variant, not a new `Refusal`**. Refusals are
collected-before-anything-is-touched and are waiver-shaped; folding a block in
there would invite a `hook_failed` waiver, and ADR-0006 drew the only clean line
in the design — *a waiver says "I accept losing **this** work, which I am
looking at"*. A script's exit code is a program's opinion, not work the user is
looking at. `Waivers` stays at six.

**A `project.toml` cannot make a plane undestroyable.** `bp destroy --no-scripts`
always works, because `run_scripts` is on the request. That is the entire answer
to the obvious objection and it needed no new mechanism.

**Accepted cost, stated plainly**: a blocking pre-script conflates *veto* with
*teardown work*, so a failure partway through the pass leaves earlier projects'
teardown already done with nothing removed. The user's script, the user's
unwind. A separate side-effect-free veto point is more surface than this earns.

### Execution semantics

**Sequential everywhere** — across projects in `plane.toml` member order, and
within a project in declaration order. ADR-0003 made the git fan-out parallel
because per-source-repo locks cannot contend within one plane; that
justification does **not** extend to arbitrary user commands, and two `pnpm i`
racing on one shared store is the obvious first bug, with two `docker compose
up` on one port close behind.

This is a **deliberate latency cost**: a six-repo plane whose script is
`pnpm i` serialises six installs. The escape is asymmetric in the right
direction — a script that wants concurrency can background its own work, while a
script that needs serialisation cannot invent it. No `parallel = true` knob: it
would be a per-project flag governing a cross-project property, which is the
wrong shape.

Sequential is also what makes live output legible (below): exactly one script
writes to stderr at a time, so no interleaving and no per-line prefixing.

**cwd is the worktree.** **`<project-dir>/bin` is prepended to `PATH`**, so a
project can ship its own executables and have them win — which is the only
reason to put a directory on `PATH` for one project's scripts. `bin/` rather
than `<project-dir>` itself, because the project directory holds `project.toml`
and `repo.git` and every file bitplane ever adds there would become a
command-name collision; a non-existent `PATH` entry is silently ignored, so
`bin/` costs nothing until the user makes it.

**Eight environment variables**, and the caller's environment is inherited in
full:

| variable | value |
| --- | --- |
| `BITPLANE_SCRIPT` | the script's name |
| `BITPLANE_PROJECT` | bare project name — the `@` sigil never appears here |
| `BITPLANE_PROJECT_DIR` | `<projects-dir>/<name>/` |
| `BITPLANE_SOURCE_REPO` | `repo.git` when owned, the user's checkout when adopted |
| `BITPLANE_WORKTREE` | the worktree path (= cwd) |
| `BITPLANE_BRANCH` | the **live** `HEAD`; **unset** when detached |
| `BITPLANE_PLANE_DIR` | the plane directory |
| `BITPLANE_PLANE_ID` | the plane id |

`BITPLANE_PROJECT_DIR` is not derivable from `BITPLANE_SOURCE_REPO`: for an
adopted project the two are unrelated paths. It is also what makes the
`.worktreeinclude` replacement work for **both** kinds — an owned project's
source repo is bare and has no working tree, so there is no `.env` there to
copy; the file the user wants lives in the project directory.

Full inheritance rather than a sanitised base, because a sanitised environment
would break `SSH_AUTH_SOCK`, `ssh-agent` and every credential helper, and
**git's credentials, never ours** means bitplane passes the user's environment
through untouched.

`BITPLANE_BRANCH` is the live `HEAD` per ADR-0006, and is **unset** rather than
set to a sha when detached, so a script doing
`git switch "$BITPLANE_BRANCH"` fails loudly instead of detaching again.

**Nothing in that set is stable.** The plane id is mutable by design and the
directory moves with it, so a script must persist none of it. Withholding the id
would not help — a script would take `basename` of the plane directory.

### No timeout; output is live, merged, and on stderr

**No built-in timeout.** A legitimate `pnpm i && pnpm build` can exceed any
default worth shipping, and a build killed at ten minutes is a worse failure
than a hang the user can see.

What makes that safe is fixing the visibility half instead: **stdout and stderr
are merged and streamed live** to bitplane's stderr *and* tee'd to the log, so a
hang looks like a hang rather than a freeze. This supersedes the earlier
proposal to stream only on failure.

Merged **onto stderr**, never stdout: ADR-0003 makes stdout the machine
contract, and a `pnpm` banner inside the JSON is a broken contract.

Logs land at
`<plane-dir>/.bitplane/logs/<YYYYMMDDTHHMMSSZ>-<project>-<script>.log` —
a compact timestamp because RFC 3339's colons are hostile in filenames — and
`ScriptOutcome.log` is that path. bitplane never cleans them up; `destroy`
removes them with the plane directory, and anything beyond that is ticket 10's,
which already records the retention gap. Script names are validated against the
same character set as a plane id, so a user-chosen name is never trusted into a
path.

`ScriptOutcome.finished_at` stays **non-optional** (ADR-0004), so a script
killed mid-run leaves no record at all. "Started but never finished" remains
unrepresentable, and Ctrl-C remains the escape.

### `bp run` is the eighteenth action

```rust
plane_scripts { plane: PlaneRef, names: Vec<String>, projects: Vec<ProjectName> }
```

Both vectors are non-empty or it is `ExitCode::Usage`. It is on **`Engine`, not
`Reader`** — it runs arbitrary user commands and writes logs, and "a read never
writes" is checkable by reading the trait (ADR-0003). It **refuses on a latched
plane**, same grounds and remedy as `plane_repair`. A name with no matching
`[scripts.<name>]` is `EngineError::ScriptNotFound { project, name }`; a project
that is not a member is the existing `ProjectNotInPlane`.

Scripts run in **request order**, not `plane.toml` order — unlike `create`,
the caller named them, so that is the order they meant.

The CLI spelling is `bp run @codestyle install`, resolving the plane from cwd:
it is a plane verb and therefore bare (ADR-0006), and `bp run` reads as a verb
where `bp script` would read as a noun. **The project is mandatory** — nobody
gets a six-repo script run by typing nothing.

**It takes script names only, never a point name.** Points are bindings, not
addressable things, so the two never share a namespace and no reserved-name rule
is needed. The cost is that recovering from a failed `create` means naming the
scripts rather than naming the point; `create`'s response already named exactly
which script failed, so the information is in hand.

ADR-0004 declined `plane_hooks` and explicitly left it to this ticket. What
tipped it is **reproduction cost**: re-running by hand means reconstructing
eight `BITPLANE_*` variables, the `bin/` `PATH` entry and the right cwd. That is
not something a user will get right, and getting it wrong silently is worse than
not retrying at all. It is **not idempotent** and bitplane does not pretend
otherwise — re-running `ln -s` fails the second time, and that failure is the
user's to own, exactly as the first one was.

### Unrecognised keys are a `ParseError`, not a shrug

Three cases, all at read time, all `EngineError::ParseError { path, message }`
naming the offending key and listing what was legal: an unknown key inside a
`[scripts.<name>]` table (neither `argv`, `shell`, nor a known point), an
unknown point toggle, and a `[source]` `type` that disagrees with its payload.

The forward-compatibility argument for silence is already answered by a
mechanism that exists: `version = 1` owns schema evolution, and a bitplane that
adds a third script point bumps it. Silence buys nothing and costs a script that
never runs with no trace of why — the user's setup did not happen, their
worktree is subtly wrong, and nothing in `bp create`'s output points at the
cause. A misspelled toggle in a hand-written file should fail loudly, which also
means it fails on `bp project show` and `bp doctor`, not only at create.

### The trust posture holds, with two constraints

A script in `project.toml` is there because the user typed it into their own
data directory — **shell-alias trust**. A committed `.bitplane.toml` is
categorically different, because `bp project add <url>` would then execute the
repo author's code; that is *why* in-repo declaration is out of scope rather
than deferred.

The reasoning holds only while **nothing writes `project.toml` on the user's
behalf from remote content**, and — new with `bin/` on `PATH` — only while
**nothing writes an executable into `<project-dir>/bin/` from remote content**.
With `bin/` as a named subdirectory that is trivially true; it would have been
true only by accident had `<project-dir>` itself gone on `PATH`, since
`repo.git` holds fetched remote content and merely happens to be a directory.

Both are recorded so a future "import scripts from the repo" has to re-argue the
posture rather than inherit the pass.

### `BranchIntent::Resolve` requires a fetch

Handed back unratified by ADR-0005 and tracked as an open item on the map.
`Resolve` creates a branch when it does not resolve, so
`bp add @foo:colleagues-branch --no-fetch` against a stale source repo silently
creates a **new, unrelated** branch of that name off the default branch; the
user finds out at push time, having already committed.

`BranchIntent::Resolve` with `fetch: false` is **refused at request validation**,
before anything is touched, with a remedy naming both `--fetch` and
`RequireNew` as the deliberate way to ask for a new branch. Spelled as a
validation on the *request* rather than as a behaviour change inside `Resolve`,
so `BranchIntent` stays a pure statement of intent and the rule sits where
ADR-0003 already puts every other collected-before-anything-is-touched check.

## Consequences

- **The action set is eighteen**: `plane_scripts` joins `Engine`. (ADR-0005 left
  it at seventeen after scoping out `plane_commit` and `plane_push`.)
- **A rename cascade through the contract**: `HookOutcome` → `ScriptOutcome`,
  `run_hooks` → `run_scripts` (and added to `PlaneRemoveRequest`),
  `ProjectView.hooks` → `ProjectView.scripts`. New: `EngineError::ScriptBlocked`,
  `EngineError::ScriptNotFound`.
- **`ProjectSource` changes shape**: serde tag `source` → `type`, variants
  `Url`/`LocalPath` → `Owned`/`Adopted`. A trivial amendment to ADR-0003's
  types, made explicitly rather than silently.
- **`Waivers` stays at six.** A blocked pre-script is not a waiver.
- **`bp project show` gains the resolved per-point sequence**, which is the
  price of the toggle declaration form.
- **CONTEXT.md loses `hook`** and gains `script`, `script point`, `owned
  project` and `adopted project`; `URL project`, `URL-sourced project`, `local
  project` and `local-path project` are retired by name.
- **Ticket 10 inherits the log retention gap unchanged** — bitplane never
  cleans `<plane-dir>/.bitplane/logs/`, and `destroy` removing them with the
  plane directory is the whole of the current story.
- **Ticket 09 stays closed.** Scripts being per-project is what avoids a
  plane-level config file. `ScriptOutcome.project` is always `Some` today; the
  `Option` is room to add plane-level scripts later without a contract change.
- **The map's `BranchIntent::Resolve` open item is closed.**

## Alternatives considered

**Names listed under each point, instead of toggles on the declaration.**
Genuinely close, and it wins on two properties: execution order is explicit in a
list rather than implicit in file layout, and *"what runs at create"* is one
line instead of a scan of every table. Rejected in favour of toggles for
locality — the binding sits next to the command it binds — with the ordering
hazard answered by never rewriting the table and the readability cost answered
by `bp project show`.

**A blocking pre-script as a seventh waiver (`hook_failed`).** Rejected because
it would be the one waiver not about work the user is looking at, which breaks
the sentence that gives every other waiver its meaning.

**Per-project granularity for the blocking pass** — run this project's
pre-scripts, remove this worktree, next. Rejected: a block in project #4 would
have already destroyed #1–#3, and ADR-0003 holds that a half-destroyed plane is
worse than a refused one.

**A built-in script timeout.** Rejected on the grounds that no shippable default
survives a real `pnpm i && pnpm build`, and a timeout would have to invent an
exit code for "we killed it" against a deliberately non-optional
`finished_at`. Live output removes the only real argument for it.

**`<project-dir>` itself on `PATH`**, rather than `bin/`. Rejected on collisions
— every file bitplane ever adds to the project directory becomes a command name
— and on putting a directory that holds fetched remote content on the
executable search path.

**Parallel script execution, or a `parallel = true` knob.** Rejected; see above.
Note the door is one-way in the cheap direction only: sequential → parallel
breaks scripts written against the sequential guarantee, so it cannot be
"optimised" later without a contract change.

**`action`, `recipe`, `routine`, `procedure`, `errand` as the noun.** `action`
collides with the Engine's action set and is unusable. The others are all
workable; `script` won on `package.json`'s established usage, so `bp run @api
build` teaches itself.
