#![cfg(unix)]

#[path = "fixtures/ssh/test_support.rs"]
mod support;

use std::{
    net::{TcpListener, TcpStream},
    os::unix::net::UnixStream,
    process::Stdio,
    time::Duration,
};

use support::{
    REDIS_PASSWORD_ENV, SshForwarder, fixture, fixture_tempdir, redis_auth_select_ping,
    required_env, socks5_connect,
};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn openssh_unix_socket_forward_reaches_remote_redis_tcp() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory.path().join("redis.sock");
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.redis.host,
        fixture.redis.port,
    )
    .await;
    let socket_path = forwarder.unix_path().clone();
    let password = required_env(REDIS_PASSWORD_ENV);
    tokio::task::spawn_blocking(move || {
        let mut stream = UnixStream::connect(socket_path).expect("connect forwarded Unix socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("set write timeout");
        redis_auth_select_ping(&mut stream, &password, fixture.redis.second_database);
    })
    .await
    .expect("Redis Unix forwarding worker");
    forwarder.assert_ready_marker_order();
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn openssh_tcp_forward_reaches_remote_redis_tcp() {
    let fixture = fixture();
    let forwarder =
        SshForwarder::start_tcp(&fixture.ssh, &fixture.redis.host, fixture.redis.port).await;
    let address = forwarder.tcp_addr();
    let password = required_env(REDIS_PASSWORD_ENV);
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(address).expect("connect forwarded TCP listener");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("set write timeout");
        redis_auth_select_ping(&mut stream, &password, fixture.redis.second_database);
    })
    .await
    .expect("Redis TCP forwarding worker");
    forwarder.assert_ready_marker_order();
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn openssh_socks_forward_reaches_remote_redis_tcp() {
    let fixture = fixture();
    let forwarder = SshForwarder::start_socks(&fixture.ssh).await;
    let address = forwarder.socks_addr();
    let password = required_env(REDIS_PASSWORD_ENV);
    let remote_host = fixture.redis.host.clone();
    let remote_port = fixture.redis.port;
    let database = fixture.redis.second_database;
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(address).expect("connect OpenSSH SOCKS listener");
        socks5_connect(&mut stream, &remote_host, remote_port).expect("SOCKS5 CONNECT");
        redis_auth_select_ping(&mut stream, &password, database);
    })
    .await
    .expect("Redis SOCKS forwarding worker");
    forwarder.assert_ready_marker_order();
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn openssh_rejects_wrong_private_key() {
    let fixture = fixture();
    let log = fixture_tempdir();
    let log_path = log.path().join("ssh.log");
    let status = Command::new("ssh")
        .arg("-F")
        .arg(&fixture.ssh.config_file)
        .arg("-N")
        .arg("-T")
        .arg("-i")
        .arg(&fixture.ssh.wrong_client_key)
        .arg("-p")
        .arg(fixture.ssh.port.to_string())
        .arg("-E")
        .arg(&log_path)
        .arg(format!("{}@{}", fixture.ssh.user, fixture.ssh.host))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn OpenSSH with wrong key")
        .wait()
        .await
        .expect("wait for wrong-key OpenSSH");
    assert!(!status.success(), "OpenSSH accepted the wrong private key");
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.contains("Permission denied (publickey)"),
        "unexpected OpenSSH rejection log:\n{log}"
    );
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn openssh_rejects_wrong_known_host_key() {
    let fixture = fixture();
    let log = fixture_tempdir();
    let log_path = log.path().join("ssh.log");
    let status = Command::new("ssh")
        .arg("-F")
        .arg(&fixture.ssh.config_file)
        .arg("-o")
        .arg(format!(
            "UserKnownHostsFile={}",
            fixture.ssh.wrong_known_hosts_file.display()
        ))
        .arg("-N")
        .arg("-T")
        .arg("-i")
        .arg(&fixture.ssh.client_key)
        .arg("-p")
        .arg(fixture.ssh.port.to_string())
        .arg("-E")
        .arg(&log_path)
        .arg(format!("{}@{}", fixture.ssh.user, fixture.ssh.host))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn OpenSSH with wrong host key")
        .wait()
        .await
        .expect("wait for wrong-host-key OpenSSH");
    assert!(!status.success(), "OpenSSH accepted the wrong host key");
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.contains("Host key verification failed"),
        "unexpected OpenSSH host-key rejection log:\n{log}"
    );
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn openssh_exit_on_forward_failure_rejects_an_occupied_tcp_port() {
    let fixture = fixture();
    let occupied = TcpListener::bind(("127.0.0.1", 0)).expect("bind occupied test port");
    let port = occupied.local_addr().expect("occupied address").port();
    let log = fixture_tempdir();
    let log_path = log.path().join("ssh.log");
    let status = Command::new("ssh")
        .arg("-F")
        .arg(&fixture.ssh.config_file)
        .arg("-N")
        .arg("-T")
        .arg("-i")
        .arg(&fixture.ssh.client_key)
        .arg("-p")
        .arg(fixture.ssh.port.to_string())
        .arg("-E")
        .arg(&log_path)
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-L")
        .arg(format!(
            "127.0.0.1:{port}:{}:{}",
            fixture.redis.host, fixture.redis.port
        ))
        .arg(format!("{}@{}", fixture.ssh.user, fixture.ssh.host))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn OpenSSH with an occupied local port")
        .wait()
        .await
        .expect("wait for occupied-port OpenSSH");
    assert!(
        !status.success(),
        "OpenSSH succeeded despite the occupied forwarding port"
    );
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.contains("Address already in use")
            || log.contains("Could not request local forwarding"),
        "unexpected OpenSSH occupied-port log:\n{log}"
    );
}
