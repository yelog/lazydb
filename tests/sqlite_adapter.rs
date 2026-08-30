use std::collections::HashMap;

use futures_util::future::join_all;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCapabilities, CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry,
            CatalogId, CatalogKind, CatalogMetadata, CatalogRequest, CatalogRequestKey,
            CatalogSearchRequest, CatalogTarget, ColumnMetadata, ColumnMetadataCapabilities,
            ConstraintMembership, ConstraintMetadata, DdlProvenance, IndexMetadata, NamespaceModel,
            ObjectGroup, OptionalMetadata,
        },
        value::CellValue,
    },
    identity::ConnectionIdentity,
    profile::{CatalogScope, CatalogSelection, DatabaseScope, import_connection_url},
};
use tempfile::TempDir;
use uuid::Uuid;

const ATTACHED_ALIAS: &str = "ArchiveCase";

#[tokio::test]
async fn relation_preview_preserves_metadata_limits_quotes_and_rejects_forged_ids() {
    let imported = import_connection_url("sqlite://:memory:", Some("relation-preview")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            r#"
            CREATE TABLE "odd""table" ("odd""column" INTEGER, "payload" TEXT);
            CREATE TABLE empty (id INTEGER, note TEXT);
            CREATE TABLE many (id INTEGER);
            CREATE VIEW "odd""view" AS SELECT "odd""column", "payload" FROM "odd""table";
            WITH RECURSIVE numbers(value) AS (
                SELECT 1 UNION ALL SELECT value + 1 FROM numbers WHERE value < 501
            ) INSERT INTO many SELECT value FROM numbers;
            "#,
        )
        .await
        .unwrap_or_else(|_| panic!("SQLite test setup must support the requested DDL"));

    let empty = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "empty"],
    );
    let preview = database
        .preview_relation(&empty, &Default::default())
        .await
        .unwrap();
    assert!(preview.sql.contains("LIMIT 500"));
    assert_eq!(preview.result.stats.row_count, 0);
    assert_eq!(
        preview.result.result_sets[0]
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.type_name.as_str()))
            .collect::<Vec<_>>(),
        [("id", "INTEGER"), ("note", "TEXT")]
    );

    let many = CatalogId::new(profile_id, CatalogKind::Table, [":memory:", "main", "many"]);
    let many_preview = database
        .preview_relation(&many, &Default::default())
        .await
        .unwrap();
    assert_eq!(many_preview.result.stats.row_count, 500);
    assert_eq!(many_preview.result.result_sets[0].rows.len(), 500);

    let hostile = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "odd\"table"],
    );
    let hostile_preview = database
        .preview_relation(&hostile, &Default::default())
        .await
        .unwrap();
    assert!(hostile_preview.sql.contains("\"odd\"\"table\""));
    assert!(hostile_preview.sql.contains("LIMIT 500"));

    let view = CatalogId::new(
        profile_id,
        CatalogKind::View,
        [":memory:", "main", "odd\"view"],
    );
    let view_preview = database
        .preview_relation(&view, &Default::default())
        .await
        .unwrap();
    assert_eq!(view_preview.result.stats.row_count, 0);

    for id in [
        CatalogId::new(
            Uuid::new_v4(),
            CatalogKind::Table,
            [":memory:", "main", "empty"],
        ),
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            [":memory:", "main", "empty", "id"],
        ),
        CatalogId::new(
            profile_id,
            CatalogKind::Table,
            [":memory:", "main", "missing"],
        ),
    ] {
        let error = database
            .preview_relation(&id, &Default::default())
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
    }
    database.close().await;
}

