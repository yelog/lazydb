#!/usr/bin/env python3
"""Create and consume the isolated SSH + database protocol verification fixture."""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
COMPOSE_FILE = REPO_ROOT / "docker-compose.ssh.yml"
SSH_BIN = shutil.which("ssh") or "ssh"
SSH_KEYGEN_BIN = shutil.which("ssh-keygen") or "ssh-keygen"
OPENSSL_BIN = shutil.which("openssl") or "openssl"
MANIFEST_ENV = "LAZYDB_TEST_SSH_FIXTURE_MANIFEST"
MONGO_USERNAME_ENV = "LAZYDB_TEST_SSH_MONGO_USERNAME"
SECRET_ENVS = {
    "postgres": "LAZYDB_TEST_SSH_POSTGRES_PASSWORD",
    "mysql": "LAZYDB_TEST_SSH_MYSQL_PASSWORD",
    "valkey": "LAZYDB_TEST_SSH_REDIS_PASSWORD",
    "mongo": "LAZYDB_TEST_SSH_MONGO_PASSWORD",
    "mssql": "LAZYDB_TEST_SSH_SQLSERVER_PASSWORD",
}
MONGO_MEMBERS = (
    {"service": "mongo1", "host": "mongo1.internal", "port": 27017},
    {"service": "mongo2", "host": "mongo2.internal", "port": 27018},
    {"service": "mongo3", "host": "mongo3.internal", "port": 27019},
)
PROJECT_PATTERN = re.compile(r"lazydb-ssh-[0-9a-f]{10}")
RUNTIME_PATTERN = re.compile(r"lazydb-ssh-[a-z0-9_]{8,}")
CLEANUP_TIMEOUT_SECONDS = 55.0
_TERMINATION_SIGNAL: int | None = None


class FixtureError(RuntimeError):
    pass


def handle_sigterm(signum, _frame) -> None:
    global _TERMINATION_SIGNAL
    _TERMINATION_SIGNAL = int(signum)
    raise SystemExit(128 + int(signum))


def run(
    command: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: float | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        timeout=timeout,
    )
    if check and result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise FixtureError(f"command failed ({result.returncode}): {command[0]}: {detail}")
    return result


def write_text(path: Path, contents: str, mode: int) -> None:
    path.write_text(contents)
    path.chmod(mode)


def write_secret(path: Path, contents: str) -> None:
    write_text(path, contents, 0o600)


def reserve_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def wait_for(timeout: float, description: str, probe) -> None:
    deadline = time.monotonic() + timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            if probe():
                return
        except Exception as error:
            last_error = error
        time.sleep(0.05)
    if last_error is not None:
        raise FixtureError(f"timed out waiting for {description}: {last_error}")
    raise FixtureError(f"timed out waiting for {description}")


def ssh_public_key(private_key: Path) -> str:
    result = run([SSH_KEYGEN_BIN, "-y", "-f", str(private_key)])
    public_key = result.stdout.strip()
    if not public_key.startswith(("ssh-ed25519 ", "ssh-rsa ", "ecdsa-sha2-")):
        raise FixtureError("ssh-keygen did not return a public key")
    return public_key


def generate_ssh_key(path: Path) -> None:
    if path.exists():
        path.unlink()
    run(
        [
            SSH_KEYGEN_BIN,
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "lazydb-ssh-fixture",
            "-f",
            str(path),
        ]
    )
    path.chmod(0o600)
    path.with_suffix(path.suffix + ".pub").chmod(0o644)


def generate_ca(directory: Path, name: str) -> tuple[Path, Path]:
    key = directory / f"{name}.key"
    cert = directory / f"{name}.pem"
    run(
        [
            OPENSSL_BIN,
            "genpkey",
            "-algorithm",
            "EC",
            "-pkeyopt",
            "ec_paramgen_curve:prime256v1",
            "-out",
            str(key),
        ]
    )
    run(
        [
            OPENSSL_BIN,
            "req",
            "-x509",
            "-new",
            "-key",
            str(key),
            "-sha256",
            "-days",
            "2",
            "-subj",
            f"/CN={name}",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-out",
            str(cert),
        ]
    )
    key.chmod(0o600)
    cert.chmod(0o644)
    return key, cert


def generate_certificate(
    directory: Path,
    name: str,
    ca_key: Path,
    ca_cert: Path,
    *,
    common_name: str,
    subject_alt_names: list[str],
    extended_key_usage: str,
) -> tuple[Path, Path]:
    key = directory / f"{name}.key"
    csr = directory / f"{name}.csr"
    cert = directory / f"{name}.crt"
    extension_file = directory / f"{name}.ext"
    run(
        [
            OPENSSL_BIN,
            "genpkey",
            "-algorithm",
            "EC",
            "-pkeyopt",
            "ec_paramgen_curve:prime256v1",
            "-out",
            str(key),
        ]
    )
    run(
        [
            OPENSSL_BIN,
            "req",
            "-new",
            "-key",
            str(key),
            "-subj",
            f"/CN={common_name}",
            "-out",
            str(csr),
        ]
    )
    usages = (
        "critical,digitalSignature,keyEncipherment"
        if "serverAuth" in extended_key_usage
        else "critical,digitalSignature"
    )
    write_text(
        extension_file,
        "\n".join(
            [
                "basicConstraints=critical,CA:FALSE",
                f"keyUsage={usages}",
                f"extendedKeyUsage={extended_key_usage}",
                f"subjectAltName={','.join(subject_alt_names)}",
                "",
            ]
        ),
        0o600,
    )
    run(
        [
            OPENSSL_BIN,
            "x509",
            "-req",
            "-in",
            str(csr),
            "-CA",
            str(ca_cert),
            "-CAkey",
            str(ca_key),
            "-set_serial",
            f"0x{uuid.uuid4().int:032x}",
            "-days",
            "2",
            "-sha256",
            "-extfile",
            str(extension_file),
            "-out",
            str(cert),
        ]
    )
    key.chmod(0o600)
    cert.chmod(0o644)
    csr.unlink()
    extension_file.unlink()
    return key, cert


def copy_ca(ca_cert: Path, directory: Path) -> Path:
    destination = directory / "ca.pem"
    shutil.copyfile(ca_cert, destination)
    destination.chmod(0o644)
    return destination


def create_runtime() -> Path:
    runtime_dir = Path(tempfile.mkdtemp(prefix="lazydb-ssh-", dir="/tmp"))
    runtime_dir.chmod(0o700)
    for relative, mode in [
        ("ssh", 0o700),
        ("client", 0o700),
        ("secrets", 0o700),
        ("openssl", 0o700),
        ("http", 0o755),
        ("mongo", 0o755),
        ("postgres", 0o755),
        ("mysql", 0o755),
        ("valkey", 0o755),
    ]:
        directory = runtime_dir / relative
        directory.mkdir(mode=mode)
        directory.chmod(mode)
    return runtime_dir


