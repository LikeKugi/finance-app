---
name: update-version
description: >
  Release helper for FinanceApp. Use when the user invokes /update-version next
  or /update-version vMAJOR.MINOR.PATCH to bump the app version, refresh Cargo.lock,
  build the release binary, write release notes from Git history, and commit the release.
allowed-tools: Bash(git:*) Bash(cargo:*) Bash(python3:*) Read Write Edit Glob Grep
---

# Update version

## Trigger and version rules

- `/update-version next` (or no argument) reads `[package].version` from
  `crates/app/Cargo.toml` and increments only the patch component:
  `0.0.6` becomes `v0.0.7`.
- `/update-version vMAJOR.MINOR.PATCH` is accepted only when the full argument
  matches `v<integer>.<integer>.<integer>` (for example `v1.2.3`). Components
  must be non-negative integers without leading zeroes.
- Any other argument (missing `v`, non-integer component, extra text, etc.)
  is invalid: do not ask the user, silently fall back to `next`, and mention
  the fallback in the final report.
- The `v` prefix is used for the tag, commit message, and note file name.
  `crates/app/Cargo.toml` and `Cargo.lock` use bare SemVer (`1.2.3`).

## Procedure

1. Build the deterministic context. Run from the repository root:

   ```bash
   python3 .agents/skills/update-version/scripts/version_info.py "<argument>" --repo .
   ```

   Pass `next` when the user supplied no argument. The helper is read-only.
   Use its JSON fields: `current_version`, `manifest_version`, `target_version`,
   `manifest_version_line`, `note_path`, `range`, `commits`, `today`, and
   `fallback_reason`. If it exits non-zero, stop and report the error.

2. Check preconditions before changing anything:

   ```bash
   git status --porcelain -- crates/app/Cargo.toml Cargo.lock docs/releases/v<new>.md
   ```

   The output must be empty. If the target note already exists or any target
   file has local changes, stop: never overwrite user work. Unrelated dirty
   files are allowed but must never be staged or committed by this skill.

3. Update the app manifest only. Replace the `[package]` version line —
   `version = "<current_version>"` — with
   `version = "<manifest_version>"` in `crates/app/Cargo.toml`.
   Do not change workspace dependencies or any other version fields.

4. Refresh the lock file and build the release:

   ```bash
   cargo update -w
   cargo +stable build -p finance-app --release --locked
   ```

   After `cargo update -w`, confirm that the `finance-app` entry in
   `Cargo.lock` shows `<manifest_version>`. If either command fails, stop
   before committing, leave the files for inspection, and report the exact
   failing command. Do not claim success.

5. Write the patch note at the `note_path` from step 1. Inspect the real
   history in the helper's `range`:

   ```bash
   git log --stat --format='%h %s%n%b' <range>
   ```

   Use only that history — plus observed file changes when needed — to write
   an accurate Russian note. Do not invent features, migrations, checks, or
   compatibility claims. Follow the style of `docs/releases/v0.0.6.md` and
   include:

   - `# FinanceApp v<new> — <short Russian title>`
   - `**Дата:**` from `today` in Russian date format
   - `**Тег для публикации:** \`v<new>\` (совпадает с версией в ...)`
   - `**Диапазон изменений:** \`<range>\``
   - `## Что нового` with grouped bullets derived from commit subjects/diffs
   - `## Проверки` listing the actually executed build command and `OK`
   - final line: `Полная история изменений: \`<range>\` (N коммитов).`

   If there are no commits in the range, say so explicitly; do not fabricate
   changes. Keep all text in Russian except commands, paths, and identifiers.

6. Commit only the release files:

   ```bash
   git add -- crates/app/Cargo.toml Cargo.lock docs/releases/v<new>.md
   git commit -m "feat: update version to v<new>"
   ```

   Verify with `git show --stat --oneline HEAD`. The commit must contain only
   the manifest, lock file, and new patch note.

## Failure and reporting

- Never commit after a failed update, lock refresh, build, or note generation.
- Never stage unrelated files and never use destructive Git commands.
- Final report must state: old version, new version, whether `next` fallback
  happened, build result, note path, commit hash, and any remaining limitation.
