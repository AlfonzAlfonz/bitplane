# The bitplane documentation site

The site published at <https://alfonzalfonz.github.io/bitplane/>. Built with
[Docusaurus](https://docusaurus.io/), deployed to GitHub Pages by
`.github/workflows/docs.yml` on every merge to `main`.

## Why there is a Node toolchain in a Rust repo

ADR-0001 decides what the **product** is written in and how it ships: Rust, one
static binary, no runtime at the install site. It does not decide what the
project's development tooling is written in, and nothing in it is violated by a
static site generator that runs on a maintainer's laptop and in CI.

Concretely, nothing here reaches the user:

- The site is **not** in the Cargo workspace, and `cargo build` never sees it.
- Nothing in this directory is linked into, embedded in, or shipped with `bp`.
  The artefact is a directory of HTML.
- A contributor working on the Rust crates never installs Node. A contributor
  working on the docs never installs Rust.

Docusaurus was picked over a Rust-based generator (mdBook, Zola) because the
shape these docs are heading for — a per-command reference sitting beside
narrative guides, versioned once there are releases to version — is what it does
without being made to, and because the cost of it being Node is paid by whoever
edits the docs rather than by anyone who installs `bp`. A preference for one
language across a whole repo is not worth rebuilding docs infrastructure over.

## Working on the site

```sh
pnpm install
pnpm start          # dev server with hot reload
pnpm build          # production build; this is what CI runs
pnpm serve          # serve the production build locally
```

`pnpm build` is the test. Docusaurus is configured to **throw** on a broken
internal link, a broken anchor and a broken markdown link, so a page that points
at something that does not exist fails the build and therefore fails CI.

Pages are MDX. The v4 future flags are on, which means MDX 1 compatibility is
off: an admonition takes its title as a directive label, `:::note[Title]`, and
the older `:::note Title` form renders as literal text without warning.

## Deployment

`.github/workflows/docs.yml` builds the site on every pull request that touches
it, and publishes to GitHub Pages when the change lands on `main`. It deploys
through `actions/deploy-pages`, which requires the repository's **Settings →
Pages → Source** to be set to **GitHub Actions** once, by hand.

The published URL is assembled in `docusaurus.config.js` from the `owner` and
`repo` constants at the top of the file. Those two lines are the only thing to
change if the repository moves.

## Layout

- `docs/` — every page.
- `docusaurus.config.js` — site config, including the GitHub Pages URL.
- `sidebars.js` — the navigation, written out by hand.
- `src/css/custom.css` — the only styling override.
- `pnpm-workspace.yaml` — answers pnpm's build-script prompt, nothing else.
