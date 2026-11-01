#!/usr/bin/env python3
"""Exercise REST queries, write confirmation, and read-only mode in the real TUI."""

import argparse
import base64
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
import tempfile
import termios
import time
import urllib.error
import urllib.request
from urllib.parse import urlsplit
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/lazydb")
    parser.add_argument("--url", default=os.environ.get("LAZYDB_TEST_ELASTICSEARCH_URL"))
    parser.add_argument("--ca-cert", help="PEM CA for an elasticsearchs:// fixture")
    parser.add_argument("--api-key", action="store_true", help="create a temporary API key for the TUI")
    parser.add_argument("--diagnostics-dir", type=Path, help="save a redacted terminal tail on failure")
    args = parser.parse_args()

    def interrupted(signum, frame):
        # The CI deadline sends SIGTERM; unwind finally blocks to stop the PTY
        # child and invalidate the disposable API key before the wrapper exits.
        raise RuntimeError("terminal acceptance interrupted")

    signal.signal(signal.SIGTERM, interrupted)
    if not args.url:
        parser.error("set LAZYDB_TEST_ELASTICSEARCH_URL or --url to an isolated fixture")
    parts = urlsplit(args.url)
    if parts.scheme not in ("elasticsearch", "elasticsearchs") or not parts.username or not parts.password:
        parser.error("this smoke test requires an Elasticsearch URL with test admin credentials")
    host = f"[{parts.hostname}]" if ":" in parts.hostname else parts.hostname
    tls = parts.scheme == "elasticsearchs"
    endpoint = f"{'https' if tls else 'http'}://{host}:{parts.port or 9200}"
    context = ssl.create_default_context(cafile=args.ca_cert) if tls else None
    auth = base64.b64encode(f"{parts.username}:{parts.password}".encode()).decode()

    def api(method, path, body=None):
        request = urllib.request.Request(
            endpoint + path, method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": "Basic " + auth, "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=10, context=context) as result:
                return result.status, None if method == "HEAD" else json.load(result)
        except urllib.error.HTTPError as error:
            return error.code, None if method == "HEAD" else json.load(error)

    for read_only in (False, True):
        index = "lazydb-tui-" + uuid.uuid4().hex
        created = False
        key_id = None
        credential = parts.password
        if args.api_key:
            status, key = api("POST", "/_security/api_key", {
                "name": index, "expiration": "5m",
                "role_descriptors": {"tui": {
                    "cluster": ["monitor"],
                    "indices": [{"names": [index, index + "-missing"], "privileges": ["all"]}],
                }},
            })
            assert status == 200, "API key creation failed"
            key_id, credential = key["id"], key["encoded"]
        with tempfile.TemporaryDirectory(prefix="lazydb-es-tui-") as root:
            Path(root, "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 150, 0, 0))
            env = os.environ.copy()
            env.update(LAZYDB_CONFIG_HOME=root, TERM="xterm-256color")
            env["LAZYDB_PASSWORD"] = credential
            # Keep credentials out of argv and profile metadata; exercise the saved
            # profile options used by the connection form and normal startup.
            profile_lines = [
                "version = 6", "[[profiles]]", f'id = "{uuid.uuid4()}"',
                'name = "tui-search"', 'kind = "elasticsearch"',
                f'url_format = "elasticsearch{"-tls" if tls else ""}"',
                "host = " + json.dumps(parts.hostname), f"port = {parts.port or 9200}",
                'database = "default"', f'read_only = {str(read_only).lower()}',
                f'ssl_mode = "{"verify-full" if tls else "disable"}"',
                "[profiles.credential_policy]", 'policy = "prompt"',
                "[profiles.catalog_scope.databases]", 'mode = "all"',
                "[profiles.elasticsearch_options]", f'auth = "{"api_key" if args.api_key else "basic"}"',
            ]
            if args.ca_cert:
                profile_lines.append("ca_cert_path = " + json.dumps(str(Path(args.ca_cert).resolve())))
            if not args.api_key:
                profile_lines.insert(7, "user = " + json.dumps(parts.username))
            Path(root, "connections.toml").write_text("\n".join(profile_lines) + "\n")
            command = [args.binary, "--profile", "tui-search", "--mouse", "off", "--color", "never"]
            proc = subprocess.Popen(
                command, stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True
            )
            os.close(slave)
            transcript = bytearray()

            def pump():
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        return
                    transcript.extend(chunk)
                    if b"\x1b[6n" in chunk:
                        os.write(master, b"\x1b[1;1R")

            def text(start=0):
                return re.sub(
                    r"\x1b\[[0-?]*[ -/]*[@-~]", "", transcript[start:].decode(errors="replace")
                )

            def redacted_tail():
                output = text()[-6000:]
                for secret in (credential, auth, parts.password):
                    output = output.replace(secret, "[REDACTED]")
                return output

            def wait(check, label, timeout=20):
                deadline = time.monotonic() + timeout
                while time.monotonic() < deadline:
                    pump()
                    if check():
                        return
                    if proc.poll() is not None:
                        # Exit may happen between the predicate and poll checks.
                        if check():
                            return
                        raise RuntimeError(f"TUI exited with {proc.returncode}: {label}")
                raise TimeoutError(label + "\n" + redacted_tail())

            def send(data):
                os.write(master, data)

            def history(sql):
                path = Path(root, "sql-history.sqlite3")
                if not path.exists():
                    return None
                try:
                    with sqlite3.connect(path) as db:
                        return db.execute(
                            "select status, language from history_executions "
                            "where sql=? order by requested_at desc limit 1", (sql,)
                        ).fetchone()
                except sqlite3.Error:
                    return None

            def new_request(sql):
                start = len(transcript)
                send(b"\x1b[17~")  # F6 opens the console manager.
                wait(lambda: b"CONSOLES" in transcript[start:], "console manager")
                start = len(transcript)
                send(b"a")
                wait(lambda: b"INSERT" in transcript[start:], "new REST console")
                start = len(transcript)
                send(b"\x1b[200~" + sql.encode() + b"\x1b[201~")
                wait(lambda: sql.splitlines()[0].replace(" ", "") in text(start).replace(" ", ""), "pasted request")
                start = len(transcript)
                send(b"\x1b")
                wait(lambda: b"NORMAL" in transcript[start:], "editor Normal mode")
                send(b"\x1b[15~")  # F5 executes the request.

            try:
                wait(lambda: not (termios.tcgetattr(master)[3] & termios.ICANON), "raw terminal")
                send(b"\r")
                wait(lambda: "●" in text(), "connected indicator")
                read = "GET /_cluster/health"
                new_request(read)
                wait(lambda: history(read) == ("succeeded", "rest"), "REST read result and history")
                assert b"REST CONSOLE" in transcript
                print(f"PASS: read_only={read_only}: real REST query and REST history")
                write = f'PUT /{index}/_doc/1?refresh=wait_for\n{{"title":"TUI smoke"}}'
                start = len(transcript)
                new_request(write)
                if read_only:
                    wait(lambda: "read-only" in text(start).lower(), "read-only denial")
                    assert b"EXECUTION CONFIRMATION" not in transcript[start:]
                    assert history(write) is None
                    assert api("HEAD", "/" + index)[0] == 404
                    print("PASS: read-only TUI blocks writes before confirmation or dispatch")
                else:
                    wait(lambda: "Execute REST request?" in text(start), "REST write confirmation")
                    assert api("HEAD", "/" + index)[0] == 404
                    assert history(write) is None
                    send(b"\t\r")  # Move from Cancel to Execute and confirm.
                    wait(lambda: history(write) == ("succeeded", "rest"), "confirmed write result")
                    created = True
                    assert api("GET", f"/{index}/_doc/1")[1]["_source"]["title"] == "TUI smoke"
                    print("PASS: confirmation precedes mutation; confirmed write reaches Elasticsearch")
                    bulk = (f'POST /{index}/_bulk?refresh=wait_for\n'
                            '{"create":{"_id":"1"}}\n{"title":"conflict"}\n'
                            '{"index":{"_id":"2"}}\n{"title":"partial success"}')
                    start = len(transcript)
                    new_request(bulk)
                    wait(lambda: "Execute REST request?" in text(start), "bulk confirmation")
                    send(b"\t\r")
                    wait(lambda: history(bulk) == ("failed", "rest"), "bulk failed history")
                    wait(lambda: "bulk:1of2" in text(start).replace(" ", ""), "bulk failure summary")
                    assert api("GET", f"/{index}/_doc/1")[1]["_source"]["title"] == "TUI smoke"
                    assert api("GET", f"/{index}/_doc/2")[1]["_source"]["title"] == "partial success"
                    print("PASS: bulk partial failure keeps successful write and records failed history")
                    multi = (f'POST /{index}/_msearch\n{{}}\n{{"size":0}}\n'
                             f'{{"index":"{index}-missing"}}\n{{"size":0}}')
                    start = len(transcript)
                    new_request(multi)
                    wait(lambda: history(multi) == ("failed", "rest"), "msearch failed history")
                    wait(lambda: "msearch:1of2" in text(start).replace(" ", ""), "msearch failure summary")
                    print("PASS: msearch partial failure shows summary and records failed history")
                send(b"\x03")
                wait(lambda: proc.poll() is not None, "clean TUI exit", 5)
                assert proc.returncode == 0
                assert b"\x1b[?1049l" in transcript
                assert termios.tcgetattr(master)[3] & termios.ICANON
                print("PASS: clean exit restores terminal")
            except Exception:
                if args.diagnostics_dir:
                    args.diagnostics_dir.mkdir(parents=True, exist_ok=True)
                    mode = "api-key" if args.api_key else "basic"
                    path = args.diagnostics_dir / f"console-{mode}-readonly-{read_only}.log"
                    path.write_text(redacted_tail())
                raise
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGTERM)
                    proc.wait(timeout=5)
                os.close(master)
                try:
                    status, _ = api("DELETE", "/" + index)
                    assert status in (200, 404), f"test index cleanup failed: {status}"
                    if created:
                        assert status == 200
                finally:
                    if key_id is not None:
                        status, result = api("DELETE", "/_security/api_key", {"ids": [key_id]})
                        assert status == 200 and key_id in result["invalidated_api_keys"]
                assert credential not in text(), "credential appeared in terminal output"
                for file in Path(root).glob("*.toml"):
                    assert credential not in file.read_text(), "credential appeared in config metadata"
                for file in Path(root).glob("sql-history.sqlite3*"):
                    assert credential.encode() not in file.read_bytes(), "credential appeared in history"


if __name__ == "__main__":
    main()
