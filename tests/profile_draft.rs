use lazydb::{
    model::{
        profile_manager::{
            CatalogScopeMode, CredentialUpdate, ProfileDraft, ProfileField, ProfileManagerPage,
            ProfileManagerState, ScopeSelectionState,
        },
        text_input::TextInput,
    },
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy,
        DatabaseKind, DatabaseScope, Environment, SslMode,
    },
};
use secrecy::ExposeSecret;

fn valid_postgres_draft() -> ProfileDraft {
    let mut draft = ProfileDraft::new(DatabaseKind::Postgres);
    draft.name.set("primary");
    draft.database.set("lazydb");
    draft
}

fn saved_postgres_profile() -> ConnectionProfile {
    valid_postgres_draft().validate(&[]).unwrap().profile
}

fn discovered_postgres_scope(selected_schemas: &[&str]) -> ProfileManagerState {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let draft = state.draft.as_mut().unwrap();
    draft.name.set("primary");
    draft.database.set("moss_biz");
    draft.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "moss_biz".into(),
            schemas: CatalogSelection::Selected(
                selected_schemas
                    .iter()
                    .map(|schema| (*schema).into())
                    .collect(),
            ),
        }]),
    };
    draft.catalog_scope_mode = CatalogScopeMode::Explicit;
    let profile = draft.validate(&[]).unwrap().profile;
    let fingerprint =
        lazydb::model::profile_manager::DiscoveryFingerprint::for_profile(&profile, false, 0);
    draft.begin_catalog_discovery(fingerprint);
    draft.apply_catalog_discovery(lazydb::model::profile_manager::ProfileCatalogDiscovery {
        fingerprint,
        server: lazydb::db::ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16".into(),
            database: "moss_biz".into(),
        },
        capabilities: lazydb::db::catalog::CatalogCapabilities {
            namespace_model: lazydb::db::catalog::NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![],
            column_metadata: Default::default(),
            supports_lazy_children: false,
        },
        discovery: Ok(lazydb::db::catalog::CatalogDiscovery {
            databases: vec![lazydb::db::catalog::DiscoveredDatabase {
                name: "moss_biz".into(),
                schemas: ["coa", "public", "tools"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            }],
            warnings: Vec::new(),
        }),
    });
    state.open_scope_picker();
    state
}

#[test]
fn moving_from_database_keeps_schemas_visible_and_selectable() {
    let mut state = discovered_postgres_scope(&["public"]);
    assert_eq!(
        state.scope_selected_row.as_deref(),
        Some("database:moss_biz")
    );
    assert!(state.scope_row("database:moss_biz:schema:coa").is_some());

    state.move_scope_selection(1);
    assert_eq!(
        state.scope_selected_row.as_deref(),
        Some("database:moss_biz:schema:coa")
    );
    assert!(state.scope_row("database:moss_biz:schema:public").is_some());
    assert!(state.scope_row("database:moss_biz:schema:all").is_none());
}

#[test]
fn database_toggle_uses_tri_state_select_all_and_remove() {
    let mut state = discovered_postgres_scope(&["public"]);
    assert_eq!(
        state.scope_row("database:moss_biz").unwrap().selection,
        ScopeSelectionState::Partial
    );

    assert!(state.toggle_scope_row("database:moss_biz"));
    assert_eq!(
        state.scope_row("database:moss_biz").unwrap().selection,
        ScopeSelectionState::Checked
    );
    let CatalogSelection::Selected(databases) =
        &state.draft.as_ref().unwrap().catalog_scope.databases
    else {
        panic!("database toggle must keep explicit scope")
    };
    assert!(matches!(databases[0].schemas, CatalogSelection::All));

    assert!(state.toggle_scope_row("database:moss_biz"));
    assert_eq!(
        state.scope_row("database:moss_biz").unwrap().selection,
        ScopeSelectionState::Unchecked
    );
    assert!(matches!(
        state.draft.as_ref().unwrap().catalog_scope.databases,
        CatalogSelection::Selected(ref databases) if databases.is_empty()
    ));
}

