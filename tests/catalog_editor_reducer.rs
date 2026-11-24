use lazydb::{
    action::Action,
    app::App,
    db::{
        catalog::{CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName},
        catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationMode, CatalogObjectType, SequenceBound,
            ViewOption,
        },
    },
    model::explorer_add::ExplorerAddKind,
    model::{
        catalog_editor::{
            CatalogDraft, CatalogEditorPage, CatalogEditorState, CatalogFormFocus,
            MaterializedViewDraft, SequenceDraft, ViewDraft,
        },
        explorer::{ExplorerMutationIntent, ExplorerNodeId, StatusRowKind},
        workspace::Overlay,
    },
    profile::{ConnectionProfile, import_connection_url},
};
use uuid::Uuid;

#[test]
fn table_navigation_reaches_and_leaves_every_action() {
    let mut app = App::new(Vec::new());
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Group {
            schema: CatalogId::new(Uuid::nil(), CatalogKind::Schema, ["app", "public"]),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Table),
            label: "Table".into(),
        }],
    );
    assert!(editor.select_object_type(CatalogObjectType::Catalog(CatalogKind::Table)));
    app.catalog_editor = Some(editor);

    app.update(Action::CatalogEditorFieldNext);
    assert!(matches!(
        app.catalog_editor.as_ref().and_then(|editor| editor.draft.as_ref()),
        Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft))
            if draft.focus
                == lazydb::model::catalog_editor::TableEditorFocus::General(
                    lazydb::model::catalog_editor::TableGeneralField::Schema
                )
    ));
    let Some(CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft should be selected");
    };
    draft.columns.clear();
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Action(
        lazydb::model::catalog_editor::TableActionField::AddColumn,
    );
    for expected in [
        lazydb::model::catalog_editor::TableActionField::AddConstraintPrimaryKey,
        lazydb::model::catalog_editor::TableActionField::AddConstraintUnique,
        lazydb::model::catalog_editor::TableActionField::AddConstraintCheck,
        lazydb::model::catalog_editor::TableActionField::AddConstraintForeignKey,
        lazydb::model::catalog_editor::TableActionField::Review,
        lazydb::model::catalog_editor::TableActionField::Cancel,
    ] {
        draft.focus_next();
        assert_eq!(
            draft.focus,
            lazydb::model::catalog_editor::TableEditorFocus::Action(expected)
        );
    }
}

fn table_editor_for_paste() -> App {
    let mut app = App::new(Vec::new());
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        CatalogMutationAnchor::Group {
            schema: CatalogId::new(Uuid::nil(), CatalogKind::Schema, ["app", "public"]),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Table),
            label: "Table".into(),
        }],
    );
    assert!(editor.select_object_type(CatalogObjectType::Catalog(CatalogKind::Table)));
    app.catalog_editor = Some(editor);
    app
}

fn simple_catalog_editor(mode: CatalogMutationMode, draft: CatalogDraft) -> App {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(CatalogEditorState {
        database_kind: None,
        mode,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: Uuid::nil(),
        },
        object_type: None,
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: Vec::new(),
        selected_option: 0,
        draft: Some(draft),
        baseline: None,
        plan: None,
        error: None,
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
    });
    app
}

fn catalog_profile_form(draft: CatalogDraft) -> App {
    let mut app = simple_catalog_editor(CatalogMutationMode::Create, draft);
    let editor = app.catalog_editor.as_mut().expect("catalog editor");
    editor.object_type = match editor.draft.as_ref().expect("catalog draft") {
        CatalogDraft::Database(_) => Some(CatalogObjectType::Catalog(CatalogKind::Database)),
        CatalogDraft::Role(role) if role.login => Some(CatalogObjectType::LoginRole),
        CatalogDraft::Role(_) => Some(CatalogObjectType::Role),
        _ => panic!("profile form must be a database or role"),
    };
    app
}

fn catalog_profile_form_name(app: &App) -> &str {
    match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
        .expect("catalog draft")
    {
        CatalogDraft::Database(draft) => draft.name.value(),
        CatalogDraft::Role(draft) => draft.name.value(),
        _ => panic!("profile form must be a database or role"),
    }
}

fn connected_postgres_app(current_user: Option<&str>) -> (App, Uuid) {
    let profile = postgres_profile();
    let profile_id = profile.id;
    let mut app = App::new(vec![profile.clone()]);
    connect_postgres_profile(&mut app, &profile, current_user);
    (app, profile_id)
}

fn postgres_profile() -> ConnectionProfile {
    let mut profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    profile.user = Some("login_user".into());
    profile
}

fn connect_postgres_profile(
    app: &mut App,
    profile: &ConnectionProfile,
    current_user: Option<&str>,
) {
    let generation = match app.update(Action::RequestConnect(profile.id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
            current_user: current_user.map(str::to_owned),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });
}

fn open_database_from_add_menu(app: &mut App, profile_id: Uuid) {
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    app.update(Action::OpenExplorerAdd);
    let index = match app.overlay.as_ref() {
        Some(Overlay::ExplorerAdd(menu)) => menu
            .options
            .iter()
            .position(|option| option.kind == ExplorerAddKind::Database)
            .expect("database option"),
        overlay => panic!("expected explorer add menu, got {overlay:?}"),
    };
    app.update(Action::ExplorerAddSelect(index));
    app.update(Action::ExplorerAddConfirm);
}

fn database_draft(app: &App) -> &lazydb::model::catalog_editor::DatabaseDraft {
    match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    {
        Some(CatalogDraft::Database(draft)) => draft,
        draft => panic!("expected database draft, got {draft:?}"),
    }
}

#[test]
fn new_database_owner_uses_effective_user_from_add_menu() {
    let (mut app, profile_id) = connected_postgres_app(Some("effective_role"));
    open_database_from_add_menu(&mut app, profile_id);

    assert_eq!(app.overlay, Some(Overlay::CatalogEditor));
    assert_eq!(
        app.catalog_editor.as_ref().map(|editor| editor.page),
        Some(CatalogEditorPage::Form)
    );
    let draft = database_draft(&app);
    assert_eq!(draft.owner.value(), "effective_role");
    assert!(draft.name.value().is_empty());
    assert_eq!(draft.focus, CatalogFormFocus::Name);
}

#[test]
fn new_database_owner_falls_back_to_profile_user_when_server_user_is_missing() {
    for current_user in [None, Some(""), Some(" \t ")] {
        let (mut app, profile_id) = connected_postgres_app(current_user);
        open_database_from_add_menu(&mut app, profile_id);
        assert_eq!(database_draft(&app).owner.value(), "login_user");
    }

    let mut profile = postgres_profile();
    profile.user = Some(" \t ".into());
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let profile = app.profiles.first().expect("profile").clone();
    connect_postgres_profile(&mut app, &profile, Some("  "));
    open_database_from_add_menu(&mut app, profile_id);
    assert!(database_draft(&app).owner.value().is_empty());
}

#[test]
fn new_database_owner_keeps_effective_user_verbatim() {
    let (mut app, profile_id) = connected_postgres_app(Some("  effective role  "));
    open_database_from_add_menu(&mut app, profile_id);
    assert_eq!(database_draft(&app).owner.value(), "  effective role  ");
}

#[test]
fn new_database_owner_does_not_use_another_profiles_server_user() {
    let (mut app, active_profile_id) = connected_postgres_app(Some("active_role"));
    let target_profile = postgres_profile();
    let target_profile_id = target_profile.id;
    app.profiles.push(target_profile.clone());
    app.explorer.normalized.profiles.insert(
        target_profile_id,
        lazydb::model::explorer::ExplorerProfileState::new(
            target_profile_id,
            target_profile.name.clone(),
            target_profile.kind,
            "localhost".into(),
            lazydb::model::explorer::ProfileProvenance::Saved,
            lazydb::model::explorer::ProfilePlacement::default(),
        ),
    );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(target_profile_id));

    app.update(Action::OpenCatalogCreate);
    let index = app
        .catalog_editor
        .as_ref()
        .expect("catalog editor")
        .options
        .iter()
        .position(|option| option.object_type == CatalogObjectType::Catalog(CatalogKind::Database))
        .expect("database option");
    app.update(Action::CatalogEditorMove(index as isize));
    app.update(Action::CatalogEditorSelect);

    assert_eq!(
        app.connection.active_identity().unwrap().profile_id,
        active_profile_id
    );
    assert_eq!(database_draft(&app).owner.value(), "login_user");
}

#[test]
fn new_database_owner_is_not_applied_to_other_database_kinds() {
    for (url, kind) in [
        (
            "mysql://login_user@localhost/app",
            lazydb::profile::DatabaseKind::MySql,
        ),
        (
            "mariadb://login_user@localhost/app",
            lazydb::profile::DatabaseKind::MariaDb,
        ),
        (
            "clickhouse://login_user@localhost/app",
            lazydb::profile::DatabaseKind::ClickHouse,
        ),
    ] {
        let profile = import_connection_url(url, Some("test-profile"))
            .unwrap()
            .profile;
        let profile_id = profile.id;
        let mut app = App::new(vec![profile]);
        let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
            [lazydb::action::Command::Connect { generation, .. }] => *generation,
            commands => panic!("unexpected commands: {commands:?}"),
        };
        app.update(Action::ConnectionSucceeded {
            profile_id,
            generation,
            server: lazydb::db::ServerInfo {
                kind,
                version: "test server".into(),
                database: "app".into(),
                current_user: Some("server_user".into()),
            },
            mutation_capabilities: match kind {
                lazydb::profile::DatabaseKind::MySql | lazydb::profile::DatabaseKind::MariaDb => {
                    lazydb::db::mysql::MySqlAdapter::catalog_mutation_capabilities()
                }
                lazydb::profile::DatabaseKind::ClickHouse => {
                    lazydb::db::clickhouse::ClickHouseAdapter::catalog_mutation_capabilities()
                }
                _ => unreachable!(),
            },
        });
        open_database_from_add_menu(&mut app, profile_id);
        let draft = database_draft(&app);
        assert_eq!(draft.database_kind, kind);
        assert!(draft.owner.value().is_empty());
    }
}

