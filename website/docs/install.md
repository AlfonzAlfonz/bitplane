---
title: Installation
---

# Installation

:::in-progress

Every channel below is **built and wired up**, and **no release has been tagged
yet**, so every one of them is still empty. What is missing is a tag, not
plumbing: pushing one runs the release, and each channel fills in.

Until then, [building from source](#building-from-source) is the only way to get
a `bp` on your machine.

:::

`bp` is one statically linked binary with no runtime and no dependency tree.
Whichever channel you use, what lands is a single file you could equally well
have copied there yourself.

## Before anything: git 2.36

bitplane's one runtime dependency is **git 2.36 or newer**, on `PATH`.

```sh
git --version
```

The floor is not arbitrary. bitplane reads git's worktree list with
`git worktree list --porcelain -z`, the NUL-terminated form and the only one
that survives a repository path or a lock reason containing a newline or a
quote. That flag landed in git 2.36.0.

**Every command checks this once, before it touches anything**, so an unusable
git never leaves a plane half-built. What you get instead is
[exit code `4`](./reference/exit-codes.md#4-is-checked-once-before-anything-is-touched)
and a sentence naming the requirement:

```
error[git_too_old]: git 2.35.9 is too old; bitplane requires git 2.36 or newer

remedy: Upgrade git to 2.36 or newer.
```

There are four spellings of this, depending on whether git is missing, present
but unrunnable, present but mute about its version, or simply old. All four exit
`4` and all four name the same requirement — the
[exit codes page](./reference/exit-codes.md#4-is-checked-once-before-anything-is-touched)
lists them verbatim.

There is **no capability detection and no degraded mode**. Working around a
missing `-z` would mean two code paths for a situation nobody is in.

## Platforms

| platform | archive | supported |
| --- | --- | --- |
| macOS, Apple silicon | `aarch64-apple-darwin` | yes |
| macOS, Intel | `x86_64-apple-darwin` | yes |
| Linux x86-64 | `x86_64-unknown-linux-musl` | yes, statically linked |
| Linux arm64 | `aarch64-unknown-linux-musl` | yes, statically linked |
| Windows | — | no — use WSL2 |

The middle column is the name in the middle of the archive on the release, for
when you are picking one by hand.

Both Linux builds are musl-linked and depend on no shared library, so they run
on a distribution older than the one they were built on and inside a
`FROM scratch` container. That is checked on every pull request rather than
assumed: CI reads each built binary back and fails if it asks for a shared
library or a dynamic loader.

**Windows is not supported natively, and is deferred rather than rejected.**
Scripts run through `sh -c`, and path derivation, plane layout and file locking
all assume POSIX. WSL2 works, and is where your worktrees want to live anyway.

## Channels

### Install script

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/AlfonzAlfonz/bitplane/releases/latest/download/bitplane-cli-installer.sh | sh
```

Detects your platform, downloads that archive from the GitHub release, and puts
`bp` in `~/.local/bin`, adding it to your `PATH` if it is not there already.

The script is named after the crate that produces the binary, `bitplane-cli`,
rather than after the project. That is the only place the workspace's internal
split is visible from outside.

### GitHub Releases

One `.tar.xz` per platform on
[the releases page](https://github.com/AlfonzAlfonz/bitplane/releases), each
next to its own `.sha256`. Unpack it and move `bp` onto your `PATH`. This is the channel with no
magic in it, and the one to use when you want to see what you are installing
before you install it.

### npm

```sh
npm install -g bitplane
```

The npm package is a thin wrapper. Installing it runs a `postinstall` script
that works out which platform you are on and downloads that one archive from the
GitHub release — the same archive the other channels ship. Nothing is compiled,
and the only JavaScript involved is the handful of lines that do the download.

Shipping in Rust does not cost the npm channel, which is the point. Use it if
npm is how your team installs tools; there is no other reason to prefer it.

**Two caveats, because the download happens at install time rather than at
resolve time.** `npm install --ignore-scripts` gets you the wrapper and no
binary, and an offline or mirrored registry does not help, because the binary
does not come from the registry. If either applies to you, use the install
script or the archive.

### Homebrew

**Not set up**, deliberately, and not as an oversight of the release plumbing. A
tap is a thing that has to be kept alive for as long as anyone uses it, and it
gets made once there are users asking for it.

### Not crates.io

bitplane is **not published to crates.io**, so `cargo install bitplane` will not
get you this tool. The accepted consequence is that the name stays first-come
there and somebody else may take it.

`cargo install --git` works and is supported:

```sh
cargo install --git https://github.com/AlfonzAlfonz/bitplane bitplane-cli
```

## Building from source

```sh
git clone https://github.com/AlfonzAlfonz/bitplane
cd bitplane
cargo build --release
```

The binary lands at `target/release/bp`. The toolchain is pinned in
`rust-toolchain.toml` and rustup will honour it without being asked. **No MSRV
is claimed** — the pin is what the project is developed and released with, not a
floor anyone has promised to hold.

## Check it worked

```sh
bp --version
bp list
```

`bp list` on a machine that has never run bitplane prints nothing and exits `0`.
That is the correct answer: you have no planes yet.

## Where it puts things

Two directories, and they are deliberately in different places.

| directory | default | what is in it |
| --- | --- | --- |
| planes | `~/planes` | your worktrees — the directories you `cd` into daily |
| projects | `$XDG_DATA_HOME/bitplane/projects` | one directory per registered project, holding its repository |

The planes directory is **not** hidden and **not** under XDG, because it holds
your actual working trees. `cd ~/.local/share/bitplane/planes/bp-a3f9c2e1` is
hostile; `cd ~/planes/auth-work` is not. The projects directory is the opposite
case — it holds application-owned data you never need to open — so it goes where
such data goes. `XDG_DATA_HOME` falls back to `~/.local/share` on macOS as well
as Linux.

Both are configurable, by flag, by environment variable or by a config file, in
that precedence. That is documented once, on
[Global flags and directories](./reference/global-flags.md#directory-resolution).

There is no third location. bitplane writes a config file only if you write one,
keeps no database, no index and no cache, and leaves exactly one kind of mark
outside those two directories: git's own administrative record of a worktree,
inside repositories it did not create.
[`bp doctor --footprint`](./reference/plane/doctor.md#--footprint) lists all of
it, which is also the uninstall answer.

## Uninstalling

Remove the binary the way your channel installed it — `npm uninstall -g
bitplane`, or delete the file. The install script leaves two more things of its
own: `~/.config/bitplane-cli/`, holding its receipt and the `env` scripts that
put `~/.local/bin` on your `PATH`, and the line sourcing them in your shell's rc
file. Neither belongs to `bp` and neither is listed by `--footprint` below.

Then, if you want the data gone too:

```sh
bp doctor --footprint     # see everything bp owns, before deleting any of it
```

Destroy your planes with [`bp destroy`](./reference/plane/destroy.md) rather
than deleting `~/planes` by hand: destroying is what removes git's record of
each worktree from the repository it came from, and what deletes the branches in
projects bitplane owns. Deleting the directory instead leaves those records
behind, and git will keep insisting those branches are checked out somewhere.

## Next

- [Run a task across several repos](./guides/run-a-task-across-repos.md) — from
  nothing to a working plane.
- [The plane](./concepts/plane.md) — what you just installed a manager for.
