---
name: release
description: Prepare LazyDB Beta or stable releases; use when asked to release, cut a version, publish a beta, update CHANGELOG.md, or create a release tag.
---

# LazyDB Release Skill

Use this skill only for an explicit LazyDB release request. The supported
commands are `release beta` and `release stable`.

This skill exists because a release is a coordinated, partly irreversible
operation: the version, changelog, source tree, commit, tag, remote branch,
and GitHub Actions publication must all describe the same release. It keeps
that sequence reproducible while retaining explicit maintainer approval at
the three points where a mistaken choice has a wider impact.

## Safety

- Never revert, stash, force-push, overwrite an existing tag, or rewrite history.
- Stop if unrelated worktree changes are present.
- Fetch tags and inspect the actual diff before recommending a version.
- Do not expose credentials or put them in files.
- GitHub Actions must never create a version or modify the tagged source.

## Interaction Protocol

Treat the maintainer's short replies as state transitions, not as unrelated
questions:

1. `release beta` or `release stable` starts `INSPECT`.
2. After inspection, present one recommended exact version and ask for
   `confirm`, `override VERSION`, or `stop`. Do not edit before confirmation.
3. After changelog/version edits and all checks, present the diff summary and
   proposed commands. Ask for `confirm commit` or `stop`.
4. After the commit and annotated tag are verified, ask separately for
   `confirm push` or `stop`.
5. A bare `confirm` means confirmation of the currently displayed choice only.
   It must never approve a later commit, tag, or push step implicitly.
6. If the reply includes an explicit version, use it as an override only when
   it passes the release validators and does not already exist as a tag.
7. On `stop`, leave all changes already made in place, report the exact state,
   and do not clean up by reverting, stashing, or resetting.

Do not repeat a completed question. Record the confirmed version and completed
state in the running response so the next prompt is deterministic. The only
questions that remain mandatory are version approval, local commit/tag
approval, and remote push approval; tests and inspections should be automatic.

## Procedure

1. Verify `main`, its upstream, a clean release-related worktree, and the
   `yelog/lazydb` remote. Fetch tags without changing user files. Stop before
   editing if unrelated worktree changes are present.
2. Determine the candidate line from actual tag history, then run
   `scripts/release/collect-commits.sh beta VERSION` or
   `scripts/release/collect-commits.sh stable VERSION`. Save its JSON output
   to a temporary file when it is large, and inspect both that data and
   `git diff BASE..HEAD`.
3. Recommend `MAJOR.MINOR.PATCH` using breaking changes, Conventional Commit
   evidence, affected code, and existing tags. For Beta use `VERSION-beta.1`,
   or increment the existing same-line Beta number. Validate the candidate
   before presenting it.
4. Wait for the version transition described above. A confirmed version is
   the single source of truth for all following commands.
5. Generate a dated Keep a Changelog body with Added, Changed, Fixed,
   Security, Deprecated, Removed, or Internal categories as appropriate. Add
   every collected commit exactly once to `### Commits`, including merge,
   revert, documentation, test, and CI commits. Use short SHA links to
   `https://github.com/yelog/lazydb`.
6. Write the body to a temporary file outside the repository and invoke
   `python3 scripts/release/update-changelog.py VERSION BODY_FILE DATE` so the
   section is inserted before `Unreleased`. Use `python3` explicitly because
   the script may not have its executable bit set. Update compare links if the
   repository maintains them.
7. Run `scripts/release/set-version.sh VERSION`, then validate the exact
   heading with `scripts/release/validate-changelog.sh VERSION` and
   `scripts/release/validate-version.sh --pre-tag vVERSION`.
8. Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
   -- -D warnings`, `cargo test --all-targets --all-features`, and a release
   binary `version --json` smoke test. Report each command's actual result;
   never infer success from a partial or concurrent command.
9. Show the complete release diff, the files changed, the baseline and commit
   count, and the exact proposed commands. Wait for `confirm commit` before
   creating commit `chore(release): prepare vVERSION` and annotated tag
   `vVERSION` with message `LazyDB vVERSION`.
10. Verify the commit, tag target, clean worktree, and tag uniqueness. Then
    wait for `confirm push` before running `git push origin main` followed by
    `git push origin vVERSION`. Pushing the tag starts GitHub publication.
11. After pushing, verify local/remote refs and report the URLs of the
    triggered CI and Release workflows when `gh` is available. Do not wait
    for workflow completion unless explicitly requested.

## Changelog baselines

- First Beta: previous published tag, or repository root if no tag exists.
- Later Beta: previous Beta for the same base version.
- Stable: previous stable tag, intentionally including intervening Betas.

Do not silently omit merge or revert commits. If a commit is not user-facing,
put it in `Internal` or the traceable commit list.

## Automation Rules

- Prefer one structured inspection pass and parallel read-only checks where
  possible; serialize commands that share Cargo's package or build locks.
- Use the repository's release scripts as the source of truth instead of
  reproducing version or changelog logic in ad hoc commands.
- Keep temporary release-body and JSON files outside the repository and remove
  them after use.
- Never commit, tag, or push based on a previous confirmation. Each is a
  separate state transition because the consequences differ.
- If a command fails, remain in the current state, report the failure and
  corrective action, and do not advance to the next approval gate.