#[test]
fn new_database_owner_uses_effective_user_from_object_picker() {
    let (mut app, profile_id) = connected_postgres_app(Some("effective_role"));
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    app.update(Action::OpenCatalogCreate);
    let index = app
        .catalog_editor
        .as_ref()
        .expect("catalog editor")
        .options
        .iter()
        .position(|option| option.object_type == CatalogObjectType::Catalog(CatalogKind::Database))
        .expect("database option");
    app.update(Action::CatalogEditorMove(index as isize));
    app.update(Action::CatalogEditorSelect);

    assert_eq!(database_draft(&app).owner.value(), "effective_role");
}

#[test]
fn new_database_owner_preserves_edits_and_reinitializes_only_when_reopened() {
    let (mut app, profile_id) = connected_postgres_app(Some("effective_role"));
    open_database_from_add_menu(&mut app, profile_id);
    app.update(Action::CatalogEditorFocusFormField(CatalogFormFocus::Owner));
    for _ in 0.."effective_role".chars().count() {
        app.update(Action::CatalogEditorBackspace);
    }
    for character in "custom_owner".chars() {
        app.update(Action::CatalogEditorInsert(character));
    }
    app.update(Action::CatalogEditorFieldNext);
    assert_eq!(database_draft(&app).owner.value(), "custom_owner");

    app.update(Action::CatalogEditorFocusFormField(CatalogFormFocus::Owner));
    for _ in 0.."custom_owner".chars().count() {
        app.update(Action::CatalogEditorBackspace);
    }
    app.update(Action::CatalogEditorFieldNext);
    assert!(database_draft(&app).owner.value().is_empty());
    app.update(Action::CatalogEditorFocusFormField(CatalogFormFocus::Name));
    for character in "app_db".chars() {
        app.update(Action::CatalogEditorInsert(character));
    }
    let error = database_draft(&app).validate().unwrap_err().to_string();
    assert!(error.contains("database name and owner are required"));

    app.update(Action::CatalogEditorCancel);
    open_database_from_add_menu(&mut app, profile_id);
    assert_eq!(database_draft(&app).owner.value(), "effective_role");
}

#[test]
fn new_database_owner_uses_target_profile_user_for_a_different_anchor() {
    let (mut app, active_profile_id) = connected_postgres_app(Some("active_role"));
    let target_profile = postgres_profile();
    let target_profile_id = target_profile.id;
    app.profiles.push(target_profile);
    let mut editor = CatalogEditorState::new(
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Profile {
            profile_id: target_profile_id,
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Database),
            label: "Database".into(),
        }],
    );
    editor.database_kind = Some(lazydb::profile::DatabaseKind::Postgres);
    app.catalog_editor = Some(editor);
    app.update(Action::CatalogEditorSelect);

    assert_eq!(database_draft(&app).owner.value(), "login_user");
    assert_ne!(
        app.connection.active_identity().unwrap().profile_id,
        target_profile_id
    );
    assert_ne!(database_draft(&app).owner.value(), "active_role");
    assert_ne!(active_profile_id, target_profile_id);
}

#[test]
fn catalog_profile_form_reducer_inserts_text_for_database_and_both_role_types() {
    let drafts = [
        CatalogDraft::Database(lazydb::model::catalog_editor::DatabaseDraft::new("")),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(true)),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(false)),
    ];

    for draft in drafts {
        let mut app = catalog_profile_form(draft);
        for character in ['a', '界', 'b'] {
            app.update(Action::CatalogEditorInsert(character));
        }
        assert_eq!(catalog_profile_form_name(&app), "a界b");
    }
}

#[test]
fn catalog_profile_form_reducer_edits_unicode_text_at_the_cursor() {
    let drafts = [
        CatalogDraft::Database(lazydb::model::catalog_editor::DatabaseDraft::new("")),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(true)),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(false)),
    ];

    for draft in drafts {
        let mut app = catalog_profile_form(draft);
        for character in ['a', '界', 'b'] {
            app.update(Action::CatalogEditorInsert(character));
        }

        app.update(Action::CatalogEditorMoveHome);
        app.update(Action::CatalogEditorMoveRight);
        app.update(Action::CatalogEditorDelete);
        assert_eq!(catalog_profile_form_name(&app), "ab");

        app.update(Action::CatalogEditorInsert('中'));
        assert_eq!(catalog_profile_form_name(&app), "a中b");

        app.update(Action::CatalogEditorMoveEnd);
        app.update(Action::CatalogEditorMoveLeft);
        app.update(Action::CatalogEditorBackspace);
        assert_eq!(catalog_profile_form_name(&app), "ab");

        app.update(Action::CatalogEditorMoveRight);
        app.update(Action::CatalogEditorInsert('x'));
        assert_eq!(catalog_profile_form_name(&app), "abx");
    }
}

#[test]
fn catalog_profile_form_reducer_deletes_previous_word_and_to_start() {
    let drafts = [
        CatalogDraft::Database(lazydb::model::catalog_editor::DatabaseDraft::new("")),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(true)),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(false)),
    ];

    for draft in drafts {
        let mut app = catalog_profile_form(draft);
        for character in "alpha beta".chars() {
            app.update(Action::CatalogEditorInsert(character));
        }
        app.update(Action::CatalogEditorDeletePreviousWord);
        assert_eq!(catalog_profile_form_name(&app), "alpha ");
        app.update(Action::CatalogEditorDeleteToStart);
        assert_eq!(catalog_profile_form_name(&app), "");
    }
}

#[test]
fn catalog_profile_form_password_actions_are_secret_and_only_edit_password_focus() {
    let mut app = catalog_profile_form(CatalogDraft::Role(
        lazydb::model::catalog_editor::RoleDraft::new(true),
    ));
    app.update(Action::CatalogEditorFocusFormField(
        CatalogFormFocus::Password,
    ));

    let input = lazydb::model::profile_manager::ProfileInput::from("sensitive-value");
    let insert_action = Action::CatalogEditorPasswordInsert(
        lazydb::model::profile_manager::ProfileInput::from('x'),
    );
    let paste_action = Action::CatalogEditorPasswordPaste(input);
    assert!(!format!("{insert_action:?}").contains('x'));
    assert!(!format!("{paste_action:?}").contains("sensitive-value"));

    app.update(paste_action);
    let role = match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
        .unwrap()
    {
        CatalogDraft::Role(role) => role,
        _ => panic!("role draft expected"),
    };
    assert_eq!(role.password.as_ref().unwrap().expose(), "sensitive-value");
    assert!(!format!("{role:?}").contains("sensitive-value"));

    app.update(Action::CatalogEditorFocusFormField(CatalogFormFocus::Name));
    app.update(insert_action);
    assert_eq!(catalog_profile_form_name(&app), "");
    let role = match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
        .unwrap()
    {
        CatalogDraft::Role(role) => role,
        _ => panic!("role draft expected"),
    };
    assert_eq!(role.password.as_ref().unwrap().expose(), "sensitive-value");
}

#[test]
fn catalog_profile_form_paste_is_one_undoable_edit_including_passwords() {
    let drafts = [
        CatalogDraft::Database(lazydb::model::catalog_editor::DatabaseDraft::new("")),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(true)),
        CatalogDraft::Role(lazydb::model::catalog_editor::RoleDraft::new(false)),
    ];

    for draft in drafts {
        let password_focus = matches!(&draft, CatalogDraft::Role(_));
        let mut app = catalog_profile_form(draft);
        if password_focus {
            app.update(Action::CatalogEditorFocusFormField(
                CatalogFormFocus::Password,
            ));
            app.update(Action::CatalogEditorPasswordPaste(
                lazydb::model::profile_manager::ProfileInput::from("alpha 界 beta"),
            ));
        } else {
            app.update(Action::CatalogEditorPaste("alpha 界 beta".into()));
        }

        app.update(Action::CatalogEditorUndo);
        if password_focus {
            let CatalogDraft::Role(role) = app
                .catalog_editor
                .as_ref()
                .and_then(|editor| editor.draft.as_ref())
                .unwrap()
            else {
                panic!("role draft expected");
            };
            assert!(role.password.is_none());
            app.update(Action::CatalogEditorRedo);
            let CatalogDraft::Role(role) = app
                .catalog_editor
                .as_ref()
                .and_then(|editor| editor.draft.as_ref())
                .unwrap()
            else {
                panic!("role draft expected");
            };
            assert_eq!(role.password.as_ref().unwrap().expose(), "alpha 界 beta");
            assert!(!format!("{role:?}").contains("alpha 界 beta"));
        } else {
            assert_eq!(catalog_profile_form_name(&app), "");
            app.update(Action::CatalogEditorRedo);
            assert_eq!(catalog_profile_form_name(&app), "alpha 界 beta");
        }
    }
}