#[test]
fn schema_toggle_expands_all_and_last_schema_removes_database() {
    let mut state = discovered_postgres_scope(&["coa", "public", "tools"]);
    state.draft.as_mut().unwrap().catalog_scope.databases =
        CatalogSelection::Selected(vec![DatabaseScope {
            name: "moss_biz".into(),
            schemas: CatalogSelection::All,
        }]);

    assert!(state.toggle_scope_row("database:moss_biz:schema:tools"));
    let CatalogSelection::Selected(databases) =
        &state.draft.as_ref().unwrap().catalog_scope.databases
    else {
        panic!("schema exclusion must become explicit")
    };
    assert_eq!(
        databases[0].schemas,
        CatalogSelection::Selected(vec!["coa".into(), "public".into()])
    );
    assert_eq!(
        state.scope_row("database:moss_biz").unwrap().selection,
        ScopeSelectionState::Partial
    );

    state.draft.as_mut().unwrap().catalog_scope.databases =
        CatalogSelection::Selected(vec![DatabaseScope {
            name: "moss_biz".into(),
            schemas: CatalogSelection::Selected(vec!["public".into()]),
        }]);
    assert!(state.toggle_scope_row("database:moss_biz:schema:public"));
    assert!(matches!(
        state.draft.as_ref().unwrap().catalog_scope.databases,
        CatalogSelection::Selected(ref databases) if databases.is_empty()
    ));
}

#[test]
fn text_input_edits_unicode_by_character_position() {
    let mut input = TextInput::from("数据");
    input.move_end();
    input.move_left();
    input.insert('库');

    assert_eq!(input.value(), "数库据");
    assert_eq!(input.cursor(), 2);

    input.backspace();
    assert_eq!(input.value(), "数据");
    assert_eq!(input.cursor(), 1);
}

#[test]
fn text_input_supports_unicode_paste_delete_and_bounded_movement() {
    let mut input = TextInput::from("a界z");
    input.move_home();
    input.move_left();
    input.move_right();
    input.paste("数据");

    assert_eq!(input.value(), "a数据界z");
    assert_eq!(input.cursor(), 3);

    input.delete();
    assert_eq!(input.value(), "a数据z");
    input.move_end();
    input.move_right();
    input.delete();
    assert_eq!(input.cursor(), 4);
    assert_eq!(input.value(), "a数据z");

    input.set("é🙂");
    assert_eq!(input.cursor(), 2);
    input.backspace();
    assert_eq!(input.value(), "é");
}

#[test]
fn new_postgres_uses_server_defaults() {
    let draft = ProfileDraft::new(DatabaseKind::Postgres);

    assert_eq!(draft.host.value(), "localhost");
    assert_eq!(draft.port.value(), "5432");
    assert_eq!(draft.schema.value(), "public");
    assert_eq!(draft.ssl_mode, SslMode::Prefer);
    assert!(!draft.sqlite_memory);
    assert_eq!(
        draft.password_storage,
        lazydb::profile::PasswordStorageChoice::LocalEncrypted
    );
}

#[test]
fn new_mysql_and_sqlite_use_driver_defaults() {
    let mysql = ProfileDraft::new(DatabaseKind::MySql);
    assert_eq!(mysql.host.value(), "localhost");
    assert_eq!(mysql.port.value(), "3306");
    assert_eq!(
        mysql.password_storage,
        lazydb::profile::PasswordStorageChoice::LocalEncrypted
    );

    let sqlite = ProfileDraft::new(DatabaseKind::Sqlite);
    assert!(!sqlite.sqlite_memory);
    assert!(sqlite.sqlite_path.value().is_empty());
    assert_eq!(sqlite.ssl_mode, SslMode::Disable);
    assert!(
        !sqlite
            .visible_fields()
            .contains(&ProfileField::PasswordStorage)
    );
}

#[test]
fn new_profiles_derive_catalog_scope_defaults() {
    let postgres = valid_postgres_draft().validate(&[]).unwrap().profile;
    assert_eq!(
        postgres.catalog_scope,
        CatalogScope::for_profile(DatabaseKind::Postgres, "lazydb", Some("public"))
    );

    let mut mysql = ProfileDraft::new(DatabaseKind::MySql);
    mysql.name.set("mysql");
    mysql.database.set("sales");
    let mysql = mysql.validate(&[]).unwrap().profile;
    assert_eq!(
        mysql.catalog_scope,
        CatalogScope::for_profile(DatabaseKind::MySql, "sales", None)
    );

    let mut sqlite = ProfileDraft::new(DatabaseKind::Sqlite);
    sqlite.name.set("sqlite");
    sqlite.sqlite_path.set("./data/lazy.db");
    let sqlite = sqlite.validate(&[]).unwrap().profile;
    assert_eq!(
        sqlite.catalog_scope,
        CatalogScope::for_profile(DatabaseKind::Sqlite, "./data/lazy.db", Some("main"))
    );
}

