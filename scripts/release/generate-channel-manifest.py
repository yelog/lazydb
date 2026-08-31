#!/usr/bin/env python3
"""Generate the immutable channel metadata consumed by installers."""

import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path
from typing import NoReturn


TARGETS = (
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
)
REPOSITORY = "yelog/lazydb"
TAG_RE = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-beta\.[1-9][0-9]*)?$")
SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")


def fail(message: str) -> "NoReturn":
    print(f"channel manifest: {message}", file=sys.stderr)
    raise SystemExit(1)


def parse_checksums(path: Path) -> dict[str, str]:
    if not path.is_file():
        fail(f"missing SHA256SUMS: {path}")
    checksums: dict[str, str] = {}
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        fields = line.split()
        if len(fields) != 2 or not SHA256_RE.fullmatch(fields[0]):
            fail(f"invalid SHA256SUMS entry on line {line_number}")
        name = fields[1][1:] if fields[1].startswith("*") else fields[1]
        if name in checksums:
            fail(f"duplicate checksum entry: {name}")
        checksums[name] = fields[0].lower()
    return checksums


def main() -> None:
    if len(sys.argv) != 6:
        fail("usage: generate-channel-manifest.py CHANNEL VERSION PUBLISHED_AT ASSET_DIR OUTPUT")
    channel, version, published_at, asset_dir, output = sys.argv[1:]
    if channel not in {"stable", "beta"}:
        fail(f"invalid channel: {channel}")
    tag = f"v{version}"
    if not TAG_RE.fullmatch(tag):
        fail(f"invalid release version: {version}")
    is_beta = "-beta." in version
    if (channel == "beta") != is_beta:
        fail(f"channel/version mismatch: {channel} {version}")

    asset_dir_path = Path(asset_dir)
    checksums = parse_checksums(asset_dir_path / "SHA256SUMS")
    assets = {}
    for target in TARGETS:
        name = f"lazydb_{version}_{target}.tar.xz"
        asset = asset_dir_path / name
        if not asset.is_file():
            fail(f"missing target archive: {name}")
        if name not in checksums:
            fail(f"checksum entry missing for {name}")
        digest = hashlib.sha256(asset.read_bytes()).hexdigest()
        if checksums[name] != digest:
            fail(f"checksum mismatch for {name}")
        assets[target] = {
            "url": f"https://github.com/{REPOSITORY}/releases/download/{tag}/{name}",
            "sha256": checksums[name],
        }

    manifest = {
        "schema": 1,
        "product": "lazydb",
        "channel": channel,
        "version": version,
        "tag": tag,
        "prerelease": is_beta,
        "published_at": published_at,
        "release_url": f"https://github.com/{REPOSITORY}/releases/tag/{tag}",
        "assets": assets,
    }
    output_path = Path(output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{output_path.name}.", dir=output_path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(manifest, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, output_path)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


if __name__ == "__main__":
    main()
