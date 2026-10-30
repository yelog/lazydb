# Installation, Updates, and Uninstall

Use the [README quickstart](../README.md#quickstart) for the shortest path to
your first connection. This guide covers installation choices, release channels,
updates, removal, and troubleshooting.

## macOS and Linux

```sh
curl -fsSL https://lazydb.yelog.org/install.sh | sh
```

The installer supports Intel and Apple Silicon macOS, plus x86_64 and ARM64
Linux. It requires `curl`, Python 3 with working `lzma` support, standard Unix
utilities, and `sha256sum` or `shasum`. Homebrew is not required. The installer
verifies the archive checksum and staged executable before activation.

The launcher defaults to `$HOME/.local/bin/lazydb`. Follow the installer's
instructions to activate the command in your current shell, or open a new
terminal. Native releases and configuration share the application directory;
see [configuration files](configuration.md#configuration-files) for its location
and migration rules.

Linux installers check the release's published glibc requirement before
downloading its archive. See [Linux runtime compatibility](linux-runtime-compatibility.md)
to inspect your system and understand how the requirement is measured.

To customize the launcher location or leave shell profiles unchanged:

```sh
curl -fsSL https://lazydb.yelog.org/install.sh | sh -s -- \
  --install-dir "$HOME/.local/bin" --no-modify-path
```

When using `--no-modify-path`, ensure the selected directory is already in
`PATH` or add it yourself. Review the [Unix installer source](../pages/install-core.sh)
for the complete options.

## Windows

Run this command in PowerShell:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "irm https://lazydb.yelog.org/install.ps1.txt -ErrorAction Stop | iex"
```

The installer supports 64-bit Windows with the x86_64 MSVC release. It downloads
release metadata and the ZIP archive over HTTPS, verifies the SHA-256 checksum,
and adds `%LOCALAPPDATA%\LazyDB\bin` to the user `PATH`. No administrator
privileges are required. Open a new terminal after installation.

The command downloads and executes a script from the LazyDB site. The `.txt`
endpoint is generated from the same installer source and served as text for
PowerShell compatibility. Review the [Windows installer source](../pages/install.ps1),
or download the ZIP from [GitHub Releases](https://github.com/yelog/lazydb/releases/latest)
for a manual installation.

### Download the Windows installer to a file

For troubleshooting or environments where piping scripts to `Invoke-Expression`
is unsuitable, use a temporary file. This still executes downloaded code;
saving the script does not authenticate it.

<!-- windows-installer-file-start -->
```powershell
& {
    $ErrorActionPreference = 'Stop'
    $installer = Join-Path ([IO.Path]::GetTempPath()) (
        [IO.Path]::GetRandomFileName() + '.ps1'
    )
    try {
        Invoke-WebRequest -Uri 'https://lazydb.yelog.org/install.ps1' `
            -UseBasicParsing -OutFile $installer
        if ((Get-Item -LiteralPath $installer).Length -eq 0) {
            throw 'Downloaded installer is empty.'
        }
        powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer
        if ($LASTEXITCODE -ne 0) {
            throw "LazyDB installer failed with exit code $LASTEXITCODE."
        }
    } finally {
        Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
    }
}
```

<!-- windows-installer-file-end -->

## Homebrew

On macOS:

```sh
brew install yelog/tap/lazydb
```

Homebrew owns this installation. Use `brew upgrade yelog/tap/lazydb` to update
it and `brew uninstall lazydb` to remove it.

## Release Channels

The standard installers select the **stable** release channel. This identifies
which releases to download; LazyDB's product maturity and database-specific
limits are described in the [README](../README.md) and
[capability reference](database-capabilities.md).

To install the beta channel on macOS or Linux:

```sh
curl -fsSL https://lazydb.yelog.org/install-beta.sh | sh
```

On Windows, set the channel before running the installer:

```powershell
$env:LAZYDB_CHANNEL = 'beta'
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "irm https://lazydb.yelog.org/install.ps1.txt -ErrorAction Stop | iex"
```

## Manual Release Archives

Download the archive matching your platform from
[GitHub Releases](https://github.com/yelog/lazydb/releases/latest) and verify
its checksum against the release's published SHA-256 value.

| Platform | Archive suffix |
| --- | --- |
| macOS, Apple Silicon | `aarch64-apple-darwin.tar.xz` |
| macOS, Intel | `x86_64-apple-darwin.tar.xz` |
| Linux, x86_64 | `x86_64-unknown-linux-gnu.tar.xz` |
| Linux, ARM64 | `aarch64-unknown-linux-gnu.tar.xz` |
| Windows, x86_64 | `x86_64-pc-windows-msvc.zip` |

For example, after downloading and verifying an Apple Silicon archive, replace
`VERSION` with the downloaded version:

```sh
tar -xJf lazydb_VERSION_aarch64-apple-darwin.tar.xz
mkdir -p "$HOME/.local/bin"
cp lazydb_VERSION_aarch64-apple-darwin/lazydb "$HOME/.local/bin/lazydb"
chmod +x "$HOME/.local/bin/lazydb"
lazydb version
```

Ensure `$HOME/.local/bin` is in `PATH`. On Windows, extract the ZIP, place
`lazydb.exe` in a directory on your user `PATH`, and run `lazydb version`.

Manual archive installations are updated by downloading and verifying the new
archive, then replacing the executable. Native installers maintain additional
ownership metadata required by the built-in update and uninstall commands.

## Build from Source

Use Rust and Cargo matching the minimum version in
[`Cargo.toml`](../Cargo.toml), currently Rust 1.94 or newer. From a repository
checkout:

```sh
cargo build --locked --release
./target/release/lazydb
```

On Windows, the executable is `target\release\lazydb.exe`. See
[Contributing](../CONTRIBUTING.md) for local checks and
[CI build notes](ci.md) for platform build details.

## Optional Database Runtimes

- **Oracle:** A working Oracle Instant Client is required at runtime. The default
  LazyDB build includes the adapter; see
  [client discovery](configuration.md#connection-profile-fields) for library paths.
- **DuckDB:** A compatible DuckDB 1.5.6 C API shared library is required only
  when opening a DuckDB connection. It is not bundled or downloaded automatically;
  see [DuckDB runtime setup](duckdb.md#install-the-optional-runtime).

These requirements do not apply when using unrelated database drivers.

## Optional Coding-Agent Setup

From the target project, run:

```sh
lazydb mcp setup
```

The wizard supports Claude Code, Codex, and OpenCode, offers user-level and
project-level configuration, and shows the target file before writing it. It
does not install a coding agent or copy database credentials. Generated MCP
entries deny database writes by default.

Native installers can offer the wizard after an interactive installation;
accept the prompt to enter it or defer setup until later. For unattended Unix
installations, use `--mcp-setup skip`; `LAZYDB_MCP_SETUP=skip` is also supported
by both installers. Package installation does not register an MCP server
automatically. See [automated MCP setup](coding-agent-access.md#automated-setup)
for scope selection, non-interactive commands, and troubleshooting.

## Updates

For a native script installation:

```sh
lazydb update --check
lazydb update
```

The command immediately shows the running version, followed by the check result,
download progress, installation stages, and outcome. Interactive terminals show
a live progress bar when the download size is known; redirected output uses
stage logs. Add `--json` for a single-line machine-readable report.

The recorded installation channel is used by default. Select a channel for the
operation with `--channel stable` or `--channel beta`. The Update Center (`F9`)
also offers updates for supported native installations. Restart LazyDB to run
the newly installed version.

Package-manager installations must be updated through their owning manager.
LazyDB does not overwrite Homebrew, Cargo, or system-package installations.
For manual or source installations, replace or rebuild the executable yourself.
See [installation updates](configuration.md#installation-updates) for manager
commands and [release pruning](configuration.md#prune-old-native-releases) to
inspect and remove eligible old native releases.

## Uninstall

### Native macOS and Linux installations

Inspect the removal plan, then uninstall:

```sh
lazydb uninstall --dry-run
lazydb uninstall
```

The default uninstall removes the launcher, native releases, current release
link, installation metadata, and the PATH block managed by the LazyDB installer.
It preserves connection profiles, encrypted credentials, settings, workspace
state, SQL files, and unknown files. Use `--yes` for a non-interactive uninstall;
`--json` requires either `--dry-run` or `--yes`.

To also remove the allowlisted LazyDB data files:

```sh
lazydb uninstall --purge --dry-run
lazydb uninstall --purge
```

`--purge` may remove saved connection definitions, local credential keys,
settings, workspace state, and update-check state after profile and credential
checks. Unknown files and database files are preserved. System keyring entries
are removed only when valid LazyDB profiles reference them and the native
secret store is available.

If installation metadata is missing or paths no longer match the recorded
installation, uninstall stops without deleting anything and reports the paths
that need manual review.

### Other installation methods

Remove Homebrew, Cargo, and system-package installations through their original
package manager. For manual or source installations, remove the executable you
placed on `PATH`.

Windows removal is not provided by the native POSIX uninstall command yet.
For a script installation, close LazyDB and remove its executable from
`%LOCALAPPDATA%\LazyDB\bin` (or your custom install directory), then remove
that directory's entry from your user `PATH` if it is no longer needed. Review
`%APPDATA%\lazydb\` separately before removing configuration or saved data.

### MCP configuration

Uninstall does not modify MCP client configuration. Remove the `lazydb` entry
from the user-level or project-level file selected during setup, such as
`.mcp.json`, `.codex/config.toml`, or `opencode.json[c]`. See
[MCP configuration discovery](coding-agent-access.md#automated-setup) to locate
the relevant file.