def build_runtime_material(runtime_dir: Path) -> dict[str, object]:
    ssh_dir = runtime_dir / "ssh"
    client_dir = runtime_dir / "client"
    openssl_dir = runtime_dir / "openssl"
    secrets_dir = runtime_dir / "secrets"
    postgres_dir = runtime_dir / "postgres"
    mysql_dir = runtime_dir / "mysql"
    valkey_dir = runtime_dir / "valkey"
    http_dir = runtime_dir / "http"
    mongo_dir = runtime_dir / "mongo"

    host_key = ssh_dir / "ssh_host_ed25519_key"
    client_key = client_dir / "id_ed25519"
    wrong_client_key = client_dir / "id_ed25519_wrong"
    wrong_host_key = ssh_dir / "ssh_host_ed25519_wrong_key"
    generate_ssh_key(host_key)
    generate_ssh_key(client_key)
    generate_ssh_key(wrong_client_key)
    generate_ssh_key(wrong_host_key)

    authorized_keys = ssh_dir / "authorized_keys"
    write_secret(authorized_keys, client_key.with_suffix(".pub").read_text())

    ca_key, ca_cert = generate_ca(openssl_dir, "lazydb-ssh-test-ca")
    _, wrong_ca_cert = generate_ca(openssl_dir, "lazydb-ssh-test-wrong-ca")
    postgres_key, postgres_cert = generate_certificate(
        openssl_dir,
        "postgres",
        ca_key,
        ca_cert,
        common_name="postgres.internal",
        subject_alt_names=["DNS:postgres.internal"],
        extended_key_usage="serverAuth",
    )
    mysql_key, mysql_cert = generate_certificate(
        openssl_dir,
        "mysql",
        ca_key,
        ca_cert,
        common_name="mysql.internal",
        subject_alt_names=["DNS:mysql.internal"],
        extended_key_usage="serverAuth",
    )
    valkey_key, valkey_cert = generate_certificate(
        openssl_dir,
        "valkey",
        ca_key,
        ca_cert,
        common_name="redis.internal",
        subject_alt_names=["DNS:redis.internal", "DNS:valkey.internal"],
        extended_key_usage="serverAuth",
    )
    http_key, http_cert = generate_certificate(
        openssl_dir,
        "http",
        ca_key,
        ca_cert,
        common_name="clickhouse.internal",
        subject_alt_names=[
            "DNS:clickhouse.internal",
            "DNS:search.internal",
            "DNS:opensearch.internal",
        ],
        extended_key_usage="serverAuth",
    )
    for member in MONGO_MEMBERS:
        mongo_key, mongo_cert = generate_certificate(
            openssl_dir,
            f"mongo-{member['service']}",
            ca_key,
            ca_cert,
            common_name=member["host"],
            subject_alt_names=[f"DNS:{member['host']}", "IP:127.0.0.1"],
            extended_key_usage="serverAuth,clientAuth",
        )
        combined_pem = mongo_dir / f"{member['service']}.pem"
        shutil.copyfile(mongo_cert, combined_pem)
        with combined_pem.open("ab") as combined:
            combined.write(mongo_key.read_bytes())
        combined_pem.chmod(0o600)
    client_cert_key, client_cert = generate_certificate(
        openssl_dir,
        "client",
        ca_key,
        ca_cert,
        common_name="lazydb",
        subject_alt_names=["DNS:lazydb"],
        extended_key_usage="clientAuth",
    )

    for destination, source in [
        (postgres_dir / "postgres.key", postgres_key),
        (postgres_dir / "postgres.crt", postgres_cert),
        (mysql_dir / "mysql.key", mysql_key),
        (mysql_dir / "mysql.crt", mysql_cert),
        (valkey_dir / "valkey.key", valkey_key),
        (valkey_dir / "valkey.crt", valkey_cert),
        (http_dir / "http.key", http_key),
        (http_dir / "http.crt", http_cert),
    ]:
        shutil.copyfile(source, destination)
        if destination.parent in (valkey_dir, http_dir):
            destination.chmod(0o644)
        else:
            destination.chmod(0o600 if destination.suffix == ".key" else 0o644)
    postgres_ca = copy_ca(ca_cert, postgres_dir)
    mysql_ca = copy_ca(ca_cert, mysql_dir)
    valkey_ca = copy_ca(ca_cert, valkey_dir)
    http_ca = copy_ca(ca_cert, http_dir)
    mongo_ca = copy_ca(ca_cert, mongo_dir)
    write_secret(
        mongo_dir / "keyfile",
        uuid.uuid4().hex + uuid.uuid4().hex + uuid.uuid4().hex + "\n",
    )
    validate_ca = copy_ca(wrong_ca_cert, client_dir)
    validate_ca.chmod(0o644)
    shutil.copyfile(client_cert, client_dir / "client.crt")
    shutil.copyfile(client_cert_key, client_dir / "client.key")
    (client_dir / "client.crt").chmod(0o644)
    (client_dir / "client.key").chmod(0o600)
    shutil.copyfile(
        REPO_ROOT / "tests/fixtures/ssh/postgres/pg_hba.conf",
        postgres_dir / "pg_hba.conf",
    )
    (postgres_dir / "pg_hba.conf").chmod(0o644)

    passwords = {
        "postgres": "pg-" + uuid.uuid4().hex + uuid.uuid4().hex,
        "mysql": "my-" + uuid.uuid4().hex + uuid.uuid4().hex,
        "mysql_root": "mr-" + uuid.uuid4().hex + uuid.uuid4().hex,
        "valkey": "vk-" + uuid.uuid4().hex + uuid.uuid4().hex,
        "mongo": "mg-" + uuid.uuid4().hex + uuid.uuid4().hex,
        "mssql": "Ms-" + uuid.uuid4().hex + "!7",
    }
    mongo_username = "lazydb_" + uuid.uuid4().hex[:16]
    write_secret(secrets_dir / "postgres_password", passwords["postgres"])
    write_secret(secrets_dir / "mysql_password", passwords["mysql"])
    write_secret(secrets_dir / "mysql_root_password", passwords["mysql_root"])
    write_secret(secrets_dir / "valkey_password", passwords["valkey"])
    write_secret(secrets_dir / "mssql_password", passwords["mssql"])
    write_secret(
        secrets_dir / "mysql-client.cnf",
        "\n".join(
            [
                "[client]",
                "user=root",
                f"password={passwords['mysql_root']}",
                "",
            ]
        ),
    )

    write_text(
        valkey_dir / "valkey.conf",
        "\n".join(
            [
                "bind 0.0.0.0",
                "protected-mode no",
                "port 6379",
                "tls-port 6380",
                "databases 16",
                'save ""',
                "appendonly no",
                f"requirepass {passwords['valkey']}",
                "tls-cert-file /run/lazydb-valkey/valkey.crt",
                "tls-key-file /run/lazydb-valkey/valkey.key",
                "tls-ca-cert-file /run/lazydb-valkey/ca.pem",
                "tls-auth-clients no",
                'tls-protocols "TLSv1.2 TLSv1.3"',
                "",
            ]
        ),
        0o644,
    )

    return {
        "host_key": host_key,
        "host_public_key": ssh_public_key(host_key),
        "wrong_host_public_key": ssh_public_key(wrong_host_key),
        "client_key": client_key,
        "wrong_client_key": wrong_client_key,
        "client_cert": client_dir / "client.crt",
        "client_cert_key": client_dir / "client.key",
        "ca_cert": ca_cert,
        "wrong_ca_cert": validate_ca,
        "postgres_ca": postgres_ca,
        "mysql_ca": mysql_ca,
        "valkey_ca": valkey_ca,
        "http_ca": http_ca,
        "mongo_ca": mongo_ca,
        "mongo_members": [dict(member) for member in MONGO_MEMBERS],
        "mongo_username": mongo_username,
        "passwords": passwords,
    }