#[test]
fn empty_postgres_schema_means_all_schemas_in_the_derived_scope() {
    let mut draft = valid_postgres_draft();
    draft.schema.set("");

    let profile = draft.validate(&[]).unwrap().profile;

    assert_eq!(profile.default_schema, None);
    assert_eq!(
        profile.catalog_scope,
        CatalogScope::for_profile(DatabaseKind::Postgres, "lazydb", None)
    );
}

#[test]
fn derived_scope_follows_database_and_schema_edits() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let draft = state.draft.as_mut().unwrap();
    draft.name.set("primary");

    state.focus_field(ProfileField::Database);
    state.paste("warehouse");
    state.focus_field(ProfileField::Schema);
    for _ in 0.."public".len() {
        state.backspace();
    }
    state.paste("audit");

    let draft = state.draft.as_ref().unwrap();
    assert_eq!(draft.catalog_scope_mode, CatalogScopeMode::Derived);
    assert_eq!(
        draft.catalog_scope,
        CatalogScope::for_profile(DatabaseKind::Postgres, "warehouse", Some("audit"))
    );
    assert_eq!(
        draft.validate(&[]).unwrap().profile.catalog_scope,
        draft.catalog_scope
    );
}

#[test]
fn edited_profiles_detect_derived_and_explicit_scopes() {
    let profile = saved_postgres_profile();
    assert_eq!(
        ProfileDraft::edit(&profile, false).catalog_scope_mode,
        CatalogScopeMode::Derived
    );

    let mut explicit = profile;
    explicit.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "lazydb".into(),
            schemas: CatalogSelection::Selected(vec!["public".into(), "audit".into()]),
        }]),
    };
    assert_eq!(
        ProfileDraft::edit(&explicit, false).catalog_scope_mode,
        CatalogScopeMode::Explicit
    );
}

#[test]
fn sqlite_accepts_memory_or_a_non_empty_path() {
    let mut draft = ProfileDraft::new(DatabaseKind::Sqlite);
    draft.name.set("scratch");

    let error = draft.validate(&[]).unwrap_err();
    assert_eq!(error.field, ProfileField::SqlitePath);

    draft.sqlite_path.set(" ./data/lazy.db ");
    let file = draft.validate(&[]).unwrap().profile;
    assert_eq!(
        file.sqlite_path.unwrap().to_string_lossy(),
        "./data/lazy.db"
    );
    assert_eq!(file.database.as_deref(), Some("./data/lazy.db"));

    draft.sqlite_memory = true;
    let memory = draft.validate(&[]).unwrap().profile;
    assert!(memory.sqlite_path.is_none());
    assert_eq!(memory.database.as_deref(), Some(":memory:"));
}

#[test]
fn name_is_required_and_unique_case_insensitively() {
    let existing = saved_postgres_profile();
    let mut draft = valid_postgres_draft();
    draft.name.set("  ");

    assert_eq!(draft.validate(&[]).unwrap_err().field, ProfileField::Name);

    draft.name.set(" PRIMARY ");
    assert_eq!(
        draft.validate(&[existing]).unwrap_err().field,
        ProfileField::Name
    );
}

#[test]
fn editing_excludes_its_own_name_from_duplicate_validation() {
    let mut profile = saved_postgres_profile();
    profile.name = "Primary".into();
    let draft = ProfileDraft::edit(&profile, false);

    let submission = draft.validate(&[profile.clone()]).unwrap();
    assert_eq!(submission.profile.id, profile.id);
    assert_eq!(submission.profile.name, "Primary");
}

#[test]
fn server_host_and_database_are_required() {
    let mut draft = valid_postgres_draft();
    draft.host.set(" ");
    assert_eq!(draft.validate(&[]).unwrap_err().field, ProfileField::Host);

    draft.host.set("localhost");
    draft.database.set("");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::Database
    );
}

#[test]
fn port_must_be_an_integer_in_the_tcp_range() {
    let mut draft = valid_postgres_draft();

    for invalid in ["", "0", "65536", "abc"] {
        draft.port.set(invalid);
        assert_eq!(draft.validate(&[]).unwrap_err().field, ProfileField::Port);
    }

    for valid in ["1", "65535"] {
        draft.port.set(valid);
        assert_eq!(
            draft.validate(&[]).unwrap().profile.port,
            valid.parse().ok()
        );
    }
}