#[test]
fn catalog_profile_form_reducer_ignores_paste_during_busy_operations() {
    let mut app = catalog_profile_form(CatalogDraft::Database(
        lazydb::model::catalog_editor::DatabaseDraft::new(""),
    ));
    app.catalog_editor.as_mut().unwrap().operation =
        Some(lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 1 });
    app.update(Action::CatalogEditorPaste("must not insert".into()));
    assert_eq!(catalog_profile_form_name(&app), "");
}

fn materialized_view_editor(mode: CatalogMutationMode, focus: CatalogFormFocus) -> App {
    simple_catalog_editor(
        mode,
        CatalogDraft::MaterializedView(MaterializedViewDraft {
            name: "mv".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "comment".into(),
            query: "SELECT 1".into(),
            tablespace: "fast".into(),
            with_data: true,
            focus,
            query_editable: mode == CatalogMutationMode::Create,
            clickhouse_native: None,
        }),
    )
}

fn materialized_view_draft(app: &App) -> &MaterializedViewDraft {
    match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
        .expect("catalog draft")
    {
        CatalogDraft::MaterializedView(draft) => draft,
        _ => panic!("materialized view draft expected"),
    }
}

#[test]
fn materialized_view_storage_controls_update_independently() {
    let mut app =
        materialized_view_editor(CatalogMutationMode::Create, CatalogFormFocus::Tablespace);

    app.update(Action::CatalogEditorInsert(' '));
    assert_eq!(materialized_view_draft(&app).tablespace.value(), "fast ");
    assert!(materialized_view_draft(&app).with_data);

    app.catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
        .and_then(|draft| match draft {
            CatalogDraft::MaterializedView(draft) => Some(draft),
            _ => None,
        })
        .unwrap()
        .focus = CatalogFormFocus::WithData;
    app.update(Action::CatalogEditorToggleFocused);

    assert_eq!(materialized_view_draft(&app).tablespace.value(), "fast ");
    assert!(!materialized_view_draft(&app).with_data);
}

#[test]
fn materialized_view_edit_skips_and_does_not_edit_query() {
    let mut app = materialized_view_editor(CatalogMutationMode::Edit, CatalogFormFocus::Comment);

    app.update(Action::CatalogEditorFieldNext);
    assert_eq!(
        materialized_view_draft(&app).focus,
        CatalogFormFocus::Tablespace
    );

    app.catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
        .and_then(|draft| match draft {
            CatalogDraft::MaterializedView(draft) => Some(draft),
            _ => None,
        })
        .unwrap()
        .focus = CatalogFormFocus::Query;
    app.update(Action::CatalogEditorInsert('x'));
    assert_eq!(materialized_view_draft(&app).query.value(), "SELECT 1");

    app.catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
        .and_then(|draft| match draft {
            CatalogDraft::MaterializedView(draft) => Some(draft),
            _ => None,
        })
        .unwrap()
        .focus = CatalogFormFocus::WithData;
    app.update(Action::CatalogEditorToggleFocused);
    assert!(materialized_view_draft(&app).with_data);
}

#[test]
fn catalog_preview_focuses_the_first_invalid_view_field() {
    let mut app = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::View(ViewDraft {
            name: "".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            query: "SELECT 1".into(),
            output_columns: "".into(),
            security_barrier: ViewOption::available(None),
            security_invoker: ViewOption::available(None),
            check_option: ViewOption::available(None),
            focus: CatalogFormFocus::Review,
            clickhouse_native: None,
        }),
    );
    app.update(Action::CatalogEditorPreview);
    assert_eq!(view_focus(&app), CatalogFormFocus::Name);

    view_draft_mut(&mut app).name = "v".into();
    view_draft_mut(&mut app).query = "".into();
    app.update(Action::CatalogEditorPreview);
    assert_eq!(view_focus(&app), CatalogFormFocus::Query);
}

#[test]
fn catalog_preview_focuses_invalid_materialized_view_query() {
    let mut app = materialized_view_editor(CatalogMutationMode::Create, CatalogFormFocus::Review);
    materialized_view_draft_mut(&mut app).query = "".into();
    app.update(Action::CatalogEditorPreview);
    assert_eq!(materialized_view_draft(&app).focus, CatalogFormFocus::Query);
}

#[test]
fn catalog_preview_focuses_each_invalid_sequence_field() {
    for focus in [
        CatalogFormFocus::Increment,
        CatalogFormFocus::MinValue,
        CatalogFormFocus::OwnedBy,
    ] {
        let mut app = simple_catalog_editor(
            CatalogMutationMode::Create,
            CatalogDraft::Sequence(SequenceDraft {
                name: "seq".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "".into(),
                data_type: "bigint".into(),
                increment: "1".into(),
                min_value: SequenceBound::Unset.into(),
                max_value: SequenceBound::Unset.into(),
                start_value: "1".into(),
                restart_value: "".into(),
                cache: "1".into(),
                cycle: false,
                owned_by: "NONE".into(),
                focus: CatalogFormFocus::Review,
            }),
        );
        let draft = sequence_draft_mut(&mut app);
        match focus {
            CatalogFormFocus::Increment => draft.increment = "bad".into(),
            CatalogFormFocus::MinValue => {
                draft.min_value = lazydb::model::catalog_editor::SequenceBoundDraft {
                    kind: lazydb::model::catalog_editor::SequenceBoundKind::Custom,
                    value: "bad".into(),
                };
            }
            CatalogFormFocus::OwnedBy => draft.owned_by = "public.table".into(),
            _ => unreachable!(),
        }
        app.update(Action::CatalogEditorPreview);
        assert_eq!(sequence_focus(&app), focus);
    }
}

fn view_focus(app: &App) -> CatalogFormFocus {
    match app.catalog_editor.as_ref().unwrap().draft.as_ref().unwrap() {
        CatalogDraft::View(draft) => draft.focus,
        _ => panic!("view draft expected"),
    }
}

fn view_draft_mut(app: &mut App) -> &mut ViewDraft {
    match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        CatalogDraft::View(draft) => draft,
        _ => panic!("view draft expected"),
    }
}

fn materialized_view_draft_mut(app: &mut App) -> &mut MaterializedViewDraft {
    match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        CatalogDraft::MaterializedView(draft) => draft,
        _ => panic!("materialized view draft expected"),
    }
}

fn sequence_draft_mut(app: &mut App) -> &mut SequenceDraft {
    match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        CatalogDraft::Sequence(draft) => draft,
        _ => panic!("sequence draft expected"),
    }
}

fn sequence_focus(app: &App) -> CatalogFormFocus {
    match app.catalog_editor.as_ref().unwrap().draft.as_ref().unwrap() {
        CatalogDraft::Sequence(draft) => draft.focus,
        _ => panic!("sequence draft expected"),
    }
}

#[test]
fn typed_form_focus_delegates_to_view_materialized_view_and_sequence_drafts() {
    let mut view = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::View(ViewDraft {
            name: "v".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            query: "SELECT 1".into(),
            output_columns: "".into(),
            security_barrier: ViewOption::available(None),
            security_invoker: ViewOption::available(None),
            check_option: ViewOption::available(None),
            focus: CatalogFormFocus::Name,
            clickhouse_native: None,
        }),
    );
    view.update(Action::CatalogEditorFocusFormField(CatalogFormFocus::Query));
    assert_eq!(
        view.catalog_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .focused_action(),
        None
    );
    assert_eq!(
        match view
            .catalog_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
        {
            CatalogDraft::View(draft) => draft.focus,
            _ => panic!("view draft expected"),
        },
        CatalogFormFocus::Query
    );

    let mut sequence = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::Sequence(SequenceDraft {
            name: "seq".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            data_type: "bigint".into(),
            increment: "1".into(),
            min_value: SequenceBound::Unset.into(),
            max_value: SequenceBound::Unset.into(),
            start_value: "1".into(),
            restart_value: "".into(),
            cache: "1".into(),
            cycle: false,
            owned_by: "NONE".into(),
            focus: CatalogFormFocus::Name,
        }),
    );
    sequence.update(Action::CatalogEditorFocusFormField(
        CatalogFormFocus::MinValue,
    ));
    assert_eq!(
        match sequence
            .catalog_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
        {
            CatalogDraft::Sequence(draft) => draft.focus,
            _ => panic!("sequence draft expected"),
        },
        CatalogFormFocus::MinValue
    );
}

#[test]
fn typed_form_focus_rejects_disabled_and_read_only_fields() {
    let mut view = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::View(ViewDraft {
            name: "v".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            query: "SELECT 1".into(),
            output_columns: "".into(),
            security_barrier: ViewOption::unavailable("unsupported"),
            security_invoker: ViewOption::available(None),
            check_option: ViewOption::available(None),
            focus: CatalogFormFocus::Name,
            clickhouse_native: None,
        }),
    );
    view.update(Action::CatalogEditorFocusFormField(
        CatalogFormFocus::SecurityBarrier,
    ));
    assert_eq!(
        match view
            .catalog_editor
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
        {
            CatalogDraft::View(draft) => draft.focus,
            _ => panic!("view draft expected"),
        },
        CatalogFormFocus::Name
    );

    let mut materialized =
        materialized_view_editor(CatalogMutationMode::Edit, CatalogFormFocus::Name);
    materialized.update(Action::CatalogEditorFocusFormField(CatalogFormFocus::Query));
    assert_eq!(
        materialized_view_draft(&materialized).focus,
        CatalogFormFocus::Name
    );
}

