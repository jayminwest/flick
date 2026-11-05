## Architecture

Read `ARCHITECTURE.md` before changing layers, the `Module` trait, events, the store, config
or the control socket. Update it in the same commit when one of those contracts changes.

- New feature = new directory under `src/modules/` plus its registration in
  `src/modules/mod.rs`. Do not add feature code to `src/app.rs`, `src/ui.rs` or `src/core/`.
- `unsafe` and macOS APIs only in `src/platform/`; modules never import each other
  (`scripts/layer-rules.toml`). They meet through item ids and view names.
- Item id keys and released migrations are permanent: existing flick.db files depend on them.
- Old flat config keys map only in `LEGACY` in `src/config.rs`.
- Tests under `src/characterization/` pin user-visible behavior. A changed assertion there is a
  behavior change; say so in the commit.

## Quality gates

`scripts/check-all.sh` runs every gate in order: lint (rustfmt + clippy), check:layers,
check:agents, check:deps, check:size, check:debt, check:privacy, check:binary-size,
check:coverage (the tests), check:ci-parity. CI and the pre-commit hook run only this script. Warnings fail.

- `scripts/check-all.sh --bail` stops at the first failure; `CHECK_ALL_VERBOSE=1` streams output.
- Re-run one gate: `scripts/checks/lint.sh`, `scripts/checks/layers.sh`, and so on.
- One-time setup: `scripts/setup.sh` (points `core.hooksPath` at `scripts/hooks`, lists missing tools).
- Lints: `Cargo.toml` `[lints]`, `clippy.toml`, `rustfmt.toml`. Mark an accepted violation with
  `#[expect(lint, reason = "...")]`, never a bare `allow`.
- Budgets only tighten: `scripts/file-size-budgets.json`, `scripts/coverage-budgets.json`,
  `scripts/binary-size-budget.json`, `scripts/debt-allowlist.json`. Layer rules and their
  allow entries: `scripts/layer-rules.toml`. The scripts never edit these files.
- TODO/FIXME/HACK/XXX must cite a seed (`flick-xxxx`), an issue (`#N`) or a URL.
- Public repo: never write real tailnet names (`tail<hex>.ts.net`), Tailscale IPs, personal machine names,
  ssh users or home paths into code, docs, tests, seeds or mulch. Use placeholders
  (`server.example.ts.net`, 100.64.0.x, `my-macbook-pro`, `me@server`). `check:privacy` enforces it.

<!-- seeds:start -->
## Issue Tracking (Seeds)
<!-- seeds-onboard:v0.5.15 -->
<!-- seeds-onboard-schema:7 -->

This project uses [Seeds](https://github.com/jayminwest/seeds) v0.5.15 for git-native issue tracking.

**At the start of every session**, run:
```
sd prime
```

This injects session context: rules, command reference, and workflows. Pass `--format json|compact|markdown|plain|ids` on any command for agent-friendly output.

**Quick reference:**
- `sd ready` — Find unblocked work
- `sd search <query>` — Full-text search across titles + descriptions
- `sd create --title "..." --type task --priority 2` — Create issue
- `sd update <id> --status in_progress` — Claim work
- `sd close <id>` — Complete work
- `sd dep add <id> <depends-on>` — Add dependency between issues
- `sd sync` — Sync with git (run before pushing)

### Planning
Use `sd plan` when work is large or ambiguous enough that an LLM benefits from structured decomposition. Submit spawns one child seed per step; `step.blocks` uses forward semantics (step i with `blocks: [j]` means step i blocks step j, and step j gets step i's id in its `blockedBy`).

- `sd plan templates` — List built-ins (`feature`, `bug`, `refactor`) plus custom templates
- `sd plan prompt <seed-id>` — Emit a structured prompt the LLM fills in
- `sd plan submit <seed-id> --plan <file>` — Validate + spawn child seeds
- `sd plan show <pl-id>` — View sections, children, sub-plans
- `sd plan edit <id> [--name | --section <name> <text> | --step <i> --title/--priority/--type]` — In-place field edits; bumps revision
- `sd plan outcome <pl-id> --result success|partial|failure` — Record outcome (storage-only)
- `sd plan review <pl-id> --by <name>` — Record reviewer (informational)

### Before You Finish
1. Close completed issues: `sd close <id>`
2. File issues for remaining work: `sd create --title "..."`
3. Sync and push: `sd sync && git push`
<!-- seeds:end -->