#[tokio::test]
async fn relation_ddl_with_only_the_relation_uses_native_catalog_provenance() {
    let imported = import_connection_url("sqlite://:memory:", Some("relation-ddl")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute("CREATE TABLE records (id INTEGER PRIMARY KEY, label TEXT NOT NULL DEFAULT 'x');")
        .await
        .unwrap();
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let ddl = database.relation_ddl(&relation).await.unwrap();
    assert_eq!(ddl.relation.id, relation);
    assert!(ddl.sql.contains("CREATE TABLE records"));
    assert_eq!(ddl.provenance, DdlProvenance::NativeCatalog);
    assert_eq!(ddl.relation.native_kind, "table");
    database.close().await;
}

#[tokio::test]
async fn relation_ddl_includes_explicit_indexes_and_triggers_but_not_autoindexes() {
    let imported =
        import_connection_url("sqlite://:memory:", Some("relation-composite-ddl")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            r#"
            CREATE TABLE records (
                id INTEGER PRIMARY KEY,
                label TEXT NOT NULL UNIQUE,
                updated_at TEXT
            );
            CREATE INDEX records_label_expression_idx ON records (lower(label));
            CREATE TRIGGER records_set_updated_at
            AFTER UPDATE ON records
            BEGIN
                UPDATE records SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
            END;
            "#,
        )
        .await
        .unwrap();

    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let ddl = database.relation_ddl(&relation).await.unwrap();

    assert!(ddl.sql.contains("-- Table\n\nCREATE TABLE records"));
    assert!(ddl.sql.contains(
        "-- Indexes\n\nCREATE INDEX records_label_expression_idx ON records (lower(label));"
    ));
    assert!(
        ddl.sql
            .contains("-- Triggers\n\nCREATE TRIGGER records_set_updated_at")
    );
    assert!(!ddl.sql.contains("sqlite_autoindex"));
    assert!(ddl.sql.trim_end().ends_with("END;"));
    assert_eq!(ddl.provenance, DdlProvenance::AdapterGenerated);
    assert!(ddl.children.entries.iter().any(|entry| {
        entry.kind == CatalogKind::Column && entry.qualified_name.object == "label"
    }));

    database.close().await;
}

#[tokio::test]
async fn relation_ddl_rejects_excluded_scope_before_loading_ddl() {
    let imported = import_connection_url("sqlite://:memory:", Some("relation-scope")).unwrap();
    let profile_id = imported.profile.id;
    let mut profile = imported.profile;
    profile.catalog_scope = selected_scope(":memory:", &["other"]);
    let database = DatabaseConnection::connect(&profile, None).await.unwrap();
    database
        .execute("CREATE TABLE records (id INTEGER);")
        .await
        .unwrap();

    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let error = database.relation_ddl(&relation).await.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    database.close().await;
}

struct CatalogFixture {
    _temp: TempDir,
    database: DatabaseConnection,
    profile_id: Uuid,
    configured_database: String,
}

impl CatalogFixture {
    async fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let main_path = temp.path().join("catalog.db");
        let attached_path = temp.path().join("archive.db");
        let imported = import_connection_url(
            &format!("sqlite://{}", main_path.display()),
            Some("catalog-pages"),
        )
        .unwrap();
        let profile_id = imported.profile.id;
        let configured_database = imported.profile.database.clone().unwrap();
        let database = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap();
        let attached_path = attached_path.to_string_lossy().replace('\'', "''");

        database
            .execute(&format!(
                r#"
                PRAGMA foreign_keys = ON;
                ATTACH DATABASE '{attached_path}' AS "ArchiveCase";

                CREATE TABLE parent (
                    tenant_id INTEGER,
                    parent_id INTEGER,
                    code TEXT NOT NULL,
                    PRIMARY KEY (tenant_id, parent_id),
                    UNIQUE (tenant_id, code)
                ) WITHOUT ROWID;
                CREATE TABLE child (
                    tenant_id INTEGER NOT NULL,
                    child_id INTEGER NOT NULL,
                    owner_id INTEGER NOT NULL DEFAULT 7,
                    label TEXT NOT NULL DEFAULT 'new',
                    label_key TEXT GENERATED ALWAYS AS (lower(label)) STORED,
                    PRIMARY KEY (tenant_id, child_id),
                    UNIQUE (tenant_id, label),
                    FOREIGN KEY (tenant_id, owner_id)
                        REFERENCES parent(tenant_id, parent_id)
                ) WITHOUT ROWID;
                CREATE INDEX child_lookup_idx ON child(owner_id, label);
                CREATE TABLE Alpha (id INTEGER PRIMARY KEY);
                CREATE TABLE beta (id INTEGER PRIMARY KEY);
                CREATE TABLE shared_name (main_value TEXT);
                CREATE VIEW child_view AS
                    SELECT tenant_id, child_id, label_key FROM child;
                CREATE TRIGGER child_label_guard BEFORE INSERT ON child
                    WHEN NEW.label = ''
                    BEGIN
                        SELECT RAISE(ABORT, 'label required');
                    END;

                CREATE TABLE "ArchiveCase".shared_name (archive_value INTEGER);
                CREATE TABLE "ArchiveCase".excluded_only (id INTEGER PRIMARY KEY);
                "#
            ))
            .await
            .unwrap();

        Self {
            _temp: temp,
            database,
            profile_id,
            configured_database,
        }
    }

    fn scope(&self, schemas: &[&str]) -> CatalogScope {
        selected_scope(&self.configured_database, schemas)
    }

    fn database_id(&self) -> CatalogId {
        CatalogId::new(
            self.profile_id,
            CatalogKind::Database,
            [self.configured_database.clone()],
        )
    }

    fn schema_id(&self, schema: &str) -> CatalogId {
        CatalogId::new(
            self.profile_id,
            CatalogKind::Schema,
            [self.configured_database.clone(), schema.to_owned()],
        )
    }

    async fn close(self) {
        self.database.close().await;
    }

    async fn search(
        &self,
        query: &str,
        schemas: &[&str],
        limit: usize,
    ) -> lazydb::db::catalog::CatalogSearchPage {
        self.database
            .search_catalog(&CatalogSearchRequest {
                connection: ConnectionIdentity {
                    profile_id: self.profile_id,
                    generation: 9,
                },
                session_id: 11,
                generation: 12,
                query: query.into(),
                scope: self.scope(schemas),
                limit,
            })
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn catalog_search_finds_unloaded_objects_children_paths_scope_and_limit() {
    let fixture = CatalogFixture::new().await;
    let connection = ConnectionIdentity {
        profile_id: fixture.profile_id,
        generation: 9,
    };
    let page = fixture
        .database
        .search_catalog(&CatalogSearchRequest {
            connection,
            session_id: 1,
            generation: 4,
            query: "CHILD.LABEL".into(),
            scope: fixture.scope(&["main"]),
            limit: 100,
        })
        .await
        .unwrap();

    assert!(page.hits.iter().any(|hit| {
        hit.entry.kind == CatalogKind::Column
            && hit.entry.qualified_name.object == "label"
            && hit.qualified_path().to_lowercase().contains("child.label")
    }));
    assert!(page.hits.iter().all(|hit| {
        hit.entry
            .qualified_name
            .schema
            .as_deref()
            .is_none_or(|schema| schema == "main")
    }));

    let limited = fixture
        .database
        .search_catalog(&CatalogSearchRequest {
            connection,
            session_id: 1,
            generation: 5,
            query: "a".into(),
            scope: fixture.scope(&["main"]),
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(limited.hits.len(), 1);
    assert!(limited.truncated);
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_preserves_all_sqlite_kinds_metadata_and_ancestors() {
    let fixture = CatalogFixture::new().await;

    let cases = [
        (&fixture.configured_database[..], CatalogKind::Database),
        ("MAIN", CatalogKind::Schema),
        ("child", CatalogKind::Table),
        ("child_view", CatalogKind::View),
        ("child_label_guard", CatalogKind::Trigger),
        ("label_key", CatalogKind::Column),
        ("child_lookup_idx", CatalogKind::Index),
        ("sqlite_autoindex_child_1", CatalogKind::PrimaryKey),
        ("sqlite_autoindex_child_2", CatalogKind::UniqueConstraint),
        ("fk_child_0", CatalogKind::ForeignKey),
    ];
    for (query, kind) in cases {
        let page = fixture.search(query, &["main"], 100).await;
        assert_eq!(page.total_count, None);
        let hit = page
            .hits
            .iter()
            .find(|hit| {
                hit.entry.kind == kind
                    && hit.entry.qualified_name.object.eq_ignore_ascii_case(query)
            })
            .unwrap_or_else(|| panic!("missing {kind:?} search hit for {query}"));
        assert_eq!(hit.entry.id.connection_id, fixture.profile_id);
        if kind.is_relation_child() {
            assert_eq!(hit.ancestors.len(), 3);
            assert!(hit.ancestors[2].kind.is_relation());
            assert_eq!(hit.entry.relation_id.as_ref(), Some(&hit.ancestors[2].id));
        }
    }

    let fallback = fixture.search("primary_key", &["main"], 100).await;
    let alpha_primary_key = fallback
        .hits
        .iter()
        .find(|hit| {
            hit.entry.kind == CatalogKind::PrimaryKey
                && hit
                    .ancestors
                    .last()
                    .is_some_and(|owner| owner.qualified_name.object == "Alpha")
        })
        .expect("INTEGER PRIMARY KEY must expose the fallback primary_key entry");
    assert_eq!(
        alpha_primary_key.entry.id.native_path.last().unwrap(),
        "primary_key"
    );
    assert_eq!(
        alpha_primary_key.entry.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
            columns: vec!["id".to_owned()]
        })
    );

    let trigger = fixture.search("child_label_guard", &["main"], 100).await;
    let trigger = trigger
        .hits
        .iter()
        .find(|hit| hit.entry.kind == CatalogKind::Trigger)
        .unwrap();
    assert_eq!(
        trigger
            .ancestors
            .iter()
            .map(|entry| entry.qualified_name.object.as_str())
            .collect::<Vec<_>>(),
        [fixture.configured_database.as_str(), "main", "child"]
    );
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_is_case_insensitive_scoped_literal_and_globally_ranked() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute(
            r#"
            CREATE TABLE "literal%name" ("literal_value" TEXT);
            CREATE TABLE "literal_name" (other TEXT);
            CREATE TABLE rankneedle (path_only TEXT);
            CREATE TABLE rankneedle_prefix (id INTEGER);
            CREATE TABLE xrankneedlex (id INTEGER);
            "#,
        )
        .await
        .unwrap();

    let percent = fixture.search("%", &["main"], 100).await;
    assert!(
        percent
            .hits
            .iter()
            .any(|hit| hit.entry.qualified_name.object == "literal%name")
    );
    assert!(
        percent
            .hits
            .iter()
            .all(|hit| hit.qualified_path().contains('%'))
    );

    let underscore = fixture.search("_", &["main"], 100).await;
    assert!(underscore.hits.iter().all(|hit| {
        hit.entry.qualified_name.object.contains('_') || hit.qualified_path().contains('_')
    }));
    let scoped = fixture.search("SHARED_NAME", &[ATTACHED_ALIAS], 100).await;
    assert!(scoped.hits.iter().any(|hit| {
        hit.entry.kind == CatalogKind::Table
            && hit.entry.qualified_name.schema.as_deref() == Some(ATTACHED_ALIAS)
    }));
    assert!(scoped.hits.iter().all(|hit| {
        hit.entry
            .qualified_name
            .schema
            .as_deref()
            .is_none_or(|schema| schema == ATTACHED_ALIAS)
    }));

    let ranked = fixture.search("RANKNEEDLE", &["main"], 100).await;
    let ranks = ranked
        .hits
        .iter()
        .map(|hit| {
            let name = hit.entry.qualified_name.object.to_lowercase();
            if name == "rankneedle" {
                0
            } else if name.starts_with("rankneedle") {
                1
            } else if name.contains("rankneedle") {
                2
            } else {
                3
            }
        })
        .collect::<Vec<_>>();
    assert!(ranks.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(ranks.first(), Some(&0));
    assert!(
        ranks.contains(&3),
        "qualified owner paths must participate in ranking"
    );

    let repeated = fixture.search("rankneedle", &["main"], 100).await;
    assert_eq!(
        ranked
            .hits
            .iter()
            .map(|hit| &hit.entry.id)
            .collect::<Vec<_>>(),
        repeated
            .hits
            .iter()
            .map(|hit| &hit.entry.id)
            .collect::<Vec<_>>()
    );
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_rejects_wrong_profile_and_skips_unrelated_relation_hydration() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute("CREATE TABLE poison (value INTEGER REFERENCES missing_parent);")
        .await
        .unwrap();

    let page = fixture.search("child.label", &["main"], 100).await;
    assert!(
        page.hits
            .iter()
            .any(|hit| hit.entry.qualified_name.object == "label")
    );

    let error = fixture
        .database
        .search_catalog(&CatalogSearchRequest {
            connection: ConnectionIdentity {
                profile_id: Uuid::new_v4(),
                generation: 9,
            },
            session_id: 1,
            generation: 1,
            query: "child".into(),
            scope: fixture.scope(&["main"]),
            limit: 100,
        })
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("invalid_catalog_request"));
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_trigger_path_uses_the_canonical_owner() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute(
            r#"
            CREATE TABLE MixedOwner (id INTEGER PRIMARY KEY);
            CREATE TRIGGER mixed_table_guard BEFORE INSERT ON mixedowner
                BEGIN SELECT 1; END;
            PRAGMA writable_schema = ON;
            INSERT INTO sqlite_schema (type, name, tbl_name, rootpage, sql)
                VALUES (
                    'view',
                    'mixedowner',
                    'mixedowner',
                    0,
                    'CREATE VIEW mixedowner AS SELECT 1 AS id'
                );
            PRAGMA writable_schema = OFF;
            "#,
        )
        .await
        .unwrap();

    let page = fixture
        .search("mixedowner.mixed_table_guard", &["main"], 100)
        .await;
    let trigger = page
        .hits
        .iter()
        .find(|hit| hit.entry.kind == CatalogKind::Trigger)
        .expect("trigger owner path should be searchable");
    let owner = trigger.ancestors.last().unwrap();
    assert_eq!(owner.kind, CatalogKind::Table);
    assert_eq!(owner.qualified_name.object, "MixedOwner");
    assert_eq!(trigger.entry.relation_id.as_ref(), Some(&owner.id));
    fixture.close().await;
}

fn selected_scope(database: &str, schemas: &[&str]) -> CatalogScope {
    CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database.to_owned(),
            schemas: CatalogSelection::Selected(
                schemas.iter().map(|schema| (*schema).to_owned()).collect(),
            ),
        }]),
    }
}