#[test]
fn catalog_editor_paste_supports_view_materialized_view_and_sequence_text_fields() {
    let mut view = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::View(ViewDraft {
            name: "v".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            query: "SELECT ".into(),
            output_columns: "".into(),
            security_barrier: ViewOption::available(None),
            security_invoker: ViewOption::available(None),
            check_option: ViewOption::available(None),
            focus: CatalogFormFocus::Query,
            clickhouse_native: None,
        }),
    );
    view.update(Action::CatalogEditorPaste("名 前".into()));
    let Some(CatalogDraft::View(draft)) = view
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert_eq!(draft.query.value(), "SELECT 名 前");

    let mut materialized_view =
        materialized_view_editor(CatalogMutationMode::Create, CatalogFormFocus::Tablespace);
    materialized_view.update(Action::CatalogEditorPaste(" 表 空间".into()));
    assert_eq!(
        materialized_view_draft(&materialized_view)
            .tablespace
            .value(),
        "fast 表 空间"
    );

    let mut sequence = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::Sequence(SequenceDraft {
            name: "seq".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            data_type: "bigint".into(),
            increment: "1".into(),
            min_value: SequenceBound::Unset.into(),
            max_value: SequenceBound::Unset.into(),
            start_value: "1".into(),
            restart_value: "".into(),
            cache: "1".into(),
            cycle: false,
            owned_by: "public.table.".into(),
            focus: CatalogFormFocus::OwnedBy,
        }),
    );
    sequence.update(Action::CatalogEditorPaste("列 名".into()));
    let Some(CatalogDraft::Sequence(draft)) = sequence
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("sequence draft expected");
    };
    assert_eq!(draft.owned_by.value(), "public.table.列 名");
}

#[test]
fn catalog_editor_paste_is_noop_for_read_only_choice_toggle_and_action_focus() {
    let mut materialized_view =
        materialized_view_editor(CatalogMutationMode::Edit, CatalogFormFocus::Query);
    materialized_view.update(Action::CatalogEditorPaste(" ignored".into()));
    assert_eq!(
        materialized_view_draft(&materialized_view).query.value(),
        "SELECT 1"
    );

    for focus in [CatalogFormFocus::WithData, CatalogFormFocus::Review] {
        let mut app = materialized_view_editor(CatalogMutationMode::Create, focus);
        app.update(Action::CatalogEditorPaste(" ignored".into()));
        let draft = materialized_view_draft(&app);
        assert_eq!(draft.tablespace.value(), "fast");
        assert!(draft.with_data);
    }

    for focus in [
        CatalogFormFocus::SecurityBarrier,
        CatalogFormFocus::SecurityInvoker,
        CatalogFormFocus::Cancel,
    ] {
        let mut view = simple_catalog_editor(
            CatalogMutationMode::Create,
            CatalogDraft::View(ViewDraft {
                name: "v".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "".into(),
                query: "SELECT 1".into(),
                output_columns: "".into(),
                security_barrier: ViewOption::unavailable("unsupported"),
                security_invoker: ViewOption::available(None),
                check_option: ViewOption::available(None),
                focus,
                clickhouse_native: None,
            }),
        );
        view.update(Action::CatalogEditorPaste(" ignored".into()));
        let Some(CatalogDraft::View(draft)) = view
            .catalog_editor
            .as_ref()
            .and_then(|editor| editor.draft.as_ref())
        else {
            panic!("view draft expected");
        };
        assert_eq!(draft.query.value(), "SELECT 1");
        assert_eq!(
            draft.security_barrier,
            ViewOption::unavailable("unsupported")
        );
        assert_eq!(draft.security_invoker, ViewOption::available(None));
    }

    for focus in [
        CatalogFormFocus::MinValue,
        CatalogFormFocus::Cycle,
        CatalogFormFocus::Cancel,
    ] {
        let mut sequence = simple_catalog_editor(
            CatalogMutationMode::Create,
            CatalogDraft::Sequence(SequenceDraft {
                name: "seq".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "".into(),
                data_type: "bigint".into(),
                increment: "1".into(),
                min_value: SequenceBound::Unset.into(),
                max_value: SequenceBound::Unset.into(),
                start_value: "1".into(),
                restart_value: "".into(),
                cache: "1".into(),
                cycle: false,
                owned_by: "NONE".into(),
                focus,
            }),
        );
        sequence.update(Action::CatalogEditorPaste(" ignored".into()));
        let Some(CatalogDraft::Sequence(draft)) = sequence
            .catalog_editor
            .as_ref()
            .and_then(|editor| editor.draft.as_ref())
        else {
            panic!("sequence draft expected");
        };
        assert_eq!(draft.min_value.to_bound(), SequenceBound::Unset);
        assert!(!draft.cycle);
        assert_eq!(draft.owned_by.value(), "NONE");
    }
}

#[test]
fn view_option_actions_cycle_only_the_focused_available_choice() {
    let mut app = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::View(ViewDraft {
            name: "v".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            query: "SELECT 1".into(),
            output_columns: "".into(),
            security_barrier: ViewOption::available(None),
            security_invoker: ViewOption::unavailable("unsupported"),
            check_option: ViewOption::available(None),
            focus: CatalogFormFocus::SecurityBarrier,
            clickhouse_native: None,
        }),
    );

    app.update(Action::CatalogEditorCycleChoice(1));
    let Some(CatalogDraft::View(draft)) =
        app.catalog_editor.as_ref().and_then(|e| e.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert_eq!(draft.security_barrier.value, Some(true));
    assert_eq!(draft.check_option.value, None);

    app.catalog_editor
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .and_then(|draft| match draft {
            CatalogDraft::View(draft) => Some(draft),
            _ => None,
        })
        .unwrap()
        .focus = CatalogFormFocus::SecurityInvoker;
    app.update(Action::CatalogEditorCycleChoice(1));
    let Some(CatalogDraft::View(draft)) =
        app.catalog_editor.as_ref().and_then(|e| e.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert_eq!(
        draft.security_invoker,
        ViewOption::unavailable("unsupported")
    );
}

#[test]
fn sequence_control_actions_edit_custom_bound_and_toggle_independently() {
    let mut app = simple_catalog_editor(
        CatalogMutationMode::Create,
        CatalogDraft::Sequence(SequenceDraft {
            name: "seq".into(),
            schema: "public".into(),
            owner: "postgres".into(),
            comment: "".into(),
            data_type: "bigint".into(),
            increment: "1".into(),
            min_value: SequenceBound::Unset.into(),
            max_value: SequenceBound::Unset.into(),
            start_value: "1".into(),
            restart_value: "".into(),
            cache: "1".into(),
            cycle: false,
            owned_by: "NONE".into(),
            focus: CatalogFormFocus::MinValue,
        }),
    );

    app.update(Action::CatalogEditorCycleChoice(1));
    app.update(Action::CatalogEditorCycleChoice(1));
    app.update(Action::CatalogEditorInsert('-'));
    app.update(Action::CatalogEditorPaste("100".into()));
    let draft = match app
        .catalog_editor
        .as_ref()
        .and_then(|e| e.draft.as_ref())
        .unwrap()
    {
        CatalogDraft::Sequence(draft) => draft,
        _ => panic!("sequence draft expected"),
    };
    assert_eq!(
        draft.min_value.to_bound(),
        SequenceBound::Value("-100".into())
    );
    assert_eq!(draft.max_value.to_bound(), SequenceBound::Unset);

    app.catalog_editor
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .and_then(|draft| match draft {
            CatalogDraft::Sequence(draft) => Some(draft),
            _ => None,
        })
        .unwrap()
        .focus = CatalogFormFocus::Cycle;
    app.update(Action::CatalogEditorToggleFocused);
    let draft = match app
        .catalog_editor
        .as_ref()
        .and_then(|e| e.draft.as_ref())
        .unwrap()
    {
        CatalogDraft::Sequence(draft) => draft,
        _ => panic!("sequence draft expected"),
    };
    assert!(draft.cycle);
    assert_eq!(
        draft.min_value.to_bound(),
        SequenceBound::Value("-100".into())
    );
}

#[test]
fn table_column_details_actions_open_confirm_and_cancel_atomically() {
    let mut app = table_editor_for_paste();
    draft_mut(&mut app).columns[0].name = "id".into();
    draft_mut(&mut app).focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;

    app.update(Action::CatalogEditorOpenTableColumnDetails);
    app.update(Action::CatalogEditorInsert('2'));
    assert_eq!(draft(&app).columns[0].name.value(), "id");

    app.update(Action::CatalogEditorCancelTableColumnDetails);
    assert_eq!(draft(&app).columns[0].name.value(), "id");

    app.update(Action::CatalogEditorOpenTableColumnDetails);
    app.update(Action::CatalogEditorInsert('2'));
    app.update(Action::CatalogEditorConfirmTableColumnDetails);
    assert_eq!(draft(&app).columns[0].name.value(), "id2");
}

#[test]
fn dirty_table_cancel_opens_discard_confirmation_and_keeps_or_discards_draft() {
    let mut app = table_editor_for_paste();
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    else {
        panic!("table draft expected");
    };
    draft.name = "events".into();
    assert!(app.update(Action::CatalogEditorCancel).is_empty());
    assert!(matches!(
        app.overlay,
        Some(
            lazydb::model::workspace::Overlay::CatalogEditorDiscardConfirm {
                focus: lazydb::model::workspace::CatalogEditorDiscardFocus::KeepEditing
            }
        )
    ));

    app.update(Action::CatalogEditorDiscardKeepEditing);
    assert_eq!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::CatalogEditor)
    );
    assert_eq!(
        app.catalog_editor
            .as_ref()
            .and_then(|editor| editor.draft.as_ref())
            .and_then(|draft| match draft {
                lazydb::model::catalog_editor::CatalogDraft::Table(draft) => {
                    Some(draft.name.value())
                }
                _ => None,
            }),
        Some("events")
    );

    app.update(Action::CatalogEditorCancel);
    app.update(Action::CatalogEditorDiscardChanges);
    assert!(app.catalog_editor.is_none());
    assert!(app.overlay.is_none());
}

