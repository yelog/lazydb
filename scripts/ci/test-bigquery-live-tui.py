#!/usr/bin/env python3
"""Opt-in production TUI acceptance against an existing disposable BigQuery dataset.

Requires --run-live and all five LAZYDB_TEST_BIGQUERY_* values documented in
tests/fixtures/bigquery/README.md. Creates one uniquely named synthetic table
with a one-hour expiry, then drops only that table. Never creates projects or
datasets, enables APIs, changes IAM or upgrades billing. Uses the production
HTTPS endpoint, explicit ADC, a 10 MiB billed-byte cap and a 60-second timeout.
Terminal transcripts and credential contents are never saved or printed.
"""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile
import urllib.parse
import uuid

SPEC = importlib.util.spec_from_file_location(
    "bigquery_profile_harness", Path(__file__).with_name("test-bigquery-profile-tui.py"))
PROFILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILE)
HARNESS = PROFILE.HARNESS
require, SmokeFailure = HARNESS.require, HARNESS.SmokeFailure
PROFILE_NAME = "BigQuery cloud acceptance"


def settings():
    values = {}
    for key in ("PROJECT", "DATA_PROJECT", "DATASET", "LOCATION", "CREDENTIALS_FILE"):
        name = "LAZYDB_TEST_BIGQUERY_" + key
        value = os.environ.get(name, "")
        require(bool(value.strip()), name + " must be explicitly configured")
        values[key] = value
    for key in ("PROJECT", "DATA_PROJECT"):
        require(bool(re.fullmatch(r"[a-z][a-z0-9-]{4,28}[a-z0-9]", values[key])),
                key + " must be a Google project ID")
    require(bool(re.fullmatch(r"[A-Za-z0-9_]{1,1024}", values["DATASET"])),
            "DATASET must be a dataset identifier")
    adc = Path(values["CREDENTIALS_FILE"])
    require(adc.is_absolute() and adc.is_file(), "explicit absolute ADC file must exist")
    return values


def cli(binary, root, *arguments):
    env = {key: value for key, value in os.environ.items() if not key.startswith("LAZYDB_")}
    env["LAZYDB_CONFIG_HOME"] = str(root)
    completed = subprocess.run([str(binary), *arguments], env=env, capture_output=True,
                               text=True, timeout=90)
    require(completed.returncode == 0, "production CLI " + arguments[0] + " failed")
    return json.loads(completed.stdout)


def history(root, sql):
    path = root / "sql-history.sqlite3"
    if not path.is_file():
        return []
    try:
        with sqlite3.connect(str(path)) as db:
            return db.execute("SELECT database_name, schema_name FROM history_executions "
                              "WHERE sql=? AND status='succeeded' ORDER BY requested_at", (sql,)).fetchall()
    except sqlite3.Error:
        return []


def check_saved(root, values):
    saved = HARNESS.profile_metadata(root)
    expected = {
        ("profiles", "kind"): "bigquery",
        ("profiles", "database"): values["PROJECT"],
        ("profiles", "default_schema"): values["DATASET"],
        ("profiles.bigquery_options", "data_project"): values["DATA_PROJECT"],
        ("profiles.bigquery_options", "location"): values["LOCATION"],
        ("profiles.bigquery_options", "credentials_file"): values["CREDENTIALS_FILE"],
        ("profiles.bigquery_options", "maximum_bytes_billed"): 10485760,
        ("profiles.bigquery_options", "query_timeout_secs"): 60,
    }
    for field, value in expected.items():
        require(saved.get(field) == value, "persisted native setting " + field[1])
    # Inspect only the generated profile, never the referenced credential file.
    for path in root.rglob("profiles.toml"):
        text = path.read_text()
        for secret_key in ("refresh_token", "private_key", "access_token"):
            require(secret_key not in text, "credential contents must not be persisted")