fn catalog_request(
    profile_id: Uuid,
    target: CatalogTarget,
    scope: CatalogScope,
    page_size: usize,
    cursor: Option<CatalogCursor>,
    request_id: u64,
) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 9,
            },
            catalog_epoch: 4,
            request_id,
            target,
            cursor,
        },
        scope,
        page_size,
    }
}

fn expected_catalog_capabilities() -> CatalogCapabilities {
    CatalogCapabilities {
        namespace_model: NamespaceModel::DatabaseAndSchema,
        top_level_groups: vec![
            ObjectGroup::Tables,
            ObjectGroup::Views,
            ObjectGroup::Triggers,
        ],
        column_metadata: ColumnMetadataCapabilities {
            default_expression: true,
            hidden: true,
            ..ColumnMetadataCapabilities::default()
        },
        supports_lazy_children: true,
    }
}

#[tokio::test]
async fn catalog_capabilities_match_implemented_sqlite_metadata() {
    let imported = import_connection_url("sqlite://:memory:", Some("capabilities")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    assert_eq!(
        database.catalog_capabilities(),
        expected_catalog_capabilities()
    );
    database.close().await;
}

#[tokio::test]
async fn catalog_aliases_remain_connection_local_for_file_and_memory_profiles() {
    let fixture = CatalogFixture::new().await;
    assert_connection_local_alias(
        &fixture.database,
        fixture.profile_id,
        &fixture.configured_database,
        ATTACHED_ALIAS,
        "excluded_only",
    )
    .await;
    fixture.close().await;

    let imported = import_connection_url("sqlite://:memory:", Some("memory-alias")).unwrap();
    let profile_id = imported.profile.id;
    let configured_database = imported.profile.database.clone().unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            r#"
            ATTACH DATABASE ':memory:' AS "MemoryAlias";
            CREATE TABLE "MemoryAlias".memory_object (id INTEGER PRIMARY KEY);
            "#,
        )
        .await
        .unwrap();
    assert_connection_local_alias(
        &database,
        profile_id,
        &configured_database,
        "MemoryAlias",
        "memory_object",
    )
    .await;
    database.close().await;
}