#[test]
fn draft_uuid_is_stable_across_validation() {
    let draft = valid_postgres_draft();

    let first = draft.validate(&[]).unwrap().profile.id;
    let second = draft.validate(&[]).unwrap().profile.id;

    assert_eq!(first, second);
}

#[test]
fn editing_preserves_uuid_and_structured_catalog_scope() {
    let mut profile = saved_postgres_profile();
    profile.environment = Environment::Production;
    profile.read_only = true;
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "lazydb".into(),
                schemas: CatalogSelection::Selected(vec!["public".into(), "audit".into()]),
            },
            DatabaseScope {
                name: "warehouse".into(),
                schemas: CatalogSelection::All,
            },
        ]),
    };

    let mut draft = ProfileDraft::edit(&profile, false);
    draft.name.set("renamed");
    let edited = draft.validate(&[profile.clone()]).unwrap().profile;

    assert_eq!(edited.id, profile.id);
    assert_eq!(edited.environment, Environment::Production);
    assert!(edited.read_only);
    assert_eq!(edited.catalog_scope, profile.catalog_scope);
}

#[test]
fn changing_unrelated_fields_does_not_reset_a_custom_scope() {
    let mut draft = valid_postgres_draft();
    let custom_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "lazydb".into(),
            schemas: CatalogSelection::Selected(vec!["public".into(), "audit".into()]),
        }]),
    };
    draft.catalog_scope = custom_scope.clone();
    draft.catalog_scope_mode = CatalogScopeMode::Explicit;
    draft.environment = Environment::Staging;
    draft.read_only = true;

    let profile = draft.validate(&[]).unwrap().profile;

    assert_eq!(profile.catalog_scope, custom_scope);
    assert_eq!(profile.environment, Environment::Staging);
    assert!(profile.read_only);
}

#[test]
fn visible_objects_can_exclude_the_connection_default_schema() {
    let mut draft = valid_postgres_draft();
    draft.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "lazydb".into(),
            schemas: CatalogSelection::Selected(vec!["private".into()]),
        }]),
    };
    draft.catalog_scope_mode = CatalogScopeMode::Explicit;

    let profile = draft.validate(&[]).unwrap().profile;

    assert_eq!(profile.default_schema.as_deref(), Some("public"));
    assert!(!profile.catalog_scope.allows_schema("lazydb", "public"));
    assert!(profile.catalog_scope.allows_schema("lazydb", "private"));
}

#[test]
fn credential_intent_preserves_or_replaces_stored_passwords() {
    let mut profile = saved_postgres_profile();
    profile.credential_policy = CredentialPolicy::System(format!("keyring:test/{}", profile.id));

    let draft = ProfileDraft::edit(&profile, true);
    assert!(matches!(
        draft.validate(&[profile.clone()]).unwrap().credential,
        CredentialUpdate::Preserve
    ));

    let mut remember = draft;
    remember.set_password("remembered-password");
    let submission = remember.validate(&[profile]).unwrap();
    match submission.credential {
        CredentialUpdate::System(secret) => {
            assert_eq!(secret.expose_secret(), "remembered-password");
        }
        other => panic!("expected system credential, got {other:?}"),
    }
    assert_eq!(
        submission.profile.credential_policy,
        CredentialPolicy::Prompt
    );
}

#[test]
fn new_password_uses_selected_storage() {
    let mut draft = valid_postgres_draft();
    draft.set_password("temporary");

    assert!(matches!(
        draft.validate(&[]).unwrap().credential,
        CredentialUpdate::LocalEncrypted(_)
    ));
}

#[test]
fn editing_prompt_profile_defaults_to_local_replacement_storage() {
    let mut prompt = saved_postgres_profile();
    prompt.credential_policy = CredentialPolicy::Prompt;
    assert_eq!(
        ProfileDraft::edit(&prompt, false).password_storage,
        lazydb::profile::PasswordStorageChoice::LocalEncrypted
    );

    let passwordless = saved_postgres_profile();
    assert_eq!(
        ProfileDraft::edit(&passwordless, false).password_storage,
        lazydb::profile::PasswordStorageChoice::LocalEncrypted
    );
}

