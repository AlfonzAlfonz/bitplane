# bitplane

A change rarely stops at one repository. The API needs a field, the web client
needs to read it, the docs need to say so — three repos, three branches, three
working trees to make, keep straight and remember to clean up. bitplane makes
that set a single thing you can create and tear down with one command.

Full documentation: <https://alfonzalfonz.github.io/bitplane/>

## What is a plane

A **plane** is a named set of git worktrees, one per repository, created
together, torn down together, living in one directory you can `cd` into.

```
~/planes/auth-work/
  acme/api      on feat-login
  acme/web      on feat-login
  acme/docs     on feat-login
```

`bp status` shows all members at once, and `bp destroy` removes them all —
refusing, member by member, if any still holds uncommitted or unpushed work.

Between creating and removing a worktree, bitplane stays out of your way: it
never commits, pushes, stashes, checks out or resets. It speaks git, not GitHub,
and uses your existing git credentials.

## Installation

Requires **git 2.36 or newer** on `PATH`. Supported on macOS and Linux;
on Windows, use WSL2.

### Prebuilt binaries

TODO

### Build from source

```sh
git clone https://github.com/AlfonzAlfonz/bitplane
cd bitplane
cargo build --release
```

The binary lands at `target/release/bp`; copy it anywhere on your `PATH`. The
Rust toolchain is pinned in `rust-toolchain.toml`, and rustup picks it up
automatically.

Alternatively, install straight from the repository:

```sh
cargo install --git https://github.com/AlfonzAlfonz/bitplane bitplane-cli
```

Check it worked:

```sh
bp --version
```

## License

GPL-2.0-or-later
