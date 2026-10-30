#!/usr/bin/env python3
"""Exercise Ctrl-C exit against healthy and paused MongoDB Compose fixtures."""

from __future__ import annotations

import argparse
import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import tempfile
import time
from typing import Optional


LEAVE_ALTERNATE_SCREEN = b"\x1b[?1049l"


def run(command: list[str], *, timeout: float = 30.0) -> str:
    result = subprocess.run(
        command,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=timeout,
    )
    return result.stdout


def compose(args: argparse.Namespace, *command: str) -> list[str]:
    invocation = ["docker", "compose"]
    if args.project:
        invocation.extend(["--project-name", args.project])
    invocation.extend(["--file", args.compose_file, *command])
    return invocation


def seed_terminal(fd: int, columns: int = 120, rows: int = 36) -> None:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))


def read_available(
    fd: int,
    process: subprocess.Popen[bytes],
    transcript: bytearray,
    answered_queries: set[bytes],
) -> None:
    while True:
        ready, _, _ = select.select([fd], [], [], 0)
        if not ready:
            return
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            return
        if not chunk:
            return
        transcript.extend(chunk)
        cursor_query = b"\x1b[6n"
        if cursor_query in transcript and cursor_query not in answered_queries:
            os.write(fd, b"\x1b[1;1R")
            answered_queries.add(cursor_query)


def cleanup_process(process: subprocess.Popen[bytes], fd: int) -> None:
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=2)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass
    try:
        os.close(fd)
    except OSError:
        pass


def exercise_exit(args: argparse.Namespace, paused: bool) -> None:
    config_home = tempfile.TemporaryDirectory(prefix="lazydb-exit-config-")
    master, slave = pty.openpty()
    seed_terminal(slave)
    original_attrs = termios.tcgetattr(slave)
    settings_path = os.path.join(config_home.name, "settings.toml")
    with open(settings_path, "w", encoding="utf-8") as settings:
        settings.write("[updates]\ncheck_on_startup = false\n")

    environment = os.environ.copy()
    environment["LAZYDB_CONFIG_HOME"] = config_home.name
    environment.pop("LAZYDB_PASSWORD", None)
    process = subprocess.Popen(
        [args.binary, "--url", args.uri, "--mouse", "off", "--color", "never"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=environment,
        start_new_session=True,
    )
    os.close(slave)
    transcript = bytearray()
    answered_queries: set[bytes] = set()
    paused_by_test = False
    try:
        connected_deadline = time.monotonic() + 20.0
        raw_deadline: Optional[float] = None
        while time.monotonic() < connected_deadline:
            read_available(master, process, transcript, answered_queries)
            if process.poll() is not None:
                raise RuntimeError(
                    "LazyDB exited before the MongoDB connection was visible: "
                    + transcript.decode("utf-8", errors="replace")[-4000:]
                )
            current_attrs = termios.tcgetattr(master)
            is_raw = not (current_attrs[3] & termios.ICANON) and not (current_attrs[3] & termios.ECHO)
            if is_raw and raw_deadline is None:
                raw_deadline = time.monotonic()
            if raw_deadline is not None:
                break
            time.sleep(0.02)
        else:
            raise TimeoutError("LazyDB did not enter raw terminal mode")

        # The CLI selects the ad-hoc profile but leaves connection explicit.
        os.write(master, b"\r")
        connected_deadline = time.monotonic() + 20.0
        while time.monotonic() < connected_deadline:
            read_available(master, process, transcript, answered_queries)
            if process.poll() is not None:
                raise RuntimeError("LazyDB exited before its MongoDB connection was ready")
            if "●".encode("utf-8") in transcript:
                break
            time.sleep(0.02)
        else:
            raise TimeoutError(
                "MongoDB connected marker did not appear in the TUI: "
                + transcript.decode("utf-8", errors="replace")[-4000:]
            )
        time.sleep(0.5)

        if paused:
            run(compose(args, "pause", args.service), timeout=10.0)
            paused_by_test = True

        exit_requested = time.monotonic()
        os.write(master, b"\x03")
        left_alternate_at: Optional[float] = None
        restored_at: Optional[float] = None
        exit_deadline = exit_requested + 5.0
        while time.monotonic() < exit_deadline:
            read_available(master, process, transcript, answered_queries)
            now = time.monotonic()
            if left_alternate_at is None and LEAVE_ALTERNATE_SCREEN in transcript:
                left_alternate_at = now
            try:
                attrs = termios.tcgetattr(master)
            except termios.error:
                attrs = None
            terminal_restored = attrs is not None and (attrs[3] & termios.ICANON) and (attrs[3] & termios.ECHO)
            if restored_at is None and terminal_restored and left_alternate_at is not None:
                restored_at = now
            if process.poll() is not None:
                break
            time.sleep(0.01)

        if process.poll() is None:
            raise TimeoutError("LazyDB did not exit within the five-second failure watchdog")
        elapsed = time.monotonic() - exit_requested
        alternate_ms = (
            (left_alternate_at - exit_requested) * 1000 if left_alternate_at is not None else None
        )
        restored_ms = (restored_at - exit_requested) * 1000 if restored_at is not None else None
        print(
            f"case={'paused' if paused else 'healthy'} "
            f"alternate_screen_ms={alternate_ms!r} "
            f"termios_restored_ms={restored_ms!r} exit_ms={elapsed * 1000:.1f}"
        )
        if left_alternate_at is None or restored_at is None:
            raise AssertionError("TUI did not emit terminal restoration and restore raw mode")
        if alternate_ms is None or alternate_ms > 200:
            raise AssertionError(f"alternate screen restoration exceeded 200 ms: {alternate_ms}")
        if elapsed > 1.5:
            raise AssertionError(f"process exit exceeded 1.5 seconds: {elapsed:.3f}s")
        if process.returncode != 0:
            raise RuntimeError(f"LazyDB exited with status {process.returncode}")
    finally:
        if paused_by_test:
            run(compose(args, "unpause", args.service), timeout=10.0)
        cleanup_process(process, master)
        config_home.cleanup()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--compose-file", required=True)
    parser.add_argument("--project", default=None)
    parser.add_argument("--service", default="mongodb-standalone")
    parser.add_argument("--uri", default="mongodb://127.0.0.1:27018/lazydb_exit")
    args = parser.parse_args()

    running = run(compose(args, "ps", "--status", "running", "--quiet", args.service))
    if not running.strip():
        raise RuntimeError("the MongoDB Compose service must already be running")
    exercise_exit(args, paused=False)
    exercise_exit(args, paused=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, RuntimeError, subprocess.SubprocessError, TimeoutError) as error:
        print(f"mongodb-tui-exit: {error}", file=sys.stderr)
        raise SystemExit(1)
