#!/usr/bin/env python3
"""Accept real OpenSearch Basic/HTTPS and None/HTTP profile forms.

Uses only Python 3.9+ stdlib and the existing ES PTY primitives. No profile input
files are synthesized: create, test, save, edit and encrypted restart use the UI.
Diagnostics contain fixed check labels only. Docker lifecycle is external.
"""

import argparse
import base64
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import signal
import ssl
import struct
import termios
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

# Load trusted repository helpers by absolute path; -I needs no sys.path changes.
_spec = importlib.util.spec_from_file_location(
    "lazydb_es_profile_pty", Path(__file__).resolve().with_name("test-elasticsearch-profile-tui.py"))
_helpers = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_helpers)
Tui, require, SmokeFailure = _helpers.Tui, _helpers.require, _helpers.SmokeFailure


def check_profile(root, name, host, port, ca_cert, read_only=False, no_auth=False):
    values = _helpers.profile_metadata(root)
    expected = {
        ("profiles", "name"): name, ("profiles", "kind"): "opensearch",
        ("profiles", "host"): host, ("profiles", "port"): port,
        ("profiles", "url_format"): "open-search" if no_auth else "open-search-tls",
        ("profiles", "ssl_mode"): "disable" if no_auth else "verify-full",
        ("profiles", "read_only"): read_only,
        ("profiles.opensearch_options", "auth"): "none" if no_auth else "basic",
    }
    for field, value in expected.items():
        require(values.get(field) == value, "persisted OpenSearch field " + field[1])
    require(values.get(("profiles.opensearch_options", "ca_cert_path")) == ca_cert,
            "persisted OpenSearch CA metadata")
    require(not any(section.startswith("profiles.elasticsearch_options") for section, _ in values),
            "independent OpenSearch options")
    if no_auth:
        require(values.get(("profiles.credential_policy", "policy")) == "none",
                "None uses passwordless credential policy")
    else:
        require(values.get(("profiles.credential_policy", "policy")) == "local_encrypted",
                "encrypted Basic storage")
        require(bool(values.get(("profiles.credential_policy.reference", "ciphertext"))),
                "encrypted Basic payload")
        key = root / "credential.key"
        require(key.is_file() and key.stat().st_mode & 0o077 == 0, "private local key")
    return values