#[test]
fn debug_output_redacts_passwords() {
    let mut draft = valid_postgres_draft();
    draft.set_password("never-print-this-password");
    let submission = draft.validate(&[]).unwrap();

    assert!(!format!("{draft:?}").contains("never-print-this-password"));
    assert!(!format!("{submission:?}").contains("never-print-this-password"));
    assert!(!format!("{:?}", submission.credential).contains("never-print-this-password"));
}

#[test]
fn url_commit_is_atomic_and_moves_password_to_secret() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let draft = state.draft.as_mut().unwrap();
    draft.name.set("primary");
    let old_host = draft.host.value().to_owned();

    state.focus_field(ProfileField::Url);
    state.draft.as_mut().unwrap().move_home(ProfileField::Url);
    state.paste("not-a-url");
    assert!(state.commit_url().is_err());
    let draft = state.draft.as_ref().unwrap();
    assert_eq!(draft.host.value(), old_host);
    assert!(draft.url_error().is_some());

    state.start_new(DatabaseKind::Postgres);
    let draft = state.draft.as_mut().unwrap();
    while draft.url_cursor() > 0 {
        draft.backspace(ProfileField::Url);
    }
    draft.paste(
        ProfileField::Url,
        "postgresql://alice:super-secret@db.example:5440/app?sslmode=require",
    );
    assert!(!draft.url_display().contains("super-secret"));
    assert!(format!("{draft:?}").contains("[REDACTED]"));
    draft.commit_url().unwrap();
    assert_eq!(draft.host.value(), "db.example");
    assert_eq!(draft.port.value(), "5440");
    assert_eq!(draft.password().expose_secret(), "super-secret");
    assert!(!draft.url_display().contains("super-secret"));
    assert!(!draft.url_display().contains("***"));
}

#[test]
fn structured_edits_refresh_url_and_invalid_port_keeps_last_valid_url() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let original = state.draft.as_ref().unwrap().url_display();
    state.focus_field(ProfileField::Host);
    state.paste("-changed");
    let changed = state.draft.as_ref().unwrap().url_display();
    assert_ne!(changed, original);

    state.focus_field(ProfileField::Port);
    state.paste("invalid");
    assert_eq!(state.draft.as_ref().unwrap().url_display(), changed);
    assert_eq!(
        state
            .draft
            .as_ref()
            .unwrap()
            .validate(&[])
            .unwrap_err()
            .field,
        ProfileField::Name
    );
}

#[test]
fn url_format_cycles_only_compatible_values_and_driver_resets_default() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    // URL format remains internal state even though it is no longer navigable.
    state.selected_field = ProfileField::UrlFormat;
    state.cycle(1);
    assert_eq!(
        state.draft.as_ref().unwrap().url_format,
        ConnectionUrlFormat::JdbcPostgreSql
    );
    state.select_driver(DatabaseKind::MySql);
    assert_eq!(
        state.draft.as_ref().unwrap().url_format,
        ConnectionUrlFormat::MySql
    );
    assert!(
        state
            .draft
            .as_ref()
            .unwrap()
            .url_display()
            .starts_with("mysql://")
    );
}

#[test]
fn visible_fields_follow_the_selected_driver_and_sqlite_mode() {
    let postgres = ProfileDraft::new(DatabaseKind::Postgres);
    assert_eq!(
        postgres.visible_fields(),
        &[
            ProfileField::Kind,
            ProfileField::Name,
            ProfileField::Host,
            ProfileField::Port,
            ProfileField::User,
            ProfileField::Password,
            ProfileField::Database,
            ProfileField::Schema,
            ProfileField::VisibleObjects,
            ProfileField::SslMode,
            ProfileField::Environment,
            ProfileField::ReadOnly,
            ProfileField::PasswordStorage,
            ProfileField::Url,
            ProfileField::Test,
            ProfileField::Save,
            ProfileField::SaveAndConnect,
            ProfileField::Cancel,
        ]
    );

    let mysql = ProfileDraft::new(DatabaseKind::MySql);
    assert!(!mysql.visible_fields().contains(&ProfileField::Schema));

    let mut sqlite = ProfileDraft::new(DatabaseKind::Sqlite);
    assert!(sqlite.visible_fields().contains(&ProfileField::SqlitePath));
    sqlite.sqlite_memory = true;
    assert!(!sqlite.visible_fields().contains(&ProfileField::SqlitePath));
    assert!(!sqlite.visible_fields().contains(&ProfileField::Password));
}

