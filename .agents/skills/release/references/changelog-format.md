# Changelog Format

Use one exact heading per release:

```markdown
## [0.2.0-beta.1] - 2026-08-29

### Added

- User-facing change ([`abc1234`](https://github.com/yelog/lazydb/commit/abc1234)).

### Commits

- [`abc1234`](https://github.com/yelog/lazydb/commit/abc1234) feat: user-facing change
```

The tracked `CHANGELOG.md` keeps the complete `Commits` list expanded. When
`changelog-section.sh` prepares GitHub Release notes, it wraps that subsection
in a collapsed `<details>` block so the release page stays concise while every
commit remains available on demand.

Every commit in the selected Git range must occur once in `Commits`. Keep
summaries factual and concise. Stable notes use the previous stable tag as the
baseline; Beta notes use the previous Beta on the same release line.
