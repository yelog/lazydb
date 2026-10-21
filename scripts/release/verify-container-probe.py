#!/usr/bin/env python3
"""Temporary cloud-only probe for the committed Linux wrapper and cache action."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
PROBE = ROOT / ".container-probe"
WRAPPER = PROBE / "scripts/release/build-linux.sh"


def run(target, *command):
    return subprocess.run(
        ["sh", str(WRAPPER), target, "--exec", *command],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def prepare():
    (PROBE / "scripts/release").mkdir(parents=True)
    (PROBE / "src").mkdir()
    for name in ("build-linux.sh", "Dockerfile.linux"):
        shutil.copyfile(ROOT / "scripts/release" / name, PROBE / "scripts/release" / name)
    (PROBE / "Cargo.toml").write_text('''[package]
name = "lazydb"
version = "0.0.0"
edition = "2024"

[dependencies]
libsqlite3-sys = { version = "=0.37.0", features = ["bundled"] }

[profile.release]
lto = "thin"
strip = "symbols"
''')
    (PROBE / "src/main.rs").write_text('''use std::ffi::CStr;

fn main() {
    let version = unsafe { CStr::from_ptr(libsqlite3_sys::sqlite3_libversion()) };
    println!("sqlite {}", version.to_str().expect("valid version"));
}
''')


def metadata(target):
    compiler = run(target, "rustc", "-vV")
    assert "release: 1.94.0" in compiler, compiler
    assert "host: " + target in compiler, compiler
    toolchains = run(target, "rustup", "toolchain", "list", "--quiet").splitlines()
    assert toolchains == ["1.94.0-" + target], toolchains
    assert run(target, "rustup", "run", toolchains[0], "rustc", "-vV") == compiler
    run(target, "cargo", "generate-lockfile")
    document = json.loads(run(target, "cargo", "metadata", "--all-features", "--format-version", "1"))
    assert document["workspace_root"] == str(PROBE), document["workspace_root"]
    assert document["target_directory"] == str(PROBE / "target"), document["target_directory"]
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
    native = next(package for package in document["packages"] if package["name"] == "libsqlite3-sys")
    assert native["manifest_path"].startswith(str(cargo_home) + "/registry/"), native["manifest_path"]
    assert Path(native["manifest_path"]).is_file(), native["manifest_path"]
    print(json.dumps({"compiler": compiler, "toolchains": toolchains, "native_manifest": native["manifest_path"]}, indent=2))


def result(target):
    output = run(target, str(PROBE / "target" / target / "release/lazydb"))
    assert output.startswith("sqlite 3."), output
    log = (ROOT / "container-build.log").read_text()
    hit = os.environ.get("CACHE_HIT") == "true"
    native_compiled = "Compiling libsqlite3-sys " in log
    if hit:
        assert not native_compiled, "Native dependency recompiled despite an exact cache hit"
    evidence = {
        "target": target,
        "cache_hit": hit,
        "native_dependency_compiled": native_compiled,
        "executable_output": output.strip(),
    }
    (ROOT / "container-result.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    if sys.argv[1] == "prepare":
        prepare()
    elif sys.argv[1] == "metadata":
        metadata(sys.argv[2])
    elif sys.argv[1] == "result":
        result(sys.argv[2])
    else:
        raise SystemExit("unknown probe mode")
