use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail};
use rmcp::transport::TokioChildProcess;
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

#[derive(Debug, serde::Serialize)]
pub struct ProbeReport {
    pub stage: &'static str,
    pub status: &'static str,
    pub tool_count: Option<usize>,
    pub detail: Option<String>,
}

pub async fn run(
    command: &[String],
    cwd: Option<&Path>,
    environment: &[(String, String)],
    startup_timeout: Duration,
    catalog_timeout: Duration,
) -> Result<ProbeReport> {
    let (program, args) = command.split_first().context("MCP command is empty")?;
    let mut child_command = Command::new(program);
    child_command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        child_command.current_dir(cwd);
    }
    for (key, value) in environment {
        child_command.env(key, value);
    }

    let (transport, stderr) = TokioChildProcess::builder(child_command)
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start MCP server")?;
    let stderr_task = stderr.map(|mut stderr| {
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes).await;
            bytes.truncate(64 * 1024);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    });

    let mut client = match timeout(startup_timeout, rmcp::serve_client((), transport)).await {
        Ok(Ok(client)) => client,
        Ok(Err(error)) => bail!("MCP initialize failed: {error}"),
        Err(_) => bail!(
            "MCP initialize timed out after {} ms",
            startup_timeout.as_millis()
        ),
    };
    let tools = match timeout(catalog_timeout, client.list_all_tools()).await {
        Ok(Ok(tools)) => tools,
        Ok(Err(error)) => bail!("MCP tools/list failed: {error}"),
        Err(_) => bail!(
            "MCP tools/list timed out after {} ms",
            catalog_timeout.as_millis()
        ),
    };
    let _ = client.close_with_timeout(Duration::from_secs(3)).await;
    if let Some(task) = stderr_task {
        let _ = task.await;
    }
    Ok(ProbeReport {
        stage: "tools/list",
        status: "ok",
        tool_count: Some(tools.len()),
        detail: None,
    })
}
