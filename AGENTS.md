## Quality gates

`scripts/check-all.sh` runs every gate in order: lint (rustfmt + clippy), check:layers,
check:agents, check:deps, check:size, check:debt, check:binary-size, check:coverage
(the tests), check:ci-parity. CI and the pre-commit hook run only this script. Warnings fail.

- `scripts/check-all.sh --bail` stops at the first failure; `CHECK_ALL_VERBOSE=1` streams output.
- Re-run one gate: `scripts/checks/lint.sh`, `scripts/checks/layers.sh`, and so on.
- One-time setup: `scripts/setup.sh` (points `core.hooksPath` at `scripts/hooks`, lists missing tools).
- Lints: `Cargo.toml` `[lints]`, `clippy.toml`, `rustfmt.toml`. Mark an accepted violation with
  `#[expect(lint, reason = "...")]`, never a bare `allow`.
- Budgets only tighten: `scripts/file-size-budgets.json`, `scripts/coverage-budgets.json`,
  `scripts/binary-size-budget.json`, `scripts/debt-allowlist.json`. Layer rules and their
  allow entries: `scripts/layer-rules.toml`. The scripts never edit these files.
- TODO/FIXME/HACK/XXX must cite a seed (`flick-xxxx`), an issue (`#N`) or a URL.

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

<!-- mulch:start -->
## Project Expertise (Mulch)
<!-- mulch-onboard:v0.11.0 -->

This project uses [Mulch](https://github.com/jayminwest/mulch) v0.11.0 for structured expertise management.

**At the start of every session**, run:
```bash
ml prime
```

Prints a budget-capped index of project expertise: one line per record (id, type, summary,
anchors), failures first. Run `ml show <id>` for a full record and `ml prime --files src/foo.ts`
before editing a file to load only records relevant to that path.

**Record only what a future agent would get wrong without it.** Prefer `failure` records
(symptom -> root cause -> preventive check); anchor conventions and patterns with `--files` or
`--dir-anchor`. `ml record` blocks exact duplicates and flags near-duplicates of existing
records (suggesting `--supersedes <id>`; `--force` records anyway):
```bash
ml record <domain> --type <convention|pattern|failure|decision|reference|guide> --description "..."
```

Evidence auto-populates from git (current commit + changed files). Link explicitly with
`--evidence-seeds <id>` / `--evidence-gh <id>` / `--evidence-linear <id>` / `--evidence-bead <id>`,
`--evidence-commit <sha>`, or `--relates-to <mx-id>`. Upserts of named records merge outcomes
instead of replacing them; validation failures print a copy-paste retry hint with missing fields
pre-filled.

Run `ml status` for domain health, `ml doctor` to check record integrity (add `--fix` to strip
broken file anchors), `ml --help` for the full command list. Write commands use file locking and
atomic writes, so multiple agents can record concurrently. Expertise survives `git worktree`
cleanup — `.mulch/` resolves to the main repo.

`ml prune` soft-archives stale records to `.mulch/archive/` instead of deleting them; pass
`--hard` for true deletion. Restore an archived record with `ml restore <id>`. Do not read
`.mulch/archive/` directly — those records are stale by definition. If you need historical
context, run `ml search --archived <query>`.

### Before You Finish

Record only what a future agent would get wrong without it. Prefer a `failure` record
(symptom -> root cause -> preventive check). Do not restate code, CLAUDE.md, or existing
records; if an existing record helped, confirm it instead:

```bash
ml learn                                                                    # see what files changed
ml record <domain> --type <convention|pattern|failure|decision|reference|guide> --description "..."
ml outcome <domain> <id> --status success                                   # confirm a record that helped
ml sync                                                                     # validate, stage, commit
```

Skip if nothing qualifies. Filler records are noise every future session pays to read.
<!-- mulch:end -->