async fn assert_connection_local_alias(
    database: &DatabaseConnection,
    profile_id: Uuid,
    configured_database: &str,
    alias: &str,
    expected_object: &str,
) {
    let discoveries = join_all((0..6).map(|_| {
        let database = database.clone();
        async move { database.discover_catalog_scope().await }
    }))
    .await;
    for discovery in discoveries {
        let discovery = discovery.unwrap();
        assert_eq!(discovery.databases.len(), 1);
        assert!(
            discovery.databases[0]
                .schemas
                .iter()
                .any(|schema| schema == alias),
            "every pool acquisition must observe attached alias {alias}"
        );
    }

    let schema_id = CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        [configured_database.to_owned(), alias.to_owned()],
    );
    let target = CatalogTarget::objects(schema_id, ObjectGroup::Tables).unwrap();
    let scope = selected_scope(configured_database, &[alias]);
    let pages = join_all((0..6).map(|request_id| {
        let database = database.clone();
        let request = catalog_request(
            profile_id,
            target.clone(),
            scope.clone(),
            10,
            None,
            request_id,
        );
        async move { database.load_catalog_page(&request).await }
    }))
    .await;
    for page in pages {
        let page = page.unwrap();
        assert!(
            page.entries
                .iter()
                .any(|entry| entry.qualified_name.object == expected_object),
            "every paged request must observe objects in attached alias {alias}"
        );
    }
}