#[test]
fn visible_field_navigation_reaches_url_before_actions_without_url_format() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let fields = state.visible_fields();
    assert!(!fields.contains(&ProfileField::UrlFormat));
    assert_eq!(fields[fields.len() - 5], ProfileField::Url);

    state.selected_field = ProfileField::PasswordStorage;
    state.move_field(1);
    assert_eq!(state.selected_field, ProfileField::Url);
    state.move_field(1);
    assert_eq!(state.selected_field, ProfileField::Test);
    state.move_field(-1);
    assert_eq!(state.selected_field, ProfileField::Url);
}

#[test]
fn manager_state_initializes_new_and_edit_forms() {
    let mut state = ProfileManagerState::new(true);
    assert_eq!(state.page, ProfileManagerPage::Form);
    assert!(state.opened_automatically);

    state.start_new(DatabaseKind::MySql);
    assert_eq!(state.page, ProfileManagerPage::Form);
    assert_eq!(state.selected_field, ProfileField::Kind);
    assert_eq!(state.draft.as_ref().unwrap().port.value(), "3306");

    let profile = saved_postgres_profile();
    state.start_edit(&profile, false);
    assert_eq!(state.draft.as_ref().unwrap().profile_id(), profile.id);
    assert_eq!(
        state.visible_fields(),
        state.draft.as_ref().unwrap().visible_fields()
    );
}

#[test]
fn scope_picker_has_one_visible_objects_field_and_stable_summary() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    state.draft.as_mut().unwrap().name.set("primary");

    assert!(
        state
            .visible_fields()
            .contains(&ProfileField::VisibleObjects)
    );
    assert_eq!(
        state.draft.as_ref().unwrap().visible_objects_summary(),
        "1 database"
    );
    state.open_scope_picker();
    assert_eq!(state.page, ProfileManagerPage::Scope);
    assert_eq!(
        state.draft.as_ref().unwrap().catalog_scope_mode,
        CatalogScopeMode::Derived
    );
}

#[test]
fn toggling_scope_makes_it_explicit_and_later_field_edits_preserve_it() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    state.focus_field(ProfileField::Database);
    state.paste("lazydb");
    state.open_scope_picker();

    assert!(state.toggle_scope_row("database:lazydb"));
    let explicit_scope = state.draft.as_ref().unwrap().catalog_scope.clone();
    assert_eq!(
        state.draft.as_ref().unwrap().catalog_scope_mode,
        CatalogScopeMode::Explicit
    );

    state.close_scope_picker();
    state.focus_field(ProfileField::Database);
    state.paste("-changed");
    state.focus_field(ProfileField::Schema);
    state.paste("-changed");

    assert_eq!(state.draft.as_ref().unwrap().catalog_scope, explicit_scope);
}

#[test]
fn scope_picker_keeps_all_and_selected_schema_modes_exclusive() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    state.draft.as_mut().unwrap().database.set("localhost");
    state.open_scope_picker();

    state.toggle_scope_row("database:lazydb:schema:all");
    state.toggle_scope_row("database:lazydb:schema:public");

    let scope = &state.draft.as_ref().unwrap().catalog_scope;
    let CatalogSelection::Selected(databases) = &scope.databases else {
        panic!("expected selected database");
    };
    assert!(matches!(
        databases[0].schemas,
        CatalogSelection::Selected(_)
    ));
}

#[test]
fn mysql_mirrored_schema_row_is_selected_and_read_only() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::MySql);
    state.draft.as_mut().unwrap().database.set("sales");
    state.open_scope_picker();

    let row = state
        .scope_row("database:sales:schema:sales")
        .expect("mirror row");
    assert_eq!(
        row.selection,
        lazydb::model::profile_manager::ScopeSelectionState::Checked
    );
    assert!(row.read_only);
    assert!(!state.toggle_scope_row("database:sales:schema:sales"));
}

#[test]
fn stale_discovery_preserves_custom_scope_and_unavailable_names() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let draft = state.draft.as_mut().unwrap();
    draft.name.set("primary");
    draft.database.set("db");
    draft.database.set("missing_db");
    draft.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "missing_db".into(),
            schemas: CatalogSelection::Selected(vec!["missing_schema".into()]),
        }]),
    };
    draft.catalog_scope_mode = CatalogScopeMode::Explicit;
    draft.invalidate_discovery_for_test();
    state.open_scope_picker();

    assert!(state.scope_warning().is_some());
    assert!(state.scope_row("database:missing_db").is_some());
    assert!(
        state
            .scope_row("database:missing_db:schema:missing_schema")
            .is_some()
    );
}