def compose_command(project: str) -> list[str]:
    return ["docker", "compose", "-p", project, "-f", str(COMPOSE_FILE)]


def compose_environment(runtime_dir: Path) -> dict[str, str]:
    env = os.environ.copy()
    env["LAZYDB_SSH_RUNTIME_DIR"] = str(runtime_dir)
    return env


def start_containers(
    project: str, runtime_dir: Path, timeout: float, *, with_sqlserver: bool = False
) -> None:
    command = compose_command(project)
    if with_sqlserver:
        command += ["--profile", "sqlserver"]
    run(
        command
        + ["up", "-d", "--wait", "--wait-timeout", str(int(timeout)), "--build"],
        env=compose_environment(runtime_dir),
        timeout=timeout + 120,
    )


def ssh_host_port(project: str, runtime_dir: Path) -> int:
    result = run(
        compose_command(project) + ["port", "sshd", "22"],
        env=compose_environment(runtime_dir),
    )
    output = result.stdout.strip()
    if not output:
        raise FixtureError("sshd did not publish a loopback port")
    port_text = output.rsplit(":", 1)[-1]
    try:
        port = int(port_text)
    except ValueError as error:
        raise FixtureError(f"invalid sshd port mapping: {output}") from error
    return port


def service_host_port(
    project: str,
    runtime_dir: Path,
    service: str,
    container_port: int = 22,
) -> int:
    result = run(
        compose_command(project) + ["port", service, str(container_port)],
        env=compose_environment(runtime_dir),
    )
    output = result.stdout.strip()
    if not output:
        raise FixtureError(f"{service} did not publish a loopback port")
    port_text = output.rsplit(":", 1)[-1]
    try:
        port = int(port_text)
    except ValueError as error:
        raise FixtureError(f"invalid {service} port mapping: {output}") from error
    return port


def wait_for_ssh_banner(host: str, port: int, timeout: float) -> None:
    def probe() -> bool:
        with socket.create_connection((host, port), timeout=1.0) as connection:
            connection.settimeout(1.0)
            banner = connection.recv(256)
            return banner.startswith(b"SSH-2.0-")

    wait_for(timeout, "OpenSSH banner", probe)


def mongo_exec(
    project: str,
    runtime_dir: Path,
    service: str,
    port: int,
    script: str,
    *,
    env: dict[str, str] | None = None,
    env_names: tuple[str, ...] = (),
    timeout: float = 30.0,
) -> subprocess.CompletedProcess[str]:
    command = compose_command(project) + ["exec", "-T"]
    for name in env_names:
        command.extend(["-e", name])
    command.extend(
        [
            service,
            "mongosh",
            "--quiet",
            "--norc",
            "--tls",
            "--tlsCAFile",
            "/run/lazydb-mongo-runtime/ca.pem",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--eval",
            script,
        ]
    )
    return run(
        command,
        env=env or compose_environment(runtime_dir),
        timeout=timeout,
        check=False,
    )


def mongo_result_output(result: subprocess.CompletedProcess[str]) -> str:
    return "\n".join(
        part.strip() for part in (result.stdout, result.stderr) if part and part.strip()
    ).strip()


def mongo_shell(
    project: str,
    runtime_dir: Path,
    service: str,
    port: int,
    script: str,
    *,
    env: dict[str, str] | None = None,
    env_names: tuple[str, ...] = (),
    allow_failure: bool = False,
) -> str:
    result = mongo_exec(
        project,
        runtime_dir,
        service,
        port,
        script,
        env=env,
        env_names=env_names,
    )
    output = mongo_result_output(result)
    if result.returncode != 0 and not allow_failure:
        raise FixtureError(f"MongoDB shell failed ({result.returncode}): {output}")
    return output


def mongo_auth_environment(
    runtime_dir: Path,
    username: str,
    password: str,
) -> dict[str, str]:
    env = compose_environment(runtime_dir)
    env[MONGO_USERNAME_ENV] = username
    env[SECRET_ENVS["mongo"]] = password
    return env


def mongo_auth_prefix(script: str) -> str:
    username_env = MONGO_USERNAME_ENV
    password_env = SECRET_ENVS["mongo"]
    return (
        'const admin = db.getSiblingDB("admin");'
        f'admin.auth(process.env["{username_env}"], process.env["{password_env}"]);'
        "const status = admin.runCommand({connectionStatus: 1});"
        "const users = (status.authInfo && status.authInfo.authenticatedUsers) || [];"
        "if (users.length === 0) { quit(2); }"
        + script
    )