#[tokio::test]
async fn catalog_pages_apply_scope_and_return_exact_group_summaries() {
    let fixture = CatalogFixture::new().await;
    let scope = fixture.scope(&["main"]);

    let databases = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::Databases,
            scope.clone(),
            10,
            None,
            1,
        ))
        .await
        .unwrap();
    assert_eq!(databases.total_count, CatalogCount::Exact(1));
    assert_eq!(databases.entries.len(), 1);
    assert_eq!(databases.entries[0].id, fixture.database_id());
    assert_eq!(databases.entries[0].comment, OptionalMetadata::Unsupported);

    let schemas = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::schemas(fixture.database_id()).unwrap(),
            scope.clone(),
            10,
            None,
            2,
        ))
        .await
        .unwrap();
    assert_eq!(schemas.total_count, CatalogCount::Exact(1));
    assert_eq!(schemas.entries.len(), 1);
    assert_eq!(schemas.entries[0].qualified_name.object, "main");
    assert_eq!(schemas.entries[0].comment, OptionalMetadata::Unsupported);
    assert!(
        schemas
            .entries
            .iter()
            .all(|entry| entry.qualified_name.object != ATTACHED_ALIAS)
    );

    let groups = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::groups(fixture.schema_id("main")).unwrap(),
            scope.clone(),
            10,
            None,
            3,
        ))
        .await
        .unwrap();
    assert_eq!(groups.total_count, CatalogCount::Exact(3));
    assert_eq!(groups.completeness, CatalogCompleteness::Complete);
    assert!(groups.entries.is_empty());
    let counts = groups
        .group_summaries
        .iter()
        .map(|summary| (summary.group, summary.object_count))
        .collect::<HashMap<_, _>>();
    assert_eq!(
        counts.get(&ObjectGroup::Tables),
        Some(&CatalogCount::Exact(5))
    );
    assert_eq!(
        counts.get(&ObjectGroup::Views),
        Some(&CatalogCount::Exact(1))
    );
    assert_eq!(
        counts.get(&ObjectGroup::Triggers),
        Some(&CatalogCount::Exact(1))
    );

    let views = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Views).unwrap(),
            scope.clone(),
            10,
            None,
            4,
        ))
        .await
        .unwrap();
    assert_eq!(views.total_count, CatalogCount::Exact(1));
    assert_eq!(views.entries[0].kind, CatalogKind::View);
    assert_eq!(views.entries[0].qualified_name.object, "child_view");

    let triggers = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Triggers).unwrap(),
            scope,
            10,
            None,
            5,
        ))
        .await
        .unwrap();
    assert_eq!(triggers.total_count, CatalogCount::Exact(1));
    let trigger = &triggers.entries[0];
    assert_eq!(trigger.kind, CatalogKind::Trigger);
    assert_eq!(trigger.parent_id.as_ref(), Some(&fixture.schema_id("main")));
    assert_eq!(
        trigger.relation_id,
        Some(CatalogId::new(
            fixture.profile_id,
            CatalogKind::Table,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "child".to_owned(),
            ],
        ))
    );
    assert_eq!(trigger.comment, OptionalMetadata::Unsupported);

    let case_mismatched_scope = fixture.scope(&["archivecase"]);
    let case_mismatched_schema = fixture.schema_id("archivecase");
    let error = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(case_mismatched_schema, ObjectGroup::Tables).unwrap(),
            case_mismatched_scope,
            10,
            None,
            6,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));

    fixture.close().await;
}

#[tokio::test]
async fn catalog_object_pages_use_stable_binary_keyset_cursors() {
    let fixture = CatalogFixture::new().await;
    let (first_names, first_ids, completeness) = collect_table_pages(&fixture, 20).await;
    let (repeated_names, repeated_ids, _) = collect_table_pages(&fixture, 40).await;

    assert_eq!(
        first_names,
        ["Alpha", "beta", "child", "parent", "shared_name"]
    );
    assert_eq!(first_names, repeated_names);
    assert_eq!(first_ids, repeated_ids);
    assert_eq!(
        first_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        first_ids.len()
    );
    assert_eq!(
        completeness,
        [
            CatalogCompleteness::Partial,
            CatalogCompleteness::Partial,
            CatalogCompleteness::Partial,
            CatalogCompleteness::Partial,
            CatalogCompleteness::Complete,
        ]
    );

    fixture.close().await;
}

async fn collect_table_pages(
    fixture: &CatalogFixture,
    first_request_id: u64,
) -> (Vec<String>, Vec<CatalogId>, Vec<CatalogCompleteness>) {
    let target = CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Tables).unwrap();
    let mut cursor = None;
    let mut names = Vec::new();
    let mut ids = Vec::new();
    let mut completeness = Vec::new();

    for page_index in 0..10 {
        let request = catalog_request(
            fixture.profile_id,
            target.clone(),
            fixture.scope(&["main"]),
            1,
            cursor.clone(),
            first_request_id + page_index,
        );
        let page = fixture.database.load_catalog_page(&request).await.unwrap();
        assert_eq!(page.total_count, CatalogCount::Exact(5));
        assert_eq!(page.entries.len(), 1);
        let entry = &page.entries[0];
        assert_eq!(entry.comment, OptionalMetadata::Unsupported);
        assert_eq!(entry.qualified_name.schema.as_deref(), Some("main"));
        names.push(entry.qualified_name.object.clone());
        ids.push(entry.id.clone());
        completeness.push(page.completeness);

        cursor = page.next_cursor;
        if let Some(next) = cursor.as_ref() {
            let (sort_key, tie_breaker) = next.keyset_parts().unwrap();
            assert!(!sort_key.is_empty());
            assert!(!tie_breaker.is_empty());
        } else {
            break;
        }
    }

    (names, ids, completeness)
}

