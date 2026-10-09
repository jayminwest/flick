---
name: release
description: Prepare, cut, and verify a Flick release: tracker audits, version bump, CHANGELOG curation, gates, tag, push, GitHub release with the signed app zip, then verify CI and the published release.
---

# Release

Prepare a Flick release, publish it, and verify it shipped. Flick has no release
workflow: CI (`.github/workflows/ci.yml`) only runs `scripts/check-all.sh`. The
release is built and published from this Mac, because only this Mac has the signing
identity `scripts/bundle.sh` uses. A release exists when `gh release view vX.Y.Z`
shows it as Latest with `Flick-X.Y.Z-arm64.zip` attached, and CI is green on the
tagged commit.

Invoking this command is the go-ahead to push `main`, push the tag and publish the
GitHub release. Stop and ask before any step this file does not describe.

## 1. Pre-flight

- Working tree is clean, on `main`, and not behind origin (`git status`,
  `git fetch && git status -sb`). Agent work is committed locally and is often
  ahead of origin; that is expected. Never pull or rebase over it without asking.
- No stray agent worktrees: `git worktree list` shows only the main checkout.
- No open PRs that should ride along: `gh pr list --state open`.
- Find the last release tag: `git describe --tags --abbrev=0`.

## 2. Tracker audits (two parallel subagents)

Spawn both in one message so they run concurrently. Neither pushes: their commits
ride out with the single push in step 9.

- **Seeds audit**: a subagent that invokes the `seeds-issue-audit` skill and
  follows it exactly. It auto-closes only HIGH-confidence issues with citable
  evidence, runs `sd sync` after closing, reports borderline cases and never
  pushes. It returns its two tables (auto-closed and borderline).
- **GitHub issue sweep**: a subagent that lists open issues
  (`gh issue list --state open`) and checks each against HEAD. It closes an issue
  only when the fix is verifiably on `main`, naming the commit in the comment
  (`gh issue close <n> --comment "..."`). Anything uncertain goes in a borderline
  table, untouched. Staleness alone never closes anything.

Hold both reports. The borderline tables go in the final summary (step 11).

## 3. Review changes since the last tag

- `git log <tag>..HEAD --oneline --no-merges` and `git diff <tag>..HEAD --stat`.
- Ignore tracker bookkeeping (`chore:` commits touching only `.seeds/`/`.mulch/`).
- Pick the bump level. **Always patch unless the user explicitly asks for minor or
  major.**

## 4. Bump the version

The version lives in three places. Update all of them:

- `Cargo.toml` `version = "X.Y.Z"`.
- `Cargo.lock`: run `cargo update --workspace --offline`, which rewrites only
  Flick's own entry.
- `README.md` `## Status` line: ``Early (`X.Y.Z`)``.

Do not touch version strings in tests or in ARCHITECTURE.md examples; they are not
release sites. `scripts/bundle.sh` reads the version from `Cargo.toml` and stamps
`X.Y.Z+<sha>` into `CFBundleVersion`.

## 5. Curate the CHANGELOG

`CHANGELOG.md` keeps an `## Unreleased` section that agents append to. Turn it into
the release:

- Rename it to `## X.Y.Z — YYYY-MM-DD` (today, em dash, matching earlier entries)
  and add a fresh empty `## Unreleased` above it.
- Fill gaps from the step 3 log: every user-visible feature, behavior change or fix
  since the last tag gets one line. Group by area (launcher, windows, activity,
  tasks, herdr, capture, remote, ...) with **bold** area leads when the list is
  long.
- Name config keys and commands exactly (`[activity] urls`, `flick --host`).
  Call out breaking changes and anything that needs a new macOS permission
  (Accessibility, Automation, Screen Recording) on its own line.
- Write for the person reading the GitHub release, not for the git log. No seed
  ids, no internal module names.

## 6. Doc pass

The gates keep AGENTS.md honest (`check:agents`), so this pass is prose only:

- `README.md`: features, configuration examples and install steps match what
  ships. Never add line counts or binary sizes to the README (user request).
- `ARCHITECTURE.md` and `AGENTS.md` only if layers, contracts or commands changed
  and the change missed them.

## 7. Run the gates

```bash
scripts/check-all.sh
```

CI runs this same script. A release must not tag a commit CI will reject.

## 8. Commit and tag

- `sd sync` and `ml sync`, so the tracker state is committed.
- Commit the version sites, CHANGELOG and docs:
  `git commit -m "chore(release): X.Y.Z"`. Let the pre-commit hook run.
- Tag the release commit: `git tag -a vX.Y.Z -m "Flick X.Y.Z"`.

## 9. Build the release artifact

Build from the tagged commit with a clean tree, so the stamp is not `-dirty`:

```bash
scripts/bundle.sh
/usr/libexec/PlistBuddy -c 'Print CFBundleVersion' target/Flick.app/Contents/Info.plist
ditto -c -k --keepParent target/Flick.app target/Flick-X.Y.Z-arm64.zip
```

The printed version must be `X.Y.Z+<short sha of the tag>` with no `-dirty`.
bundle.sh prints the signing identity; if it fell back to ad-hoc (`-`), stop and
tell the user. Use `ditto`, not `zip`, so the code signature survives.

## 10. Push and publish

```bash
git push origin main
git push origin vX.Y.Z
```

Write the release notes to a temp file: the curated CHANGELOG section, with
the area leads in bold, followed by the `### Install` block from the previous
release (`gh release view <last tag> --json body -q .body`), with the version in
the zip name updated. Then:

```bash
gh release create vX.Y.Z target/Flick-X.Y.Z-arm64.zip \
  --title "Flick X.Y.Z" --notes-file <notes> --latest
```

## 11. Verify

- CI on the release commit: `gh run list --workflow=ci.yml --limit 1`, then
  `gh run watch <run-id>`. If it fails, say so plainly. Do not delete the tag or
  the release without the user's say.
- `gh release view vX.Y.Z` shows it as Latest, not a draft, with the zip
  attached.
- Ask whether to install the release locally (`scripts/bundle.sh --install`).
  Never install without asking: it restarts the running Flick.

Finish with a summary: version, highlights, the release URL, the CI run, and the
two borderline tables from step 2 for human review.
