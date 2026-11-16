#!/usr/bin/env python3
"""Exercise SQLite backup through the real F2 form, review, and task history."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import uuid


def require(condition, label):
    if not condition:
        raise RuntimeError(label)
    print("PASS:", label, flush=True)


def load_support():
    sys.dont_write_bytecode = True
    repository = Path(__file__).resolve().parents[3]
    path = repository / "scripts/ci/test-elasticsearch-profile-tui.py"
    specification = importlib.util.spec_from_file_location("data_ops_pty_support", path)
    support = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(support)
    return support


def records(root):
    return [
        json.loads(path.read_text())
        for path in sorted((root / "data-jobs").glob("*.json"))
    ]


def open_command(tui, title):
    tui.send(b"\x1b[12~")
    tui.wait(lambda: tui.contains("OMNI"), "F2 command palette")
    tui.fill("> " + title)
    tui.wait(lambda: tui.contains(title), "find " + title)
    tui.send(b"\r")


def exercise(binary):
    support = load_support()
    with tempfile.TemporaryDirectory(prefix="lazydb-data-ops-pty-") as temporary:
        root = Path(temporary).resolve()
        config = root / "config"
        project = root / "project"
        config.mkdir()
        project.mkdir()
        (project / ".git").mkdir()
        source = project / "source.sqlite"
        artifact = project / "backup.sqlite"
        expected = [
            (1, "hello 世界 'quoted'", bytes([0, 255, 16, 32]), 123.25),
            (2, None, None, None),
            (3, "", b"", -0.5),
        ]
        with sqlite3.connect(str(source)) as connection:
            connection.execute(
                "CREATE TABLE records (id INTEGER PRIMARY KEY AUTOINCREMENT, "
                "value TEXT, payload BLOB, amount NUMERIC)"
            )
            connection.executemany("INSERT INTO records VALUES (?, ?, ?, ?)", expected)
            connection.execute("UPDATE sqlite_sequence SET seq = 14999 WHERE name = 'records'")
        profile_id = str(uuid.uuid4())
        profile_name = "data-ops-pty-fixture"
        profile = [
            "version = 6",
            "[[profiles]]",
            "id = " + json.dumps(profile_id),
            "name = " + json.dumps(profile_name),
            'kind = "sqlite"',
            'url_format = "sqlite"',
            "database = " + json.dumps(str(source)),
            "sqlite_path = " + json.dumps(str(source)),
            'default_schema = "main"',
            'ssl_mode = "disable"',
            "read_only = false",
            'environment = "development"',
            "[profiles.credential_policy]",
            'policy = "none"',
            "[profiles.catalog_scope.databases]",
            'mode = "all"',
        ]
        (config / "connections.toml").write_text("\n".join(profile) + "\n")
        (config / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
        original_directory = Path.cwd()
        tui = None
        successful = False
        try:
            os.chdir(project)
            tui = support.Tui(binary, config, [], profile_name)
            tui.ready()
            tui.send(b"\x17h")
            tui.connect()
            require(tui.contains("●"), "isolated saved SQLite profile connected")
            open_command(tui, "Backup Database")
            tui.wait(
                lambda: tui.contains("BACKUP DATABASE") and tui.contains("DRAFT"),
                "backup draft",
            )
            require(not artifact.exists(), "F2 opens backup draft without creating output")
            tui.send(b"\t")
            tui.wait(lambda: tui.contains("> Artifact path"), "artifact field focus")
            tui.fill(str(artifact))
            tui.send(b"\r")
            tui.wait(
                lambda: tui.contains("BACKUP DATABASE") and tui.contains("REVIEW")
                and tui.contains("immutable plan"),
                "reviewed SQLite backup plan",
            )
            require(not artifact.exists(), "review displays immutable plan without creating output")
            tui.send(b"\r")
            tui.wait(
                lambda: tui.contains("DATA TASKS") and tui.contains("Succeeded"),
                "successful backup task",
            )
            require(artifact.is_file(), "execute opens Data Tasks and creates backup")
            saved = records(config)
            require(
                len(saved) == 1 and saved[0]["status"] in ("succeeded", "succeeded_with_warnings")
                and saved[0]["plan"]["spec"]["source"]["profile_id"] == profile_id,
                "one successful job persisted for the fixture profile",
            )
            manifest = json.loads(Path(str(artifact) + ".lazydb.json").read_text())
            require(
                manifest["engine"] == "sqlite"
                and manifest["bytes"] == artifact.stat().st_size
                and manifest["sha256"] == hashlib.sha256(artifact.read_bytes()).hexdigest(),
                "backup manifest size and SHA-256 match the real file",
            )
            with sqlite3.connect(str(artifact)) as connection:
                require(
                    connection.execute("PRAGMA integrity_check").fetchone() == ("ok",)
                    and connection.execute(
                        "SELECT id, value, payload, amount FROM records ORDER BY id"
                    ).fetchall() == expected
                    and connection.execute(
                        "SELECT seq FROM sqlite_sequence WHERE name = 'records'"
                    ).fetchone() == (14999,),
                    "backup preserves rows, NULL/empty/binary/numeric values and sequence",
                )
            with sqlite3.connect(str(source)) as connection:
                require(
                    connection.execute(
                        "SELECT id, value, payload, amount FROM records ORDER BY id"
                    ).fetchall() == expected,
                    "source rows remain unchanged",
                )
            tui.send(b"\x1b")
            tui.wait(lambda: not tui.contains("DATA TASKS"), "close tasks")
            open_command(tui, "Data Tasks")
            tui.wait(
                lambda: tui.contains("DATA TASKS") and tui.contains("Succeeded"),
                "reopened task history",
            )
            require(records(config)[0]["id"] == saved[0]["id"], "F2 reopens the same persisted task")
            tui.send(b"\x1b")
            tui.wait(lambda: not tui.contains("DATA TASKS"), "close task history")
            successful = True
        except Exception:
            if tui is not None:
                print("Fixture screen at failure:\n" + tui.screen.text(), file=sys.stderr)
            raise
        finally:
            try:
                if tui is not None:
                    if not successful and tui.proc.poll() is None:
                        tui.send(b"\x1b")
                    tui.close(successful)
            finally:
                os.chdir(original_directory)
        require(successful, "clean process exit and terminal restoration")
    print("SQLite real PTY acceptance: 10 checks passed; private fixture removed.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    arguments = parser.parse_args()
    binary = arguments.binary.resolve(strict=True)
    exercise(binary)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print("sqlite-data-ops-tui:", error, file=sys.stderr)
        raise SystemExit(1)
