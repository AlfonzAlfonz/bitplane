---
title: bp doctor
---

# `bp doctor`

Find what nothing else can see.

```
bp doctor [--sweep-source-repos] [--footprint]
```

[`bp list`](./list.md) reads the planes directory, so it structurally cannot see
a worktree belonging to a plane whose file was never written, or a branch left
behind by someone who switched away from it. `doctor` sweeps the source repos
and both data directories and reports everything `bp` has a claim on.

**It reports and never repairs.** Nothing is deleted, rewritten or fixed, and
every finding names the command that fixes it. It takes no locks.

## Arguments

None.

## Flags

| flag | default | what it does |
| --- | --- | --- |
| `--sweep-source-repos` | off | Ask each source repo what worktrees it thinks it has. One git invocation per repo. |
| `--footprint` | off | List every path `bp` owns, and every mark it has left outside them. |

Plus the [global flags](../global-flags.md#global-flags).

The source-repo sweep is **off by default** because it is `O(projects)` git
invocations. Everything else `doctor` does is filesystem-only.

## The four sweeps

| sweep | what it finds | needs `--sweep-source-repos` |
| --- | --- | --- |
| plane directories holding `.bitplane/` but no `plane.toml` | the window between the `mkdir` that claims a plane and the first write into it | no |
| project directories holding `repo.git` but no `project.toml` | a `bp project add` that was interrupted. The objects were kept; a retry reuses them | no |
| worktrees the source repos know about that no plane names | orphans left by an interrupted `create` whose file never landed | yes |
| source-repo branches with no worktree and no plane | a branch someone switched away from before `bp destroy` ran | yes |

The source-repo sweep extends to **ad-hoc members'** repos, whose paths come
from the plane files `doctor` already reads. Without that they would be
invisible to every check in the tool.

The fourth sweep is the named leak in the design. `bp destroy` deletes the
branch a worktree is **actually on**, so a branch you switched away from is left
behind with nothing recording it. `doctor` lists it. `bp` never deletes it on
its own judgement.

## Output

```
$ bp doctor
```
```
planes    ~/planes
projects  ~/.local/share/bitplane/projects

findings
  ~/planes/bp-7c1e0d44
    claimed 3 days ago; no plane.toml was ever written
    nothing in it is yours; remove the directory

  ~/.local/share/bitplane/projects/codestyle
    holds repo.git but no project.toml, so it is not a project
    bp project add git@gitlab.com:acme/codestyle.git will reuse the objects

2 findings
```

## Examples

### Nothing is wrong

Exit `0`.

```
$ bp doctor
```
```
planes    ~/planes
projects  ~/.local/share/bitplane/projects

no findings
```

### A latched plane

Exit `3`. Reported as **create never completed, with how long ago it started**,
not merely as broken — which is the difference between confidently discarding a
remnant and wondering whether something is still running.

```
$ bp doctor
```
```
findings
  auth-work
    create never completed, started 3 days ago
    nothing in it is yours; bp destroy -p auth-work clears it

1 finding
```

### An orphaned branch

Exit `3`. Needs the sweep.

```
$ bp doctor --sweep-source-repos
```
```
findings
  @api  refs/heads/feat-a
    no worktree and no plane holds this branch
    git -C ~/.local/share/bitplane/projects/api/repo.git branch -D feat-a removes it

1 finding
```

`bp` prints the git command rather than running it. Deciding a branch is
disposable is a judgement about your work, and that is not `bp`'s to make.

### `--footprint`

Exit `0`. This is the uninstall answer: everything `bp` owns, plus the only
marks it leaves outside its own directories.

```
$ bp doctor --footprint
```
```
owned
  ~/planes
  ~/.local/share/bitplane/projects
  ~/.config/bitplane/config.toml

written into repos bp does not own
  ~/projects/bitplane/.git/worktrees/projects-bitplane
```

Those administrative entries are git's record of a worktree in an **adopted** or
**ad-hoc** repo. Destroying the plane that owns each one removes it.

## Exit codes

| code | when |
| --- | --- |
| `0` | nothing to report |
| `2` | bad arguments |
| `3` | at least one finding |
| `4` | git is missing, unusable or older than 2.36 |

`doctor` never exits `1`. It is a read: there is no operation for it to fail at.

## See also

- [`bp repair`](./repair.md) — the write side, for one plane
- [`bp list`](./list.md) — the findings a plain read can reach
