use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tower_lsp_server::jsonrpc::Result as LspResult;
use tower_lsp_server::ls_types::{
    CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    InitializeParams, InitializeResult, InitializedParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind,
};
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use super::catalog::{CatalogProvider, CatalogTargetLoader, CatalogTargetService};
use super::completion::{complete_document_with_catalog, complete_document_with_embedded_sql};
use super::diagnostics::diagnostics_for_document;
use super::document::Documents;
use crate::sql::{CompletionContext, CompletionIndex, SqlDialect};

#[derive(Debug)]
struct LanguageServerState {
    client: Client,
    project: Option<PathBuf>,
    dialect: crate::cli::LspDialect,
    documents: tokio::sync::Mutex<Documents>,
    catalog_service: Option<std::sync::Arc<CatalogTargetService>>,
}

impl LanguageServer for LanguageServerState {
    async fn initialize(&self, _: InitializeParams) -> LspResult<InitializeResult> {
        let _ = (&self.project, self.dialect);
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".into()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(
                tower_lsp_server::ls_types::MessageType::INFO,
                "LazyDB language server initialized",
            )
            .await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let document = {
            let documents = self.documents.lock().await;
            let Some(document) = documents.get(&params.text_document_position.text_document.uri)
            else {
                return Ok(None);
            };
            document.clone()
        };
        let position = params.text_document_position.position;
        let dialect = sql_dialect(self.dialect);
        let response = match &self.catalog_service {
            Some(service) => {
                complete_document_with_catalog(
                    &document,
                    position,
                    dialect,
                    service,
                    self.context(),
                )
                .await
            }
            None => complete_document_with_embedded_sql(
                &document,
                position,
                dialect,
                &CompletionIndex::new(&[]),
                true,
                CompletionContext::default(),
            ),
        };
        Ok(Some(response))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = self.documents.lock().await.open(params);
        self.publish_diagnostics(&document).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let changed = self.documents.lock().await.change(params);
        if let Some(document) = changed {
            self.publish_diagnostics(&document).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.documents.lock().await.save(params);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.lock().await.close(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}

impl LanguageServerState {
    async fn publish_diagnostics(&self, document: &super::document::Document) {
        let diagnostics = diagnostics_for_document(document, sql_dialect(self.dialect));
        self.client
            .publish_diagnostics(document.uri.clone(), diagnostics, Some(document.version))
            .await;
    }

    fn context(&self) -> CompletionContext<'_> {
        self.catalog_service
            .as_ref()
            .map_or(CompletionContext::default(), |service| {
                service.completion_context()
            })
    }
}

pub async fn run(
    args: crate::cli::LspArgs,
    config: Option<PathBuf>,
    profile: Option<String>,
) -> Result<()> {
    let provider =
        match CatalogProvider::from_config(args.project.as_deref(), config, profile.as_deref())
            .await
        {
            Ok(provider) => {
                eprintln!(
                    "lazydb lsp: catalog provider {}",
                    if provider.is_some() {
                        "initialized"
                    } else {
                        "not selected"
                    }
                );
                provider
            }
            Err(error) => {
                eprintln!("lazydb lsp: catalog unavailable; using offline completion: {error}");
                None
            }
        };
    let dialect = match args.dialect {
        Some(dialect) => dialect,
        None => provider
            .as_ref()
            .map(|provider| provider.dialect())
            .unwrap_or_default(),
    };
    run_server_with_provider(args.project, dialect, provider).await
}

pub async fn run_server(
    project: Option<PathBuf>,
    dialect: crate::cli::LspDialect,
    _config: Option<PathBuf>,
) -> Result<()> {
    run_server_with_provider(project, dialect, None).await
}

async fn run_server_with_provider(
    project: Option<PathBuf>,
    dialect: crate::cli::LspDialect,
    catalog_provider: Option<CatalogProvider>,
) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let catalog_service = catalog_provider.map(|provider| {
        let scope = provider.catalog_scope().clone();
        let context = provider.completion_context();
        let database = context.database.map(str::to_owned);
        let schema = context.schema.map(str::to_owned);
        let loader: Arc<dyn CatalogTargetLoader> = std::sync::Arc::new(provider);
        CatalogTargetService::new(loader, scope, database, schema)
    });
    let (service, socket) = LspService::new(move |client| LanguageServerState {
        client,
        project,
        dialect,
        documents: tokio::sync::Mutex::new(Documents::default()),
        catalog_service,
    });
    Server::new(stdin, stdout, socket)
        .concurrency_level(8)
        .serve(service)
        .await;
    Ok(())
}

fn sql_dialect(dialect: crate::cli::LspDialect) -> SqlDialect {
    match dialect {
        crate::cli::LspDialect::Postgres => SqlDialect::Postgres,
        crate::cli::LspDialect::MySql => SqlDialect::MySql,
        crate::cli::LspDialect::SqlServer => SqlDialect::SqlServer,
        crate::cli::LspDialect::Sqlite => SqlDialect::Sqlite,
        crate::cli::LspDialect::ClickHouse => SqlDialect::ClickHouse,
        crate::cli::LspDialect::Snowflake => SqlDialect::Snowflake,
        crate::cli::LspDialect::DuckDb => SqlDialect::DuckDb,
        crate::cli::LspDialect::Generic => SqlDialect::Generic,
    }
}
