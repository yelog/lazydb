#![cfg(unix)]

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct NativeFixture {
    dir: tempfile::TempDir,
    data: PathBuf,
    launcher: PathBuf,
}

impl NativeFixture {
    fn new() -> Self {
        let binary = Path::new(env!("CARGO_BIN_EXE_lazydb"));
        let dir = tempfile::tempdir_in(binary.parent().unwrap()).unwrap();
        let data = dir.path().join("config");
        let version = env!("CARGO_PKG_VERSION");
        let release = data.join("releases").join(version);
        fs::create_dir_all(&release).unwrap();
        // A concurrent spawn can inherit a copy's writable fd until exec, causing ETXTBSY.
        // Keep the fixture on the binary's filesystem and install it without a writer.
        fs::hard_link(binary, release.join("lazydb")).unwrap();
        symlink(&release, data.join("current")).unwrap();
        let launcher = dir.path().join("bin/lazydb");
        fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        symlink("../config/current/lazydb", &launcher).unwrap();
        fs::write(
            data.join("install.json"),
            serde_json::json!({
                "schema": 1, "product": "lazydb", "manager": "native",
                "channel": "stable", "version": version,
                "target": "x86_64-unknown-linux-gnu", "path": launcher
            })
            .to_string(),
        )
        .unwrap();
        Self {
            dir,
            data,
            launcher,
        }
    }

    fn command(&self, address: &str) -> Command {
        let mut command = Command::new(&self.launcher);
        command
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.dir.path().join("xdg-data"))
            .env("LAZYDB_CONFIG_HOME", &self.data)
            .env("LAZYDB_CHANNEL_BASE_URL", address)
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1");
        command
    }

    fn assert_unchanged(&self) {
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(self.data.join("install.json")).unwrap()).unwrap();
        assert_eq!(state["version"], env!("CARGO_PKG_VERSION"));
    }
}

fn manifest(version: &str) -> String {
    let targets = [
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
    ];
    let assets: serde_json::Map<String, serde_json::Value> = targets.into_iter().map(|target| {
        let extension = if target.ends_with("msvc") { "zip" } else { "tar.xz" };
        (target.to_owned(), serde_json::json!({
            "url": format!("https://github.com/yelog/lazydb/releases/download/v{version}/lazydb_{version}_{target}.{extension}"),
            "sha256": "a".repeat(64),
            "requirements": if target.ends_with("-linux-gnu") { serde_json::json!({"libc":{"family":"glibc","min_version":"2.34"}}) } else { serde_json::Value::Null }
        }))
    }).collect();
    let assets: serde_json::Map<String, serde_json::Value> = assets
        .into_iter()
        .map(|(target, mut asset)| {
            if asset["requirements"].is_null() {
                asset.as_object_mut().unwrap().remove("requirements");
            }
            (target, asset)
        })
        .collect();
    serde_json::json!({
        "schema": 2, "product": "lazydb", "channel": "stable", "version": version,
        "tag": format!("v{version}"), "prerelease": false, "published_at": "2026-09-09T00:00:00Z",
        "release_url": format!("https://github.com/yelog/lazydb/releases/tag/v{version}"), "assets": assets
    })
    .to_string()
}

fn serve_manifest(
    body: String,
    gate: Option<mpsc::Receiver<()>>,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "manifest request timed out");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]);
        assert!(request.starts_with("GET /v2/stable.json "), "{request}");
        if let Some(gate) = gate {
            gate.recv_timeout(Duration::from_secs(15))
                .expect("release delayed response");
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    (format!("http://{address}"), server)
}

#[test]
fn installed_launcher_reports_native_manager() {
    let fixture = NativeFixture::new();
    let (address, server) = serve_manifest(manifest("9.9.9"), None);
    let output = fixture
        .command(&address)
        .args(["update", "--check", "--json"])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(
        !text.contains('\x1b') && !text.contains("Checking"),
        "{text}"
    );
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["manager"], "native", "{report}");
    assert_eq!(report["status"], "update_available", "{report}");
    assert_eq!(report["target_version"], "9.9.9");
    assert_eq!(report.as_object().unwrap().len(), 7);
    fixture.assert_unchanged();
}

#[test]
fn current_version_and_checking_are_flushed_before_the_network_response() {
    let fixture = NativeFixture::new();
    let (release, gate) = mpsc::channel();
    let (address, server) = serve_manifest(manifest("9.9.9"), Some(gate));
    let mut child = fixture
        .command(&address)
        .args(["update", "--check"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    // Starting the installed launcher can be slow on cold/loaded CI workers.
    // Keep the manifest response gated while allowing the child enough time to
    // start and flush its three progress lines.
    let early = (0..3)
        .map(|_| lines.recv_timeout(Duration::from_secs(10)))
        .collect::<Result<Vec<_>, _>>();
    // Always release the fixture and reap the child, including on a timing failure.
    release.send(()).unwrap();
    let output = child.wait_with_output().unwrap();
    server.join().unwrap();
    reader.join().unwrap();
    let early = early.expect("version and checking must arrive before any manifest response");
    assert_eq!(early[0], "LazyDB update");
    assert!(early[1].contains(env!("CARGO_PKG_VERSION")), "{early:?}");
    assert!(early[2].contains("Checking for updates"), "{early:?}");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remaining = lines.try_iter().collect::<Vec<_>>().join("\n");
    assert!(
        remaining.contains("A LazyDB update is available"),
        "{remaining}"
    );
    assert!(remaining.contains("9.9.9"), "{remaining}");
    assert!(
        !remaining.contains("Downloading") && !remaining.contains('\x1b'),
        "{remaining}"
    );
    fixture.assert_unchanged();
}

#[test]
fn check_reports_up_to_date_without_installing() {
    let fixture = NativeFixture::new();
    let (address, server) = serve_manifest(manifest(env!("CARGO_PKG_VERSION")), None);
    let output = fixture
        .command(&address)
        .args(["update", "--check"])
        .output()
        .unwrap();
    server.join().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("LazyDB is already up to date"), "{text}");
    assert!(
        !text.contains("Downloading") && !text.contains("updated successfully"),
        "{text}"
    );
    fixture.assert_unchanged();
}

#[test]
fn failed_check_shows_the_cause_and_exits_unsuccessfully() {
    let fixture = NativeFixture::new();
    let (address, server) = serve_manifest("invalid JSON".to_owned(), None);
    let output = fixture
        .command(&address)
        .args(["update", "--check"])
        .output()
        .unwrap();
    server.join().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!output.status.success());
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")) && stdout.contains("Checking"),
        "{stdout}"
    );
    assert!(
        stderr.contains("Could not check for LazyDB updates"),
        "{stderr}"
    );
    assert!(stderr.contains("invalid channel manifest"), "{stderr}");
    assert!(!stdout.contains("[OK]"), "{stdout}");
    fixture.assert_unchanged();
}

#[test]
fn failed_json_check_keeps_the_machine_report_without_progress() {
    let fixture = NativeFixture::new();
    let (address, server) = serve_manifest("invalid JSON".to_owned(), None);
    let output = fixture
        .command(&address)
        .args(["update", "--check", "--json"])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["status"], "error");
    assert_eq!(report.as_object().unwrap().len(), 7);
}
