---
name: release
description: Prepare LazyDB Beta or stable releases; use when asked to release, cut a version, publish a beta, update CHANGELOG.md, or create a release tag.
---

# LazyDB Release Skill

Use this skill only for an explicit LazyDB release request. The supported
commands are `release beta` and `release stable`.

## Safety

- Never revert, stash, force-push, overwrite an existing tag, or rewrite history.
- Stop if unrelated worktree changes are present.
- Fetch tags and inspect the actual diff before recommending a version.
- Do not expose credentials or put them in files.
- GitHub Actions must never create a version or modify the tagged source.

## Procedure

1. Verify `main`, its upstream, a clean release-related worktree, and the
   `yelog/lazydb` remote. Fetch tags without changing user files.
2. Run `scripts/release/collect-commits.sh beta VERSION` or
   `scripts/release/collect-commits.sh stable VERSION` after determining the
   candidate line. Inspect both the JSON commit data and `git diff BASE..HEAD`.
3. Recommend `MAJOR.MINOR.PATCH` using breaking changes, Conventional Commit
   evidence, actual affected code, and the existing tag history. For Beta use
   `VERSION-beta.1`, or increment the existing same-line Beta number.
4. Ask the maintainer to confirm or override the exact version before editing.
5. Generate a dated Keep a Changelog body with Added, Changed, Fixed,
   Security, Deprecated, Removed, or Internal categories as appropriate. Add
   every collected commit to a `### Commits` section with short SHA and link.
6. Write the body to a temporary file and invoke
   `scripts/release/update-changelog.py VERSION BODY_FILE DATE` so the section
   is inserted before `Unreleased`. Update compare links using
   `https://github.com/yelog/lazydb`.
7. Run `scripts/release/set-version.sh VERSION`, then validate the exact
   heading with `scripts/release/validate-changelog.sh VERSION` and
   `scripts/release/validate-version.sh --pre-tag vVERSION`.
8. Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
   -- -D warnings`, `cargo test --all-targets --all-features`, and a release
   binary `version --json` smoke test.
9. Show the complete diff and proposed commands. Ask before creating commit
   `chore(release): prepare vVERSION` and annotated tag `LazyDB vVERSION`.
10. Ask separately before pushing the commit and tag. Pushing the tag starts
    GitHub publication.

## Changelog baselines

- First Beta: previous published tag, or repository root if no tag exists.
- Later Beta: previous Beta for the same base version.
- Stable: previous stable tag, intentionally including intervening Betas.

Do not silently omit merge or revert commits. If a commit is not user-facing,
put it in `Internal` or the traceable commit list.