#[test]
fn sqlite_forms_include_visible_objects_picker() {
    let mut draft = ProfileDraft::new(DatabaseKind::Sqlite);
    assert!(
        draft
            .visible_fields()
            .contains(&ProfileField::VisibleObjects)
    );
    draft.sqlite_memory = true;
    assert!(
        draft
            .visible_fields()
            .contains(&ProfileField::VisibleObjects)
    );
}

#[test]
fn scope_picker_navigation_updates_selected_row_and_viewport() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    state.draft.as_mut().unwrap().database.set("localhost");
    state.open_scope_picker();
    let first = state.scope_selected_row.clone().unwrap();
    state.move_scope_selection(1);
    assert_ne!(state.scope_selected_row.as_deref(), Some(first.as_str()));
    state.move_scope_selection(-1);
    assert_eq!(state.scope_selected_row.as_deref(), Some(first.as_str()));
    state.set_scope_viewport_for_test(1);
    assert_eq!(state.scope_viewport, 1);
}

#[test]
fn database_scope_toggle_is_actionable_without_creating_empty_selection() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    state.draft.as_mut().unwrap().database.set("localhost");
    state.open_scope_picker();
    assert!(!state.toggle_scope_row("database:"));
    assert!(state.toggle_scope_row("database:localhost"));
    assert!(
        state
            .draft
            .as_ref()
            .unwrap()
            .catalog_scope
            .validate("localhost", Some("public"))
            .is_ok()
    );
}

#[test]
fn discovery_failure_warning_is_preserved_when_picker_reopens() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let draft = state.draft.as_mut().unwrap();
    draft.name.set("primary");
    draft.database.set("db");
    let profile = draft.validate(&[]).unwrap().profile;
    let fingerprint =
        lazydb::model::profile_manager::DiscoveryFingerprint::for_profile(&profile, false, 0);
    draft.begin_catalog_discovery(fingerprint);
    draft.apply_catalog_discovery(lazydb::model::profile_manager::ProfileCatalogDiscovery {
        fingerprint,
        server: lazydb::db::ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16".into(),
            database: "db".into(),
        },
        capabilities: lazydb::db::catalog::CatalogCapabilities {
            namespace_model: lazydb::db::catalog::NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![],
            column_metadata: Default::default(),
            supports_lazy_children: false,
        },
        discovery: Err("permission denied".into()),
    });
    state.open_scope_picker();
    assert!(state.scope_warning().unwrap().contains("permission denied"));
    state.close_scope_picker();
    state.open_scope_picker();
    assert!(state.scope_warning().unwrap().contains("permission denied"));
}

#[test]
fn discovered_and_saved_schema_rows_are_deduplicated() {
    let mut state = ProfileManagerState::new(false);
    state.start_new(DatabaseKind::Postgres);
    let draft = state.draft.as_mut().unwrap();
    draft.name.set("primary");
    draft.database.set("db");
    draft.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "db".into(),
            schemas: CatalogSelection::Selected(vec!["public".into()]),
        }]),
    };
    draft.catalog_scope_mode = CatalogScopeMode::Explicit;
    let profile = draft.validate(&[]).unwrap().profile;
    let fingerprint =
        lazydb::model::profile_manager::DiscoveryFingerprint::for_profile(&profile, false, 0);
    draft.begin_catalog_discovery(fingerprint);
    draft.apply_catalog_discovery(lazydb::model::profile_manager::ProfileCatalogDiscovery {
        fingerprint,
        server: lazydb::db::ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16".into(),
            database: "db".into(),
        },
        capabilities: lazydb::db::catalog::CatalogCapabilities {
            namespace_model: lazydb::db::catalog::NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![],
            column_metadata: Default::default(),
            supports_lazy_children: false,
        },
        discovery: Ok(lazydb::db::catalog::CatalogDiscovery {
            databases: vec![lazydb::db::catalog::DiscoveredDatabase {
                name: "db".into(),
                schemas: vec!["public".into()],
            }],
            warnings: Vec::new(),
        }),
    });
    state.open_scope_picker();
    let rows = state
        .scope_rows_for_render()
        .into_iter()
        .filter(|row| row.id == "database:db:schema:public")
        .count();
    assert_eq!(rows, 1);
}
