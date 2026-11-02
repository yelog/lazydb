#!/usr/bin/env python3
"""Exercise BigQuery connect/catalog/query in a real PTY against a local fixture.

Uses only a cfg(test) child entry and fake credentials. Production binaries have
no endpoint override. Pass --test-binary or let cargo discover the library test
executable. The fixture listens on 127.0.0.1 and all configuration is temporary.
"""

import argparse
import fcntl
import http.server
import importlib.util
import json
import os
from pathlib import Path
import pty
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import urllib.parse

SPEC = importlib.util.spec_from_file_location(
    "bigquery_profile_harness", Path(__file__).with_name("test-bigquery-profile-tui.py"))
PROFILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILE)
HARNESS = PROFILE.HARNESS
require, SmokeFailure = HARNESS.require, HARNESS.SmokeFailure
CHILD = "runtime::tests::bigquery_pty_fixture_child"
SQL = "SELECT 42 AS fixture_answer"


class Fixture(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    jobs = {}
    requests = []

    def log_message(self, *_args):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))) or b"{}")
        self.requests.append(("POST", self.path, body))
        reference = body.get("jobReference", {})
        self.jobs[reference.get("jobId")] = body
        self.reply({"jobReference": reference, "status": {"state": "DONE"},
                    "statistics": {"query": {"totalBytesProcessed": "12", "totalBytesBilled": "24",
                                             "totalSlotMs": "5", "cacheHit": False}}})

    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        self.requests.append(("GET", self.path, {}))
        parts = path.split("/")
        if "/queries/" in path or "/jobs/" in path:
            job_id = parts[-1]
            job = self.jobs[job_id]
            reference = job["jobReference"]
            if "/jobs/" in path:
                self.reply({"jobReference": reference, "status": {"state": "DONE"},
                            "statistics": {"query": {"totalBytesProcessed": "12", "totalBytesBilled": "24",
                                                     "totalSlotMs": "5", "cacheHit": False}}})
            else:
                sql = job["configuration"]["query"]["query"]
                ddl = "INFORMATION_SCHEMA" in sql
                self.reply({"jobReference": reference, "jobComplete": True, "totalRows": "1",
                            "schema": {"fields": [{"name": "ddl" if ddl else "fixture_answer",
                                                    "type": "STRING" if ddl else "INTEGER"}]},
                            "rows": [{"f": [{"v": "CREATE TABLE `data-project.fixture.events` (fixture_answer INT64);"
                                              if ddl else "42"}]}]})
        elif path.endswith("/projects"):
            self.reply({"projects": [{"projectReference": {"projectId": "data-project"}, "friendlyName": "Fixture data project"}]})
        elif path.endswith("/datasets"):
            self.reply({"datasets": [{"datasetReference": {"projectId": "data-project", "datasetId": "fixture"}}]})
        elif path.endswith("/tables"):
            self.reply({"tables": [self.table()]})
        elif "/tables/events" in path:
            self.reply(self.table())
        elif path.endswith("/datasets/fixture"):
            self.reply({"datasetReference": {"projectId": "data-project", "datasetId": "fixture"}, "location": "US"})
        else:
            self.reply({})

    @staticmethod
    def table():
        return {"tableReference": {"projectId": "data-project", "datasetId": "fixture", "tableId": "events"},
                "type": "TABLE", "location": "US", "schema": {"fields": [
                    {"name": "fixture_answer", "type": "INTEGER", "mode": "NULLABLE"}]}}


class Tui(HARNESS.Tui):
    def __init__(self, binary, root, endpoint):
        self.secrets = []
        self.root = root
        self.transcript = bytearray()
        self.screen = PROFILE.Screen()
        self.master, slave = pty.openpty()
        self.original_termios = termios.tcgetattr(slave)
        self.proc = None
        try:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
            env = {key: value for key, value in os.environ.items() if not key.startswith("LAZYDB_")}
            env.update(LAZYDB_CONFIG_HOME=str(root), HOME=str(root), TERM="xterm-256color", NO_COLOR="1",
                       LAZYDB_TEST_BIGQUERY_PTY_ENDPOINT=endpoint,
                       GOOGLE_APPLICATION_CREDENTIALS=str(root / "missing-adc.json"),
                       CLOUDSDK_CONFIG=str(root / "gcloud"), GCE_METADATA_HOST="127.0.0.1:1")
            self.proc = subprocess.Popen([str(binary), "--exact", CHILD, "--ignored", "--nocapture"],
                                         stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)

    def sql_succeeded(self):
        path = self.root / "sql-history.sqlite3"
        if not path.exists():
            return False
        try:
            with sqlite3.connect(str(path)) as db:
                return bool(db.execute("SELECT COUNT(*) FROM history_executions WHERE sql=? AND status='succeeded'", (SQL,)).fetchone()[0])
        except sqlite3.Error:
            return False


