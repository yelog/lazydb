//! Explicit, real-cloud lifecycle checks. Never run in ordinary CI.
//! Credentials stay in memory; only job identities and fixed progress messages are printed.

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, ensure};
use tokio::io::AsyncReadExt;

use super::*;
use crate::{
    db::{query::QueryOutcome, value::CellValue},
    profile::{CatalogSelection, DatabaseScope, import_connection_url},
};

const USER_FILE: &str = "LAZYDB_TEST_BIGQUERY_CREDENTIALS_FILE";
const SERVICE_FILE: &str = "LAZYDB_TEST_BIGQUERY_SERVICE_ACCOUNT_CREDENTIALS_FILE";
const RESTRICTED_FILE: &str = "LAZYDB_TEST_BIGQUERY_RESTRICTED_CREDENTIALS_FILE";
const BILLING_CAP: u64 = 10 * 1024 * 1024;
const EXPIRY_WAIT: Duration = Duration::from_secs(65 * 60);

fn required(name: &str) -> anyhow::Result<String> {
    let value = std::env::var(name).with_context(|| format!("explicit {name} is required"))?;
    ensure!(!value.trim().is_empty(), "{name} must not be empty");
    Ok(value)
}

fn opt_in(name: &str) -> anyhow::Result<()> {
    ensure!(required(name)? == "1", "{name}=1 is required");
    Ok(())
}

async fn live_profile(file_variable: &str) -> anyhow::Result<(ConnectionProfile, String)> {
    let project = required("LAZYDB_TEST_BIGQUERY_PROJECT")?;
    let location = required("LAZYDB_TEST_BIGQUERY_LOCATION")?;
    let file = required(file_variable)?;
    ensure!(
        Path::new(&file).is_absolute(),
        "explicit credentials path must be absolute"
    );
    let mut bytes = Vec::new();
    tokio::fs::File::open(&file)
        .await
        .context("cannot open designated credentials file")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .context("cannot read designated credentials file")?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "designated credentials exceed 1 MiB"
    );
    // Do not attach JSON parse errors or print the parsed document.
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid designated credentials JSON"))?;
    let kind = json
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    ensure!(
        matches!(kind.as_str(), "authorized_user" | "service_account"),
        "unsupported designated credentials type"
    );
    let mut profile = import_connection_url(
        &format!("bigquery://{project}"),
        Some("BigQuery lifecycle acceptance"),
    )?
    .profile;
    let options = profile
        .bigquery_options
        .as_mut()
        .context("BigQuery options missing")?;
    options.location = Some(location);
    options.credentials_file = Some(file);
    options.maximum_bytes_billed = BILLING_CAP;
    options.query_timeout_secs = 30;
    // Constant/computation tests intentionally have no default dataset, so they
    // work without recreating previously cleaned-up acceptance datasets.
    profile.validate_bigquery()?;
    Ok((profile, kind))
}

