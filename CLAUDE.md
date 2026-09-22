# bitplane

## Agent skills

### Issue tracker

Issues live as markdown files under `.alfonz/issues/<feature>/` (gitignored) in this repo. See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Reference docs

One page per command under `website/docs/reference/`, written ahead of the code. Each carries a status admonition under its `# ` heading: `:::implemented`, `:::in-progress` (body names what is missing) or `:::not-implemented`.

Shipping a feature includes updating its page's admonition.
