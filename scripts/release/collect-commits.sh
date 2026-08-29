#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/release-lib.sh"
[ "$#" -eq 2 ] || release_die "usage: collect-commits.sh beta|stable VERSION"
channel=$1
version=$2
case "$channel" in beta|stable) ;; *) release_die "invalid channel: $channel" ;; esac

python3 - "$channel" "$version" <<'PY'
import json
import re
import subprocess
import sys

channel, version = sys.argv[1:]

def git(*args):
    return subprocess.check_output(["git", *args], text=True).rstrip("\n")

tags = git("tag", "--list").splitlines()
stable = [tag for tag in tags if re.fullmatch(r"v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)", tag)]
betas = [tag for tag in tags if re.fullmatch(rf"v{re.escape(version)}-beta\.[1-9]\d*", tag)]
if channel == "beta" and betas:
    base = sorted(betas, key=lambda tag: [int(part) for part in tag.rsplit(".", 1)[-1:]])[-1]
elif stable:
    def semver(tag):
        return tuple(int(part) for part in tag[1:].split("."))
    base = sorted(stable, key=semver)[-1]
else:
    base = git("rev-list", "--max-parents=0", "HEAD").splitlines()[-1]

has_tag_baseline = bool(stable or (channel == "beta" and betas))
revisions = f"{base}..HEAD" if has_tag_baseline else "HEAD"
format_string = "%H%x1f%h%x1f%an%x1f%aI%x1f%s%x1f%b%x1e"
raw = subprocess.check_output(["git", "log", f"--format={format_string}", revisions], text=True)
commits = []
for record in raw.split("\x1e"):
    record = record.strip("\n")
    if not record:
        continue
    fields = record.split("\x1f", 5)
    if len(fields) != 6:
        raise SystemExit("unable to parse git commit record")
    sha, short_sha, author, date, subject, body = fields
    paths = git("diff-tree", "--no-commit-id", "--name-only", "-r", sha).splitlines()
    commits.append({
        "sha": sha,
        "short_sha": short_sha,
        "author": author,
        "date": date,
        "subject": subject,
        "body": body.strip(),
        "paths": paths,
    })
print(json.dumps({"base": base, "head": "HEAD", "commits": commits}, ensure_ascii=True))
PY