#[tokio::test]
async fn catalog_relation_children_are_structured_grouped_and_stable() {
    let fixture = CatalogFixture::new().await;
    let relation = CatalogId::new(
        fixture.profile_id,
        CatalogKind::Table,
        [
            fixture.configured_database.clone(),
            "main".to_owned(),
            "child".to_owned(),
        ],
    );
    let target = CatalogTarget::relation_children(relation.clone()).unwrap();
    let request = catalog_request(
        fixture.profile_id,
        target.clone(),
        fixture.scope(&["main"]),
        100,
        None,
        60,
    );
    let page = fixture.database.load_catalog_page(&request).await.unwrap();

    assert_eq!(page.total_count, CatalogCount::Exact(11));
    assert_eq!(page.entries.len(), 11);
    assert_eq!(page.completeness, CatalogCompleteness::Complete);
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.comment == OptionalMetadata::Unsupported)
    );
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.kind != CatalogKind::Trigger)
    );

    let columns = page
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Column(metadata) => {
                Some((entry.qualified_name.object.as_str(), metadata))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(columns.len(), 5);
    assert_column(
        columns["owner_id"],
        3,
        "INTEGER",
        false,
        OptionalMetadata::Supported(Some("7".to_owned())),
        false,
    );
    assert_column(
        columns["label"],
        4,
        "TEXT",
        false,
        OptionalMetadata::Supported(Some("'new'".to_owned())),
        false,
    );
    assert_column(
        columns["label_key"],
        5,
        "TEXT",
        true,
        OptionalMetadata::Supported(None),
        false,
    );
    assert_eq!(
        columns["label_key"].generated_expression,
        OptionalMetadata::Unsupported
    );

    let indexes = page
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Index(metadata) => Some((entry, metadata)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(indexes.len(), 3, "one entry is required per native index");
    let (_, lookup_index) = indexes
        .iter()
        .find(|(entry, _)| entry.qualified_name.object == "child_lookup_idx")
        .unwrap();
    assert_eq!(
        **lookup_index,
        IndexMetadata {
            columns: vec!["owner_id".to_owned(), "label".to_owned()],
            unique: false,
        }
    );

    let primary_key = constraint(&page.entries, CatalogKind::PrimaryKey);
    let unique = constraint(&page.entries, CatalogKind::UniqueConstraint);
    let foreign_key = constraint(&page.entries, CatalogKind::ForeignKey);
    assert_eq!(
        primary_key.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
            columns: vec!["tenant_id".to_owned(), "child_id".to_owned()],
        })
    );
    assert_eq!(
        unique.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::Unique {
            columns: vec!["tenant_id".to_owned(), "label".to_owned()],
        })
    );
    assert_eq!(
        foreign_key.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
            columns: vec!["tenant_id".to_owned(), "owner_id".to_owned()],
            referenced_relation: lazydb::db::catalog::QualifiedName {
                database: Some(fixture.configured_database.clone()),
                schema: Some("main".to_owned()),
                object: "parent".to_owned(),
            },
            referenced_columns: vec!["tenant_id".to_owned(), "parent_id".to_owned()],
        })
    );

    assert_membership(columns["tenant_id"], &primary_key.id, 1);
    assert_membership(columns["tenant_id"], &unique.id, 1);
    assert_membership(columns["tenant_id"], &foreign_key.id, 1);
    assert_membership(columns["child_id"], &primary_key.id, 2);
    assert_membership(columns["label"], &unique.id, 2);
    assert_membership(columns["owner_id"], &foreign_key.id, 2);

    let repeated = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            target,
            fixture.scope(&["main"]),
            100,
            None,
            61,
        ))
        .await
        .unwrap();
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| &entry.id)
            .collect::<Vec<_>>(),
        repeated
            .entries
            .iter()
            .map(|entry| &entry.id)
            .collect::<Vec<_>>()
    );

    fixture.close().await;
}

#[tokio::test]
async fn catalog_primary_key_nullability_uses_sqlite_table_facts() {
    let fixture = CatalogFixture::new().await;

    let rowid_relation = CatalogId::new(
        fixture.profile_id,
        CatalogKind::Table,
        [
            fixture.configured_database.clone(),
            "main".to_owned(),
            "Alpha".to_owned(),
        ],
    );
    let rowid_page = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::relation_children(rowid_relation).unwrap(),
            fixture.scope(&["main"]),
            100,
            None,
            70,
        ))
        .await
        .unwrap();
    let rowid_key = column_metadata(&rowid_page.entries, "id");
    assert!(!rowid_key.nullable, "INTEGER PRIMARY KEY aliases rowid");
    assert_eq!(rowid_key.type_family, OptionalMetadata::Unsupported);

    let without_rowid_relation = CatalogId::new(
        fixture.profile_id,
        CatalogKind::Table,
        [
            fixture.configured_database.clone(),
            "main".to_owned(),
            "parent".to_owned(),
        ],
    );
    let without_rowid_page = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::relation_children(without_rowid_relation).unwrap(),
            fixture.scope(&["main"]),
            100,
            None,
            71,
        ))
        .await
        .unwrap();
    assert!(!column_metadata(&without_rowid_page.entries, "tenant_id").nullable);
    assert!(!column_metadata(&without_rowid_page.entries, "parent_id").nullable);

    fixture.close().await;
}

