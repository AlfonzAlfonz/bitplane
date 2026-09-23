# Contributing to bitplane

## Running the tests

```sh
cargo test --workspace
```

That is everything: the engine's unit tests, the end-to-end CLI tests, and the
**reference snapshot suite**, which runs the documentation. To run only the
snapshot suite:

```sh
cargo test --test reference
```

Both need a real `git` of 2.36 or newer on `PATH`. The suites build real
repositories and real worktrees in a scratch directory of their own; nothing is
stubbed, because a fake git would only test bitplane's idea of git.

CI runs `cargo fmt --all -- --check`, `cargo clippy --all-targets -- --deny
warnings` and `cargo test --workspace` on every pull request.

## The reference is the specification, and it is executable

Every command has a page under `website/docs/reference/`, and the page is
written **before** the code. Each one carries a status admonition under its
`#` heading — `:::implemented`, `:::in-progress` (whose body names what is
missing) or `:::not-implemented`.

`crates/bitplane-cli/tests/reference.rs` runs those pages. For every example —
a fenced block starting `$ bp …` and the block or two holding what it printed —
it stages the world the page assumes, runs the command **as the page writes
it**, and asserts the page's own bytes against stdout, stderr and the exit
code. Failures are asserted in both renderings: the same invocation is re-run
under `--json`, and the envelope's fields are checked against the human lines
they each map to.

### A new command means new cases

**An example with no case fails the suite.** Adding a command to the reference,
or an example to a page that has one, means adding to `cases::of(…)` in
`crates/bitplane-cli/tests/reference/cases.rs`. A case is the world the example
assumes: which projects are registered, which branch the forge already has,
which worktree was deleted behind bitplane's back.

The page's `## Flags` table is checked too, against `bp <command> --help`, so a
flag that is documented and never wired up fails CI even when no example uses
it.

### Expected-to-fail cases, and unmarking them

A page for a command that is not built yet still gets its cases. They are marked
`Expectation::NotBuilt("…")` and are expected to **disagree** with the binary —
which is what keeps the gap between the documentation and the binary visible,
and countable, rather than absent.

**The ticket that builds a command unmarks its cases.** When the binary starts
agreeing, the suite fails with a message saying so; the fix is to drop the
`NotBuilt` marker and leave `Expectation::Matches`. The same applies to a flag
excused by an `:::in-progress` admonition: when `--help` starts naming it, the
suite fails until the admonition is updated.

`Expectation::Unstageable("…")` is the third and rarest kind: an example no test
process can drive, such as a lock that blocks for two minutes. It is listed
rather than omitted, so the example is accounted for, and the reason is checked
to be a reason.

### What is normalised, and what is not

Values that are volatile by design are rewritten on **both** sides before
comparison, so a rule that would hide a difference has to hide it on the page
too: generated plane ids, dates and times, a detached `HEAD`'s object name,
script durations, ages, the text bitplane quotes from git, and the scratch paths
the fixture lives at — which carry the names the documentation gives them
(`~/planes`, `git@gitlab.com:acme/api.git`, `/Users/alfonz/projects/bitplane`).

Column **widths** are not compared, because a fixture path and a documented path
pad their neighbours differently. Alignment is asserted byte for byte by the
hand-written tests beside the suite; what a reference example is for is the
words.

## Domain vocabulary

`CONTEXT.md` is the glossary and `docs/adr/` holds the decisions. Every ticket,
page and identifier uses those words and no synonyms.

## Cutting a release

Releases are built by [dist](https://opensource.axo.dev/cargo-dist/), configured
entirely in `dist-workspace.toml`. Pushing a tag is the whole procedure:

```sh
git tag v0.1.0
git push origin v0.1.0
```

`.github/workflows/release.yml` then builds all four targets, makes the GitHub
Release, and publishes the npm package.

### release.yml is generated; never edit it

It is `dist generate`'s output. Change `dist-workspace.toml`, then:

```sh
dist generate
```

CI runs `dist generate --check` on every pull request, so an edit to the config
without a regenerate fails there rather than at the tag.

`crates/bitplane-cli/tests/release.rs` covers the rest of the plumbing: that the
targets are ADR-0001's four, that both musl targets are checked for static
linking, that nothing can reach crates.io, and that `website/docs/install.md`
names the files a release actually produces.

### Two things have to be done by hand, once

- **`NPM_TOKEN`** must exist as a repository secret, or the npm publish step
  fails. Create a granular npm access token with read-and-write on packages and
  add it under Settings → Secrets and variables → Actions.
- **GitHub Pages** must be switched to the *GitHub Actions* source, which
  `actions/deploy-pages` cannot do for itself. See `website/README.md`.