#[test]
fn discard_changes_action_is_ignored_outside_discard_confirmation() {
    let mut app = table_editor_for_paste();
    app.overlay = Some(Overlay::CatalogEditor);

    app.update(Action::CatalogEditorDiscardChanges);

    assert!(app.catalog_editor.is_some());
    assert_eq!(app.overlay, Some(Overlay::CatalogEditor));
}

#[test]
fn applying_table_editor_cannot_be_closed_by_cancel_action() {
    let mut app = table_editor_for_paste();
    app.catalog_editor.as_mut().unwrap().operation =
        Some(lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 1 });
    app.overlay = Some(lazydb::model::workspace::Overlay::CatalogEditor);

    assert!(app.update(Action::CatalogEditorCancel).is_empty());
    assert_eq!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::CatalogEditor)
    );
    assert!(app.catalog_editor.is_some());
    assert!(
        app.notifications
            .history()
            .any(|notification| { notification.body.contains("finish before closing") })
    );
}

#[test]
fn applying_table_editor_blocks_global_quit() {
    let mut app = table_editor_for_paste();
    app.catalog_editor.as_mut().unwrap().operation =
        Some(lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 1 });
    app.overlay = Some(lazydb::model::workspace::Overlay::CatalogEditor);

    assert!(app.update(Action::Quit).is_empty());
    assert!(!app.should_quit);
    assert!(app.notifications.history().any(|notification| {
        notification
            .body
            .contains("running SQL or relation loads to finish before quitting")
    }));
}

#[test]
fn invalid_column_details_confirmation_keeps_parent_draft_unchanged() {
    let mut app = table_editor_for_paste();
    app.update(Action::CatalogEditorOpenTableColumnDetails);
    let before = draft(&app).columns[0].name.value().to_owned();
    app.update(Action::CatalogEditorConfirmTableColumnDetails);

    let table = draft(&app);
    assert_eq!(table.columns[0].name.value(), before);
    assert!(table.column_editor.as_ref().unwrap().error.is_some());
    assert_eq!(
        table.focus,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name,
        )
    );
}

#[test]
fn table_new_column_is_committed_only_after_details_confirmation() {
    let mut app = table_editor_for_paste();
    draft_mut(&mut app).columns[0].name = "id".into();
    draft_mut(&mut app).focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;

    app.update(Action::CatalogEditorAddTableColumn);
    assert_eq!(draft(&app).columns.len(), 1);
    app.update(Action::CatalogEditorInsert('n'));
    app.update(Action::CatalogEditorCancelTableColumnDetails);
    assert_eq!(draft(&app).columns.len(), 1);

    app.update(Action::CatalogEditorAddTableColumn);
    app.update(Action::CatalogEditorInsert('n'));
    app.update(Action::CatalogEditorConfirmTableColumnDetails);
    assert_eq!(draft(&app).columns.len(), 2);
    assert_eq!(draft(&app).columns[1].name.value(), "n");
}

#[test]
fn table_constraint_add_actions_remain_transient_until_confirmed() {
    for action in [
        Action::CatalogEditorAddTableConstraintPrimaryKey,
        Action::CatalogEditorAddTableConstraintUnique,
        Action::CatalogEditorAddTableConstraintCheck,
        Action::CatalogEditorAddTableConstraintForeignKey,
    ] {
        let mut app =
            sqlite_table_editor(lazydb::db::catalog_mutation::CatalogMutationMode::Create);
        assert!(app.update(action).is_empty());
        assert!(draft(&app).constraint_editor.is_some());
        assert!(draft(&app).table_constraints.is_empty());

        let before = app.catalog_editor.as_ref().unwrap().page;
        assert!(app.update(Action::CatalogEditorPreview).is_empty());
        assert_eq!(app.catalog_editor.as_ref().unwrap().page, before);
        assert!(app.catalog_editor.as_ref().unwrap().plan.is_none());

        app.update(Action::CatalogEditorCancelTableConstraintDetails);
        assert!(draft(&app).constraint_editor.is_none());
        assert!(draft(&app).table_constraints.is_empty());
    }
}

#[test]
fn invalid_table_constraint_confirmation_keeps_the_details_open() {
    let mut app = sqlite_table_editor(lazydb::db::catalog_mutation::CatalogMutationMode::Create);
    app.update(Action::CatalogEditorAddTableConstraintCheck);
    app.update(Action::CatalogEditorConfirmTableConstraintDetails);
    let table = draft(&app);
    assert!(table.table_constraints.is_empty());
    assert!(table.constraint_editor.as_ref().unwrap().error.is_some());
    assert_eq!(
        table
            .constraint_editor
            .as_ref()
            .unwrap()
            .draft
            .selected_field,
        1
    );

    draft_mut(&mut app)
        .constraint_editor
        .as_mut()
        .unwrap()
        .draft
        .expression = "id > 0".into();
    app.update(Action::CatalogEditorConfirmTableConstraintDetails);
    assert!(draft(&app).constraint_editor.is_none());
    assert_eq!(draft(&app).table_constraints.len(), 1);

    let mut app = sqlite_table_editor(lazydb::db::catalog_mutation::CatalogMutationMode::Create);
    app.update(Action::CatalogEditorAddTableConstraintForeignKey);
    {
        let session = draft_mut(&mut app)
            .constraint_editor
            .as_mut()
            .expect("foreign key session");
        session.draft.columns = String::new().into();
    }
    app.update(Action::CatalogEditorConfirmTableConstraintDetails);
    assert!(draft(&app).table_constraints.is_empty());
    assert!(
        draft(&app)
            .constraint_editor
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|error| error.contains("columns are required"))
    );
}

#[test]
fn multiple_table_constraints_can_be_accepted_reopened_and_removed() {
    let mut app = sqlite_table_editor(lazydb::db::catalog_mutation::CatalogMutationMode::Create);
    for (action, kind) in [
        (Action::CatalogEditorAddTableConstraintPrimaryKey, "pk"),
        (Action::CatalogEditorAddTableConstraintUnique, "unique"),
    ] {
        app.update(action);
        draft_mut(&mut app)
            .constraint_editor
            .as_mut()
            .unwrap()
            .draft
            .name = format!("items_{kind}").into();
        app.update(Action::CatalogEditorConfirmTableConstraintDetails);
    }
    app.update(Action::CatalogEditorAddTableConstraintCheck);
    draft_mut(&mut app)
        .constraint_editor
        .as_mut()
        .unwrap()
        .draft
        .expression = "id > 0".into();
    app.update(Action::CatalogEditorConfirmTableConstraintDetails);
    assert_eq!(draft(&app).table_constraints.len(), 3);

    app.update(Action::CatalogEditorEditTableConstraint(1));
    assert!(draft(&app).constraint_editor.is_some());
    draft_mut(&mut app)
        .constraint_editor
        .as_mut()
        .unwrap()
        .draft
        .name = "items_unique_v2".into();
    app.update(Action::CatalogEditorConfirmTableConstraintDetails);
    assert_eq!(
        draft(&app).table_constraints[1].name.value(),
        "items_unique_v2"
    );

    app.update(Action::CatalogEditorRemoveTableConstraint(1));
    assert_eq!(draft(&app).table_constraints.len(), 2);
    assert!(matches!(
        draft(&app).table_constraints[1].kind,
        lazydb::db::catalog_mutation::ConstraintDefinitionKind::Check { .. }
    ));
}

#[test]
fn foreign_key_details_accept_all_seven_fields() {
    let mut app = sqlite_table_editor(lazydb::db::catalog_mutation::CatalogMutationMode::Create);
    app.update(Action::CatalogEditorAddTableConstraintForeignKey);
    for field in 0..7 {
        app.update(Action::CatalogEditorFocusTableConstraintDetails(field));
        assert_eq!(
            draft(&app)
                .constraint_editor
                .as_ref()
                .unwrap()
                .draft
                .selected_field,
            field
        );
    }
    {
        let session = draft_mut(&mut app)
            .constraint_editor
            .as_mut()
            .expect("foreign key session");
        session.draft.name = "items_parent".into();
        session.draft.columns = "parent_id".into();
        session.draft.referenced_schema = "main".into();
        session.draft.referenced_relation = "parent".into();
        session.draft.referenced_columns = "id".into();
        session.draft.on_update = "CASCADE".into();
        session.draft.on_delete = "RESTRICT".into();
    }
    app.update(Action::CatalogEditorConfirmTableConstraintDetails);
    let constraint = &draft(&app).table_constraints[0];
    assert!(matches!(
        &constraint.kind,
        lazydb::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. }
    ));
    assert_eq!(constraint.name.value(), "items_parent");
    assert_eq!(constraint.referenced_schema.value(), "main");
    assert_eq!(constraint.referenced_relation.value(), "parent");
    assert_eq!(constraint.referenced_columns.value(), "id");
    assert_eq!(constraint.on_update.value(), "CASCADE");
    assert_eq!(constraint.on_delete.value(), "RESTRICT");
}

