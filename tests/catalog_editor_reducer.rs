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
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    }
    for expected in [
        lazydb::model::catalog_editor::TableActionField::AddColumn,
        lazydb::model::catalog_editor::TableActionField::RemoveColumn,
        lazydb::model::catalog_editor::TableActionField::Review,
        lazydb::model::catalog_editor::TableActionField::Cancel,
    ] {
        app.update(Action::CatalogEditorFieldNext);
        assert!(matches!(
            app.catalog_editor.as_ref().and_then(|editor| editor.draft.as_ref()),
            Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft))
                if draft.focus == lazydb::model::catalog_editor::TableEditorFocus::Action(expected)
        ));
    }
    for expected in [
        lazydb::model::catalog_editor::TableActionField::Review,
        lazydb::model::catalog_editor::TableActionField::RemoveColumn,
        lazydb::model::catalog_editor::TableActionField::AddColumn,
        lazydb::model::catalog_editor::TableActionField::AddColumn,
    ] {
        app.update(Action::CatalogEditorFieldPrevious);
        assert!(matches!(
            app.catalog_editor.as_ref().and_then(|editor| editor.draft.as_ref()),
            Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft))
                if draft.focus == lazydb::model::catalog_editor::TableEditorFocus::Action(expected)
                    || expected == lazydb::model::catalog_editor::TableActionField::AddColumn
                        && draft.focus == lazydb::model::catalog_editor::TableEditorFocus::Columns
        ));
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
        owner_picker: Default::default(),
    });
    app
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
        owner_picker: Default::default(),
    });

    app.update(Action::CatalogMutationFailed {
        plan,
        message: "old connection failed".into(),
    });

    let editor = app.catalog_editor.as_ref().unwrap();
    assert!(matches!(
        editor.operation,
        Some(lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 2 })
    ));
    assert_eq!(editor.error, None);
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