def drive(binary, root, url, values, name, marker):
    sql = "SELECT fixture_answer AS cloud_answer FROM `" + name + "`"
    tui = HARNESS.Tui(binary, root, [])
    successful = False
    try:
        tui.ready()
        tui.send(b"\r")
        tui.wait(lambda: tui.contains("NEW CONNECTION"), "new connection form")
        for _ in range(5):
            tui.send(b"\x1b[Z")
        tui.fill(url)
        tui.wait(lambda: tui.contains("BigQuery PROFILE"), "BigQuery URL form")
        tui.tabs(6)
        tui.fill(PROFILE_NAME)
        tui.send(b"\x14")
        tui.wait(lambda: tui.contains("Connection verified:"), "real Test Connection", 60)
        tui.save()
        check_saved(root, values)
        tui.wait(lambda: tui.contains(values["DATA_PROJECT"]), "data project discovery", 60)
        print("PASS: production form/Test Connection preserves execution and data projects", flush=True)

        # Search by the unique table name rather than relying on tree row order.
        # This cannot select a concurrent integration test's transient table.
        tui.send(b"f")
        tui.send(name.encode())
        tui.wait(lambda: tui.contains("1 results") and tui.contains(name),
                 "unique owned table search", 60)
        tui.send(b"\r")
        tui.wait(lambda: not tui.contains("Enter locate"), "locate owned table")
        tui.send(b"p")
        tui.wait(lambda: tui.contains("fixture_answer") and tui.contains(marker),
                 "owned table preview", 60)
        tui.send(b"D")
        tui.wait(lambda: tui.contains("CREATE TABLE") and tui.contains(name),
                 "native table DDL", 60)
        print("PASS: Explorer finds and previews only the unique fixture and reads native DDL", flush=True)

        tui.send(b"\x1b[17~")  # F6: console manager.
        tui.wait(lambda: tui.contains("CONSOLES"), "console manager")
        tui.send(b"a")
        tui.wait(lambda: tui.contains("INSERT"), "new SQL console")
        tui.send(b"\x1b[200~" + sql.encode() + b"\x1b[201~")
        tui.send(b"\x1b")
        tui.send(b"\x1b[15~")  # F5: execute.
        tui.wait(lambda: len(history(root, sql)) == 1, "real query success history", 60)
        tui.wait(lambda: tui.contains("cloud_answer") and tui.contains("42"), "real result grid")
        # Focus the data-project node, then run the existing console again.
        tui.send(b"\x17h")  # Ctrl-W h: Explorer.
        tui.send(b"gg")
        tui.send(b"j")
        tui.send(b"\x17l")  # Ctrl-W l: Editor.
        tui.send(b"\x1b[15~")
        tui.wait(lambda: len(history(root, sql)) == 2, "query after Explorer focus change", 60)
        require(all(database == values["PROJECT"] and schema == values["DATASET"]
                    for database, schema in history(root, sql)),
                "Explorer focus must not change the existing console target")
        print("PASS: SQL grid/history retains execution project after Explorer focus changes", flush=True)
        successful = True
    finally:
        tui.close(successful)

    tui = HARNESS.Tui(binary, root, [])
    successful = False
    try:
        tui.ready()
        tui.wait(lambda: tui.contains(PROFILE_NAME), "saved profile restart")
        tui.edit()
        tui.wait(lambda: tui.contains("10485760") and tui.contains(values["DATASET"]),
                 "restored native options")
        check_saved(root, values)
        tui.send(b"\x1b")
        successful = True
    finally:
        tui.close(successful)
    print("PASS: restart restores ADC path/options and both terminals are restored", flush=True)

    result = cli(binary, root, "agent", "query", "--connection", PROFILE_NAME, "--sql", sql)
    native = result["outcome"]["bigquery"]
    require(native["project"] == values["PROJECT"], "native job must belong to execution project")
    require(native["location"].casefold() == values["LOCATION"].casefold(), "native job location")
    require(native["total_bytes_billed"] <= 10485760, "native job billed-byte cap")
    print("PASS: native query job project/location/cap confirmed; job=" + native["job_id"], flush=True)


def run(binary, values):
    require(binary.is_file() and os.access(binary, os.X_OK), "executable production binary required")
    name = "lazydb_tui_" + uuid.uuid4().hex[:12]
    marker = "fixture_" + uuid.uuid4().hex[:8]
    qualified = "`" + values["DATA_PROJECT"] + "." + values["DATASET"] + "." + name + "`"
    options = urllib.parse.urlencode({
        "data_project": values["DATA_PROJECT"], "location": values["LOCATION"],
        "maximum_bytes_billed": "10485760", "query_timeout_secs": "60",
        "credentials_file": values["CREDENTIALS_FILE"],
    })
    url = "bigquery://" + values["PROJECT"] + "/" + values["DATASET"] + "?" + options
    with tempfile.TemporaryDirectory(prefix="lazydb-bigquery-live-tui-") as temporary:
        base = Path(temporary)
        setup, root = base / "setup", base / "tui"
        setup.mkdir()
        root.mkdir()
        for directory in (setup, root):
            (directory / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
        cli(binary, setup, "--url", url, "connections", "add", "--name", "fixture setup",
            "--scope", "global", "--read-write", "--json")
        created = False
        try:
            create = ("CREATE TABLE " + qualified + " OPTIONS(expiration_timestamp="
                      "TIMESTAMP_ADD(CURRENT_TIMESTAMP(), INTERVAL 1 HOUR)) AS SELECT "
                      "42 AS fixture_answer, '" + marker + "' AS fixture_text, STRUCT(1 AS x) AS nested_value")
            # Mark before submission: an ambiguous response may still have created
            # our UUID table. Cleanup addresses only that exact generated name.
            created = True
            cli(binary, setup, "agent", "execute", "--connection", "fixture setup",
                "--write-policy", "non-production", "--sql", create)
            drive(binary, root, url, values, name, marker)
        finally:
            if created:
                cli(binary, setup, "agent", "execute", "--connection", "fixture setup",
                    "--write-policy", "non-production", "--sql", "DROP TABLE IF EXISTS " + qualified)
                print("PASS: only this workflow's unique fixture table was dropped", flush=True)
    print("PASS: isolated application files cleaned up", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/lazydb", type=Path)
    parser.add_argument("--run-live", action="store_true",
                        help="explicitly submit capped jobs and create/drop one unique test table")
    args = parser.parse_args()
    if not args.run_live:
        parser.error("--run-live is required; this workflow uses real Google Cloud resources")
    try:
        run(args.binary.expanduser().resolve(), settings())
    except BaseException as error:
        print("FAIL: BigQuery live TUI acceptance (" + type(error).__name__ + ")", file=sys.stderr)
        if isinstance(error, SmokeFailure):
            print("Check: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
