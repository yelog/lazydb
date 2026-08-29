#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/release-lib.sh"
[ "$#" -eq 2 ] || release_die "usage: generate-sbom.sh VERSION OUTPUT"
version=$1
output=$2
release_version_from_tag "v$version" >/dev/null
python3 - "$version" "$output" <<'PY'
import json
import pathlib
import subprocess
import sys

version, output = sys.argv[1:]
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], text=True))
packages = [{"type": "library", "name": p["name"], "versionInfo": p["version"], "purl": f"pkg:cargo/{p['name']}@{p['version']}"} for p in metadata["packages"]]
sbom = {"spdxVersion": "SPDX-2.3", "SPDXID": "SPDXRef-DOCUMENT", "name": f"lazydb-{version}", "documentNamespace": f"https://github.com/yelog/lazydb/releases/{version}/sbom", "creationInfo": {"creators": ["Tool: LazyDB release tooling"]}, "packages": packages}
pathlib.Path(output).write_text(json.dumps(sbom, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