#[tokio::test]
async fn catalog_triggers_resolve_canonical_owners_and_exclude_cross_schema_temp() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute(
            r#"
            CREATE TABLE MixedOwner (id INTEGER PRIMARY KEY);
            CREATE TRIGGER mixed_table_guard BEFORE INSERT ON mixedowner
                BEGIN SELECT 1; END;
            CREATE VIEW MixedView AS SELECT id FROM MixedOwner;
            CREATE TRIGGER mixed_view_guard INSTEAD OF INSERT ON mixedview
                BEGIN SELECT 1; END;
            CREATE TEMP TRIGGER excluded_temp_guard BEFORE INSERT ON main.MixedOwner
                BEGIN SELECT 1; END;
            PRAGMA writable_schema = ON;
            INSERT INTO sqlite_schema (type, name, tbl_name, rootpage, sql)
                VALUES (
                    'view',
                    'mixedowner',
                    'mixedowner',
                    0,
                    'CREATE VIEW mixedowner AS SELECT 1 AS id'
                );
            PRAGMA writable_schema = OFF;
            "#,
        )
        .await
        .unwrap();

    let main_scope = fixture.scope(&["main"]);
    let main_groups = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::groups(fixture.schema_id("main")).unwrap(),
            main_scope.clone(),
            10,
            None,
            80,
        ))
        .await
        .unwrap();
    assert_eq!(
        group_count(&main_groups, ObjectGroup::Triggers),
        CatalogCount::Exact(3)
    );
    let triggers = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Triggers).unwrap(),
            main_scope,
            10,
            None,
            81,
        ))
        .await
        .unwrap();
    assert_eq!(triggers.total_count, CatalogCount::Exact(3));
    assert_eq!(triggers.entries.len(), 3);
    let table_trigger = triggers
        .entries
        .iter()
        .find(|entry| entry.qualified_name.object == "mixed_table_guard")
        .unwrap();
    assert_eq!(
        table_trigger.relation_id,
        Some(CatalogId::new(
            fixture.profile_id,
            CatalogKind::Table,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "MixedOwner".to_owned(),
            ],
        ))
    );
    let view_trigger = triggers
        .entries
        .iter()
        .find(|entry| entry.qualified_name.object == "mixed_view_guard")
        .unwrap();
    assert_eq!(
        view_trigger.relation_id,
        Some(CatalogId::new(
            fixture.profile_id,
            CatalogKind::View,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "MixedView".to_owned(),
            ],
        ))
    );

    let temp_scope = fixture.scope(&["temp"]);
    let temp_schema = fixture.schema_id("temp");
    let temp_groups = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::groups(temp_schema.clone()).unwrap(),
            temp_scope.clone(),
            10,
            None,
            82,
        ))
        .await
        .unwrap();
    assert_eq!(
        group_count(&temp_groups, ObjectGroup::Triggers),
        CatalogCount::Exact(0)
    );
    let temp_triggers = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(temp_schema, ObjectGroup::Triggers).unwrap(),
            temp_scope,
            10,
            None,
            83,
        ))
        .await
        .unwrap();
    assert_eq!(temp_triggers.total_count, CatalogCount::Exact(0));
    assert!(temp_triggers.entries.is_empty());

    fixture.close().await;
}

#[tokio::test]
async fn catalog_native_target_misses_use_not_found_errors() {
    let fixture = CatalogFixture::new().await;
    for (request_id, relation) in [
        CatalogId::new(
            fixture.profile_id,
            CatalogKind::Table,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "missing_relation".to_owned(),
            ],
        ),
        CatalogId::new(
            fixture.profile_id,
            CatalogKind::View,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "Alpha".to_owned(),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let error = fixture
            .database
            .load_catalog_page(&catalog_request(
                fixture.profile_id,
                CatalogTarget::relation_children(relation).unwrap(),
                fixture.scope(&["main"]),
                100,
                None,
                90 + request_id as u64,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));
    }

    fixture.close().await;
}

fn column_metadata<'a>(entries: &'a [CatalogEntry], name: &str) -> &'a ColumnMetadata {
    entries
        .iter()
        .find_map(|entry| match &entry.metadata {
            CatalogMetadata::Column(metadata) if entry.qualified_name.object == name => {
                Some(metadata)
            }
            _ => None,
        })
        .unwrap()
}

fn group_count(page: &lazydb::db::catalog::CatalogPage, group: ObjectGroup) -> CatalogCount {
    page.group_summaries
        .iter()
        .find(|summary| summary.group == group)
        .unwrap()
        .object_count
}

fn assert_column(
    column: &ColumnMetadata,
    ordinal_position: u32,
    native_type: &str,
    nullable: bool,
    default_expression: OptionalMetadata<String>,
    hidden: bool,
) {
    assert_eq!(column.ordinal_position, ordinal_position);
    assert_eq!(column.native_type, native_type);
    assert_eq!(column.type_family, OptionalMetadata::Unsupported);
    assert_eq!(column.nullable, nullable);
    assert_eq!(column.default_expression, default_expression);
    assert_eq!(column.identity, OptionalMetadata::Unsupported);
    assert_eq!(column.auto_increment, OptionalMetadata::Unsupported);
    assert_eq!(column.hidden, OptionalMetadata::Supported(Some(hidden)));
    assert_eq!(column.numeric_precision, OptionalMetadata::Unsupported);
    assert_eq!(column.numeric_scale, OptionalMetadata::Unsupported);
    assert_eq!(
        column.character_maximum_length,
        OptionalMetadata::Unsupported
    );
    assert_eq!(column.collation, OptionalMetadata::Unsupported);
    assert_eq!(column.character_set, OptionalMetadata::Unsupported);
}

fn constraint(entries: &[CatalogEntry], kind: CatalogKind) -> &CatalogEntry {
    let matches = entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one grouped {kind:?}");
    matches[0]
}

fn assert_membership(column: &ColumnMetadata, constraint_id: &CatalogId, ordinal_position: u32) {
    assert!(
        column
            .constraint_memberships
            .contains(&ConstraintMembership {
                constraint_id: constraint_id.clone(),
                ordinal_position,
            })
    );
}

