#!/usr/bin/env python3
"""Real OpenSearch REST/NDJSON, confirmation, read-only and terminal acceptance.

Python 3.9+ stdlib, POSIX PTY and a built LazyDB binary suffice. Run separately
against the isolated None/HTTP and Basic/HTTPS fixtures. Uses repository ES PTY
primitives without changing the ES scripts. Diagnostics contain fixed labels only.
"""

import argparse
import base64
import importlib.util
import json
import os
import fcntl
import pty
import struct
import subprocess
import termios
from pathlib import Path
import signal
import sqlite3
import ssl
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid

_spec = importlib.util.spec_from_file_location(
    "lazydb_es_profile_pty", Path(__file__).resolve().with_name("test-elasticsearch-profile-tui.py"))
_helpers = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_helpers)
require, SmokeFailure = _helpers.require, _helpers.SmokeFailure


class Tui(_helpers.Tui):
    """Console fixture follows the existing ES LAZYDB_PASSWORD startup pattern."""

    def __init__(self, binary, root, secrets, password):
        self.secrets, self.root = secrets, root
        self.transcript = bytearray()
        self.screen = _helpers.Screen()
        self.master, slave = pty.openpty()
        self.original_termios = termios.tcgetattr(slave)
        self.proc = None
        try:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
            env = {key: value for key, value in os.environ.items() if not key.startswith("LAZYDB_")}
            env.update(LAZYDB_CONFIG_HOME=str(root), TERM="xterm-256color", NO_COLOR="1")
            if password:
                env["LAZYDB_PASSWORD"] = password
            self.proc = subprocess.Popen([str(binary), "--profile", "opensearch-tui", "--mouse", "off", "--color", "never"],
                stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)


