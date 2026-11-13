#!/usr/bin/env python3
"""Exercise a saved SSH PostgreSQL profile and SQL execution in the real TUI."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import signal
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import time
import uuid


MANIFEST_ENV = "LAZYDB_TEST_SSH_FIXTURE_MANIFEST"
POSTGRES_PASSWORD_ENV = "LAZYDB_TEST_SSH_POSTGRES_PASSWORD"
PROFILE_NAME = "ssh-postgres-tui"
SQL = "SELECT 'ssh-tui-ok'::text AS marker"
ANSI_ESCAPE = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
PRIVATE_ENV_KEYS = frozenset(
    {
        "LAZYDB_CONFIG_HOME",
        "LAZYDB_PASSWORD",
        "CLAUDE_PROJECT_DIR",
        "SSL_CERT_FILE",
    }
)


class Failure(RuntimeError):
    pass


def require(condition, label):
    if not condition:
        raise Failure(label)


def toml_string(value):
    return json.dumps(str(value), ensure_ascii=False)


def load_manifest(path):
    require(path.is_file(), "fixture manifest file is missing")
    try:
        manifest = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise Failure("fixture manifest is unreadable") from error
    require(manifest.get("schema_version") == 1, "fixture manifest schema is unsupported")
    for section in ("ssh", "postgres"):
        require(isinstance(manifest.get(section), dict), "fixture manifest section is missing")
    for section, fields in (
        ("ssh", ("host", "port", "user", "client_key", "config_file", "known_hosts_file")),
        ("postgres", ("host", "port", "user", "database", "ca_file", "client_cert", "client_key")),
    ):
        for field in fields:
            require(field in manifest[section], "fixture manifest field is missing")
    for field in ("client_key", "config_file", "known_hosts_file"):
        require(Path(manifest["ssh"][field]).is_file(), "SSH fixture file is missing")
    for field in ("ca_file", "client_cert", "client_key"):
        require(Path(manifest["postgres"][field]).is_file(), "PostgreSQL TLS fixture file is missing")
    return manifest


def load_env_file(path):
    require(path.is_file(), "fixture environment file is missing")
    try:
        lines = path.read_text().splitlines()
    except OSError as error:
        raise Failure("fixture environment file is unreadable") from error
    values = {}
    for number, raw in enumerate(lines, 1):
        line = raw.strip()
        if not line or line.startswith("#") or line.startswith("unset "):
            continue
        if line.startswith("export "):
            line = line[len("export ") :]
        key, separator, encoded = line.partition("=")
        require(
            separator == "=" and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key),
            f"fixture environment file line {number} is invalid",
        )
        try:
            decoded = shlex.split(encoded, posix=True)
        except ValueError as error:
            raise Failure("fixture environment file contains invalid quoting") from error
        require(
            len(decoded) == 1,
            f"fixture environment file line {number} is invalid",
        )
        values[key] = decoded[0]
    return values


def write_profile(config_home, manifest, ssh_config_file):
    config_home.mkdir(mode=0o700, parents=True, exist_ok=True)
    (config_home / "settings.toml").write_text("[updates]\ncheck_on_startup = false\n")
    postgres = manifest["postgres"]
    ssh = manifest["ssh"]
    lines = [
        "version = 6",
        "[[profiles]]",
        f"id = {toml_string(uuid.uuid4())}",
        f"name = {toml_string(PROFILE_NAME)}",
        'kind = "postgres"',
        f"host = {toml_string(postgres['host'])}",
        f"port = {int(postgres['port'])}",
        f"user = {toml_string(postgres['user'])}",
        f"database = {toml_string(postgres['database'])}",
        'ssl_mode = "verify-full"',
        "read_only = true",
        'environment = "development"',
        "[profiles.credential_policy]",
        'policy = "prompt"',
        "[profiles.catalog_scope.databases]",
        'mode = "all"',
        "[profiles.pg_options]",
        f"ssl_root_cert = {toml_string(postgres['ca_file'])}",
        f"ssl_client_cert = {toml_string(postgres['client_cert'])}",
        f"ssl_client_key = {toml_string(postgres['client_key'])}",
        "[profiles.ssh]",
        f"host = {toml_string(ssh['host'])}",
        f"port = {int(ssh['port'])}",
        f"user = {toml_string(ssh['user'])}",
        f"identity_file = {toml_string(ssh['client_key'])}",
        f"config_file = {toml_string(ssh_config_file)}",
        f"known_hosts_file = {toml_string(ssh['known_hosts_file'])}",
    ]
    profile_path = config_home / "connections.toml"
    profile_path.write_text("\n".join(lines) + "\n")
    profile_path.chmod(0o600)
    return profile_path


def plain_text(data):
    return ANSI_ESCAPE.sub(b"", data).decode(errors="replace")


def managed_ssh_pids(ssh_config_file):
    try:
        result = subprocess.run(
            ["ps", "-ax", "-ww", "-o", "pid=", "-o", "command="],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise Failure("unable to read the process table") from error
    if result.returncode != 0:
        raise Failure("ps failed while checking SSH processes")
    config = str(ssh_config_file)
    pids = []
    for line in result.stdout.splitlines():
        parts = line.strip().split()
        if len(parts) < 2 or not parts[0].isdigit():
            continue
        arguments = parts[1:]
        targets_config = any(
            arguments[index] == "-F" and arguments[index + 1] == config
            for index in range(len(arguments) - 1)
        )
        batch_mode = any(
            arguments[index] == "-o" and arguments[index + 1] == "BatchMode=yes"
            for index in range(len(arguments) - 1)
        )
        has_forward = any(argument in ("-L", "-D") for argument in arguments)
        if not (targets_config and batch_mode and has_forward):
            continue
        pids.append(int(parts[0]))
    return pids


def terminate_group(process):
    if process.poll() is not None:
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            pass
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=3)
        return
    except (ProcessLookupError, subprocess.TimeoutExpired):
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        pass


class Tui:
    def __init__(
        self,
        binary,
        config_home,
        profile_path,
        history_path,
        project,
        ssh_config_file,
        password,
        deadline,
    ):
        self.deadline = deadline
        self.ssh_config_file = Path(ssh_config_file)
        self.history_path = Path(history_path)
        self.password = password
        self.transcript = bytearray()
        self.master, slave = pty.openpty()
        self.original_termios = termios.tcgetattr(slave)
        self.process = None
        try:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 150, 0, 0))
            environment = os.environ.copy()
            for key in list(environment):
                if key in PRIVATE_ENV_KEYS or key.startswith("LAZYDB_TEST_SSH_"):
                    environment.pop(key, None)
            environment.update(
                LAZYDB_CONFIG_HOME=str(config_home),
                LAZYDB_PASSWORD=password,
                TERM="xterm-256color",
                NO_COLOR="1",
            )
            command = [
                str(binary),
                "--config",
                str(profile_path),
                "--profile",
                PROFILE_NAME,
                "--mouse",
                "off",
                "--color",
                "never",
            ]
            require(
                all(password not in argument for argument in command),
                "fixture password appeared in process arguments",
            )
            self.process = subprocess.Popen(
                command,
                stdin=slave,
                stdout=slave,
                stderr=slave,
                cwd=project,
                env=environment,
                start_new_session=True,
            )
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)

    def remaining(self):
        return self.deadline - time.monotonic()

    def pump(self, timeout=0.05):
        if not select.select([self.master], [], [], timeout)[0]:
            return
        try:
            chunk = os.read(self.master, 65536)
        except OSError:
            return
        if not chunk:
            return
        self.transcript.extend(chunk)
        require(len(self.transcript) <= 8 * 1024 * 1024, "terminal output exceeded the limit")
        require(
            self.password.encode() not in self.transcript,
            "fixture password appeared in terminal output",
        )
        if b"\x1b[6n" in chunk:
            os.write(self.master, b"\x1b[1;1R")

    def wait(self, predicate, label, timeout=None):
        remaining = self.remaining()
        require(remaining > 0, "timed out during " + label)
        wait_until = time.monotonic() + min(remaining, timeout if timeout is not None else remaining)
        while time.monotonic() < wait_until:
            self.pump()
            if predicate():
                return
            if self.process.poll() is not None:
                self.pump(0)
                if predicate():
                    return
                raise Failure("application exited during " + label)
        recent = "\n".join(
            line for line in plain_text(self.transcript).splitlines() if line.strip()
        )[-3000:]
        raise Failure(f"timed out during {label}; recent screen:\n{recent}")

    def send(self, data):
        os.write(self.master, data)
        self.pump(0.08)

    def text_since(self, start):
        return plain_text(self.transcript[start:])

    def start(self):
        self.wait(
            lambda: not (termios.tcgetattr(self.master)[3] & termios.ICANON),
            "raw terminal startup",
            15,
        )
        self.send(b"\r")
        self.wait(lambda: "●" in plain_text(self.transcript), "saved SSH profile connection", 30)

    def history(self, sql):
        if not self.history_path.is_file():
            return None
        try:
            with sqlite3.connect(str(self.history_path), timeout=0.5) as database:
                return database.execute(
                    "select status, language from history_executions "
                    "where sql like ? order by requested_at desc limit 1",
                    (f"%{sql}%",),
                ).fetchone()
        except sqlite3.Error:
            return None

    def history_rows(self):
        if not self.history_path.is_file():
            return "history database is missing"
        try:
            with sqlite3.connect(str(self.history_path), timeout=0.5) as database:
                return database.execute(
                    "select status, language, sql, requested_at "
                    "from history_executions order by requested_at desc limit 5"
                ).fetchall()
        except sqlite3.Error as error:
            return f"history database is unreadable: {error}"

    def execute(self):
        start = len(self.transcript)
        self.send(b"\x1b[17~")
        self.wait(lambda: "CONSOLES" in self.text_since(start), "console manager")
        start = len(self.transcript)
        self.send(b"a")
        self.wait(lambda: "INSERT" in self.text_since(start), "new SQL console")
        start = len(self.transcript)
        self.send(b"\x1b[200~" + SQL.encode() + b"\x1b[201~")
        self.wait(lambda: "ssh-tui-ok" in self.text_since(start), "SQL paste")
        self.send(b"\x1b")
        self.wait(lambda: "NORMAL" in self.text_since(start), "SQL editor Normal mode")
        self.send(b"\x1b[15~")
        try:
            self.wait(
                lambda: self.history(SQL) == ("succeeded", "sql"),
                "persisted SQL history",
                45,
            )
        except Failure as error:
            raise Failure(f"{error}; latest history rows: {self.history_rows()}") from error

    def assert_ssh_running(self):
        require(
            managed_ssh_pids(self.ssh_config_file),
            "saved profile did not keep its SSH tunnel process alive",
        )

    def request_exit(self):
        self.send(b"\x03")

    def assert_clean_exit(self):
        self.wait(lambda: self.process.poll() is not None, "clean TUI exit", 10)
        self.pump(0)
        require(self.process.returncode == 0, "TUI exited unsuccessfully")
        require(b"\x1b[?1049l" in self.transcript, "TUI did not restore the alternate screen")
        require(
            bool(termios.tcgetattr(self.master)[3] & termios.ICANON),
            "TUI did not restore terminal canonical mode",
        )

    def cleanup(self):
        terminate_group(self.process)
        try:
            termios.tcsetattr(self.master, termios.TCSANOW, self.original_termios)
        except termios.error:
            pass
        try:
            os.close(self.master)
        except OSError:
            pass


def wait_for_ssh_cleanup(ssh_config_file, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not managed_ssh_pids(ssh_config_file):
            return
        time.sleep(0.05)
    require(
        not managed_ssh_pids(ssh_config_file),
        "managed SSH tunnel processes were not reaped",
    )


def kill_leaked_ssh(ssh_config_file):
    deadline = time.monotonic() + 2
    pids = managed_ssh_pids(ssh_config_file)
    while pids and time.monotonic() < deadline:
        for pid in pids:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        time.sleep(0.05)
        pids = managed_ssh_pids(ssh_config_file)
    require(not pids, "could not terminate leaked SSH tunnel processes")


def assert_secret_absent_from_files(config_home, password):
    secret = password.encode()
    for path in config_home.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix != ".toml" and not path.name.startswith("sql-history.sqlite3"):
            continue
        require(secret not in path.read_bytes(), "fixture password was persisted to disk")


def run(args, manifest, password):
    binary = Path(args.binary).expanduser().resolve()
    require(binary.is_file() and os.access(binary, os.X_OK), "lazydb executable is unavailable")
    deadline = time.monotonic() + args.timeout
    with tempfile.TemporaryDirectory(prefix="lazydb-ssh-tui-") as temporary:
        root = Path(temporary)
        config_home = root / "config"
        project = root / "project"
        project.mkdir()
        (project / ".git").mkdir()
        ssh_config_file = root / "ssh_config"
        ssh_config_file.symlink_to(Path(manifest["ssh"]["config_file"]))
        profile_path = write_profile(config_home, manifest, ssh_config_file)
        require(
            password not in profile_path.read_text(),
            "fixture password was written to profile metadata",
        )
        history_path = config_home / "sql-history.sqlite3"
        tui = Tui(
            binary,
            config_home,
            profile_path,
            history_path,
            project,
            ssh_config_file,
            password,
            deadline,
        )
        passed = False
        try:
            tui.start()
            tui.execute()
            tui.assert_ssh_running()
            tui.request_exit()
            tui.assert_clean_exit()
            assert_secret_absent_from_files(config_home, password)
            passed = True
        finally:
            tui.cleanup()
            try:
                if passed:
                    wait_for_ssh_cleanup(
                        tui.ssh_config_file,
                        min(10, max(1, tui.remaining())),
                    )
            except BaseException:
                kill_leaked_ssh(tui.ssh_config_file)
                raise
            if not passed:
                kill_leaked_ssh(tui.ssh_config_file)
    print(
        "PASS: saved SSH PostgreSQL profile executed SQL, persisted history, "
        "and reaped its SSH tunnel"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/lazydb")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=None,
    )
    parser.add_argument("--env-file", type=Path, default=None)
    parser.add_argument("--password-env", default=POSTGRES_PASSWORD_ENV)
    parser.add_argument("--timeout", type=float, default=120.0)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    env_file = load_env_file(args.env_file) if args.env_file is not None else {}
    if args.manifest is None:
        configured = os.environ.get(MANIFEST_ENV) or env_file.get(MANIFEST_ENV)
        if not configured:
            parser.error(f"set {MANIFEST_ENV}, pass --manifest, or pass --env-file")
        args.manifest = Path(configured)
    password = os.environ.get(args.password_env) or env_file.get(args.password_env)
    if not password:
        parser.error(
            f"set {args.password_env} or provide it through --env-file"
        )

    def interrupted(signum, frame):
        raise Failure("terminal acceptance interrupted")

    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        manifest = load_manifest(args.manifest)
        run(args, manifest, password)
    except Failure as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    except Exception as error:
        print(f"FAIL: {type(error).__name__}", file=sys.stderr)
        return 1
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    return 0


if __name__ == "__main__":
    sys.exit(main())