#[test]
fn structured_constraint_actions_are_rejected_outside_new_table_creation() {
    for (kind, mode) in [
        (
            lazydb::profile::DatabaseKind::Postgres,
            lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        ),
        (
            lazydb::profile::DatabaseKind::Sqlite,
            lazydb::db::catalog_mutation::CatalogMutationMode::Edit,
        ),
        (
            lazydb::profile::DatabaseKind::DuckDb,
            lazydb::db::catalog_mutation::CatalogMutationMode::Edit,
        ),
    ] {
        let mut app = sqlite_table_editor(mode);
        draft_mut(&mut app).database_kind = kind;
        app.update(Action::CatalogEditorAddTableConstraintPrimaryKey);
        assert!(draft(&app).constraint_editor.is_none());
        assert!(draft(&app).table_constraints.is_empty());
        assert!(
            app.catalog_editor
                .as_ref()
                .and_then(|editor| editor.error.as_deref())
                .is_some_and(|error| !error.is_empty())
        );
    }
}

#[test]
fn table_preview_opens_column_details_for_the_first_invalid_column_field() {
    let mut app = table_editor_for_paste();
    draft_mut(&mut app).name = "events".into();
    draft_mut(&mut app).focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;

    app.update(Action::CatalogEditorPreview);

    let draft = draft(&app);
    assert_eq!(draft.selected_column, 0);
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name
        )
    );
    assert!(draft.column_editor.is_some());
    assert!(
        app.catalog_editor
            .as_ref()
            .and_then(|editor| editor.error.as_deref())
            .is_some_and(|error| error.contains("column 1 name is required"))
    );
}

fn draft(app: &App) -> &lazydb::model::catalog_editor::TableDraft {
    match app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
        .expect("catalog draft")
    {
        lazydb::model::catalog_editor::CatalogDraft::Table(draft) => draft,
        _ => panic!("table draft expected"),
    }
}

fn sqlite_table_editor(mode: CatalogMutationMode) -> App {
    let mut draft = lazydb::model::catalog_editor::TableDraft::new_for_database(
        "main",
        lazydb::profile::DatabaseKind::Sqlite,
    );
    draft.name = "items".into();
    draft.columns[0].name = "id".into();
    draft.columns[0].native_type = "INTEGER".into();
    let mut app = simple_catalog_editor(mode, CatalogDraft::Table(draft));
    app.overlay = Some(Overlay::CatalogEditor);
    app
}

#[test]
fn catalog_editor_paste_writes_multicharacter_table_name_at_general_name_focus() {
    let mut app = table_editor_for_paste();

    app.update(Action::CatalogEditorPaste("events\n数据🙂".into()));

    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(draft.name.value(), "events\n数据🙂");
}

#[test]
fn catalog_editor_paste_writes_multicharacter_column_name_at_column_name_focus() {
    let mut app = table_editor_for_paste();
    draft_mut(&mut app).begin_edit_selected_column();

    app.update(Action::CatalogEditorPaste("user\n名前🙂".into()));

    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(draft.columns[0].name.value(), "");
    assert_eq!(
        draft.column_editor.as_ref().unwrap().draft.name.value(),
        "user\n名前🙂"
    );
}

fn profile() -> ConnectionProfile {
    import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile
}

fn catalog_id(profile_id: Uuid) -> CatalogId {
    CatalogId::new(
        profile_id,
        lazydb::db::catalog::CatalogKind::Table,
        ["app", "public", "users"],
    )
}

#[test]
fn explorer_selection_resolves_direct_nodes_without_inheriting_owner_actions() {
    let profile = profile();
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);

    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    assert_eq!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::EditProfile(profile_id))
    );
    assert_eq!(
        app.resolve_explorer_mutation_intent(false),
        Some(ExplorerMutationIntent::Create(
            CatalogMutationAnchor::Profile { profile_id }
        ))
    );
    app.update(Action::OpenCatalogEdit);
    assert!(matches!(app.overlay, Some(Overlay::ProfileManager)));

    let id = catalog_id(profile_id);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id.clone()));
    assert_eq!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::Edit(
            CatalogMutationAnchor::Catalog(id.clone())
        ))
    );
    assert_eq!(
        app.resolve_explorer_mutation_intent(false),
        Some(ExplorerMutationIntent::Create(
            CatalogMutationAnchor::Catalog(id)
        ))
    );

    app.explorer.normalized.selected = Some(ExplorerNodeId::Group {
        parent: CatalogId::new(
            profile_id,
            lazydb::db::catalog::CatalogKind::Schema,
            ["app", "public"],
        ),
        group: lazydb::db::catalog::ObjectGroup::Tables,
    });
    assert!(matches!(
        app.resolve_explorer_mutation_intent(false),
        Some(ExplorerMutationIntent::Create(
            CatalogMutationAnchor::Group { .. }
        ))
    ));
    assert_eq!(app.resolve_explorer_mutation_intent(true), None);

    app.explorer.normalized.selected = Some(ExplorerNodeId::Status {
        owner: lazydb::model::explorer::ExplorerOwnerId::Profile(profile_id),
        kind: StatusRowKind::Loading,
    });
    assert_eq!(app.resolve_explorer_mutation_intent(false), None);
    assert_eq!(app.resolve_explorer_mutation_intent(true), None);
}

#[test]
fn help_edit_shortcut_uses_direct_selection_resolution() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = lazydb::model::workspace::Focus::Explorer;
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 1;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.target =
        Some(lazydb::model::execution_target::ExecutionTarget::from_profile(&app.profiles[0]));
    app.connection.mutation_capabilities =
        lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities();

    let id = catalog_id(profile_id);
    let entry = lazydb::db::catalog::CatalogEntry::relation(
        id.clone(),
        CatalogId::new(
            profile_id,
            lazydb::db::catalog::CatalogKind::Schema,
            ["app", "public"],
        ),
        lazydb::db::catalog::QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "users".into(),
        },
        "table",
        lazydb::db::catalog::OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog;
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::database(
                CatalogId::new(
                    profile_id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: None,
                    object: "app".into(),
                },
                "database",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::schema(
                CatalogId::new(
                    profile_id,
                    lazydb::db::catalog::CatalogKind::Schema,
                    ["app", "public"],
                ),
                CatalogId::new(
                    profile_id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: Some("public".into()),
                    object: "public".into(),
                },
                "schema",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog.insert(entry).unwrap();
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id));
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("edit selected object".into()));
    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::ExplorerEditCatalog)
    );
    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::ExplorerEditProfile,
    ));
    assert!(app.profile_manager.is_none());
    assert!(app.catalog_editor.is_none());
    assert_ne!(app.overlay, Some(Overlay::CatalogEditor));
}

