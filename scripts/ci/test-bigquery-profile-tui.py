#!/usr/bin/env python3
"""Create, save, and reopen a BigQuery profile through the real TUI, offline.

Uses the existing PTY/VT harness. No credentials or live BigQuery fixture are
required. Save currently also requests a connection, so an explicitly missing
ADC file in the isolated process environment makes that request fail locally.
The saved profile references only that missing temporary path, never credentials.
"""

import argparse
import importlib.util
import os
import re
from pathlib import Path
import sys
import tempfile
import urllib.parse


HARNESS_PATH = Path(__file__).with_name("test-elasticsearch-profile-tui.py")
SPEC = importlib.util.spec_from_file_location("lazydb_profile_tui_harness", HARNESS_PATH)
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)
Tui, require, SmokeFailure = HARNESS.Tui, HARNESS.require, HARNESS.SmokeFailure


class Screen(HARNESS.Screen):
    """Extend the shared screen with ratatui's erase-character sequence."""

    def feed(self, data):
        # Feed complete characters incrementally so CSI X erases at the current
        # cursor before the base reader consumes that otherwise unsupported CSI.
        for byte in data:
            match = re.fullmatch(r"\x1b\[([0-9]*)", self.pending)
            if byte == ord("X") and match:
                count = int(match.group(1) or "1")
                end = min(self.columns, self.column + count)
                self.grid[self.row][self.column:end] = [" "] * (end - self.column)
                self.pending = ""
            else:
                super().feed(bytes([byte]))


HARNESS.Screen = Screen

PROFILE_NAME = "BigQuery offline profile"
PROFILE_URL = "bigquery://billing-project/fixture?data_project=data-project&location=US"


def check_saved(root):
    values = HARNESS.profile_metadata(root)
    expected = {
        ("profiles", "name"): PROFILE_NAME,
        ("profiles", "kind"): "bigquery",
        ("profiles", "url_format"): "bigquery",
        ("profiles", "database"): "billing-project",
        ("profiles", "default_schema"): "fixture",
        ("profiles.credential_policy", "policy"): "none",
        ("profiles.bigquery_options", "data_project"): "data-project",
        ("profiles.bigquery_options", "location"): "US",
        ("profiles.bigquery_options", "maximum_bytes_billed"): 1000000000,
        ("profiles.bigquery_options", "query_timeout_secs"): 120,
    }
    for field, value in expected.items():
        require(values.get(field) == value, "persisted profile field " + field[1])
    for key in ("host", "port", "user"):
        require(not values.get(("profiles", key)), "BigQuery has no " + key)
    require(values.get(("profiles.bigquery_options", "credentials_file")) == str(root / "missing-adc.json"),
            "explicit missing ADC path prevents ambient discovery")
    require(not (root / "credential.key").exists(), "ADC creates no password encryption key")


def check_form(tui):
    tui.wait(lambda: tui.contains("Billing project") and tui.contains("Query timeout")
             and not tui.contains("Password") and not tui.contains("SSL mode"),
             "complete BigQuery form redraw")
    for label in ("Billing project", "Default dataset", "Data project", "ADC file",
                  "Location", "Max billed bytes", "Query timeout"):
        require(tui.contains(label), "BigQuery form label " + label)
    require(not tui.contains("Password"), "BigQuery form has no password field")
    require(not tui.contains("SSL mode"), "BigQuery form has no endpoint TLS setting")
    require(tui.contains("billing-project") and tui.contains("data-project")
            and tui.contains("fixture"), "separate execution and data project fields")


def run(binary):
    require(binary.is_file() and os.access(binary, os.X_OK), "executable lazydb binary required")
    with tempfile.TemporaryDirectory(prefix="lazydb-bigquery-profile-tui-") as temporary:
        root = Path(temporary)
        (root / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
        # The harness strips LAZYDB_* env and creates LAZYDB_CONFIG_HOME itself.
        # It preserves Google ADC variables, which we deliberately isolate here.
        isolated_env = {
            "GOOGLE_APPLICATION_CREDENTIALS": str(root / "missing-adc.json"),
            "CLOUDSDK_CONFIG": str(root / "gcloud"),
            "GCE_METADATA_HOST": "127.0.0.1:1",
            "HOME": str(root),
        }
        previous = {key: os.environ.get(key) for key in isolated_env}
        os.environ.update(isolated_env)
        try:
            tui = Tui(binary, root, [])
            successful = False
            try:
                tui.ready()
                tui.send(b"\r")
                tui.wait(lambda: tui.contains("NEW CONNECTION"), "new profile form")
                # New forms start on Driver. BackTab wraps via Category,
                # Cancel, Save, Test to the URL field without selecting a driver.
                tui.send(b"\x1b[Z")  # Driver -> Category.
                tui.send(b"\x1b[Z")  # Category -> Cancel.
                tui.send(b"\x1b[Z")  # Cancel -> Save.
                tui.send(b"\x1b[Z")  # Save -> Test.
                tui.send(b"\x1b[Z")  # Test -> URL.
                missing_adc = urllib.parse.quote(str(root / "missing-adc.json"), safe="")
                tui.fill(PROFILE_URL + "&credentials_file=" + missing_adc)
                tui.wait(lambda: tui.contains("BigQuery PROFILE"), "URL selects BigQuery driver")
                check_form(tui)
                # URL -> Test -> Save -> Cancel -> Category -> Driver -> Name.
                tui.tabs(6)
                tui.fill(PROFILE_NAME)
                tui.save()
                check_saved(root)
                tui.wait(lambda: tui.contains("FAILED") or tui.contains("credentials"),
                         "missing isolated ADC rejected locally")
                successful = True
            finally:
                tui.close(successful)
            print("PASS: real form creates and saves ADC BigQuery profile", flush=True)

            tui = Tui(binary, root, [])
            successful = False
            try:
                tui.ready()
                tui.wait(lambda: tui.contains(PROFILE_NAME), "saved profile restart")
                tui.edit()
                check_form(tui)
                require(tui.contains("US") and tui.contains("1000000000"),
                        "options restored in real edit form")
                check_saved(root)
                tui.send(b"\x1b")
                successful = True
            finally:
                tui.close(successful)
            print("PASS: restart restores separate projects, dataset and ADC options", flush=True)
        finally:
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value
    print("PASS: isolated profile files removed and both terminals restored", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/lazydb", type=Path)
    args = parser.parse_args()
    try:
        run(args.binary.expanduser().resolve())
    except BaseException as error:
        print("FAIL: BigQuery profile TUI acceptance (" + type(error).__name__ + ")", file=sys.stderr)
        if isinstance(error, SmokeFailure):
            print("Check: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
