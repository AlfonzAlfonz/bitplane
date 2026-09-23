# ADR-0001: bitplane is written in Rust and ships as a static binary

Status: accepted
Date: 2026-09-20
Ticket: `.alfonz/issues/bitplane-architecture/issues/04-language-and-distribution.md`

## Context

The repo was scaffolded as pnpm + ESM, so Node/TS was the default by inertia rather than by decision. Ticket 02 measured the alternatives and killed two of the four arguments that had been in play:

- **Per-operation runtime cost is irrelevant.** Node boots in 28-34 ms against Rust's ~3 ms, on top of an SSH floor of 43 ms (local) to 1.2 s (real internet) that bitplane pays either way.
- **"Node now, compile to a binary later" is not a hedge.** Node SEA measures 142 MB, `deno compile` 67 MB, Bun ~57 MB — every one worse than shipping the 31 MB Node tarball. The reversal does not exist, so this could not be deferred as "decide later."

Ticket 04's grilling then removed the last remaining technical argument *for* Rust as it had been framed. bitplane shells out to git by its own boundary rule (`git protocol yes, forge API no`), and its entire action set is `git worktree add/remove/list --porcelain`, `mkdir`, writing `plane.toml`, and running hooks. Every one of those is a shell command. **A host that cannot run git is useless to bitplane in any language**, so every reachable host already carries git and a POSIX shell — which means the remote artefact can be a shell script and the 15x bootstrap difference (31 MB Node vs ~7.9 MB static agent) buys nothing.

That left the decision to be made on grounds other than the ones the ticket was written around.

## Decision

### Language

**Rust.** The rationale of record, stated honestly:

1. **Runtime-independent single-file install.** The audience is developers who use git worktrees, not developers who have Node. One file with no runtime is materially easier for them than `npm i -g`.
2. **No dependency tree at the install site.** Rust dependencies compile in; they do not materialise as `node_modules`.
3. **It preserves binary injection as a future remote option at zero cost.** With a 7.9 MB static musl binary in hand, pushing a real binary to a remote host is available later for free. Node foreclosed this permanently (ticket 02). The shell-script servant becomes a starting point rather than a constraint.
4. **A stated goal of more Rust practice.** This is a solo project; recording the motivation is more honest than laundering it through the technical arguments.

Two arguments were considered and **rejected as inaccurate**, and are recorded here so they do not reappear:

- *"Rust is more portable."* Backwards. Node is more portable per artefact — one `npm i -g` covers macOS, Linux and Windows. Rust needs a cross-compilation matrix. What Rust buys is runtime independence, which is argument 1, not portability.
- *"Node's stdlib is missing important tools."* Not for bitplane's needs. Neither Node nor Rust `std` ships TOML parsing, argument parsing, or file locking; both take third-party code. Rust's edge is how that code is delivered, which is argument 2.

### Structure

A Cargo workspace:

- **`bitplane-core`** — the `Engine` trait, domain types, serde. The machine contract.
- **`bitplane-cli`** — renders the contract for humans.

This is the structural expression of the already-settled "machine contract first; the human CLI renders it," and an eventual MCP surface becomes a second binary against the same core. The pnpm/ESM skeleton is deleted.

### Concurrency model

**Sync. `std::thread` for per-project fan-out. No tokio.**

The entire workload is spawn-subprocess-and-wait plus small TOML reads and writes; there is no IO multiplexing problem, and a plane holds perhaps 2-6 projects. Async would colour every caller of `Engine` and buy nothing. This binds `Engine`'s signature — ticket 06 cites this rather than reopening it.

### Targets

| target | |
| --- | --- |
| `aarch64-apple-darwin` | |
| `x86_64-apple-darwin` | |
| `x86_64-unknown-linux-musl` | static |
| `aarch64-unknown-linux-musl` | static |

**No native Windows.** The hook model runs user shell commands via `sh -c`, and path derivation, plane layout and locking all assume POSIX. Supporting Windows means a second code path for every hook invocation, forever. WSL2 covers Windows developers and is where their worktrees want to live anyway.

Windows is deferred, not rejected, and the deferral binds exactly one standing constraint: **paths are `PathBuf`, never strings, never hardcoded `/`.** That is nearly free if adopted now and expensive to reclaim later. It explicitly does **not** cover hook invocation, file-locking semantics, symlinks, permissions, or case-sensitivity — all of which stay openly POSIX-only.

### Linking

**Every target links statically. No runtime dynamic library dependencies.** Vendored C compiled into the binary is permitted, but each instance is a cross-compilation cost (a C cross-toolchain for `aarch64-unknown-linux-musl`) to be justified at the point of use. Pure-Rust crates remain the default preference.

