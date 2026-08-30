# Changelog

All notable changes to LazyDB are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/).

## Unreleased

Changes that are not part of a tagged release go here.

### Added

- Documented per-connection workspaces: profile switches are committed only
  after a successful connection, failed switches preserve the current workspace,
  disconnecting hides rather than deletes a workspace, relation tabs restore as
  lazy shells without persisting result data across restarts, and profile
  deletion removes the workspace.

[unreleased]: https://github.com/yelog/lazydb/compare/HEAD...HEAD