#[tokio::test]
async fn catalog_object_ddl_uses_the_verified_schema_alias() {
    let fixture = CatalogFixture::new().await;

    let main = fixture
        .database
        .object_ddl(CatalogKind::Table, "main", "shared_name")
        .await
        .unwrap()
        .unwrap();
    let attached = fixture
        .database
        .object_ddl(CatalogKind::Table, ATTACHED_ALIAS, "shared_name")
        .await
        .unwrap()
        .unwrap();
    assert!(main.contains("main_value TEXT"));
    assert!(!main.contains("archive_value"));
    assert!(attached.contains("archive_value INTEGER"));
    assert!(!attached.contains("main_value"));

    let error = fixture
        .database
        .object_ddl(CatalogKind::Table, "archivecase", "shared_name")
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));

    fixture.close().await;
}

#[tokio::test]
async fn discovery_returns_the_configured_sqlite_database_and_ordered_aliases() {
    let imported = import_connection_url("sqlite://:memory:", Some("discovery")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute("ATTACH DATABASE ':memory:' AS analytics")
        .await
        .unwrap();

    assert_eq!(
        database.catalog_capabilities(),
        expected_catalog_capabilities()
    );
    let discovery = database.discover_catalog_scope().await.unwrap();
    assert_eq!(discovery.databases.len(), 1);
    assert_eq!(discovery.databases[0].name, ":memory:");
    assert_eq!(discovery.databases[0].schemas, ["main", "analytics"]);
    database.close().await;
}

#[tokio::test]
async fn probes_catalogs_queries_and_reads_ddl() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("catalog.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("catalog")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    database
        .execute(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE teams (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                team_id INTEGER REFERENCES teams(id),
                name TEXT NOT NULL,
                score REAL,
                payload BLOB
            );
            CREATE INDEX users_name_idx ON users(name);
            CREATE VIEW active_users AS SELECT id, name FROM users;
            CREATE TRIGGER users_name_guard BEFORE INSERT ON users
            WHEN NEW.name = '' BEGIN SELECT RAISE(ABORT, 'name required'); END;
            INSERT INTO teams VALUES (1, 'core');
            INSERT INTO users VALUES (1, 1, 'Ada', 9.5, X'0001FF');
            INSERT INTO users VALUES (2, NULL, 'Lin', NULL, NULL);
            "#,
        )
        .await
        .unwrap();

    let server = database.probe().await.unwrap();
    assert_eq!(
        server.database,
        ":memory:".replace(":memory:", &path.to_string_lossy())
    );
    assert!(server.version.chars().next().unwrap().is_ascii_digit());

    let outcome = database
        .execute("SELECT id, team_id, name, score, payload FROM users ORDER BY id")
        .await
        .unwrap();
    let result = outcome.result_sets.last().unwrap();
    assert_eq!(
        result
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "team_id", "name", "score", "payload"]
    );
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], CellValue::Integer(1));
    assert_eq!(result.rows[0][2], CellValue::Text("Ada".into()));
    assert_eq!(result.rows[0][3], CellValue::Float(9.5));
    assert_eq!(result.rows[0][4], CellValue::Bytes(vec![0, 1, 255]));
    assert_eq!(result.rows[1][1], CellValue::Null);
    assert_eq!(result.rows[1][3], CellValue::Null);

    let ddl = database
        .object_ddl(CatalogKind::Table, "main", "users")
        .await
        .unwrap()
        .unwrap();
    assert!(ddl.contains("CREATE TABLE users"));
    database.close().await;
}

#[tokio::test]
async fn enforces_sqlite_read_only_at_connection_level() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("readonly.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("readonly")).unwrap();
    let writable = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    writable
        .execute("CREATE TABLE records (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    writable.close().await;

    let mut read_only_profile = imported.profile;
    read_only_profile.read_only = true;
    let read_only = DatabaseConnection::connect(&read_only_profile, None)
        .await
        .unwrap();

    let error = read_only
        .execute("INSERT INTO records VALUES (1)")
        .await
        .unwrap_err();
    assert!(error.to_string().to_ascii_lowercase().contains("readonly"));
    read_only.close().await;
}

#[tokio::test]
async fn preserves_result_sets_counts_timing_empty_values_and_errors() {
    let imported = import_connection_url("sqlite://:memory:", Some("results")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    let outcome = database
        .execute("SELECT '' AS empty_value, NULL AS missing_value; SELECT 7 AS second_result")
        .await
        .unwrap();
    assert_eq!(outcome.result_sets.len(), 2);
    assert_eq!(outcome.stats.row_count, 2);
    assert!(outcome.stats.total() >= outcome.stats.execution);
    assert_eq!(
        outcome.result_sets[0].rows[0][0],
        CellValue::Text(String::new())
    );
    assert_eq!(outcome.result_sets[0].rows[0][1], CellValue::Null);
    assert_eq!(outcome.result_sets[1].rows[0][0], CellValue::Integer(7));

    database
        .execute("CREATE TABLE affected (value TEXT); INSERT INTO affected VALUES ('x'), ('y')")
        .await
        .unwrap();
    let affected = database
        .execute("UPDATE affected SET value = value")
        .await
        .unwrap();
    assert_eq!(affected.result_sets.last().unwrap().affected_rows, 2);

    let error = database
        .execute("SELECT * FROM missing_table")
        .await
        .unwrap_err();
    assert_eq!(error.category, lazydb::db::ErrorCategory::Sql);
    database.close().await;
}