def initialize_mongo_replica_set(
    project: str,
    runtime_dir: Path,
    timeout: float,
    username: str,
    password: str,
) -> None:
    members = ",".join(
        f'{{_id:{index},host:"{member["host"]}:{member["port"]}"}}'
        for index, member in enumerate(MONGO_MEMBERS)
    )
    init_result = mongo_exec(
        project,
        runtime_dir,
        "mongo1",
        27017,
        f'rs.initiate({{_id:"rs0",members:[{members}]}})',
    )
    init_output = mongo_result_output(init_result)
    if init_result.returncode != 0 and "already initialized" not in init_output.lower():
        raise FixtureError(f"MongoDB replica set initiation failed: {init_output}")

    primary: dict[str, object] | None = None

    def find_primary() -> bool:
        nonlocal primary
        for member in MONGO_MEMBERS:
            output = mongo_shell(
                project,
                runtime_dir,
                str(member["service"]),
                int(member["port"]),
                "db.hello().isWritablePrimary",
                allow_failure=True,
            )
            if output.splitlines()[-1:] == ["true"]:
                primary = member
                return True
        return False

    wait_for(timeout, "MongoDB replica-set primary", find_primary)
    if primary is None:
        raise FixtureError("MongoDB replica set did not elect a primary")

    auth_env = mongo_auth_environment(runtime_dir, username, password)
    env_names = (MONGO_USERNAME_ENV, SECRET_ENVS["mongo"])
    create_script = (
        'const admin = db.getSiblingDB("admin");'
        "print(EJSON.stringify(admin.runCommand({"
        f'createUser: process.env["{MONGO_USERNAME_ENV}"],'
        f'pwd: process.env["{SECRET_ENVS["mongo"]}"],'
        'roles: [{role: "root", db: "admin"}],'
        'mechanisms: ["SCRAM-SHA-256"],'
        'writeConcern: {w: "majority", wtimeout: 10000}'
        "})));"
    )
    create_result = mongo_exec(
        project,
        runtime_dir,
        str(primary["service"]),
        int(primary["port"]),
        create_script,
        env=auth_env,
        env_names=env_names,
    )
    create_output = mongo_result_output(create_result)
    if create_result.returncode != 0:
        raise FixtureError(f"MongoDB user creation failed: {create_output}")
    try:
        create_payload = json.loads(create_output.splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise FixtureError(f"MongoDB user creation returned invalid output: {create_output}") from error
    if create_payload.get("ok") != 1:
        raise FixtureError(f"MongoDB user creation failed: {create_output}")

    def all_members_authenticated() -> bool:
        for member in MONGO_MEMBERS:
            output = mongo_shell(
                project,
                runtime_dir,
                str(member["service"]),
                int(member["port"]),
                mongo_auth_prefix('print("authenticated");'),
                env=auth_env,
                env_names=env_names,
                allow_failure=True,
            )
            if output.splitlines()[-1:] != ["authenticated"]:
                return False
        return True

    wait_for(
        timeout,
        "MongoDB SCRAM authentication on all members",
        all_members_authenticated,
    )

    def replica_ready() -> bool:
        output = mongo_shell(
            project,
            runtime_dir,
            str(primary["service"]),
            int(primary["port"]),
            mongo_auth_prefix(
                "const repl = admin.runCommand({replSetGetStatus: 1});"
                "print(JSON.stringify(repl.members.map(m => ({"
                "name: m.name, state: m.stateStr, health: m.health"
                "}))));"
            ),
            env=auth_env,
            env_names=env_names,
            allow_failure=True,
        )
        try:
            members = json.loads(output.splitlines()[-1])
        except (IndexError, json.JSONDecodeError):
            return False
        if not isinstance(members, list) or len(members) != 3:
            return False
        states = [
            member.get("state")
            for member in members
            if isinstance(member, dict)
        ]
        healthy = all(
            isinstance(member, dict) and member.get("health") == 1
            for member in members
        )
        return (
            healthy
            and states.count("PRIMARY") == 1
            and states.count("SECONDARY") >= 2
        )

    wait_for(timeout, "MongoDB primary and two healthy secondaries", replica_ready)

    hello_script = mongo_auth_prefix(
        "const h = db.hello();"
        "print(JSON.stringify({"
        "setName: h.setName, hosts: h.hosts, primary: h.primary,"
        "isWritablePrimary: h.isWritablePrimary"
        "}));"
    )
    hello_output = mongo_shell(
        project,
        runtime_dir,
        str(primary["service"]),
        int(primary["port"]),
        hello_script,
        env=auth_env,
        env_names=env_names,
    )
    try:
        hello = json.loads(hello_output.splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise FixtureError(f"MongoDB hello returned invalid output: {hello_output}") from error
    expected_hosts = {f"{member['host']}:{member['port']}" for member in MONGO_MEMBERS}
    if hello.get("setName") != "rs0" or set(hello.get("hosts", [])) != expected_hosts:
        raise FixtureError("MongoDB replica set did not advertise all three members")

    wrong_env = auth_env.copy()
    wrong_env[SECRET_ENVS["mongo"]] = "invalid-" + uuid.uuid4().hex
    wrong_output = mongo_shell(
        project,
        runtime_dir,
        str(primary["service"]),
        int(primary["port"]),
        mongo_auth_prefix('print("authenticated");'),
        env=wrong_env,
        env_names=env_names,
        allow_failure=True,
    )
    if wrong_output.splitlines()[-1:] == ["authenticated"]:
        raise FixtureError("MongoDB accepted an invalid SCRAM password")


def append_known_host(
    path: Path,
    host: str,
    port: int,
    public_key: str,
    *,
    mode: int = 0o600,
) -> None:
    host_pattern = f"[{host}]:{port}" if port != 22 else host
    with path.open("a") as known_hosts:
        known_hosts.write(f"{host_pattern} {public_key}\n")
    path.chmod(mode)


def write_known_hosts(path: Path, lines: list[tuple[str, int, str]]) -> None:
    path.write_text("")
    for host, port, public_key in lines:
        append_known_host(path, host, port, public_key)


def write_ssh_config(
    path: Path,
    known_hosts: Path,
    client_key: Path,
    jump_port: int,
) -> None:
    write_text(
        path,
        "\n".join(
            [
                "Host *",
                "  BatchMode yes",
                "  StrictHostKeyChecking yes",
                f'  UserKnownHostsFile "{known_hosts}"',
                "  GlobalKnownHostsFile /dev/null",
                "  IdentitiesOnly yes",
                "  PasswordAuthentication no",
                "  KbdInteractiveAuthentication no",
                "  PreferredAuthentications publickey",
                "  ConnectTimeout 5",
                "  ConnectionAttempts 1",
                "  ExitOnForwardFailure yes",
                "  ServerAliveInterval 5",
                "  ServerAliveCountMax 3",
                "  RequestTTY no",
                "  PermitLocalCommand no",
                "  ControlMaster no",
                "  ControlPath none",
                "  LogLevel DEBUG3",
                "Host lazydb-jump",
                "  HostName 127.0.0.1",
                f"  Port {jump_port}",
                "  User lazydb",
                f'  IdentityFile "{client_key}"',
                "  IdentitiesOnly yes",
                "",
            ]
        ),
        0o600,
    )


def resp_command(connection: socket.socket, *parts: str) -> str:
    payload = f"*{len(parts)}\r\n".encode()
    for part in parts:
        encoded = part.encode()
        payload += f"${len(encoded)}\r\n".encode() + encoded + b"\r\n"
    connection.sendall(payload)
    response = connection.recv(4096)
    if not response:
        raise FixtureError("Redis closed the connection without a response")
    first = response[:1]
    if first == b"-":
        raise FixtureError(response.decode(errors="replace").strip())
    return response.decode(errors="replace").strip()


def verify_redis_plain(addr: tuple[str, int], password: str) -> None:
    with socket.create_connection(addr, timeout=3.0) as connection:
        connection.settimeout(3.0)
        if not resp_command(connection, "AUTH", password).startswith("+OK"):
            raise FixtureError("plain Redis AUTH did not return OK")
        if not resp_command(connection, "SELECT", "2").startswith("+OK"):
            raise FixtureError("plain Redis SELECT 2 did not return OK")
        if resp_command(connection, "PING") != "+PONG":
            raise FixtureError("plain Redis PING did not return PONG")


def verify_redis_tls(addr: tuple[str, int], password: str, ca_file: Path) -> None:
    context = ssl.create_default_context(cafile=str(ca_file))
    context.check_hostname = True
    context.verify_mode = ssl.CERT_REQUIRED
    with socket.create_connection(addr, timeout=3.0) as raw:
        with context.wrap_socket(raw, server_hostname="redis.internal") as connection:
            connection.settimeout(3.0)
            if not resp_command(connection, "AUTH", password).startswith("+OK"):
                raise FixtureError("TLS Redis AUTH did not return OK")
            if not resp_command(connection, "SELECT", "2").startswith("+OK"):
                raise FixtureError("TLS Redis SELECT 2 did not return OK")
            if resp_command(connection, "PING") != "+PONG":
                raise FixtureError("TLS Redis PING did not return PONG")


def control_check(
    ssh_config: Path,
    control_path: Path,
    host: str,
    port: int,
    user: str,
) -> bool:
    result = run(
        [
            SSH_BIN,
            "-F",
            str(ssh_config),
            "-S",
            str(control_path),
            "-p",
            str(port),
            "-O",
            "check",
            f"{user}@{host}",
        ],
        check=False,
    )
    return result.returncode == 0


def start_redis_forward(
    runtime_dir: Path,
    ssh_config: Path,
    known_hosts: Path,
    client_key: Path,
    host: str,
    port: int,
    user: str,
    password: str,
    ca_file: Path,
    timeout: float,
) -> dict[str, object]:
    ssh_dir = runtime_dir / "ssh"
    last_error = ""
    for attempt in range(3):
        plain_port = reserve_loopback_port()
        tls_port = reserve_loopback_port()
        if tls_port == plain_port:
            continue
        control_path = ssh_dir / f"redis-control-{attempt}"
        log_path = ssh_dir / f"redis-forward-{attempt}.log"
        result = run(
            [
                SSH_BIN,
                "-F",
                str(ssh_config),
                "-N",
                "-T",
                "-M",
                "-S",
                str(control_path),
                "-f",
                "-i",
                str(client_key),
                "-p",
                str(port),
                "-E",
                str(log_path),
                "-o",
                "ExitOnForwardFailure=yes",
                "-L",
                f"127.0.0.1:{plain_port}:redis.internal:6379",
                "-L",
                f"127.0.0.1:{tls_port}:redis.internal:6380",
                f"{user}@{host}",
            ],
            timeout=timeout,
            check=False,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            if log_path.exists():
                log_tail = log_path.read_text(errors="replace").strip().splitlines()[-20:]
                detail = "\n".join([detail, *log_tail]).strip()
            last_error = detail
            continue

        def ready() -> bool:
            if not control_check(ssh_config, control_path, host, port, user):
                return False
            verify_redis_plain(("127.0.0.1", plain_port), password)
            verify_redis_tls(("127.0.0.1", tls_port), password, ca_file)
            return True

        try:
            wait_for(timeout, "Redis SSH forward", ready)
        except FixtureError as error:
            detail = str(error)
            if log_path.exists():
                log_tail = log_path.read_text(errors="replace").strip().splitlines()[-20:]
                detail = "\n".join([detail, *log_tail]).strip()
            last_error = detail
            stop_forward(control_path, host, port, user)
            continue
        return {
            "plain_port": plain_port,
            "tls_port": tls_port,
            "control_path": control_path,
            "log_path": log_path,
        }
    raise FixtureError(f"unable to establish Redis SSH forward: {last_error}")


def stop_forward(
    control_path: Path,
    host: str,
    port: int,
    user: str,
) -> None:
    if not control_path.exists():
        return
    run(
        [
            SSH_BIN,
            "-F",
            os.devnull,
            "-S",
            str(control_path),
            "-p",
            str(port),
            "-O",
            "exit",
            f"{user}@{host}",
        ],
        timeout=5,
        check=False,
    )


def stop_owned_forwards(
    runtime_dir: Path,
    ssh_port: int | None,
    host: str = "127.0.0.1",
    user: str = "lazydb",
) -> None:
    if ssh_port is None:
        return
    for attempt in range(3):
        try:
            stop_forward(
                runtime_dir / "ssh" / f"redis-control-{attempt}",
                host,
                ssh_port,
                user,
            )
        except (OSError, subprocess.SubprocessError) as error:
            print(f"fixture SSH forward cleanup failed: {error}", file=sys.stderr)


def validate_cleanup_manifest(manifest: dict[str, object]) -> tuple[str, Path]:
    if manifest.get("schema_version") != 1:
        raise FixtureError("refusing cleanup for an unsupported fixture manifest")
    project = str(manifest.get("project", ""))
    if not PROJECT_PATTERN.fullmatch(project):
        raise FixtureError("refusing cleanup for a non-fixture Compose project")

    compose_file = Path(str(manifest.get("compose_file", "")))
    if not compose_file.is_absolute() or compose_file.resolve() != COMPOSE_FILE.resolve():
        raise FixtureError("refusing cleanup for an unrelated Compose file")

    runtime_value = str(manifest.get("runtime_dir", ""))
    runtime = Path(runtime_value)
    if (
        not runtime.is_absolute()
        or runtime.is_symlink()
        or runtime.parent.resolve() != Path("/tmp").resolve()
        or not RUNTIME_PATTERN.fullmatch(runtime.name)
    ):
        raise FixtureError("refusing cleanup outside the fixture runtime directory")
    resolved_runtime = runtime.resolve()

    ssh = manifest.get("ssh")
    if not isinstance(ssh, dict):
        raise FixtureError("fixture manifest is missing SSH cleanup metadata")
    try:
        ssh_port = int(ssh["port"])
    except (KeyError, TypeError, ValueError) as error:
        raise FixtureError("fixture manifest has an invalid SSH port") from error
    if (
        ssh.get("host") != "127.0.0.1"
        or ssh.get("user") != "lazydb"
        or not 1 <= ssh_port <= 65535
    ):
        raise FixtureError("fixture manifest has unsafe SSH cleanup metadata")
    for field in ("config_file", "known_hosts_file", "wrong_known_hosts_file"):
        value = ssh.get(field)
        if not isinstance(value, str):
            raise FixtureError(f"fixture manifest is missing SSH {field}")
        path = Path(value)
        if not path.is_absolute() or not path.resolve().is_relative_to(resolved_runtime):
            raise FixtureError(f"fixture SSH {field} is outside the runtime directory")
    return project, resolved_runtime


def validate_owned_project(project: str, runtime_dir: Path) -> None:
    container_ids = run(
        [
            "docker",
            "ps",
            "-aq",
            "--filter",
            f"label=com.docker.compose.project={project}",
        ]
    ).stdout.split()
    if container_ids:
        config_files = run(
            [
                "docker",
                "inspect",
                "--format",
                '{{index .Config.Labels "com.docker.compose.project.config_files"}}',
                *container_ids,
            ]
        ).stdout.splitlines()
        expected = {str(COMPOSE_FILE.resolve())}
        for line in config_files:
            actual = {
                str(Path(value.strip()).resolve())
                for value in line.split(",")
                if value.strip()
            }
            if actual != expected:
                raise FixtureError(
                    f"refusing cleanup for Compose project {project}: "
                    "containers were not created from the fixture Compose file"
                )

    volume_prefix = f"{project}_"
    volumes = run(
        [
            "docker",
            "volume",
            "ls",
            "--filter",
            f"label=com.docker.compose.project={project}",
            "--format",
            "{{.Name}}",
        ]
    ).stdout.splitlines()
    if any(not volume.startswith(volume_prefix) for volume in volumes):
        raise FixtureError(f"refusing cleanup for unrelated volumes in project {project}")
    networks = run(
        [
            "docker",
            "network",
            "ls",
            "--filter",
            f"label=com.docker.compose.project={project}",
            "--format",
            "{{.Name}}",
        ]
    ).stdout.splitlines()
    if any(not network.startswith(volume_prefix) for network in networks):
        raise FixtureError(f"refusing cleanup for unrelated networks in project {project}")


def compose_down(project: str, runtime_dir: Path) -> bool:
    result = run(
        compose_command(project)
        + ["--profile", "sqlserver", "down", "--volumes", "--remove-orphans"],
        env=compose_environment(runtime_dir),
        timeout=45,
        check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        print(
            f"fixture compose down failed ({result.returncode}): {detail}",
            file=sys.stderr,
        )
        return False
    return True


def remove_runtime(runtime_dir: Path) -> bool:
    allowed = (
        not runtime_dir.is_symlink()
        and runtime_dir.parent.resolve() == Path("/tmp").resolve()
        and RUNTIME_PATTERN.fullmatch(runtime_dir.name)
    )
    if not allowed:
        return False
    try:
        shutil.rmtree(runtime_dir)
    except FileNotFoundError:
        return True
    except OSError as error:
        print(f"fixture runtime removal failed: {error}", file=sys.stderr)
        return False
    return not runtime_dir.exists()


def cleanup(manifest: dict[str, object]) -> None:
    project, runtime_dir = validate_cleanup_manifest(manifest)
    validate_owned_project(project, runtime_dir)
    ssh = manifest["ssh"]
    assert isinstance(ssh, dict)
    stop_owned_forwards(
        runtime_dir,
        int(ssh["port"]),
        str(ssh["host"]),
        str(ssh["user"]),
    )
    compose_ok = compose_down(project, runtime_dir)
    runtime_ok = remove_runtime(runtime_dir)
    if not compose_ok or not runtime_ok:
        raise FixtureError("fixture cleanup did not complete")


def cleanup_bounded(manifest: dict[str, object]) -> bool:
    errors: list[BaseException] = []

    def target() -> None:
        try:
            cleanup(manifest)
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=target, name="lazydb-fixture-cleanup", daemon=True)
    worker.start()
    worker.join(CLEANUP_TIMEOUT_SECONDS)
    if worker.is_alive():
        print(
            f"fixture cleanup exceeded {CLEANUP_TIMEOUT_SECONDS:.0f}s; continuing shutdown",
            file=sys.stderr,
        )
        return False
    for error in errors:
        print(f"fixture cleanup failed: {error}", file=sys.stderr)
    return not errors


def cleanup_generated_fixture(
    project: str,
    runtime_dir: Path,
    ssh_port: int | None,
) -> None:
    stop_owned_forwards(runtime_dir, ssh_port)
    compose_ok = compose_down(project, runtime_dir)
    runtime_ok = remove_runtime(runtime_dir)
    if not compose_ok or not runtime_ok:
        raise FixtureError("fixture cleanup did not complete")


def cleanup_generated_fixture_bounded(
    project: str,
    runtime_dir: Path,
    ssh_port: int | None,
) -> bool:
    errors: list[BaseException] = []

    def target() -> None:
        try:
            cleanup_generated_fixture(project, runtime_dir, ssh_port)
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=target, name="lazydb-fixture-cleanup", daemon=True)
    worker.start()
    worker.join(CLEANUP_TIMEOUT_SECONDS)
    if worker.is_alive():
        print(
            f"fixture cleanup exceeded {CLEANUP_TIMEOUT_SECONDS:.0f}s; continuing shutdown",
            file=sys.stderr,
        )
        return False
    for error in errors:
        print(f"fixture cleanup failed: {error}", file=sys.stderr)
    return not errors


def manifest_environment(
    manifest_path: Path,
    passwords: dict[str, str],
    manifest: dict[str, object],
    mongo_username: str,
) -> dict[str, str]:
    env = os.environ.copy()
    env.pop("SSL_CERT_FILE", None)
    env[MANIFEST_ENV] = str(manifest_path)
    env[SECRET_ENVS["postgres"]] = passwords["postgres"]
    env[SECRET_ENVS["mysql"]] = passwords["mysql"]
    env[SECRET_ENVS["valkey"]] = passwords["valkey"]
    env[MONGO_USERNAME_ENV] = mongo_username
    env[SECRET_ENVS["mongo"]] = passwords["mongo"]
    env[SECRET_ENVS["mssql"]] = passwords["mssql"]
    env["LAZYDB_REQUIRE_DATABASE_TESTS"] = "1"
    redis = manifest["redis"]
    assert isinstance(redis, dict)
    env["LAZYDB_TEST_SSH_REDIS_URL"] = str(redis["logical_url"])
    env["LAZYDB_TEST_SSH_REDIS_TLS_URL"] = str(redis["tls_logical_url"])
    env["LAZYDB_TEST_SSH_REDIS_ROUTED_ADDR"] = str(redis["routed_addr"])
    env["LAZYDB_TEST_SSH_REDIS_TLS_ROUTED_ADDR"] = str(redis["tls_routed_addr"])
    env["LAZYDB_TEST_SSH_REDIS_DB"] = str(redis["database"])
    env["LAZYDB_TEST_SSH_REDIS_DB2"] = str(redis["second_database"])
    env["LAZYDB_TEST_SSH_REDIS_TLS_CA_FILE"] = str(redis["tls_ca_file"])
    return env


def write_environment_file(
    path: Path,
    manifest_path: Path,
    passwords: dict[str, str],
    manifest: dict[str, object],
    mongo_username: str,
) -> None:
    redis = manifest["redis"]
    assert isinstance(redis, dict)
    values = {
        MANIFEST_ENV: str(manifest_path),
        SECRET_ENVS["postgres"]: passwords["postgres"],
        SECRET_ENVS["mysql"]: passwords["mysql"],
        SECRET_ENVS["valkey"]: passwords["valkey"],
        MONGO_USERNAME_ENV: mongo_username,
        SECRET_ENVS["mongo"]: passwords["mongo"],
        SECRET_ENVS["mssql"]: passwords["mssql"],
        "LAZYDB_TEST_SSH_REDIS_URL": str(redis["logical_url"]),
        "LAZYDB_TEST_SSH_REDIS_TLS_URL": str(redis["tls_logical_url"]),
        "LAZYDB_TEST_SSH_REDIS_ROUTED_ADDR": str(redis["routed_addr"]),
        "LAZYDB_TEST_SSH_REDIS_TLS_ROUTED_ADDR": str(redis["tls_routed_addr"]),
        "LAZYDB_TEST_SSH_REDIS_DB": str(redis["database"]),
        "LAZYDB_TEST_SSH_REDIS_DB2": str(redis["second_database"]),
        "LAZYDB_TEST_SSH_REDIS_TLS_CA_FILE": str(redis["tls_ca_file"]),
        "LAZYDB_REQUIRE_DATABASE_TESTS": "1",
    }
    contents = "\n".join(f"export {key}={shlex.quote(value)}" for key, value in values.items())
    write_secret(path, "unset SSL_CERT_FILE\n" + contents + "\n")


def build_manifest(
    project: str,
    runtime_dir: Path,
    material: dict[str, object],
    ssh_port: int,
    jump_port: int,
    ssh_config: Path,
    known_hosts: Path,
    wrong_known_hosts: Path,
    forward: dict[str, object],
    manifest_path: Path,
    env_path: Path,
) -> dict[str, object]:
    passwords = material["passwords"]
    assert isinstance(passwords, dict)
    plain_port = int(forward["plain_port"])
    tls_port = int(forward["tls_port"])
    direct_host_resolves = True
    try:
        socket.getaddrinfo("redis.internal", 6379, type=socket.SOCK_STREAM)
    except OSError:
        direct_host_resolves = False
    return {
        "schema_version": 1,
        "project": project,
        "runtime_dir": str(runtime_dir),
        "compose_file": str(COMPOSE_FILE),
        "manifest_file": str(manifest_path),
        "environment_file": str(env_path),
        "cleanup_command": [
            sys.executable,
            str(Path(__file__).resolve()),
            "--cleanup-manifest",
            str(manifest_path),
        ],
        "ssh": {
            "host": "127.0.0.1",
            "port": ssh_port,
            "user": "lazydb",
            "client_key": str(material["client_key"]),
            "wrong_client_key": str(material["wrong_client_key"]),
            "config_file": str(ssh_config),
            "known_hosts_file": str(known_hosts),
            "wrong_known_hosts_file": str(wrong_known_hosts),
            "redis_forward_control_path": str(forward["control_path"]),
            "redis_forward_log": str(forward["log_path"]),
        },
        "jump": {
            "host": "127.0.0.1",
            "port": jump_port,
            "user": "lazydb",
            "final_host": "sshd",
            "final_port": 22,
            "proxy_alias": "lazydb-jump",
        },
        "http": {
            "clickhouse_url": "clickhouse://default@clickhouse.internal:8080/default",
            "elasticsearch_url": "elasticsearchs://search.internal:8443",
            "opensearch_url": "opensearchs://opensearch.internal:8443",
            "ca_file": str(material["http_ca"]),
            "wrong_ca_file": str(material["wrong_ca_cert"]),
        },
        "mongo": {
            "url": (
                "mongodb://mongo1.internal:27017/app"
                "?replicaSet=rs0&authSource=admin&retryWrites=false&tls=true&tlsCAFile="
                + str(material["mongo_ca"])
            ),
            "ca_file": str(material["mongo_ca"]),
            "database": "app",
            "replica_set": "rs0",
            "members": material["mongo_members"],
            "username_env": MONGO_USERNAME_ENV,
            "password_env": SECRET_ENVS["mongo"],
            "auth_source": "admin",
            "published_host_port": False,
        },
        "postgres": {
            "host": "postgres.internal",
            "port": 5432,
            "user": "lazydb",
            "database": "lazydb",
            "ca_file": str(material["postgres_ca"]),
            "wrong_ca_file": str(material["wrong_ca_cert"]),
            "client_cert": str(material["client_cert"]),
            "client_key": str(material["client_cert_key"]),
        },
        "mysql": {
            "host": "mysql.internal",
            "port": 3306,
            "user": "lazydb",
            "database": "lazydb",
            "ca_file": str(material["mysql_ca"]),
            "wrong_ca_file": str(material["wrong_ca_cert"]),
        },
        "redis": {
            "host": "redis.internal",
            "port": 6379,
            "tls_port": 6380,
            "database": 0,
            "second_database": 2,
            "logical_url": "redis://redis.internal:6379/0",
            "tls_logical_url": "rediss://redis.internal:6380/0",
            "routed_addr": f"127.0.0.1:{plain_port}",
            "tls_routed_addr": f"127.0.0.1:{tls_port}",
            "password_env": SECRET_ENVS["valkey"],
            "username_env": None,
            "tls_ca_file": str(material["valkey_ca"]),
            "direct_host_resolves_from_host": direct_host_resolves,
            "published_host_port": False,
        },
    }


def prepare(args: argparse.Namespace) -> tuple[dict[str, object], dict[str, str], Path]:
    runtime_dir = create_runtime()
    project = f"lazydb-ssh-{uuid.uuid4().hex[:10]}"
    ssh_port: int | None = None
    try:
        material = build_runtime_material(runtime_dir)
        passwords = material["passwords"]
        assert isinstance(passwords, dict)
        mongo_username = str(material["mongo_username"])
        start_containers(
            project, runtime_dir, args.timeout, with_sqlserver=args.with_sqlserver
        )
        ssh_port = ssh_host_port(project, runtime_dir)
        jump_port = service_host_port(project, runtime_dir, "jump")
        wait_for_ssh_banner("127.0.0.1", ssh_port, args.timeout)
        wait_for_ssh_banner("127.0.0.1", jump_port, args.timeout)
        initialize_mongo_replica_set(
            project,
            runtime_dir,
            args.timeout,
            mongo_username,
            str(passwords["mongo"]),
        )

        ssh_dir = runtime_dir / "ssh"
        known_hosts = ssh_dir / "known_hosts"
        wrong_known_hosts = ssh_dir / "known_hosts_wrong"
        write_known_hosts(
            known_hosts,
            [
                ("127.0.0.1", ssh_port, str(material["host_public_key"])),
                ("127.0.0.1", jump_port, str(material["host_public_key"])),
                ("sshd", 22, str(material["host_public_key"])),
            ],
        )
        write_known_hosts(
            wrong_known_hosts,
            [
                (
                    "127.0.0.1",
                    ssh_port,
                    str(material["wrong_host_public_key"]),
                ),
                (
                    "127.0.0.1",
                    jump_port,
                    str(material["wrong_host_public_key"]),
                ),
                ("sshd", 22, str(material["wrong_host_public_key"])),
            ],
        )
        ssh_config = ssh_dir / "ssh_config"
        write_ssh_config(
            ssh_config,
            known_hosts,
            Path(str(material["client_key"])),
            jump_port,
        )
        forward = start_redis_forward(
            runtime_dir,
            ssh_config,
            known_hosts,
            Path(str(material["client_key"])),
            "127.0.0.1",
            ssh_port,
            "lazydb",
            str(passwords["valkey"]),
            Path(str(material["valkey_ca"])),
            args.timeout,
        )

        manifest_path = runtime_dir / "manifest.json"
        env_path = runtime_dir / "fixture.env"
        manifest = build_manifest(
            project,
            runtime_dir,
            material,
            ssh_port,
            jump_port,
            ssh_config,
            known_hosts,
            wrong_known_hosts,
            forward,
            manifest_path,
            env_path,
        )
        manifest["sqlserver"] = {
            "enabled": args.with_sqlserver,
            "host": "mssql.internal",
            "port": 1433,
            "password_env": SECRET_ENVS["mssql"],
        }
        write_text(manifest_path, json.dumps(manifest, indent=2, sort_keys=True) + "\n", 0o600)
        write_environment_file(
            env_path,
            manifest_path,
            passwords,
            manifest,
            mongo_username,
        )
        env = manifest_environment(manifest_path, passwords, manifest, mongo_username)
        return manifest, env, manifest_path
    except BaseException:
        if args.keep_on_failure and _TERMINATION_SIGNAL is None:
            print(
                f"fixture startup failed; runtime retained at {runtime_dir}",
                file=sys.stderr,
            )
            raise
        if not cleanup_generated_fixture_bounded(project, runtime_dir, ssh_port):
            print("fixture startup cleanup did not complete", file=sys.stderr)
        raise


def print_ready(manifest: dict[str, object], manifest_path: Path, *, kept: bool) -> None:
    redis = manifest["redis"]
    ssh = manifest["ssh"]
    http = manifest["http"]
    mongo = manifest["mongo"]
    assert isinstance(redis, dict)
    assert isinstance(ssh, dict)
    assert isinstance(http, dict)
    assert isinstance(mongo, dict)
    print(f"fixture project: {manifest['project']}")
    print(f"manifest: {manifest_path}")
    print(f"environment: {manifest['environment_file']}")
    print(f"ssh: {ssh['host']}:{ssh['port']} user={ssh['user']}")
    print(
        "redis: "
        f"{redis['logical_url']} via {redis['routed_addr']}; "
        f"TLS {redis['tls_logical_url']} via {redis['tls_routed_addr']}"
    )
    print(
        "http: "
        f"{http['clickhouse_url']}; {http['elasticsearch_url']}; "
        f"{http['opensearch_url']}"
    )
    members = ", ".join(
        f"{member['host']}:{member['port']}" for member in mongo["members"]
    )
    print(
        f"mongo: {mongo['url']} replicaSet={mongo['replica_set']} members={members}"
    )
    if kept:
        print(f"cleanup: {' '.join(shlex.quote(part) for part in manifest['cleanup_command'])}")


def cleanup_manifest(path: Path) -> int:
    try:
        manifest = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise FixtureError(f"unable to read fixture manifest {path}: {error}") from error
    if not cleanup_bounded(manifest):
        return 1
    print(f"cleaned fixture: {manifest['project']}")
    return 0


def cargo_target_directory(env: dict[str, str]) -> Path:
    result = run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"],
        env=env,
        timeout=120,
    )
    try:
        metadata = json.loads(result.stdout)
        return Path(metadata["target_directory"])
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise FixtureError("cargo metadata did not provide a target directory") from error


def run_tui_tests(
    manifest_path: Path,
    env_path: Path,
    env: dict[str, str],
) -> int:
    script = REPO_ROOT / "scripts/ci/test-ssh-tui.py"
    if not script.is_file():
        print(f"PTY test failed: {script} is not present", file=sys.stderr)
        return 1
    target_directory = cargo_target_directory(env)
    binary = target_directory / "debug" / "lazydb"
    build = subprocess.run(
        ["cargo", "build", "--locked", "--bin", "lazydb"],
        cwd=REPO_ROOT,
        env=env,
    )
    if build.returncode != 0:
        return build.returncode
    if not binary.is_file():
        print(f"PTY test failed: binary not found at {binary}", file=sys.stderr)
        return 1
    return subprocess.run(
        [
            sys.executable,
            str(script),
            "--manifest",
            str(manifest_path),
            "--env-file",
            str(env_path),
            "--binary",
            str(binary),
        ],
        cwd=REPO_ROOT,
        env=env,
    ).returncode


def main() -> int:
    if hasattr(signal, "SIGTERM"):
        signal.signal(signal.SIGTERM, handle_sigterm)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--keep-on-failure", action="store_true")
    parser.add_argument("--cleanup-manifest", type=Path)
    parser.add_argument("--cargo-target-dir")
    parser.add_argument("--with-sqlserver", action="store_true")
    args = parser.parse_args()

    if args.cleanup_manifest:
        return cleanup_manifest(args.cleanup_manifest)

    manifest, env, manifest_path = prepare(args)
    env_path = Path(str(manifest["environment_file"]))
    if args.cargo_target_dir:
        env["CARGO_TARGET_DIR"] = str(Path(args.cargo_target_dir).resolve())
    tests_passed = False
    failure_code = 1
    try:
        print_ready(manifest, manifest_path, kept=args.prepare_only or args.keep)
        if args.prepare_only:
            failure_code = 0
        else:
            test_env = env.copy()
            test_env.pop("SSL_CERT_FILE", None)
            result = subprocess.run(
                [
                    "cargo",
                    "test",
                    "--locked",
                    "--test",
                    "ssh_transport",
                    "--test",
                    "ssh_integration",
                    "--test",
                    "ssh_http",
                    "--test",
                    "ssh_mongo",
                    "--test",
                    "ssh_services",
                    *(["--test", "ssh_sqlserver"] if args.with_sqlserver else []),
                    "--",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ],
                cwd=REPO_ROOT,
                env=test_env,
            )
            failure_code = result.returncode
            tests_passed = failure_code == 0
            if tests_passed:
                failure_code = run_tui_tests(manifest_path, env_path, test_env)
                tests_passed = failure_code == 0
            if tests_passed:
                redis = manifest["redis"]
                assert isinstance(redis, dict)
                redis_env = test_env.copy()
                redis_env["SSL_CERT_FILE"] = str(redis["tls_ca_file"])
                result = subprocess.run(
                    [
                        "cargo",
                        "test",
                        "--locked",
                        "--test",
                        "ssh_redis",
                        "--",
                        "--ignored",
                        "--nocapture",
                        "--test-threads=1",
                    ],
                    cwd=REPO_ROOT,
                    env=redis_env,
                )
                failure_code = result.returncode
                tests_passed = failure_code == 0
            if not tests_passed and args.keep_on_failure:
                print(
                    "tests failed; fixture retained at "
                    f"{manifest_path}\ncleanup: "
                    + " ".join(
                        shlex.quote(part) for part in manifest["cleanup_command"]
                    ),
                    file=sys.stderr,
                )
                failure_code = failure_code or 1
    finally:
        if _TERMINATION_SIGNAL is not None:
            if not cleanup_bounded(manifest):
                print("fixture cleanup failed after termination", file=sys.stderr)
            failure_code = 128 + _TERMINATION_SIGNAL
        else:
            retain_on_failure = args.keep_on_failure and not tests_passed
            if not args.prepare_only and not args.keep and not retain_on_failure:
                if not cleanup_bounded(manifest):
                    print("fixture cleanup failed", file=sys.stderr)
                    if failure_code == 0:
                        failure_code = 1
    return failure_code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except FixtureError as error:
        print(f"fixture error: {error}", file=sys.stderr)
        raise SystemExit(1)