def test_binary():
    result = subprocess.run(["cargo", "test", "--all-features", "--lib", "--no-run", "--message-format=json"],
                            capture_output=True, text=True, check=True)
    for line in reversed(result.stdout.splitlines()):
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message.get("executable") and message.get("profile", {}).get("test"):
            return Path(message["executable"])
    raise SmokeFailure("library test executable discovery")


def run(binary):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="lazydb-bigquery-tui-") as temporary:
            root = Path(temporary)
            (root / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
            tui = Tui(binary, root, "http://127.0.0.1:%d/bigquery/v2/" % server.server_port)
            successful = False
            try:
                tui.ready()
                tui.send(b"\r")
                tui.wait(lambda: tui.contains("NEW CONNECTION"), "new connection form")
                for _ in range(5):
                    tui.send(b"\x1b[Z")
                tui.fill("bigquery://billing-project/fixture?data_project=data-project&location=US")
                tui.wait(lambda: tui.contains("BigQuery PROFILE"), "BigQuery URL form")
                tui.tabs(6)
                tui.fill("BigQuery fixture")
                tui.test_ok()
                tui.save()
                tui.wait(lambda: any("/datasets" in path for _, path, _ in Fixture.requests), "dataset catalog request")
                tui.wait(lambda: tui.contains("data-project"), "data project catalog render")
                print("PASS: real TUI verifies and connects separate billing/data projects", flush=True)
                # Expand project, dataset and table group through normal Explorer keys.
                for _ in range(8):
                    tui.send(b"\x1b[C")
                    tui.send(b"j")
                    if tui.contains("events"):
                        break
                tui.wait(lambda: tui.contains("events"), "table catalog render")
                print("PASS: real Explorer loads projects, datasets and tables", flush=True)
                tui.send(b"p")
                tui.wait(lambda: any("`data-project.fixture.events`" in body.get("configuration", {}).get("query", {}).get("query", "")
                                     for method, _, body in Fixture.requests if method == "POST"), "table preview job")
                tui.wait(lambda: tui.contains("fixture_answer") and tui.contains("42"), "relation data preview")
                tui.send(b"D")
                tui.wait(lambda: any("INFORMATION_SCHEMA" in body.get("configuration", {}).get("query", {}).get("query", "")
                                     for method, _, body in Fixture.requests if method == "POST"), "native DDL job")
                tui.wait(lambda: tui.contains("CREATE TABLE"), "native DDL rendering")
                print("PASS: real relation preview and native DDL render through billing project", flush=True)
                tui.send(b"\x1b[17~")  # F6 console manager.
                tui.wait(lambda: tui.contains("CONSOLES"), "console manager")
                tui.send(b"a")
                tui.wait(lambda: tui.contains("INSERT"), "new SQL console")
                tui.send(b"\x1b[200~" + SQL.encode() + b"\x1b[201~")
                tui.send(b"\x1b")
                tui.send(b"\x1b[15~")  # F5 execute.
                tui.wait(tui.sql_succeeded, "SQL result recorded")
                tui.wait(lambda: tui.contains("fixture_answer") and tui.contains("42"), "SQL result grid")
                require(any(body.get("configuration", {}).get("query", {}).get("query") == SQL
                            and body.get("jobReference", {}).get("projectId") == "billing-project"
                            for method, _, body in Fixture.requests if method == "POST"), "query billed to execution project")
                print("PASS: real SQL console submits native job and renders results", flush=True)
                successful = True
            finally:
                tui.close(successful)
    finally:
        server.shutdown()
        server.server_close()
    print("PASS: isolated fixture and terminal cleaned up", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path)
    args = parser.parse_args()
    try:
        run((args.test_binary or test_binary()).expanduser().resolve())
    except BaseException as error:
        print("FAIL: BigQuery TUI acceptance (" + type(error).__name__ + ")", file=sys.stderr)
        if isinstance(error, SmokeFailure):
            print("Check: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