#[test]
fn opening_create_on_schema_uses_capability_ordered_options() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
            current_user: Some("effective_role".into()),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });
    let database = CatalogId::new(profile_id, CatalogKind::Database, ["app"]);
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog;
    catalog
        .insert(
            CatalogEntry::database(
                database.clone(),
                QualifiedName {
                    database: Some("app".into()),
                    schema: None,
                    object: "app".into(),
                },
                "database",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog
        .insert(
            CatalogEntry::schema(
                schema.clone(),
                database,
                QualifiedName {
                    database: Some("app".into()),
                    schema: Some("public".into()),
                    object: "public".into(),
                },
                "schema",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    app.explorer.normalized.selected =
        Some(lazydb::model::explorer::ExplorerNodeId::Catalog(schema));
    let catalog_epoch = app
        .explorer
        .normalized
        .profiles
        .get(&profile_id)
        .unwrap()
        .catalog_epoch;

    app.update(Action::OpenCatalogCreate);
    let owner_request = app
        .update(Action::OpenCatalogCreate)
        .into_iter()
        .find_map(|command| match command {
            lazydb::action::Command::LoadCatalogOwnerContext(request) => Some(request),
            _ => None,
        });
    assert!(
        owner_request.is_none(),
        "owner request should be deduplicated"
    );
    let editor = app.catalog_editor.as_ref().expect("catalog editor");
    assert_eq!(
        editor
            .options
            .iter()
            .map(|option| option.object_type)
            .collect::<Vec<_>>(),
        vec![
            CatalogObjectType::Catalog(CatalogKind::Table),
            CatalogObjectType::Catalog(CatalogKind::View),
            CatalogObjectType::Catalog(CatalogKind::MaterializedView),
            CatalogObjectType::Catalog(CatalogKind::Sequence),
        ]
    );
    assert_eq!(editor.catalog_epoch, catalog_epoch);

    app.update(Action::CatalogEditorMove(1));
    app.update(Action::CatalogEditorSelect);
    let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert!(draft.security_invoker.availability.is_available());

    app.update(Action::CatalogEditorCancel);
    app.explorer.normalized.selected = Some(lazydb::model::explorer::ExplorerNodeId::Group {
        parent: CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]),
        group: lazydb::db::catalog::ObjectGroup::Tables,
    });
    app.update(Action::OpenCatalogCreate);
    let editor = app.catalog_editor.as_ref().expect("table editor");
    assert_eq!(
        editor.page,
        lazydb::model::catalog_editor::CatalogEditorPage::Form
    );
    assert_eq!(
        editor.object_type,
        Some(CatalogObjectType::Catalog(CatalogKind::Table))
    );
    assert!(matches!(
        editor.draft,
        Some(lazydb::model::catalog_editor::CatalogDraft::Table(_))
    ));
    app.update(Action::CatalogEditorOpenTableColumnDetails);
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name,
        )
    );
    app.update(Action::CatalogEditorCancelTableColumnDetails);
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::Columns
    );

    app.update(Action::CatalogEditorCancel);
    app.explorer.normalized.selected = Some(lazydb::model::explorer::ExplorerNodeId::Catalog(
        CatalogId::new(profile_id, CatalogKind::Database, ["app"]),
    ));
    app.update(Action::OpenCatalogCreate);
    let editor = app.catalog_editor.as_ref().expect("schema editor");
    assert_eq!(
        editor.page,
        lazydb::model::catalog_editor::CatalogEditorPage::Form
    );
    assert_eq!(
        editor.object_type,
        Some(CatalogObjectType::Catalog(CatalogKind::Schema))
    );
    assert!(matches!(
        editor.draft,
        Some(lazydb::model::catalog_editor::CatalogDraft::Schema(_))
    ));
    let Some(lazydb::model::catalog_editor::CatalogDraft::Schema(draft)) = editor.draft.as_ref()
    else {
        panic!("schema draft expected");
    };
    assert_eq!(draft.owner.value(), "effective_role");
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Schema(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.owner.set("");
    }

    app.update(Action::CatalogEditorInsert('n'));
    app.update(Action::CatalogEditorFieldNext);
    app.update(Action::CatalogEditorInsert('o'));
    app.update(Action::CatalogEditorFocusField(2));
    app.update(Action::CatalogEditorInsert('c'));
    app.update(Action::CatalogEditorFieldPrevious);
    app.update(Action::CatalogEditorInsert('w'));
    app.update(Action::CatalogEditorFocusField(2));
    let Some(lazydb::model::catalog_editor::CatalogDraft::Schema(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("schema draft expected");
    };
    assert_eq!(draft.name.value(), "n");
    assert_eq!(draft.owner.value(), "ow");
    assert_eq!(draft.comment.value(), "c");
    assert_eq!(draft.selected_field, 2);
}

#[test]
fn views_group_opens_view_form_with_connected_capabilities() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
            current_user: Some("postgres".into()),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Group {
        parent: schema,
        group: lazydb::db::catalog::ObjectGroup::Views,
    });

    app.update(Action::OpenCatalogCreate);

    let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert!(draft.security_invoker.availability.is_available());
}

#[test]
fn mutation_refresh_api_accepts_unique_targets_and_leaves_selection_for_reload() {
    let profile = profile();
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let target = lazydb::db::catalog::CatalogTarget::Databases;
    let commands = app.commands_for_catalog_targets(profile_id, &[target.clone(), target.clone()]);
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, lazydb::action::Command::LoadCatalogPage(_)))
            .count(),
        0
    );
    assert_eq!(
        app.explorer.selected_id(),
        Some(&ExplorerNodeId::Profile(profile_id))
    );
}

#[test]
fn relation_invalidation_is_exposed_by_catalog_mutation_impact() {
    let profile = profile();
    let profile_id = profile.id;
    let relation = CatalogId::new(
        profile_id,
        lazydb::db::catalog::CatalogKind::Table,
        ["db", "public", "users"],
    );
    let tab = lazydb::model::relation::RelationTab::with_descriptor(
        lazydb::model::relation::RelationDescriptor {
            key: lazydb::model::relation::RelationKey {
                profile_id,
                object_id: relation.clone(),
            },
            qualified_name: lazydb::db::catalog::QualifiedName {
                database: Some("db".into()),
                schema: Some("public".into()),
                object: "users".into(),
            },
            kind: lazydb::db::catalog::CatalogKind::Table,
            title: "users".into(),
        },
        lazydb::model::relation::RelationView::Data,
    );
    let impact = lazydb::db::catalog_mutation::CatalogMutationImpact {
        old_object_id: CatalogId::new(
            profile_id,
            lazydb::db::catalog::CatalogKind::Column,
            ["db", "public", "users", "id"],
        ),
        owning_relation_id: Some(relation),
        namespace: lazydb::db::catalog_mutation::CatalogMutationNamespace {
            database: None,
            schema: None,
        },
        native_identity_changed: false,
    };
    assert!(tab.invalidated_by_catalog_mutation(&impact));
}

#[test]
fn constraint_edit_is_allowed_from_a_direct_catalog_selection() {
    let profile = profile();
    let mut app = App::new(vec![profile.clone()]);
    let id = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::ForeignKey,
        ["app", "public", "events", "42", "9"],
    );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id.clone()));
    assert_eq!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::Edit(
            CatalogMutationAnchor::Catalog(id)
        ))
    );
}

#[test]
fn sequence_edit_is_allowed_from_a_direct_catalog_selection() {
    let profile = profile();
    let mut app = App::new(vec![profile.clone()]);
    let id = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::Sequence,
        ["app", "public", "seq", "42"],
    );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id));
    assert!(matches!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::Edit(_))
    ));
}

#[test]
fn table_column_selection_and_add_actions_sync_focus_and_details() {
    let profile = profile();
    let schema = CatalogId::new(profile.id, CatalogKind::Schema, ["app", "public"]);
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        CatalogMutationAnchor::Group {
            schema,
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        1,
        Vec::new(),
    );
    assert!(editor.select_object_type(CatalogObjectType::Catalog(CatalogKind::Table)));
    let mut app = App::new(vec![profile]);
    app.catalog_editor = Some(editor);

    app.update(Action::CatalogEditorSelectTableColumn(0));
    let table = app.catalog_editor.as_ref().unwrap().draft.as_ref().unwrap();
    let lazydb::model::catalog_editor::CatalogDraft::Table(table_draft) = table else {
        panic!("table draft expected");
    };
    assert_eq!(table_draft.selected_column, 0);
    assert_eq!(
        table_draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::Columns
    );

    draft_mut(&mut app).columns[0].name = "id".into();
    app.update(Action::CatalogEditorAddTableColumn);
    let table = app.catalog_editor.as_ref().unwrap().draft.as_ref().unwrap();
    let lazydb::model::catalog_editor::CatalogDraft::Table(table_draft) = table else {
        panic!("table draft expected");
    };
    assert_eq!(table_draft.columns[0].name.value(), "id");
    assert_eq!(table_draft.selected_column, 0);
    assert_eq!(table_draft.columns.len(), 1);
    assert_eq!(
        table_draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name
        )
    );
    draft_mut(&mut app).insert('n');
    app.update(Action::CatalogEditorConfirmTableColumnDetails);
    let final_draft = draft(&app);
    assert_eq!(final_draft.selected_column, 1);
    assert_eq!(final_draft.columns.len(), 2);
}

#[test]
fn catalog_mutation_failure_from_an_old_connection_is_ignored() {
    let old_profile = profile();
    let old_profile_id = old_profile.id;
    let new_profile = import_connection_url(":memory:", Some("new"))
        .unwrap()
        .profile;
    let old_connection = lazydb::identity::ConnectionIdentity {
        profile_id: old_profile.id,
        generation: 1,
    };
    let mut app = App::new(vec![old_profile, new_profile.clone()]);
    app.update(Action::ConnectionSucceeded {
        profile_id: new_profile.id,
        generation: 1,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3.50.0".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });

    let anchor = CatalogMutationAnchor::Catalog(CatalogId::new(
        old_profile_id,
        CatalogKind::Database,
        ["app"],
    ));
    let request = lazydb::db::catalog_mutation::CatalogMutationRequest::new(
        old_connection,
        2,
        0,
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor.clone(),
        CatalogObjectType::Catalog(CatalogKind::Schema),
    )
    .unwrap();
    let plan = lazydb::db::catalog_mutation::CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(CatalogKind::Schema),
        lazydb::db::catalog_mutation::CatalogMutationExecutionMode::Transactional,
        lazydb::db::catalog_mutation::CatalogMutationTarget::maintenance("postgres").unwrap(),
        vec![lazydb::db::catalog::CatalogTarget::Databases],
        lazydb::db::catalog_mutation::CatalogSelectionHint::Parent(
            lazydb::db::catalog::CatalogTarget::Databases,
        ),
        None,
        Vec::new(),
        vec!["CREATE SCHEMA events".into()],
    )
    .unwrap();
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        database_kind: None,
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor,
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Schema)),
        page: lazydb::model::catalog_editor::CatalogEditorPage::SqlPreview,
        operation: Some(
            lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 2 },
        ),
        catalog_epoch: 0,
        options: Vec::new(),
        selected_option: 0,
        draft: None,
        baseline: None,
        plan: Some(plan.clone()),
        error: None,
        outcome_unknown: false,
        owner_picker: Default::default(),
        preview_scroll: 0,
    });

    app.update(Action::CatalogMutationFailed {
        plan,
        message: "old connection failed".into(),
        outcome_unknown: false,
    });

    let editor = app.catalog_editor.as_ref().unwrap();
    assert!(matches!(
        editor.operation,
        Some(lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 2 })
    ));
    assert_eq!(editor.error, None);
}