def run(args, report):
    parts = urllib.parse.urlsplit(args.url)
    require(parts.scheme in ("opensearch", "opensearchs") and bool(parts.hostname)
            and parts.path in ("", "/") and not parts.query and not parts.fragment,
            "isolated OpenSearch fixture URL required")
    tls = parts.scheme == "opensearchs"
    require(bool(parts.username) == bool(parts.password), "Basic URL needs username and password")
    user = urllib.parse.unquote(parts.username or "")
    password = urllib.parse.unquote(parts.password or "")
    require(not tls or bool(args.ca_cert), "HTTPS fixture requires PEM CA")
    binary = Path(args.binary).resolve()
    require(binary.is_file() and os.access(binary, os.X_OK), "executable binary required")
    host = "[" + parts.hostname + "]" if ":" in parts.hostname else parts.hostname
    endpoint = f"{'https' if tls else 'http'}://{host}:{parts.port or 9200}"
    auth = base64.b64encode((user + ":" + password).encode()).decode() if user else ""
    secrets = [value for value in (password, auth) if value]
    context = ssl.create_default_context(cafile=args.ca_cert) if tls else None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
        urllib.request.HTTPSHandler(context=context))

    def api(method, path):
        headers = {"Authorization": "Basic " + auth} if user else {}
        request = urllib.request.Request(endpoint + path, method=method, headers=headers)
        try:
            with opener.open(request, timeout=10) as response:
                return response.status, None if method == "HEAD" else json.load(response)
        except urllib.error.HTTPError as error:
            status = error.code
            error.close()
            return status, None

    for read_only in (False, True):
        index = "lazydb-os-tui-" + uuid.uuid4().hex
        with tempfile.TemporaryDirectory(prefix="lazydb-os-console-") as temporary:
            root = Path(temporary)
            (root / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
            lines = ["version = 6", "[[profiles]]", f'id = "{uuid.uuid4()}"',
                'name = "opensearch-tui"', 'kind = "opensearch"',
                'url_format = "open-search' + ('-tls"' if tls else '"'),
                "host = " + json.dumps(parts.hostname), f"port = {parts.port or 9200}",
                'database = "default"', f"read_only = {str(read_only).lower()}",
                'ssl_mode = "verify-full"' if tls else 'ssl_mode = "disable"',
                "[profiles.credential_policy]", 'policy = "prompt"' if user else 'policy = "none"',
                "[profiles.catalog_scope.databases]", 'mode = "all"',
                "[profiles.opensearch_options]", 'auth = "basic"' if user else 'auth = "none"']
            if user:
                lines.insert(7, "user = " + json.dumps(user))
            if args.ca_cert:
                lines.append("ca_cert_path = " + json.dumps(str(Path(args.ca_cert).resolve())))
            (root / "connections.toml").write_text("\n".join(lines) + "\n")
            # The profile-form script separately verifies restart without secret env.
            tui = Tui(binary, root, secrets, password)
            successful = False

            def history(sql):
                path = root / "sql-history.sqlite3"
                if not path.exists():
                    return None
                try:
                    with sqlite3.connect(path) as db:
                        return db.execute("select status, language from history_executions where sql=? order by requested_at desc limit 1", (sql,)).fetchone()
                except sqlite3.Error:
                    return None

            def request(sql):
                tui.send(b"\x1b[17~")
                tui.wait(lambda: tui.contains("CONSOLES"), "console manager")
                tui.send(b"a")
                tui.wait(lambda: tui.contains("INSERT"), "new REST console")
                tui.send(b"\x1b[200~" + sql.encode() + b"\x1b[201~")
                tui.wait(lambda: tui.contains(sql.splitlines()[0]), "REST request paste")
                tui.send(b"\x1b")
                tui.wait(lambda: tui.contains("NORMAL"), "REST Normal mode")
                tui.send(b"\x1b[15~")

            try:
                report["phase"] = f"connect read_only={read_only}"
                tui.ready()
                tui.send(b"\r")
                tui.wait(lambda: tui.contains("●"), "OpenSearch connection")
                read = "GET /_cluster/health"
                request(read)
                tui.wait(lambda: history(read) == ("succeeded", "rest"), "REST query and history")
                require(tui.contains("REST CONSOLE"), "REST interaction model")
                write = f'PUT /{index}/_doc/1?refresh=wait_for\n{{"title":"OpenSearch TUI"}}'
                report["phase"] = f"write policy read_only={read_only}"
                request(write)
                if read_only:
                    tui.wait(lambda: tui.contains("read-only"), "read-only denial")
                    require(not tui.contains("Execute REST request?"), "no confirmation for denied write")
                    require(history(write) is None, "denied write absent from history")
                    require(api("HEAD", "/" + index)[0] == 404, "read-only request never dispatched")
                else:
                    tui.wait(lambda: tui.contains("Execute REST request?"), "REST write confirmation")
                    require(api("HEAD", "/" + index)[0] == 404, "confirmation precedes mutation")
                    require(history(write) is None, "confirmation precedes history dispatch")
                    tui.send(b"\t\r")
                    tui.wait(lambda: history(write) == ("succeeded", "rest"), "confirmed write history")
                    require(api("GET", f"/{index}/_doc/1")[1]["_source"]["title"] == "OpenSearch TUI",
                            "confirmed server document")
                    report["phase"] = "bulk and msearch NDJSON"
                    bulk = (f'POST /{index}/_bulk?refresh=wait_for\n'
                            '{"create":{"_id":"1"}}\n{"title":"conflict"}\n'
                            '{"index":{"_id":"2"}}\n{"title":"partial success"}')
                    request(bulk)
                    tui.wait(lambda: tui.contains("Execute REST request?"), "bulk confirmation")
                    tui.send(b"\t\r")
                    tui.wait(lambda: history(bulk) == ("failed", "rest"), "partial bulk failed history")
                    tui.wait(lambda: "bulk:1of2" in tui.screen.text().replace(" ", ""), "bulk failure summary")
                    require(api("GET", f"/{index}/_doc/2")[1]["_source"]["title"] == "partial success",
                            "successful bulk item committed")
                    multi = f'POST /{index}/_msearch\n{{}}\n{{"size":0}}\n{{"index":"{index}-missing"}}\n{{"size":0}}'
                    request(multi)
                    tui.wait(lambda: history(multi) == ("failed", "rest"), "partial msearch failed history")
                    tui.wait(lambda: "msearch:1of2" in tui.screen.text().replace(" ", ""), "msearch failure summary")
                successful = True
                report["passed"].append(f"read_only={read_only}")
                print(f"PASS: read_only={read_only}: REST history, write policy and server state", flush=True)
            finally:
                try:
                    tui.close(successful)
                    _helpers.check_disk_secrets(root, secrets)
                finally:
                    status, _ = api("DELETE", "/" + index)
                    require(status in (200, 404), "temporary console index cleanup")
                    report["cleanup"] = "test index removed; PTY stopped; private configuration removed on exit"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/lazydb")
    parser.add_argument("--url", default=os.environ.get("LAZYDB_TEST_OPENSEARCH_URL"))
    parser.add_argument("--ca-cert")
    parser.add_argument("--diagnostics-dir", type=Path)
    args = parser.parse_args()
    if not args.url:
        parser.error("set LAZYDB_TEST_OPENSEARCH_URL or --url")
    report = {"phase": "input validation", "passed": [], "cleanup": "not started"}

    def interrupted(signum, frame):
        raise SmokeFailure("acceptance interrupted")

    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        run(args, report)
    except BaseException as error:
        report["error_type"] = type(error).__name__
        if isinstance(error, SmokeFailure):
            report["check"] = str(error)
        print("FAIL: " + report["phase"] + " (" + report["error_type"] + ")", file=sys.stderr)
        if "check" in report:
            print("Check: " + report["check"], file=sys.stderr)
        if args.diagnostics_dir:
            args.diagnostics_dir.mkdir(parents=True, exist_ok=True)
            (args.diagnostics_dir / "opensearch-console-sanitized.json").write_text(json.dumps(report, indent=2) + "\n")
        return 1
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    print("PASS: credential masking and terminal restoration", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