def run(args, report):
    parts = urllib.parse.urlsplit(args.url)
    require(parts.scheme == "opensearchs" and parts.hostname == "localhost"
            and bool(parts.username) and bool(parts.password) and parts.path in ("", "/")
            and not parts.query and not parts.fragment, "isolated localhost HTTPS admin URL required")
    user, password = urllib.parse.unquote(parts.username), urllib.parse.unquote(parts.password)
    port = parts.port or 9200
    cert = Path(args.ca_cert).expanduser().resolve()
    binary = Path(args.binary).expanduser().resolve()
    require(cert.is_file(), "PEM CA file required")
    require(binary.is_file() and os.access(binary, os.X_OK), "executable lazydb binary required")
    auth = base64.b64encode((user + ":" + password).encode()).decode()
    secrets = [password, auth, args.url]
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
        urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(cert))))
    index = "lazydb-form-" + uuid.uuid4().hex

    def api(method, path, body=None):
        request = urllib.request.Request(f"https://localhost:{port}" + path, method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": "Basic " + auth, "Content-Type": "application/json"})
        try:
            with opener.open(request, timeout=10) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            status = error.code
            error.close()
            return status, {}

    with tempfile.TemporaryDirectory(prefix="lazydb-os-profile-tui-") as temporary:
        root = Path(temporary)
        (root / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
        name = "os-form-" + uuid.uuid4().hex[:12]

        def phase(label, operation):
            report["phase"] = label
            tui = Tui(binary, root, secrets, name if (root / "connections.toml").exists() else None)
            successful = False
            try:
                tui.ready()
                if (root / "connections.toml").exists():
                    tui.send(b" c")
                    tui.send(b"gg")
                operation(tui)
                successful = True
            finally:
                tui.close(successful)
                _helpers.check_disk_secrets(root, secrets)
            report["passed"].append(label)
            print("PASS: " + label, flush=True)

        def create(tui):
            require(not (root / "connections.toml").exists(), "empty profile configuration")
            tui.send(b"\r")
            tui.wait(lambda: tui.contains("NEW CONNECTION"), "new connection form")
            tui.send(b"\x1b[Z")
            tui.send(b"\x1b[C")  # Category -> non-relational.
            tui.tabs(1)
            for _ in range(8):
                if tui.contains("OpenSearch PROFILE"):
                    break
                tui.send(b"\x1b[C")
            require(tui.contains("OpenSearch PROFILE"), "OpenSearch driver selection")
            require(not tui.contains("API Key"), "OpenSearch has no API key mode")
            tui.tabs(1)
            tui.fill(name)
            tui.tabs(1)
            tui.fill("localhost")
            tui.tabs(1)
            tui.fill(str(port))
            tui.tabs(1)
            tui.fill("default")
            tui.tabs(3)  # Visible objects, Basic auth, user.
            tui.fill(user)
            tui.tabs(1)
            tui.fill(password)
            tui.tabs(1)
            require(tui.contains("Local encrypted"), "local encrypted storage selected")
            tui.tabs(1)
            tui.send(b"\x1b[C")
            tui.tabs(1)
            tui.fill(str(cert))
            tui.test_ok()
            tui.save()
            values = check_profile(root, name, "localhost", port, str(cert))
            require(values.get(("profiles", "user")) == user, "saved Basic user")

        def browse(tui):
            tui.connect()
            tui.send(b" c")  # Restored console may own focus; explicitly focus Explorer.
            tui.send(b"gg")
            # Server-backed Explorer search locates this exact index among plugin indices.
            tui.send(b"f")
            tui.send(index.encode())
            tui.wait(lambda: tui.contains("1 results"), "Explorer index search")
            tui.send(b"\r")
            tui.send(b"\x1b")
            tui.send(b"l")
            tui.wait(lambda: tui.contains("title"), "mapping field discovery")
            tui.send(b"p")
            tui.wait(lambda: tui.contains("_source") and tui.contains("1-1 of 1"),
                     "real document preview")
            # Resize the actual PTY while preserving the selected relation and
            # JSON columns. Fresh VT projections avoid matching stale screen text.
            for columns in (100, 120, 160):
                tui.wait(lambda: tui.contains("OpenSearch profile preview")
                         or tui.contains("_index"), "preview before terminal resize")
                tui.screen = _helpers.Screen(rows=45, columns=columns)
                fcntl.ioctl(tui.master, termios.TIOCSWINSZ,
                            struct.pack("HHHH", 45, columns, 0, 0))
                os.killpg(tui.proc.pid, signal.SIGWINCH)
                # Drain the resize redraw before keys can coalesce with SIGWINCH.
                resize_deadline = time.monotonic() + 0.5
                while time.monotonic() < resize_deadline:
                    tui.pump(0.02)
                tui.send(b"0")
                tui.wait(lambda: tui.contains("_index") and tui.contains("1-1 of 1"),
                         "preview metadata after terminal resize")
                tui.send(b"$")  # Title follows metadata and nested source columns.
                tui.wait(lambda: tui.contains("OpenSearch profile preview"),
                         "preview document value after terminal resize")
            tui.send(b"v")
            tui.wait(lambda: tui.contains("Enter view value"), "record inspector")
            tui.send(b"jjj")  # Record view starts at _index; select complete _source.
            tui.send(b"\r")
            tui.wait(lambda: tui.contains('"author"') and tui.contains('"payload"')
                     and tui.contains("Example"), "nested full source remains inspectable")
            tui.send(b"l" * 200)
            tui.wait(lambda: tui.contains("long-value-after-preview-limit"),
                     "full source scrolls beyond the grid preview")
            tui.send(b"\x1b")
            tui.send(b"\x1b")

        try:
            status, _ = api("PUT", "/" + index, {"mappings": {"properties": {
                "title": {"type": "keyword"},
                "author": {"type": "nested", "properties": {"name": {"type": "keyword"}}},
                "payload": {"type": "text", "index": False},
            }}})
            require(status == 200, "temporary browser index creation")
            status, _ = api("PUT", f"/{index}/_doc/1?refresh=wait_for", {
                "title": "OpenSearch profile preview", "author": [{"name": "Example"}],
                "payload": "x" * 180 + "long-value-after-preview-limit-" + "preview-long-document-" * 3000,
            })
            require(status == 201, "temporary preview document creation")
            phase("create Basic HTTPS form; test and save encrypted", create)
            phase("restart; recover Basic credential and execute REST read", lambda tui: (tui.connect(), tui.read()))
            phase("restart; browse indices and mapping; preview document", browse)

            def clear_ca(tui):
                tui.edit()
                tui.tabs(12)
                tui.fill("")
                tui.save()
                check_profile(root, name, "localhost", port, None)
            phase("clear CA in real form and save", clear_ca)

            def rejected(tui):
                tui.send(b"\r")
                tui.wait(lambda: tui.contains("FAILED"), "untrusted HTTPS rejection")
            phase("restart; reject untrusted HTTPS", rejected)

            def restore(tui):
                tui.edit()
                tui.tabs(12)
                tui.fill(str(cert))
                tui.test_ok()
                tui.save()
                check_profile(root, name, "localhost", port, str(cert))
            phase("restore CA in form; test and save", restore)
            phase("restart; restored CA and encrypted credential reconnect", lambda tui: (tui.connect(), tui.read()))

            http = urllib.parse.urlsplit(args.http_url)
            require(http.scheme == "opensearch" and http.hostname in ("localhost", "127.0.0.1")
                    and not http.username and not http.password and http.path in ("", "/")
                    and not http.query and not http.fragment, "isolated unauthenticated HTTP URL required")

            def no_auth(tui):
                tui.edit()
                tui.tabs(3)
                tui.fill(http.hostname)
                tui.tabs(1)
                tui.fill(str(http.port or 9200))
                tui.tabs(3)
                tui.send(b"\x1b[C")  # Basic -> None hides user/password/storage.
                require(not tui.contains("API Key"), "OpenSearch None has no API key mode")
                tui.tabs(1)
                tui.send(b"\x1b[D")  # VerifyFull -> Disable.
                tui.tabs(1)
                tui.fill("")
                tui.test_ok()
                tui.save()
                values = check_profile(root, name, http.hostname, http.port or 9200, None, no_auth=True)
                require(not values.get(("profiles", "user")), "None clears Basic user")
                require(not values.get(("profiles.credential_policy.reference", "ciphertext")),
                        "None removes stored Basic credential")
            phase("edit to None HTTP in form; clear Basic credential; test and save", no_auth)
            phase("restart; None HTTP reconnect and REST read", lambda tui: (tui.connect(), tui.read()))
        finally:
            report["cleanup"] = "pending"
            status, _ = api("DELETE", "/" + index)
            require(status in (200, 404), "temporary browser index cleanup")
            report["cleanup"] = "test index removed; private configuration removed on exit"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/lazydb")
    parser.add_argument("--url", default=os.environ.get("LAZYDB_TEST_OPENSEARCH_TLS_URL"))
    parser.add_argument("--ca-cert", default=os.environ.get("LAZYDB_TEST_OPENSEARCH_TLS_CERT"))
    parser.add_argument("--http-url", default=os.environ.get("LAZYDB_TEST_OPENSEARCH_URL"),
                        help="security-disabled HTTP fixture for None authentication form acceptance")
    parser.add_argument("--diagnostics-dir", type=Path)
    args = parser.parse_args()
    if not args.url or not args.ca_cert or not args.http_url:
        parser.error("set LAZYDB_TEST_OPENSEARCH_TLS_URL, LAZYDB_TEST_OPENSEARCH_TLS_CERT and LAZYDB_TEST_OPENSEARCH_URL")
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
            (args.diagnostics_dir / "opensearch-profile-sanitized.json").write_text(json.dumps(report, indent=2) + "\n")
        return 1
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    print("PASS: credentials absent from terminal, metadata and history; terminal restored", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
