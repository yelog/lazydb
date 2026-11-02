#!/usr/bin/env python3
"""Accept the real Elasticsearch HTTPS profile form using only a disposable fixture.

Required: LAZYDB_TEST_ELASTICSEARCH_TLS_URL (or TLS_URL), an elasticsearchs://
URL containing fixture admin credentials, and LAZYDB_TEST_ELASTICSEARCH_TLS_CERT
(or TLS_CERT), its PEM CA. No Docker lifecycle operations are performed. Python
3.9+ and the standard library suffice. Diagnostics contain allowlisted facts only;
configuration, key files, HTTP responses and terminal transcripts are never saved.
"""

import argparse
import base64
import codecs
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import sqlite3
import ssl
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
import uuid


class SmokeFailure(Exception):
    """Messages are fixed check labels, never external exception text."""


def require(condition, label):
    if not condition:
        raise SmokeFailure(label)


class Screen:
    """Small VT screen reader for ratatui's absolute cursor updates, not a shell."""

    def __init__(self, rows=45, columns=160):
        self.rows, self.columns = rows, columns
        self.grid = [[" "] * columns for _ in range(rows)]
        self.row = self.column = 0
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith("\x1b["):
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if match is None:
                    return
                params, _, command = match.groups()
                self.pending = self.pending[match.end():]
                if params.startswith(("?", ">", "<", "=")):
                    continue
                values = [int(v or "0") for v in params.split(";")] if params else [0]
                n = values[0] or 1
                if command in ("H", "f"):
                    self.row = min(self.rows - 1, max(0, n - 1))
                    self.column = min(self.columns - 1, max(0, (values[1] if len(values) > 1 else 1) - 1))
                elif command == "A":
                    self.row = max(0, self.row - n)
                elif command == "B":
                    self.row = min(self.rows - 1, self.row + n)
                elif command == "C":
                    self.column = min(self.columns - 1, self.column + n)
                elif command == "D":
                    self.column = max(0, self.column - n)
                elif command == "G":
                    self.column = min(self.columns - 1, n - 1)
                elif command == "J" and values[0] in (2, 3):
                    self.grid = [[" "] * self.columns for _ in range(self.rows)]
                elif command == "K":
                    start = 0 if values[0] in (1, 2) else self.column
                    end = self.columns if values[0] in (0, 2) else self.column + 1
                    self.grid[self.row][start:end] = [" "] * (end - start)
                continue
            if self.pending.startswith("\x1b]"):
                match = re.match(r"\x1b\].*?(?:\x07|\x1b\\)", self.pending, re.S)
                if match is None:
                    return
                self.pending = self.pending[match.end():]
                continue
            char = self.pending[0]
            if char == "\x1b":
                if len(self.pending) < 2:
                    return
                self.pending = self.pending[2:]
                continue
            self.pending = self.pending[1:]
            if char == "\r":
                self.column = 0
            elif char == "\n":
                self.row = min(self.rows - 1, self.row + 1)
            elif char >= " " and char != "\x7f":
                width = 0 if unicodedata.combining(char) else (2 if unicodedata.east_asian_width(char) in ("F", "W") else 1)
                if width:
                    self.grid[self.row][self.column] = char
                    self.column = min(self.columns - 1, self.column + width)

    def text(self):
        return "\n".join("".join(row) for row in self.grid)