fn check_constant(result: &QueryOutcome, profile: &ConnectionProfile) -> anyhow::Result<()> {
    ensure!(
        result
            .result_sets
            .first()
            .is_some_and(|set| set.rows == vec![vec![CellValue::Integer(42)]]),
        "constant query returned unexpected results"
    );
    let stats = result
        .stats
        .bigquery
        .as_ref()
        .context("native job statistics missing")?;
    ensure!(
        Some(stats.project.as_str()) == profile.database.as_deref(),
        "job escaped execution project"
    );
    ensure!(
        stats
            .location
            .as_deref()
            .zip(
                profile
                    .bigquery_options
                    .as_ref()
                    .and_then(|options| options.location.as_deref())
            )
            .is_some_and(|(actual, expected)| actual.eq_ignore_ascii_case(expected)),
        "job escaped configured location"
    );
    ensure!(
        stats.total_bytes_processed == Some(0),
        "constant query unexpectedly scanned data"
    );
    ensure!(
        stats
            .total_bytes_billed
            .is_some_and(|bytes| bytes <= BILLING_CAP),
        "job exceeded billing cap or statistics are missing"
    );
    eprintln!("verified constant job {}", stats.job_id);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real cloud, explicit RUN_EXPIRY=1; waits 65 minutes in one process"]
async fn bigquery_live_natural_token_expiry_same_connection() -> anyhow::Result<()> {
    opt_in("LAZYDB_TEST_BIGQUERY_RUN_EXPIRY")?;
    let (profile, kind) = live_profile(USER_FILE).await?;
    ensure!(
        kind == "authorized_user",
        "expiry acceptance requires developer authorized_user ADC"
    );
    let adapter = BigQueryAdapter::connect(&profile).await?;
    let before = adapter.credentials.authorization().await?;
    check_constant(
        &adapter.execute("SELECT 42 AS expiry_answer").await?,
        &profile,
    )?;
    let start = std::time::Instant::now();
    eprintln!(
        "waiting 65 real minutes with the same adapter and SDK provider; no reconnect or simulated clock"
    );
    while start.elapsed() < EXPIRY_WAIT {
        tokio::time::sleep(
            (EXPIRY_WAIT - start.elapsed().min(EXPIRY_WAIT)).min(Duration::from_secs(60)),
        )
        .await;
        if (start.elapsed().as_secs() / 60).is_multiple_of(5) {
            eprintln!(
                "expiry acceptance elapsed {} minutes",
                start.elapsed().as_secs() / 60
            );
        }
    }
    let after = adapter.credentials.authorization().await?;
    // Header values are sensitive. A boolean check must not format either token.
    ensure!(
        before != after,
        "SDK token did not rotate across the natural lifetime interval"
    );
    check_constant(
        &adapter.execute("SELECT 42 AS expiry_answer").await?,
        &profile,
    )?;
    adapter.close().await;
    eprintln!(
        "PASS: token rotated and the same connection queried successfully after 65 real minutes"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "real cloud, explicit RUN_CANCEL=1; bounded no-table computation"]
async fn bigquery_live_running_job_confirmed_stopped() -> anyhow::Result<()> {
    opt_in("LAZYDB_TEST_BIGQUERY_RUN_CANCEL")?;
    let (profile, _) = live_profile(USER_FILE).await?;
    let adapter = Arc::new(BigQueryAdapter::connect(&profile).await?);
    let mut guard = JobGuard {
        adapter: adapter.clone(),
        job: JobIdentity {
            project: adapter.project.clone(),
            id: format!("lazydb_cancel_{}", Uuid::new_v4().simple()),
            location: adapter.location.clone(),
        },
        armed: true,
    };
    // Exactly 100 million generated pairs, zero scanned tables, bounded by the
    // native 20-second timeout and 10 MiB cap. A unique salt prevents cache reuse.
    let sql = format!(
        "SELECT BIT_XOR(FARM_FINGERPRINT(CONCAT(CAST(x AS STRING), ':', CAST(y AS STRING), ':{}'))) AS fingerprint FROM UNNEST(GENERATE_ARRAY(1, 10000)) AS x CROSS JOIN UNNEST(GENERATE_ARRAY(1, 10000)) AS y",
        Uuid::new_v4().simple()
    );
    let mut body = adapter.job_body(&sql, &guard.job.id);
    body["configuration"]["jobTimeoutMs"] = json!("20000");
    body["configuration"]["query"]["useQueryCache"] = json!(false);
    let validation: anyhow::Result<()> = async {
        let mut resource = match adapter.request(Method::POST, &["projects", &guard.job.project, "jobs"], &[], Some(&body)).await {
            Ok((value, _)) => value,
            // Recover only the original identity; never replay the computation.
            Err(_) => adapter.get(&["projects", &guard.job.project, "jobs", &guard.job.id], &guard.job.params()).await.context("submission uncertain; original job was inspected without SQL replay")?,
        };
        guard.job.accept_reference(&resource)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        loop {
            match resource.pointer("/status/state").and_then(Value::as_str) {
                Some("RUNNING") => break,
                Some("DONE") => anyhow::bail!("job finished before RUNNING was observed; active-job cancellation remains unverified"),
                Some("PENDING") => {},
                _ => anyhow::bail!("job returned an invalid native state"),
            }
            ensure!(tokio::time::Instant::now() < deadline, "RUNNING was not observed before the acceptance deadline");
            tokio::time::sleep(Duration::from_millis(100)).await;
            resource = adapter.get(&["projects", &guard.job.project, "jobs", &guard.job.id], &guard.job.params()).await?;
            guard.job.accept_reference(&resource)?;
        }
        eprintln!("observed RUNNING for {}", guard.job.id);
        let cancellation = adapter.cancel_job(&guard.job, "live acceptance").await;
        eprintln!("{}", cancellation.message);
        if cancellation.code.as_deref() != Some("cancel_requested") {
            // Diagnose a failed cancellation without replaying SQL or changing
            // its failed acceptance verdict. Cancelling the same job is safe.
            if let Err(error) = adapter.request(Method::POST, &["projects", &guard.job.project, "jobs", &guard.job.id, "cancel"], &guard.job.params(), None).await {
                eprintln!("native cancel diagnostic: category={:?}, code={:?}, {}", error.category, error.code, error.message);
            }
        }
        ensure!(cancellation.code.as_deref() == Some("cancel_requested"), "native cancellation was not accepted");
        // Acceptance is not termination: independently poll the same job.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            resource = adapter.get(&["projects", &guard.job.project, "jobs", &guard.job.id], &guard.job.params()).await?;
            guard.job.accept_reference(&resource)?;
            if resource.pointer("/status/state").and_then(Value::as_str) == Some("DONE") {
                guard.armed = false;
                ensure!(resource.pointer("/status/errorResult/reason").and_then(Value::as_str) == Some("stopped"), "job reached DONE without reason=stopped; cancellation raced with completion or another failure");
                break;
            }
            ensure!(tokio::time::Instant::now() < deadline, "native final state remains unknown; inspect the recorded original job");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        eprintln!("PASS: original job {} transitioned RUNNING -> DONE with reason=stopped", guard.job.id);
        Ok(())
    }.await;
    if guard.armed {
        // Cleanup is awaited even after ordinary validation errors. Drop is only
        // an interruption fallback, and still addresses this exact job.
        let cleanup = adapter
            .cancel_job(&guard.job, "live acceptance cleanup")
            .await;
        eprintln!("{}", cleanup.message);
        if let Ok(resource) = adapter
            .get(
                &["projects", &guard.job.project, "jobs", &guard.job.id],
                &guard.job.params(),
            )
            .await
            && resource.pointer("/status/state").and_then(Value::as_str) == Some("DONE")
        {
            guard.armed = false;
        }
    }
    adapter.close().await;
    validation
}

#[tokio::test]
#[ignore = "real cloud, explicit RUN_SERVICE_ACCOUNT=1 and designated existing service-account JSON"]
async fn bigquery_live_service_account_constant_query() -> anyhow::Result<()> {
    opt_in("LAZYDB_TEST_BIGQUERY_RUN_SERVICE_ACCOUNT")?;
    let (profile, kind) = live_profile(SERVICE_FILE).await?;
    ensure!(
        kind == "service_account",
        "designated credentials must be service_account JSON"
    );
    let adapter = BigQueryAdapter::connect(&profile).await?;
    check_constant(
        &adapter
            .execute("SELECT 42 AS service_account_answer")
            .await?,
        &profile,
    )?;
    adapter.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "real cloud, explicit RUN_RESTRICTED=1 and pre-existing restricted identity/table"]
async fn bigquery_live_restricted_dataset_access_and_job_denial() -> anyhow::Result<()> {
    opt_in("LAZYDB_TEST_BIGQUERY_RUN_RESTRICTED")?;
    let (mut profile, _) = live_profile(RESTRICTED_FILE).await?;
    let data_project = required("LAZYDB_TEST_BIGQUERY_DATA_PROJECT")?;
    let dataset = required("LAZYDB_TEST_BIGQUERY_DATASET")?;
    let table = required("LAZYDB_TEST_BIGQUERY_RESTRICTED_TABLE")?;
    let denied_project = required("LAZYDB_TEST_BIGQUERY_DENIED_PROJECT")?;
    ensure!(
        table
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            && table.len() <= 1024,
        "designated fixture table must be a simple native name"
    );
    crate::profile::validate_bigquery_project(&denied_project)?;
    ensure!(
        Some(denied_project.as_str()) != profile.database.as_deref(),
        "denied project must differ from the permitted execution project"
    );
    profile.default_schema = Some(dataset.clone());
    profile
        .bigquery_options
        .as_mut()
        .context("BigQuery options missing")?
        .data_project = Some(data_project.clone());
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: data_project.clone(),
            schemas: CatalogSelection::Selected(vec![dataset.clone()]),
        }]),
    };
    profile.validate_bigquery()?;
    let adapter = BigQueryAdapter::connect(&profile).await?;
    // Confirm the fixture really exercises dataset-only discovery, rather than
    // treating a broad owner identity as a restricted-IAM acceptance pass.
    let mut params = vec![("maxResults", "1000".to_owned())];
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = adapter.get(&["projects"], &params).await;
        let page = match page {
            Ok(page) => page,
            Err(error) if error.category == ErrorCategory::Permission => break,
            Err(error) => return Err(error.into()),
        };
        ensure!(
            !page
                .get("projects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|entry| entry
                    .pointer("/projectReference/projectId")
                    .and_then(Value::as_str)
                    == Some(data_project.as_str())),
            "data project is visible through project-level listing; fixture is not dataset-only"
        );
        let Some(token) = page
            .get("nextPageToken")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            break;
        };
        ensure!(
            seen.len() < 100 && seen.insert(token.to_owned()),
            "project listing did not complete within its acceptance bound"
        );
        params.retain(|(key, _)| *key != "pageToken");
        params.push(("pageToken", token.to_owned()));
    }
    let discovery = adapter.discover_catalog_scope().await?;
    ensure!(
        discovery
            .databases
            .iter()
            .any(|db| db.name == data_project && db.schemas.contains(&dataset)),
        "explicitly selected dataset disappeared from discovery"
    );
    let qualified = super::super::quote_identifier(&format!("{data_project}.{dataset}.{table}"));
    let result = adapter
        .execute(&format!("SELECT * FROM {qualified} LIMIT 1"))
        .await?;
    ensure!(
        !result.result_sets.is_empty(),
        "qualified read did not retain schema"
    );
    let relation = adapter
        .get(
            &[
                "projects",
                &data_project,
                "datasets",
                &dataset,
                "tables",
                &table,
            ],
            &[],
        )
        .await?;
    ensure!(
        relation
            .pointer("/tableReference/tableId")
            .and_then(Value::as_str)
            == Some(table.as_str()),
        "restricted metadata access did not retain fixture identity"
    );
    let mut denied = profile.clone();
    denied.database = Some(denied_project);
    let error = BigQueryAdapter::connect(&denied).await.expect_err(
        "designated identity unexpectedly has job creation permission in the denied project",
    );
    ensure!(
        error.category == ErrorCategory::Permission
            && error.code.as_deref() == Some("accessDenied")
            && error.message.contains("bigquery.jobs.create"),
        "failure did not establish the intended job-creation IAM denial"
    );
    adapter.close().await;
    eprintln!(
        "PASS: dataset-only explicit access and missing job-creation permission were independently verified"
    );
    Ok(())
}