#[test]
fn clickhouse_permission_failure_keeps_error_and_requests_catalog_refresh() {
    let mut profile = import_connection_url("clickhouse://localhost/analytics", Some("ch"))
        .unwrap()
        .profile;
    profile.kind = lazydb::profile::DatabaseKind::ClickHouse;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(
        app.profiles
            .iter()
            .find(|item| item.id == profile_id)
            .unwrap(),
    );
    app.sessions.register_attempt(
        target.clone(),
        lazydb::identity::ConnectionIdentity {
            profile_id,
            generation,
        },
    );
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::ClickHouse,
            version: "25.8".into(),
            database: "analytics".into(),
            current_user: Some("limited".into()),
        },
        mutation_capabilities:
            lazydb::db::clickhouse::ClickHouseAdapter::catalog_mutation_capabilities(),
    });
    assert!(app.sessions.accept_success(
        lazydb::identity::ConnectionIdentity {
            profile_id,
            generation
        },
        lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::ClickHouse,
            version: "25.8".into(),
            database: "analytics".into(),
            current_user: Some("limited".into()),
        },
        lazydb::db::clickhouse::ClickHouseAdapter::catalog_mutation_capabilities(),
    ));
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = generation;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.target = Some(target);
    app.explorer.catalog_sessions.insert(
        profile_id,
        lazydb::identity::ConnectionIdentity {
            profile_id,
            generation,
        },
    );
    let connection = lazydb::identity::ConnectionIdentity {
        profile_id,
        generation,
    };
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["analytics", "analytics"]);
    let request = lazydb::db::catalog_mutation::CatalogMutationRequest::new(
        connection,
        17,
        0,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Group {
            schema: schema.clone(),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        CatalogObjectType::Catalog(CatalogKind::Table),
    )
    .unwrap()
    .with_current_database("analytics");
    let refresh = lazydb::db::catalog::CatalogTarget::objects(
        schema,
        lazydb::db::catalog::ObjectGroup::Tables,
    )
    .unwrap();
    let plan = lazydb::db::catalog_mutation::CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(CatalogKind::Table),
        lazydb::db::catalog_mutation::CatalogMutationExecutionMode::Autocommit,
        lazydb::db::catalog_mutation::CatalogMutationTarget::maintenance("analytics").unwrap(),
        vec![refresh],
        lazydb::db::catalog_mutation::CatalogSelectionHint::Parent(
            lazydb::db::catalog::CatalogTarget::Databases,
        ),
        None,
        Vec::new(),
        vec!["CREATE TABLE analytics.denied (id UInt64) ENGINE = MergeTree ORDER BY id".into()],
    )
    .unwrap();
    let mut editor = CatalogEditorState::new(
        CatalogMutationMode::Create,
        plan.request.anchor.clone(),
        0,
        vec![],
    );
    editor.plan = Some(plan.clone());
    editor.page = CatalogEditorPage::SqlPreview;
    assert!(editor.begin_apply(17));
    app.catalog_editor = Some(editor);

    let commands = app.update(Action::CatalogMutationFailed {
        plan,
        message: "permission denied".into(),
        outcome_unknown: false,
    });

    let editor = app.catalog_editor.as_ref().unwrap();
    assert_eq!(editor.error.as_deref(), Some("permission denied"));
    assert!(!editor.outcome_unknown);
    assert_eq!(editor.operation, None);
    assert!(commands.iter().any(|command| matches!(
        command,
        lazydb::action::Command::LoadCatalogPage(request)
            if request.key.target == lazydb::db::catalog::CatalogTarget::objects(
                CatalogId::new(profile_id, CatalogKind::Schema, ["analytics", "analytics"]),
                lazydb::db::catalog::ObjectGroup::Tables,
            ).unwrap()
    )));
}

#[test]
fn clickhouse_unknown_mutation_outcome_keeps_editor_and_refreshes_catalog() {
    let mut profile = import_connection_url("clickhouse://localhost/analytics", Some("ch"))
        .unwrap()
        .profile;
    profile.kind = lazydb::profile::DatabaseKind::ClickHouse;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(
        app.profiles
            .iter()
            .find(|item| item.id == profile_id)
            .unwrap(),
    );
    app.sessions.register_attempt(
        target.clone(),
        lazydb::identity::ConnectionIdentity {
            profile_id,
            generation,
        },
    );
    let server = lazydb::db::ServerInfo {
        kind: lazydb::profile::DatabaseKind::ClickHouse,
        version: "25.8".into(),
        database: "analytics".into(),
        current_user: Some("limited".into()),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server.clone(),
        mutation_capabilities:
            lazydb::db::clickhouse::ClickHouseAdapter::catalog_mutation_capabilities(),
    });
    assert!(app.sessions.accept_success(
        lazydb::identity::ConnectionIdentity {
            profile_id,
            generation
        },
        server,
        lazydb::db::clickhouse::ClickHouseAdapter::catalog_mutation_capabilities(),
    ));
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = generation;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.target = Some(target);
    app.explorer.catalog_sessions.insert(
        profile_id,
        lazydb::identity::ConnectionIdentity {
            profile_id,
            generation,
        },
    );
    let connection = lazydb::identity::ConnectionIdentity {
        profile_id,
        generation,
    };
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["analytics", "analytics"]);
    let request = lazydb::db::catalog_mutation::CatalogMutationRequest::new(
        connection,
        18,
        0,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Group {
            schema: schema.clone(),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        CatalogObjectType::Catalog(CatalogKind::Table),
    )
    .unwrap()
    .with_current_database("analytics");
    let refresh = lazydb::db::catalog::CatalogTarget::objects(
        schema.clone(),
        lazydb::db::catalog::ObjectGroup::Tables,
    )
    .unwrap();
    let plan = lazydb::db::catalog_mutation::CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(CatalogKind::Table),
        lazydb::db::catalog_mutation::CatalogMutationExecutionMode::Autocommit,
        lazydb::db::catalog_mutation::CatalogMutationTarget::maintenance("analytics").unwrap(),
        vec![refresh.clone()],
        lazydb::db::catalog_mutation::CatalogSelectionHint::Parent(
            lazydb::db::catalog::CatalogTarget::Databases,
        ),
        None,
        Vec::new(),
        vec![
            "CREATE TABLE analytics.maybe_created (id UInt64) ENGINE = MergeTree ORDER BY id"
                .into(),
        ],
    )
    .unwrap();
    let mut editor = CatalogEditorState::new(
        CatalogMutationMode::Create,
        plan.request.anchor.clone(),
        0,
        vec![],
    );
    editor.plan = Some(plan.clone());
    editor.page = CatalogEditorPage::SqlPreview;
    assert!(editor.begin_apply(18));
    app.catalog_editor = Some(editor);

    let commands = app.update(Action::CatalogMutationFailed {
        plan,
        message: "ClickHouse did not confirm whether the statement was applied".into(),
        outcome_unknown: true,
    });

    let editor = app.catalog_editor.as_ref().unwrap();
    assert!(editor.outcome_unknown);
    assert!(editor.error.as_deref().unwrap().contains("did not confirm"));
    assert_eq!(editor.operation, None);
    assert_eq!(editor.page, CatalogEditorPage::SqlPreview);
    assert!(commands.iter().any(|command| matches!(
        command,
        lazydb::action::Command::LoadCatalogPage(request) if request.key.target == refresh
    )));
}

fn draft_mut(app: &mut App) -> &mut lazydb::model::catalog_editor::TableDraft {
    let draft = app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap();
    let lazydb::model::catalog_editor::CatalogDraft::Table(draft) = draft else {
        panic!("table draft expected");
    };
    draft
}

#[test]
fn view_edit_dispatches_definition_load_and_accepts_matching_view_definition() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    let generation = match app.update(Action::RequestConnect(profile.id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
            current_user: Some("postgres".into()),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });
    let id = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::View,
        ["app", "public", "v", "42"],
    );
    let schema = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::Schema,
        ["app", "public"],
    );
    let entry = lazydb::db::catalog::CatalogEntry::relation(
        id.clone(),
        schema,
        lazydb::db::catalog::QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "v".into(),
        },
        "view",
        lazydb::db::catalog::OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog;
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::database(
                CatalogId::new(
                    profile.id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: None,
                    object: "app".into(),
                },
                "database",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::schema(
                CatalogId::new(
                    profile.id,
                    lazydb::db::catalog::CatalogKind::Schema,
                    ["app", "public"],
                ),
                CatalogId::new(
                    profile.id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: Some("public".into()),
                    object: "public".into(),
                },
                "schema",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog
        .insert(entry)
        .unwrap();
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id.clone()));
    let commands = app.update(Action::OpenCatalogEdit);
    assert!(
        matches!(commands.as_slice(), [lazydb::action::Command::LoadCatalogObjectDefinition(request)] if request.object == id)
    );
}