class Tui:
    def __init__(self, binary, root, secrets, profile=None):
        self.secrets = secrets
        self.root = root
        self.transcript = bytearray()
        self.screen = Screen()
        self.master, slave = pty.openpty()
        self.original_termios = termios.tcgetattr(slave)
        self.proc = None
        try:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
            env = os.environ.copy()
            # Neither startup nor a credential prompt may obtain a test secret from env.
            for key in list(env):
                if key.startswith("LAZYDB_") or key in ("TLS_URL", "TLS_CERT"):
                    env.pop(key)
            env.update(LAZYDB_CONFIG_HOME=str(root), TERM="xterm-256color", NO_COLOR="1")
            command = [str(binary), "--mouse", "off", "--color", "never"]
            if profile:
                command += ["--profile", profile]
            self.proc = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                         env=env, start_new_session=True)
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)

    def pump(self, timeout=0.05):
        if select.select([self.master], [], [], timeout)[0]:
            try:
                chunk = os.read(self.master, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    return
                raise
            self.transcript.extend(chunk)
            self.screen.feed(chunk)
            # Crossterm may ask for the cursor position during startup.
            if b"\x1b[6n" in self.transcript[-max(16, len(chunk) + 4):]:
                os.write(self.master, b"\x1b[1;1R")
            require(len(self.transcript) <= 32 * 1024 * 1024, "terminal output exceeded limit")

    def wait(self, predicate, label, timeout=25):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump()
            if predicate():
                return
            if self.proc.poll() is not None:
                # The process can exit between predicate() and poll().
                if predicate():
                    return
                raise SmokeFailure("application exited during " + label)
        raise SmokeFailure("timed out during " + label)

    def contains(self, text):
        return text.casefold() in self.screen.text().casefold()

    def ready(self):
        self.wait(lambda: not (termios.tcgetattr(self.master)[3] & termios.ICANON), "raw terminal startup")
        self.wait(lambda: self.contains("EXPLORER"), "explorer startup")

    def send(self, data):
        os.write(self.master, data)
        # Allow escape-sequence disambiguation and one render before further keys.
        deadline = time.monotonic() + 0.12
        while time.monotonic() < deadline:
            self.pump(0.02)

    def tabs(self, count):
        for _ in range(count):
            self.send(b"\t")

    def fill(self, text):
        self.send(b"\x05\x15")  # Ctrl-E then Ctrl-U clears the whole current field.
        if text:
            self.send(b"\x1b[200~" + text.encode() + b"\x1b[201~")

    def test_ok(self):
        self.send(b"\x14")
        self.wait(lambda: self.contains("Connection verified:"), "form connection verification")

    def save(self):
        self.send(b"\r")
        self.wait(lambda: not self.contains("EDIT CONNECTION") and not self.contains("NEW CONNECTION"),
                  "form save")

    def edit(self):
        self.send(b"e")
        self.wait(lambda: self.contains("EDIT CONNECTION"), "edit saved profile")

    def connect(self):
        self.send(b"\r")
        self.wait(lambda: self.contains("●"), "saved profile reconnect")

    def history_count(self, sql):
        path = self.root / "sql-history.sqlite3"
        if not path.exists():
            return 0
        try:
            with sqlite3.connect(str(path)) as db:
                return db.execute("select count(*) from history_executions where sql=? and status='succeeded' and language='rest'",
                                  (sql,)).fetchone()[0]
        except sqlite3.Error:
            return 0

    def read(self):
        sql = "GET /_cluster/health"
        before = self.history_count(sql)
        self.send(b"\x1b[17~")  # F6, add a real REST console, paste, Normal, F5.
        self.wait(lambda: self.contains("CONSOLES"), "console manager")
        self.send(b"a")
        self.wait(lambda: self.contains("INSERT"), "new REST console")
        self.send(b"\x1b[200~" + sql.encode() + b"\x1b[201~")
        self.wait(lambda: self.contains(sql), "REST read paste")
        self.send(b"\x1b")
        self.wait(lambda: self.contains("NORMAL"), "REST Normal mode")
        self.send(b"\x1b[15~")
        self.wait(lambda: self.history_count(sql) > before, "persisted credential REST read")

    def no_secrets(self):
        # Check both raw bytes and visible text; ANSI cursor changes can split a leak.
        raw = self.transcript.decode(errors="replace")
        plain = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", raw)
        for secret in self.secrets:
            require(secret not in raw and secret not in plain and secret not in self.screen.text(),
                    "credential appeared in terminal output")

    def close(self, successful):
        failure = None
        try:
            if self.proc.poll() is None:
                self.send(b"\x03")
                self.wait(lambda: self.proc.poll() is not None, "clean application exit", 8)
            self.pump(0)
            if successful:
                require(self.proc.returncode == 0, "application exit status")
                require(b"\x1b[?1049l" in self.transcript, "alternate screen restoration")
                require(bool(termios.tcgetattr(self.master)[3] & termios.ICANON), "terminal canonical mode restoration")
            self.no_secrets()
        except BaseException as error:
            failure = error
        finally:
            try:
                if self.proc.poll() is None:
                    os.killpg(self.proc.pid, signal.SIGTERM)
                    try:
                        self.proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(self.proc.pid, signal.SIGKILL)
                        self.proc.wait(timeout=5)
            finally:
                try:
                    termios.tcsetattr(self.master, termios.TCSANOW, self.original_termios)
                finally:
                    os.close(self.master)
        if failure:
            raise failure


def profile_metadata(root):
    """Read selected generated TOML fields; never write profile configuration.

    On 3.11+ use tomllib. The 3.9 fallback deliberately reads only scalar keys
    from this app's single-profile TOML output, rejecting ambiguous duplicates.
    It is not a general TOML parser.
    """
    path = root / "connections.toml"
    require(path.exists(), "saved profile metadata exists")
    text = path.read_text()
    try:
        import tomllib
    except ImportError:
        sections = {}
        section = None
        profiles = 0
        for line in text.splitlines():
            line = line.strip()
            if line == "[[profiles]]":
                profiles += 1
                section = "profiles"
            elif line.startswith("["):
                section = line.strip("[]")
            elif section and "=" in line:
                key, value = (part.strip() for part in line.split("=", 1))
                entry = (section, key)
                require(entry not in sections, "unambiguous generated TOML fields")
                if value.startswith('"'):
                    sections[entry] = json.loads(value)
                elif value in ("true", "false"):
                    sections[entry] = value == "true"
                elif re.fullmatch(r"[0-9]+", value):
                    sections[entry] = int(value)
        require(profiles == 1, "exactly one saved profile")
        return sections
    parsed = tomllib.loads(text)
    require(len(parsed.get("profiles", [])) == 1, "exactly one saved profile")
    result = {}

    def flatten(prefix, value):
        for key, item in value.items():
            if isinstance(item, dict):
                flatten(prefix + "." + key, item)
            else:
                result[(prefix, key)] = item

    flatten("profiles", parsed["profiles"][0])
    return result


def check_profile(root, auth, ca_cert, name, host, port):
    values = profile_metadata(root)
    expected = {
        ("profiles", "name"): name, ("profiles", "kind"): "elasticsearch",
        ("profiles", "host"): host, ("profiles", "port"): port,
        ("profiles", "url_format"): "elasticsearch-tls",
        ("profiles", "ssl_mode"): "verify-full",
        ("profiles.credential_policy", "policy"): "local_encrypted",
        ("profiles.elasticsearch_options", "auth"): auth,
    }
    for field, value in expected.items():
        require(values.get(field) == value, "persisted profile field " + field[1])
    require(values.get(("profiles.elasticsearch_options", "ca_cert_path")) == ca_cert,
            "persisted CA metadata")
    require(bool(values.get(("profiles.credential_policy.reference", "ciphertext"))),
            "encrypted credential payload persisted")
    key = root / "credential.key"
    require(key.is_file() and key.stat().st_mode & 0o077 == 0, "private local encryption key")
    if auth == "api_key":
        require(not values.get(("profiles", "user")), "API key profile has no Basic user")
    return values


def check_disk_secrets(root, secrets):
    for path in root.rglob("*"):
        if path.is_file() and (path.suffix == ".toml" or "history" in path.name or path.parent.name == "sql"):
            content = path.read_bytes()
            for secret in secrets:
                require(secret.encode() not in content, "credential absent from metadata and history")
    path = root / "sql-history.sqlite3"
    if path.exists():
        with sqlite3.connect(str(path)) as db:
            require(db.execute("select count(*) from history_executions where sql like '%/_security/api_key%'").fetchone()[0] == 0,
                    "API key administration absent from application history")


def run(args, report):
    parts = urllib.parse.urlsplit(args.url)
    require(parts.scheme == "elasticsearchs" and bool(parts.hostname) and bool(parts.username) and bool(parts.password)
            and parts.path in ("", "/") and not parts.query and not parts.fragment,
            "TLS fixture URL with disposable admin credentials required")
    host = parts.hostname
    user, password = urllib.parse.unquote(parts.username), urllib.parse.unquote(parts.password)
    port = parts.port or 9200
    require(not any(ord(c) < 32 for c in user + password + host), "fixture input contains control characters")
    cert = Path(args.ca_cert).expanduser().resolve()
    binary = Path(args.binary).expanduser().resolve()
    require(cert.is_file(), "TLS fixture certificate file required")
    require(binary.is_file() and os.access(str(binary), os.X_OK), "executable lazydb binary required")
    endpoint = "https://{}:{}".format("[" + host + "]" if ":" in host else host, port)
    context = ssl.create_default_context(cafile=str(cert))
    basic = base64.b64encode((user + ":" + password).encode()).decode()
    secrets = [password, basic, args.url]
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                         urllib.request.HTTPSHandler(context=context))

    def api(method, path, body=None):
        request = urllib.request.Request(endpoint + path, method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": "Basic " + basic, "Content-Type": "application/json"})
        try:
            with opener.open(request, timeout=10) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            # Do not retain or expose a server error response.
            error.close()
            return error.code, {}

    key_id = None
    with tempfile.TemporaryDirectory(prefix="lazydb-es-profile-tui-") as temporary:
        root = Path(temporary)
        # Settings are the only input file: every profile field comes from the TUI.
        (root / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
        name = "es-form-" + uuid.uuid4().hex[:12]
        sessions = []

        def phase(label, operation, profile=None):
            report["phase"] = label
            tui = Tui(binary, root, secrets, profile)
            sessions.append(tui)
            successful = False
            try:
                tui.ready()
                operation(tui)
                successful = True
            finally:
                tui.close(successful)
                check_disk_secrets(root, secrets)
            report["passed"].append(label)
            print("PASS: " + label, flush=True)

        try:
            report["phase"] = "temporary expiring API key creation"
            status, key = api("POST", "/_security/api_key", {
                "name": name, "expiration": "15m", "role_descriptors": {"profile_acceptance": {
                    "cluster": ["monitor"],
                    "indices": [{"names": ["*"], "privileges": ["read", "view_index_metadata"]}],
                }},
            })
            require(status == 200 and bool(key.get("id")), "temporary API key creation")
            key_id = key["id"]
            credential = key.get("encoded")
            require(isinstance(credential, str) and bool(credential), "encoded temporary API key")
            secrets += [credential, key_id]
            if key.get("api_key"):
                secrets.append(key["api_key"])
            del key
            # Establish that the fixture really requires the supplied custom CA.
            report["phase"] = "unknown CA fixture precondition"
            untrusted = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                urllib.request.HTTPSHandler(context=ssl.create_default_context()))
            try:
                with untrusted.open(endpoint, timeout=10):
                    raise SmokeFailure("fixture unexpectedly trusted without custom CA")
            except urllib.error.URLError as error:
                require(isinstance(error.reason, ssl.SSLCertVerificationError), "fixture unknown CA rejection")

            def create(tui):
                require(not (root / "connections.toml").exists(), "empty profile configuration")
                tui.send(b"\r")
                tui.wait(lambda: tui.contains("NEW CONNECTION"), "new connection form")
                # Select by visible identity so adding another driver cannot
                # silently redirect this acceptance flow to a different product.
                tui.send(b"\x1b[Z")
                tui.send(b"\x1b[C")
                tui.tabs(1)
                for _ in range(8):
                    if tui.contains("Elasticsearch PROFILE"):
                        break
                    tui.send(b"\x1b[C")
                require(tui.contains("Elasticsearch PROFILE"), "Elasticsearch driver selection")
                tui.tabs(1)
                tui.fill(name)
                tui.tabs(1)
                tui.fill(host)
                tui.tabs(1)
                tui.fill(str(port))
                tui.tabs(3)  # Namespace, visible objects, authentication.
                tui.send(b"\x1b[C")
                tui.wait(lambda: tui.contains("API Key"), "API key authentication selection")
                tui.tabs(1)
                tui.fill(credential)
                tui.tabs(1)
                require(tui.contains("Local encrypted"), "local encrypted storage selected")
                tui.tabs(1)
                tui.send(b"\x1b[C")  # Disable -> VerifyFull, also updates URL scheme.
                tui.tabs(1)
                tui.fill(str(cert))
                tui.test_ok()
                tui.save()
                check_profile(root, "api_key", str(cert), name, host, port)

            phase("create API Key HTTPS profile; test and save local encrypted", create)
            phase("restart without LAZYDB_PASSWORD; API key decrypts and REST read succeeds",
                  lambda tui: (tui.connect(), tui.read()), name)

            def basic_edit(tui):
                tui.edit()
                tui.tabs(7)  # Edit starts at Category; authentication is field 7.
                tui.send(b"\x1b[C")
                tui.wait(lambda: tui.contains("Basic Auth") and tui.contains("Not set"),
                         "auth switch clears previous credential")
                require(not tui.contains("Stored locally"), "API key cannot be preserved as Basic password")
                tui.tabs(1)
                tui.fill(user)
                # Real connection attempt with the old key cleared must fail authentication.
                tui.send(b"\x14")
                tui.wait(lambda: tui.contains("HTTP 401"), "Basic authentication requires new password")
                tui.tabs(1)
                tui.fill(password)
                tui.test_ok()
                tui.save()
                values = check_profile(root, "basic", str(cert), name, host, port)
                require(values.get(("profiles", "user")) == user, "saved Basic user")

            phase("edit to Basic requires new credentials; test and save", basic_edit)
            phase("restart; encrypted Basic credentials reconnect and REST read succeeds",
                  lambda tui: (tui.connect(), tui.read()), name)

            def clear_ca(tui):
                tui.edit()
                tui.tabs(12)
                tui.fill("")
                tui.save()
                check_profile(root, "basic", None, name, host, port)

            phase("clear CA in form; saved metadata removes CA", clear_ca)

            def reject_ca(tui):
                before = tui.history_count("GET /_cluster/health")
                tui.send(b"\r")
                tui.wait(lambda: tui.contains("error sending request") or tui.contains("certificate"),
                         "saved profile rejects untrusted HTTPS")
                tui.wait(lambda: tui.contains("FAILED"), "untrusted HTTPS profile marked failed")
                require(tui.history_count("GET /_cluster/health") == before,
                        "untrusted connection adds no successful REST reads")

            phase("restart; saved profile rejects endpoint with unknown CA", reject_ca, name)

            def restore_ca(tui):
                tui.edit()
                tui.tabs(12)
                tui.fill(str(cert))
                tui.test_ok()
                tui.save()
                check_profile(root, "basic", str(cert), name, host, port)

            phase("restore CA through form; test and save", restore_ca)
            phase("restart; restored CA reconnects and REST read succeeds",
                  lambda tui: (tui.connect(), tui.read()), name)
            report["terminal_restoration"] = "all completed sessions exited cleanly and restored terminal"
        finally:
            report["cleanup"] = "pending"
            try:
                for tui in sessions:
                    tui.no_secrets()
                check_disk_secrets(root, secrets)
            finally:
                if key_id is not None:
                    status, result = api("DELETE", "/_security/api_key", {"ids": [key_id]})
                    require(status == 200 and key_id in result.get("invalidated_api_keys", []),
                            "temporary API key invalidation")
                    print("PASS: temporary API key invalidated", flush=True)
                report["cleanup"] = "temporary key invalidated; app processes stopped; private config removed on exit"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/lazydb")
    parser.add_argument("--url", default=os.environ.get("LAZYDB_TEST_ELASTICSEARCH_TLS_URL") or os.environ.get("TLS_URL"))
    parser.add_argument("--ca-cert", default=os.environ.get("LAZYDB_TEST_ELASTICSEARCH_TLS_CERT") or os.environ.get("TLS_CERT"))
    parser.add_argument("--diagnostics-dir", type=Path,
                        help="save only allowlisted failure facts, never terminal/config/HTTP data")
    args = parser.parse_args()
    if not args.url or not args.ca_cert:
        parser.error("set LAZYDB_TEST_ELASTICSEARCH_TLS_URL and LAZYDB_TEST_ELASTICSEARCH_TLS_CERT (or TLS_URL/TLS_CERT)")
    report = {"phase": "input validation", "passed": [], "cleanup": "not started"}

    def interrupted(signum, frame):
        raise SmokeFailure("acceptance interrupted")

    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        run(args, report)
    except BaseException as error:
        # Do not print str(error), traceback, response bodies or screen contents:
        # arbitrary external errors may contain credentials, URLs or private paths.
        report["error_type"] = type(error).__name__
        if isinstance(error, SmokeFailure):
            report["check"] = str(error)
        print("FAIL: " + report["phase"] + " (" + report["error_type"] + ")", file=sys.stderr)
        if "check" in report:
            print("Check: " + report["check"], file=sys.stderr)
        if args.diagnostics_dir:
            try:
                args.diagnostics_dir.mkdir(parents=True, exist_ok=True)
                path = args.diagnostics_dir / "profile-tui-sanitized.json"
                path.write_text(json.dumps(report, indent=2) + "\n")
                print("Saved allowlisted profile acceptance diagnostics.", file=sys.stderr)
            except OSError:
                print("Unable to save sanitized diagnostics.", file=sys.stderr)
        return 1
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    print("PASS: credentials absent from terminal, metadata and history; all sessions restore terminal", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