Forbidden: `libgit2` against a system libssl, non-vendored `openssl-sys`, anything requiring a runtime `.so`.

This **replaces** the map's earlier "no native dependencies" line, which meant something different under Node. Ticket 05 keeps `rusqlite` with the `bundled` feature as a live option — this ADR does not pre-decide the store.

### Distribution

Built with **`dist`** (formerly cargo-dist), which generates the whole channel set from one config:

- **GitHub Releases** — per-target archives
- **`curl | sh`** installer
- **npm wrapper** with per-platform `optionalDependencies` (the esbuild / swc / Biome pattern — shipping in Rust does not cost the npm channel)
- **Homebrew tap** — added once there are users, not before

**Not crates.io.** Accepted consequence: the `bitplane` name stays first-come there and someone else may take it. `cargo install --git` remains available to anyone who wants it.

The toolchain is pinned with `rust-toolchain.toml`. No MSRV is claimed until someone asks for one.

### Binary name

The command is **`bp`**. The crate, repo and package are `bitplane`. This is coherent with the glossary's reserved `bp-` plane-id prefix: `bp create` yields `bp-a3f9c2e1`. Accepted risk: `bp` is two letters and unnamespaced, so a future collision with another tool is possible.

### Runtime dependency

The "single self-contained file" story has one asterisk: **bitplane requires git on `PATH`**, and not just any git.

**Minimum: git 2.36.** Verified against git's release notes:

- `prunable` in `git worktree list --porcelain` — adopted from ticket 01 as the orphan oracle — landed in **2.31.0**.
- `git worktree list --porcelain -z`, the NUL-terminated form and the only one that handles paths and lock reasons containing unsafe bytes correctly, landed in **2.36.0**. Since project sources may be arbitrary local paths, bitplane uses `-z`, so 2.36 is the real floor.
- `git worktree lock`, ticket 01's liveness mutex, is ancient (2.7) and does not constrain.

The version is checked once on first git invocation and fails with a clear message naming the requirement. No runtime capability detection — degrading around a missing orphan oracle means two code paths for a situation nobody is in.

### Remote artefact

The remote servant is a **POSIX shell script**, streamed over the already-open SSH session's stdin using DevPod's `pkg/inject/inject.sh` pattern (ping/pong handshake, `noexec` check), which works identically over `docker exec`. Nothing is bootstrapped; the script drives the git already on the host.

**Binary injection is preserved as a future option at no cost** — see rationale 3 — but is not built.

This scopes the host universe: bitplane targets **hosts you control** — your own dev machines and devcontainers — not arbitrary unprovisioned containers. A scratch `alpine:latest` has no git, no writable planes directory, and none of the commands a project's hooks invoke.

### Licence

**`MIT`.** The repo goes public.

MIT matches the Rust ecosystem's overwhelmingly `MIT OR Apache-2.0` dependency tree, so no dependency's licence can conflict with ours, and it places no conditions on anyone driving `bp` or embedding `bitplane-core`.

_Amended: originally `GPL-2.0-or-later`. Relicensed to MIT while still sole copyright holder, before outside contributions made relicensing non-unilateral._

## Consequences

- Ticket 05's "no native dependencies" premise is superseded by the linking constraint above; `rusqlite` `bundled` is back on the table for the store.
- Ticket 06 inherits a sync `Engine` trait and serde-serializable request/response types.
- The map's "Remote servant protocol" entry is no longer blocked on this decision. What remains open there: the wire format, and the shell dialect (strict POSIX `sh` versus bash — Alpine-based devcontainers ship no bash).
- Every later ticket inherits: paths are `PathBuf`; static linking; no runtime `.so`; POSIX-only behaviour.
- CI must cross-compile four targets, including two musl.

## Alternatives considered

**Node/TypeScript.** Rejected. It has the better per-artefact portability story and the faster start for this author, but it foreclosed binary injection permanently (ticket 02), requires a runtime at the install site for an audience that is not JS-specific, and brings a dependency tree.

**Go.** Considered and set aside at the user's direction. Every static-agent precedent in ticket 02 — Mutagen's 7.9 MB, DevPod's injector — is Go, and it is the cheapest route to a small static cross-compiled binary. It would have been the stronger choice had the decision turned on the compiled-binary requirement; it did not, since Q1 removed that requirement entirely and the remaining reasons include an explicit preference.

**Hybrid: TS CLI plus a small static servant binary.** Not pursued. It implements the engine contract twice, which is the wrong trade for a solo project.
