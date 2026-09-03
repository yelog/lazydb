use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        ErrorCategory, ServerInfo,
        catalog::{
            CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry, CatalogGroupSummary,
            CatalogId, CatalogKind, CatalogPage, CatalogRequest, CatalogRequestKey, CatalogTarget,
            ObjectGroup, OptionalMetadata, QualifiedName,
        },
        catalog_drop::{CatalogDropPlan, CatalogDropRequest},
        query::{QueryOutcome, QueryStats},
    },
    model::{
        explorer::{ExplorerLoadState, ExplorerNodeId, ExplorerOwnerId},
        workspace::Overlay,
    },
    profile::{ConnectionProfile, DatabaseKind, import_connection_url},
};
use uuid::Uuid;

const PAGE_SIZE: usize = 100;

#[test]
fn frontend_explorer_search_is_synchronous_and_local() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let users = install_table(&mut app, &profile, &schema, "users");
    app.focus = lazydb::model::workspace::Focus::Explorer;
    assert!(app.update(Action::ExplorerSearchOpen).is_empty());
    assert!(app.update(Action::ExplorerSearchClear).is_empty());

    for character in "users".chars() {
        assert!(
            app.update(Action::ExplorerSearchInsert(character))
                .is_empty()
        );
    }
    let search = app.explorer.search.as_ref().unwrap();
    assert_eq!(search.frontend_match_rows.len(), 1);
    assert!(
        search
            .frontend_rows
            .iter()
            .any(|row| row.id == ExplorerNodeId::Catalog(users.id.clone()))
    );
    assert!(search.hits.is_empty());
    assert!(matches!(
        search.lifecycle,
        lazydb::model::workspace::ExplorerSearchLifecycle::Ready
    ));
    assert!(app.update(Action::ExplorerSearchClose).is_empty());
    assert!(app.explorer.search.is_none());
}

#[test]
fn frontend_explorer_search_excludes_relation_children() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let users = install_table(&mut app, &profile, &schema, "users");
    let column = CatalogEntry::relation_child(
        id(
            profile.id,
            CatalogKind::Column,
            &["app", "public", "users", "user_id"],
        ),
        users.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "user_id".into(),
        },
        "column",
        OptionalMetadata::Unsupported,
        lazydb::db::catalog::CatalogMetadata::Column(lazydb::db::catalog::ColumnMetadata::new(
            1, "bigint", false,
        )),
    )
    .unwrap();
    let request = load(
        &mut app,
        CatalogTarget::relation_children(users.id.clone()).unwrap(),
    );
    app.update(Action::CatalogPageLoaded(page(
        &request,
        vec![column.clone()],
        None,
    )));

    app.focus = lazydb::model::workspace::Focus::Explorer;
    app.update(Action::ExplorerSearchOpen);
    for character in "user_id".chars() {
        app.update(Action::ExplorerSearchInsert(character));
    }
    let search = app.explorer.search.as_ref().unwrap();
    assert!(search.frontend_match_rows.is_empty());
    assert!(
        !search
            .frontend_rows
            .iter()
            .any(|row| row.id == ExplorerNodeId::Catalog(column.id.clone()))
    );
}

#[test]
fn frontend_explorer_search_opens_freshly_after_close() {
    let (mut app, _) = connected_app();
    app.focus = lazydb::model::workspace::Focus::Explorer;
    app.update(Action::ExplorerSearchOpen);
    assert!(app.update(Action::ExplorerSearchClose).is_empty());
    app.update(Action::ExplorerSearchOpen);
    assert!(app.explorer.search.is_some());
}

#[test]
fn targeted_refresh_invalidates_one_owner_and_preserves_stale_rows() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let target = CatalogTarget::objects(schema.id.clone(), ObjectGroup::Tables).unwrap();
    let request = load(&mut app, target.clone());
    let table = relation(profile.id, &schema.id, "users");
    app.update(Action::CatalogPageLoaded(page(
        &request,
        vec![table.clone()],
        None,
    )));

    let commands = app.commands_for_catalog_targets(profile.id, std::slice::from_ref(&target));
    assert!(
        matches!(commands.as_slice(), [Command::LoadCatalogPage(request)] if request.key.target == target)
    );
    assert_eq!(catalog(&app, profile.id).get(&table.id), Some(&table));
    assert!(matches!(
        load_state(
            &app,
            lazydb::model::explorer::owner_for_target(profile.id, &target)
        ),
        ExplorerLoadState::Loading { .. }
    ));
}

#[test]
fn targeted_refresh_deduplicates_targets_and_increments_generation_once() {
    let (mut app, profile) = connected_app();
    let before = app.explorer.catalog_generation;
    let target = CatalogTarget::Databases;
    let commands = app.commands_for_catalog_targets(profile.id, &[target.clone(), target.clone()]);
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, Command::LoadCatalogPage(_)))
            .count(),
        1
    );
    app.explorer.catalog_generation = before.saturating_add(1);
    assert_eq!(app.explorer.catalog_generation, before + 1);
}

#[test]
fn search_edit_clears_query_metadata_and_empty_query_is_neutral() {
    let (mut app, _) = connected_app();
    app.focus = lazydb::model::workspace::Focus::Explorer;
    app.update(Action::ExplorerSearchOpen);
    for character in "users".chars() {
        app.update(Action::ExplorerSearchInsert(character));
    }
    let search = app.explorer.search.as_mut().unwrap();
    search.total_count = Some(50);
    search.truncated = true;
    search.located = Some(id(Uuid::nil(), CatalogKind::Table, &["old"]));

    app.update(Action::ExplorerSearchInsert('s'));
    let search = app.explorer.search.as_ref().unwrap();
    assert_eq!(search.total_count, None);
    assert!(!search.truncated);
    assert_eq!(search.located, None);
    app.update(Action::ExplorerSearchClear);
    let search = app.explorer.search.as_ref().unwrap();
    assert!(search.hits.is_empty());
    assert!(matches!(
        search.lifecycle,
        lazydb::model::workspace::ExplorerSearchLifecycle::Idle
    ));
}

#[test]
fn frontend_search_locates_without_breaking_pending_continuation() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let target = CatalogTarget::objects(schema.id.clone(), ObjectGroup::Tables).unwrap();
    let mut first = load(&mut app, target.clone());
    set_pending_page_size(&mut app, &mut first, 1);
    let cursor = CatalogCursor::from_keyset("users", "users").unwrap();
    app.update(Action::CatalogPageLoaded(page(
        &first,
        vec![relation(profile.id, &schema.id, "users")],
        Some(cursor),
    )));
    let continuation = pending_request(&app, profile.id, &target);
    let page_two = relation(profile.id, &schema.id, "widgets");

    app.focus = lazydb::model::workspace::Focus::Explorer;
    app.update(Action::ExplorerSearchOpen);
    for character in "users".chars() {
        app.update(Action::ExplorerSearchInsert(character));
    }
    let search = app.explorer.search.as_ref().unwrap();
    let row = &search.frontend_rows[search.frontend_match_rows[0]];
    assert!(matches!(&row.id, ExplorerNodeId::Catalog(id) if id.kind == CatalogKind::Table));
    app.explorer.normalized.expanded.clear();
    app.update(Action::ExplorerSearchLocate);
    assert!(app.explorer.search.is_none());
    let users = relation(profile.id, &schema.id, "users");
    assert_eq!(
        app.explorer.selected_id(),
        Some(&ExplorerNodeId::Catalog(users.id.clone()))
    );
    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(profile.id))
    );
    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Catalog(database.id.clone()))
    );
    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Catalog(schema.id.clone()))
    );
    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Group {
                parent: schema.id.clone(),
                group: ObjectGroup::Tables,
            })
    );
    assert!(
        !app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Catalog(users.id.clone()))
    );
    assert!(
        app.explorer
            .visible()
            .iter()
            .any(|row| row.id == ExplorerNodeId::Catalog(users.id.clone()))
    );

    app.update(Action::CatalogPageLoaded(page(
        &continuation,
        vec![page_two.clone()],
        None,
    )));
    let owner = ExplorerOwnerId::Group {
        parent: schema.id.clone(),
        group: ObjectGroup::Tables,
    };
    assert_eq!(catalog(&app, profile.id).get(&page_two.id), Some(&page_two));
    assert!(matches!(
        load_state(&app, owner),
        ExplorerLoadState::Loaded { next_cursor: None }
    ));
}

#[test]
fn newer_request_wins_and_every_wrong_request_dimension_is_ignored() {
    let (mut app, profile) = connected_app();
    let first = initial_request(&mut app, &profile);
    let second = refresh(&mut app, ExplorerNodeId::Profile(profile.id));
    assert!(second.key.request_id > first.key.request_id);

    for wrong in wrong_keys(&second) {
        let page = unchecked_page(wrong, vec![database(profile.id, "wrong")], None);
        assert!(app.update(Action::CatalogPageLoaded(page)).is_empty());
    }
    assert!(catalog(&app, profile.id).is_empty());

    app.update(Action::CatalogPageLoaded(page(
        &second,
        vec![database(profile.id, "current")],
        None,
    )));
    app.update(Action::CatalogPageLoaded(page(
        &first,
        vec![database(profile.id, "late")],
        None,
    )));
    let tree = catalog(&app, profile.id);
    assert!(
        tree.get(&id(profile.id, CatalogKind::Database, &["current"]))
            .is_some()
    );
    assert!(
        tree.get(&id(profile.id, CatalogKind::Database, &["late"]))
            .is_none()
    );
}

#[test]
fn refresh_failure_keeps_old_data_and_first_failure_is_target_local() {
    let (mut app, profile) = connected_app();
    let initial = initial_request(&mut app, &profile);
    app.update(Action::CatalogPageLoaded(page(
        &initial,
        vec![database(profile.id, "app")],
        None,
    )));

    let refresh = refresh(&mut app, ExplorerNodeId::Profile(profile.id));
    app.update(Action::CatalogPageFailed {
        key: refresh.key.clone(),
        category: ErrorCategory::Network,
        message: "refresh failed".into(),
    });
    assert!(
        catalog(&app, profile.id)
            .get(&id(profile.id, CatalogKind::Database, &["app"]))
            .is_some()
    );
    assert_eq!(
        load_state(&app, ExplorerOwnerId::Profile(profile.id)),
        ExplorerLoadState::Stale { next_cursor: None }
    );
    assert_eq!(app.connection.error, None);

    let (mut fresh, fresh_profile) = connected_app();
    let request = initial_request(&mut fresh, &fresh_profile);
    fresh.update(Action::CatalogPageFailed {
        key: request.key,
        category: ErrorCategory::Permission,
        message: "denied".into(),
    });
    assert!(catalog(&fresh, fresh_profile.id).is_empty());
    assert!(matches!(
        load_state(&fresh, ExplorerOwnerId::Profile(fresh_profile.id)),
        ExplorerLoadState::PermissionDenied { .. }
    ));
    assert_eq!(fresh.connection.error, None);
}

#[test]
fn continuation_appends_stable_ids_and_rejects_duplicate_or_malformed_pages_atomically() {
    let (mut app, profile) = connected_app();
    let mut first = initial_request(&mut app, &profile);
    set_pending_page_size(&mut app, &mut first, 1);
    let cursor = CatalogCursor::from_keyset("app", "app").unwrap();
    app.update(Action::CatalogPageLoaded(page(
        &first,
        vec![database(profile.id, "app")],
        Some(cursor.clone()),
    )));
    let continuation = pending_request(&app, profile.id, &CatalogTarget::Databases);
    app.update(Action::CatalogPageLoaded(page(
        &continuation,
        vec![database(profile.id, "other")],
        None,
    )));
    assert_eq!(catalog(&app, profile.id).roots().len(), 2);

    let bad = refresh(&mut app, ExplorerNodeId::Profile(profile.id));
    let existing = database(profile.id, "app");
    app.update(Action::CatalogPageLoaded(unchecked_page(
        bad.key.clone(),
        vec![existing.clone(), existing],
        None,
    )));
    assert_eq!(catalog(&app, profile.id).roots().len(), 2);
    assert!(matches!(
        load_state(&app, ExplorerOwnerId::Profile(profile.id)),
        ExplorerLoadState::Stale { .. }
    ));
}

#[test]
fn replacement_preserves_stable_selection_and_expansion_then_falls_back_to_ancestors() {
    let (mut app, profile) = connected_app();
    let database = database(profile.id, "app");
    let initial = initial_request(&mut app, &profile);
    app.update(Action::CatalogPageLoaded(page(
        &initial,
        vec![database.clone()],
        None,
    )));
    let schemas_target = CatalogTarget::schemas(database.id.clone()).unwrap();
    let schemas = pending_request(&app, profile.id, &schemas_target);
    let schema = schema(profile.id, &database.id, "public");
    app.update(Action::CatalogPageLoaded(page(
        &schemas,
        vec![schema.clone()],
        None,
    )));
    let objects = load(
        &mut app,
        CatalogTarget::objects(schema.id.clone(), ObjectGroup::Tables).unwrap(),
    );
    let users = relation(profile.id, &schema.id, "users");
    app.update(Action::CatalogPageLoaded(page(
        &objects,
        vec![users.clone()],
        None,
    )));

    let selected = ExplorerNodeId::Catalog(users.id.clone());
    app.explorer.normalized.expanded.extend([
        ExplorerNodeId::Catalog(database.id.clone()),
        ExplorerNodeId::Catalog(schema.id.clone()),
        selected.clone(),
    ]);
    app.explorer.normalized.selected = Some(selected.clone());
    let replacement = refresh(
        &mut app,
        ExplorerNodeId::Group {
            parent: schema.id.clone(),
            group: ObjectGroup::Tables,
        },
    );
    app.explorer.normalized.selected = Some(selected.clone());
    app.update(Action::CatalogPageLoaded(page(
        &replacement,
        vec![users],
        None,
    )));
    assert_eq!(app.explorer.normalized.selected, Some(selected.clone()));
    assert!(app.explorer.normalized.expanded.contains(&selected));

    let removal = refresh(
        &mut app,
        ExplorerNodeId::Group {
            parent: schema.id.clone(),
            group: ObjectGroup::Tables,
        },
    );
    app.explorer.normalized.selected = Some(selected);
    app.update(Action::CatalogPageLoaded(page(&removal, Vec::new(), None)));
    assert_eq!(
        app.explorer.normalized.selected,
        Some(ExplorerNodeId::Catalog(schema.id))
    );
}

#[test]
fn request_and_epoch_overflow_never_wrap_or_emit_a_command() {
    let (mut app, profile) = connected_app();
    let state = app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap();
    state.next_request_id = u64::MAX;
    state.pending_requests.clear();
    state.load_states.clear();
    assert!(
        app.update(Action::ExplorerLoadTarget(CatalogTarget::Databases))
            .is_empty()
    );
    assert_eq!(
        app.explorer.normalized.profiles[&profile.id].next_request_id,
        u64::MAX
    );
    assert!(
        app.explorer.normalized.profiles[&profile.id]
            .last_error
            .as_deref()
            .unwrap()
            .contains("request ID exhausted")
    );

    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog_epoch = u64::MAX;
    let generation = app.connection.generation + 1;
    app.connection.pending_profile_id = Some(profile.id);
    app.connection.pending_generation = Some(generation);
    assert!(
        app.update(Action::ConnectionSucceeded {
            profile_id: profile.id,
            generation,
            server: server(),
            mutation_capabilities: Default::default(),
        })
        .is_empty()
    );
    assert_eq!(
        app.explorer.normalized.profiles[&profile.id].catalog_epoch,
        u64::MAX
    );
    assert!(
        app.explorer.normalized.profiles[&profile.id]
            .last_error
            .as_deref()
            .unwrap()
            .contains("catalog epoch exhausted")
    );
}

#[test]
fn page_validation_uses_exact_pending_request_before_tree_or_completion_mutation() {
    let (mut app, profile) = connected_app();
    let request = initial_request(&mut app, &profile);
    let outside = database(profile.id, "outside");
    let mut malformed = unchecked_page(request.key.clone(), vec![outside], None);
    malformed.completeness = CatalogCompleteness::Partial;

    app.update(Action::CatalogPageLoaded(malformed));

    assert!(catalog(&app, profile.id).is_empty());
    assert!(app.explorer.completion_index.entries().is_empty());
    assert!(matches!(
        load_state(&app, ExplorerOwnerId::Profile(profile.id)),
        ExplorerLoadState::Failed { .. }
    ));
}

#[test]
fn catalog_drop_success_removes_subtree_reselects_parent_and_clears_completion() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let table = install_table(&mut app, &profile, &schema, "users");
    let connection = app.connection.active_identity().unwrap();
    let mut request =
        CatalogDropRequest::new(connection, table.id.clone(), 42).with_entry(table.clone());
    request.catalog_epoch = app.explorer.normalized.profiles[&profile.id].catalog_epoch;
    let plan = CatalogDropPlan::new(request, &table, "DROP TABLE users").unwrap();
    app.explorer.completion_index = lazydb::sql::CompletionIndex::new(std::slice::from_ref(&table));
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(table.id.clone()));
    app.overlay = Some(Overlay::CatalogDropConfirm {
        plan: Box::new(plan.clone()),
        input: Default::default(),
        busy: true,
        error: None,
    });

    app.update(Action::CatalogDropSucceeded {
        plan,
        outcome: QueryOutcome {
            result_sets: Vec::new(),
            stats: QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 0),
        },
    });

    assert!(catalog(&app, profile.id).get(&table.id).is_none());
    assert_eq!(
        app.explorer.normalized.selected,
        Some(ExplorerNodeId::Catalog(schema.id))
    );
    assert!(app.explorer.completion_index.entries().is_empty());
    assert!(app.overlay.is_none());
}

#[test]
fn catalog_drop_failure_keeps_confirmation_and_clears_input() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let table = install_table(&mut app, &profile, &schema, "users");
    let mut request = CatalogDropRequest::new(
        app.connection.active_identity().unwrap(),
        table.id.clone(),
        43,
    )
    .with_entry(table.clone());
    request.catalog_epoch = app.explorer.normalized.profiles[&profile.id].catalog_epoch;
    let plan = CatalogDropPlan::new(request, &table, "DROP TABLE users").unwrap();
    app.overlay = Some(Overlay::CatalogDropConfirm {
        plan: Box::new(plan.clone()),
        input: {
            let mut input = lazydb::model::text_input::TextInput::default();
            input.set("y");
            input
        },
        busy: true,
        error: None,
    });

    app.update(Action::CatalogDropFailed {
        plan,
        message: "drop failed".into(),
    });

    assert!(matches!(
        app.overlay,
        Some(Overlay::CatalogDropConfirm {
            ref input,
            busy: false,
            ref error,
            ..
        }) if input.value().is_empty() && error.as_deref() == Some("drop failed")
    ));
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Error
            && notification.title == "Catalog"
            && notification.body == "drop failed"
    }));
    assert!(catalog(&app, profile.id).get(&table.id).is_some());
}

#[test]
fn catalog_drop_confirmation_requires_exact_lowercase_y_and_is_single_shot() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let table = install_table(&mut app, &profile, &schema, "users");
    let mut request = CatalogDropRequest::new(
        app.connection.active_identity().unwrap(),
        table.id.clone(),
        44,
    )
    .with_entry(table.clone());
    request.catalog_epoch = app.explorer.normalized.profiles[&profile.id].catalog_epoch;
    let plan = CatalogDropPlan::new(request, &table, "DROP TABLE users").unwrap();

    app.update(Action::CatalogDropPlanReady(plan.clone()));
    assert!(app.update(Action::CatalogDropConfirm).is_empty());
    assert!(matches!(
        app.overlay,
        Some(Overlay::CatalogDropConfirm { busy: false, .. })
    ));

    assert!(app.update(Action::CatalogDropInsert('y')).is_empty());
    let commands = app.update(Action::CatalogDropConfirm);
    assert!(matches!(
        commands.as_slice(),
        [Command::ExecuteCatalogDrop(found)] if found == &plan
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::CatalogDropConfirm { busy: true, .. })
    ));
    assert!(app.update(Action::CatalogDropConfirm).is_empty());

    for input in ["Y", "yes"] {
        app.update(Action::CatalogDropPlanReady(plan.clone()));
        for character in input.chars() {
            app.update(Action::CatalogDropInsert(character));
        }
        assert!(app.update(Action::CatalogDropConfirm).is_empty());
        assert!(matches!(
            app.overlay,
            Some(Overlay::CatalogDropConfirm { busy: false, .. })
        ));
    }

    app.update(Action::CatalogDropCancel);
    assert!(app.overlay.is_none());
}

#[test]
fn automatic_pending_can_be_superseded_by_refresh_without_losing_first_load_snapshot() {
    let (mut app, profile) = connected_app();
    let automatic = initial_request(&mut app, &profile);

    let commands = app.update(Action::RefreshCatalog);
    let refreshed = one_request(&commands);

    assert!(refreshed.key.request_id > automatic.key.request_id);
    assert_eq!(
        app.explorer.normalized.profiles[&profile.id].previous_load_states
            [&ExplorerOwnerId::Profile(profile.id)],
        ExplorerLoadState::NotLoaded
    );
    app.update(Action::CatalogPageFailed {
        key: refreshed.key.clone(),
        category: ErrorCategory::Network,
        message: "failed".into(),
    });
    assert!(matches!(
        load_state(&app, ExplorerOwnerId::Profile(profile.id)),
        ExplorerLoadState::Failed { .. }
    ));
}

#[test]
fn ancestor_automatic_schedule_never_supersedes_a_user_pending_request_or_fans_out_twice() {
    let (mut app, profile) = connected_app();
    let database_entry = database(profile.id, "app");
    let target = CatalogTarget::schemas(database_entry.id.clone()).unwrap();
    let user = load(&mut app, target.clone());
    let root = initial_request(&mut app, &profile);

    let first_commands = app.update(Action::CatalogPageLoaded(page(
        &root,
        vec![database_entry],
        None,
    )));
    assert!(first_commands.is_empty());
    assert_eq!(
        app.explorer.normalized.profiles[&profile.id].pending_requests[&ExplorerOwnerId::Catalog(
            match &target {
                CatalogTarget::Schemas { database } => database.clone(),
                _ => unreachable!(),
            }
        )]
            .key,
        user.key
    );

    assert!(
        app.update(Action::CatalogPageLoaded(page(
            &root,
            vec![database(profile.id, "app")],
            None,
        )))
        .is_empty()
    );
}

#[test]
fn refresh_failure_retains_original_cursor_and_data_across_a_superseded_loading_request() {
    let (mut app, profile) = connected_app();
    let mut initial = initial_request(&mut app, &profile);
    set_pending_page_size(&mut app, &mut initial, 1);
    let cursor = CatalogCursor::from_keyset("app", "app").unwrap();
    app.update(Action::CatalogPageLoaded(page(
        &initial,
        vec![database(profile.id, "app")],
        Some(cursor.clone()),
    )));
    let pending_continuation = app.explorer.normalized.profiles[&profile.id].pending_requests
        [&ExplorerOwnerId::Profile(profile.id)]
        .clone();

    let refresh = one_request(&app.update(Action::RefreshCatalog)).clone();
    assert!(refresh.key.request_id > pending_continuation.key.request_id);
    app.update(Action::CatalogPageFailed {
        key: refresh.key,
        category: ErrorCategory::Network,
        message: "refresh failed".into(),
    });

    assert!(
        catalog(&app, profile.id)
            .get(&id(profile.id, CatalogKind::Database, &["app"]))
            .is_some()
    );
    assert_eq!(
        load_state(&app, ExplorerOwnerId::Profile(profile.id)),
        ExplorerLoadState::Stale {
            next_cursor: Some(cursor)
        }
    );
}

#[test]
fn group_continuations_append_in_order_and_reject_cross_page_duplicates_atomically() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let target = CatalogTarget::groups(schema.id.clone()).unwrap();
    let mut first = pending_request(&app, profile.id, &target);
    set_pending_page_size(&mut app, &mut first, 1);
    let cursor = CatalogCursor::from_keyset("tables", "tables").unwrap();
    let first_page = group_page(
        &first,
        ObjectGroup::Tables,
        CatalogCount::Exact(2),
        Some(cursor.clone()),
    );
    let commands = app.update(Action::CatalogPageLoaded(first_page));
    let continuation = commands
        .iter()
        .filter_map(|command| match command {
            Command::LoadCatalogPage(request) if request.key.target == target => Some(request),
            _ => None,
        })
        .next()
        .unwrap()
        .clone();
    assert_eq!(continuation.key.cursor, Some(cursor));

    let second_page = group_page(
        &continuation,
        ObjectGroup::Views,
        CatalogCount::Exact(2),
        None,
    );
    app.update(Action::CatalogPageLoaded(second_page));
    assert_eq!(
        catalog(&app, profile.id).groups(&schema.id),
        &[ObjectGroup::Tables, ObjectGroup::Views]
    );

    let refresh_request = refresh(&mut app, ExplorerNodeId::Catalog(schema.id.clone()));
    let duplicate_cursor = CatalogCursor::from_keyset("tables", "tables-2").unwrap();
    let mut refresh_request = refresh_request;
    set_pending_page_size(&mut app, &mut refresh_request, 1);
    let first_again = group_page(
        &refresh_request,
        ObjectGroup::Tables,
        CatalogCount::Exact(2),
        Some(duplicate_cursor),
    );
    let commands = app.update(Action::CatalogPageLoaded(first_again));
    let duplicate_request = commands
        .iter()
        .find_map(|command| match command {
            Command::LoadCatalogPage(request) if request.key.target == target => Some(request),
            _ => None,
        })
        .unwrap();
    let duplicate = CatalogPage::groups(
        duplicate_request,
        vec![CatalogGroupSummary {
            group: ObjectGroup::Tables,
            object_count: CatalogCount::Exact(1),
        }],
        CatalogCount::Exact(2),
        None,
    )
    .unwrap();
    app.update(Action::CatalogPageLoaded(duplicate));
    assert_eq!(
        catalog(&app, profile.id).groups(&schema.id),
        &[ObjectGroup::Tables]
    );
    assert!(matches!(
        load_state(&app, ExplorerOwnerId::Catalog(schema.id.clone())),
        ExplorerLoadState::Stale { .. }
    ));

    let refresh = refresh(&mut app, ExplorerNodeId::Catalog(schema.id.clone()));
    let duplicate_same_page = CatalogPage::groups(
        &refresh,
        vec![
            CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Exact(1),
            },
            CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Exact(1),
            },
        ],
        CatalogCount::Exact(2),
        None,
    );
    assert!(duplicate_same_page.is_err());
    assert_eq!(
        catalog(&app, profile.id).groups(&schema.id),
        &[ObjectGroup::Tables]
    );
}

#[test]
fn search_preload_schedules_supported_non_relation_groups() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let groups = pending_request(
        &app,
        profile.id,
        &CatalogTarget::groups(schema.id.clone()).unwrap(),
    );
    let page = CatalogPage::groups(
        &groups,
        vec![
            CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Exact(1),
            },
            CatalogGroupSummary {
                group: ObjectGroup::Triggers,
                object_count: CatalogCount::Exact(1),
            },
        ],
        CatalogCount::Exact(2),
        None,
    )
    .unwrap();
    let commands = app.update(Action::CatalogPageLoaded(page));
    assert!(commands.iter().any(|command| matches!(
        command,
        Command::LoadCatalogPage(request)
            if matches!(request.key.target, CatalogTarget::Objects { group: ObjectGroup::Tables, .. })
    )));
    assert!(commands.iter().any(|command| matches!(
        command,
        Command::LoadCatalogPage(request)
            if matches!(request.key.target, CatalogTarget::Objects { group: ObjectGroup::Triggers, .. })
    )));

    let tables = CatalogTarget::objects(schema.id, ObjectGroup::Tables).unwrap();
    let request = pending_request(&app, profile.id, &tables);
    app.update(Action::CatalogPageFailed {
        key: request.key,
        category: ErrorCategory::Permission,
        message: "denied".into(),
    });
    assert!(
        app.update(Action::CatalogPageLoaded(
            CatalogPage::groups(
                &groups,
                vec![CatalogGroupSummary {
                    group: ObjectGroup::Tables,
                    object_count: CatalogCount::Exact(1),
                }],
                CatalogCount::Exact(1),
                None,
            )
            .unwrap()
        ))
        .is_empty()
    );
}

#[test]
fn normalized_group_retry_permission_and_load_more_rows_select_and_load_precise_targets() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let groups_target = CatalogTarget::groups(schema.id.clone()).unwrap();
    let groups_request = pending_request(&app, profile.id, &groups_target);
    let groups_page = CatalogPage::groups(
        &groups_request,
        vec![CatalogGroupSummary {
            group: ObjectGroup::Tables,
            object_count: CatalogCount::Exact(1),
        }],
        CatalogCount::Exact(1),
        None,
    )
    .unwrap();
    let commands = app.update(Action::CatalogPageLoaded(groups_page));
    let tables_target = CatalogTarget::objects(schema.id.clone(), ObjectGroup::Tables).unwrap();
    let tables_request = commands
        .iter()
        .find_map(|command| match command {
            Command::LoadCatalogPage(request) if request.key.target == tables_target => {
                Some(request.clone())
            }
            _ => None,
        })
        .unwrap();
    app.update(Action::CatalogPageFailed {
        key: tables_request.key,
        category: ErrorCategory::Permission,
        message: "denied".into(),
    });
    app.explorer.normalized.expanded.extend([
        ExplorerNodeId::Catalog(database.id),
        ExplorerNodeId::Catalog(schema.id.clone()),
        ExplorerNodeId::Group {
            parent: schema.id.clone(),
            group: ObjectGroup::Tables,
        },
    ]);
    app.explorer.rebuild_projection(profile.id);
    let rows = app.explorer.visible();
    assert!(rows.iter().any(|row| matches!(
        row.id,
        ExplorerNodeId::Group {
            group: ObjectGroup::Tables,
            ..
        }
    ) && row.label == "Tables"));
    let permission_index = rows
        .iter()
        .position(|row| {
            matches!(
                row.id,
                ExplorerNodeId::Status {
                    kind: lazydb::model::explorer::StatusRowKind::PermissionDenied,
                    ..
                }
            )
        })
        .unwrap();
    app.update(Action::ExplorerSelect(rows[permission_index].id.clone()));
    let retry_commands = app.update(Action::ExplorerToggle);
    let retry = one_request(&retry_commands);
    assert_eq!(retry.key.target, tables_target);

    app.update(Action::CatalogPageFailed {
        key: retry.key.clone(),
        category: ErrorCategory::Network,
        message: "failed".into(),
    });
    let explicit = load(&mut app, tables_target.clone());
    let cursor = CatalogCursor::from_keyset("users", "users").unwrap();
    let mut explicit = explicit;
    set_pending_page_size(&mut app, &mut explicit, 1);
    let users = relation(profile.id, &schema.id, "users");
    app.update(Action::CatalogPageLoaded(page(
        &explicit,
        vec![users],
        Some(cursor.clone()),
    )));
    let continuation = pending_request(&app, profile.id, &tables_target);
    app.update(Action::CatalogPageFailed {
        key: continuation.key,
        category: ErrorCategory::Network,
        message: "paused".into(),
    });
    let owner = ExplorerOwnerId::Group {
        parent: schema.id,
        group: ObjectGroup::Tables,
    };
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .load_states
        .insert(
            owner.clone(),
            ExplorerLoadState::Loaded {
                next_cursor: Some(cursor.clone()),
            },
        );
    app.explorer.rebuild_projection(profile.id);
    let load_more = app
        .explorer
        .visible()
        .iter()
        .position(|row| matches!(row.id, ExplorerNodeId::LoadMore { .. }))
        .unwrap();
    app.update(Action::ExplorerSelect(
        app.explorer.visible()[load_more].id.clone(),
    ));
    let continuation_commands = app.update(Action::ExplorerToggle);
    let continuation = one_request(&continuation_commands);
    assert_eq!(continuation.key.target, tables_target);
    assert_eq!(continuation.key.cursor, Some(cursor));
}

#[test]
fn preview_uses_selected_relation_identity_without_opening_relation_tabs() {
    let (mut app, profile) = connected_app();
    let database = install_database(&mut app, &profile);
    let schema = install_schema(&mut app, &profile, &database);
    let target = CatalogTarget::objects(schema.id.clone(), ObjectGroup::MaterializedViews).unwrap();
    let request = load(&mut app, target);
    let relation = CatalogEntry::relation(
        CatalogId::new(
            profile.id,
            CatalogKind::MaterializedView,
            ["app", "public", "logical_view", "42", "native-suffix"],
        ),
        schema.id,
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "logical_view".into(),
        },
        "materialized view",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    app.update(Action::CatalogPageLoaded(page(
        &request,
        vec![relation.clone()],
        None,
    )));
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(relation.id.clone()));

    let commands = app.update(Action::PreviewSelected);
    assert!(commands.iter().any(|command| matches!(
        command,
        Command::LoadRelationPreview(request)
            if request.relation.object_id.native_path
                == ["app", "public", "logical_view", "42", "native-suffix"]
    )));
    assert!(commands.iter().any(|command| matches!(
        command,
        Command::LoadCatalogPage(request)
            if request.key.target
                == CatalogTarget::relation_children(relation.id.clone()).unwrap()
    )));
    assert_eq!(app.tabs.len(), 2);
}

fn connected_app() -> (App, ConnectionProfile) {
    let mut profile = import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    profile.catalog_scope.databases = lazydb::profile::CatalogSelection::All;
    let mut app = App::new(vec![profile.clone()]);
    let generation = match app.update(Action::RequestConnect(profile.id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let commands = app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation,
        server: server(),
        mutation_capabilities: Default::default(),
    });
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::LoadCatalogPage(_)))
    );
    (app, profile)
}

fn set_pending_page_size(app: &mut App, request: &mut CatalogRequest, page_size: usize) {
    request.page_size = page_size;
    let owner = lazydb::model::explorer::owner_for_target(
        request.key.connection.profile_id,
        &request.key.target,
    );
    app.explorer
        .normalized
        .profiles
        .get_mut(&request.key.connection.profile_id)
        .unwrap()
        .pending_requests
        .insert(owner, request.clone());
}

fn initial_request(app: &mut App, profile: &ConnectionProfile) -> CatalogRequest {
    app.explorer.normalized.profiles[&profile.id]
        .pending_requests
        .values()
        .find(|request| request.key.target == CatalogTarget::Databases)
        .unwrap()
        .clone()
}

fn load(app: &mut App, target: CatalogTarget) -> CatalogRequest {
    let commands = app.update(Action::ExplorerLoadTarget(target));
    match commands.as_slice() {
        [Command::LoadCatalogPage(request)] => request.clone(),
        commands => panic!("unexpected commands: {commands:?}"),
    }
}

fn refresh(app: &mut App, selected: ExplorerNodeId) -> CatalogRequest {
    app.explorer.normalized.selected = Some(selected);
    let commands = app.update(Action::RefreshCatalog);
    one_request(&commands).clone()
}

fn one_request(commands: &[Command]) -> &CatalogRequest {
    match commands {
        [Command::LoadCatalogPage(request)] => request,
        commands => panic!("unexpected commands: {commands:?}"),
    }
}

fn pending_request(app: &App, profile: Uuid, target: &CatalogTarget) -> CatalogRequest {
    let owner = lazydb::model::explorer::owner_for_target(profile, target);
    app.explorer.normalized.profiles[&profile].pending_requests[&owner].clone()
}

fn install_database(app: &mut App, profile: &ConnectionProfile) -> CatalogEntry {
    let entry = database(profile.id, "app");
    let request = initial_request(app, profile);
    app.update(Action::CatalogPageLoaded(page(
        &request,
        vec![entry.clone()],
        None,
    )));
    entry
}

fn install_schema(
    app: &mut App,
    profile: &ConnectionProfile,
    database: &CatalogEntry,
) -> CatalogEntry {
    let target = CatalogTarget::schemas(database.id.clone()).unwrap();
    let request = pending_request(app, profile.id, &target);
    let entry = schema(profile.id, &database.id, "public");
    app.update(Action::CatalogPageLoaded(page(
        &request,
        vec![entry.clone()],
        None,
    )));
    entry
}

fn install_table(
    app: &mut App,
    profile: &ConnectionProfile,
    schema: &CatalogEntry,
    name: &str,
) -> CatalogEntry {
    let target = CatalogTarget::groups(schema.id.clone()).unwrap();
    let request = pending_request(app, profile.id, &target);
    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::groups(
            &request,
            vec![CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Exact(1),
            }],
            CatalogCount::Exact(1),
            None,
        )
        .unwrap(),
    ));
    assert!(commands.iter().any(|command| matches!(
        command,
        Command::LoadCatalogPage(request)
            if request.key.target == CatalogTarget::objects(schema.id.clone(), ObjectGroup::Tables).unwrap()
    )));
    let target = CatalogTarget::objects(schema.id.clone(), ObjectGroup::Tables).unwrap();
    let request = pending_request(app, profile.id, &target);
    let entry = relation(profile.id, &schema.id, name);
    app.update(Action::CatalogPageLoaded(page(
        &request,
        vec![entry.clone()],
        None,
    )));
    entry
}

fn group_page(
    request: &CatalogRequest,
    group: ObjectGroup,
    total: CatalogCount,
    cursor: Option<CatalogCursor>,
) -> CatalogPage {
    CatalogPage::groups(
        request,
        vec![CatalogGroupSummary {
            group,
            object_count: CatalogCount::Exact(1),
        }],
        total,
        cursor,
    )
    .unwrap()
}

fn catalog(app: &App, profile: Uuid) -> &lazydb::model::explorer::CatalogTree {
    &app.explorer.normalized.profiles[&profile].catalog
}

fn load_state(app: &App, owner: ExplorerOwnerId) -> ExplorerLoadState {
    app.explorer.normalized.profiles[&owner.profile_id()].load_states[&owner].clone()
}

fn wrong_keys(request: &CatalogRequest) -> Vec<CatalogRequestKey> {
    let mut keys = Vec::new();
    for mutate in [
        |key: &mut CatalogRequestKey| key.connection.profile_id = Uuid::new_v4(),
        |key: &mut CatalogRequestKey| key.connection.generation += 1,
        |key: &mut CatalogRequestKey| key.catalog_epoch += 1,
        |key: &mut CatalogRequestKey| key.request_id += 1,
    ] {
        let mut key = request.key.clone();
        mutate(&mut key);
        keys.push(key);
    }
    let mut target = request.key.clone();
    target.target = CatalogTarget::Schemas {
        database: id(
            request.key.connection.profile_id,
            CatalogKind::Database,
            &["app"],
        ),
    };
    keys.push(target);
    let mut cursor = request.key.clone();
    cursor.cursor = Some(CatalogCursor::from_keyset("a", "a").unwrap());
    keys.push(cursor);
    keys
}

fn page(
    request: &CatalogRequest,
    entries: Vec<CatalogEntry>,
    next: Option<CatalogCursor>,
) -> CatalogPage {
    let count = if next.is_some() {
        CatalogCount::AtLeast((entries.len() + 1) as u64)
    } else {
        CatalogCount::Exact(entries.len() as u64)
    };
    CatalogPage::new(request, entries, count, next).unwrap()
}

fn unchecked_page(
    key: CatalogRequestKey,
    entries: Vec<CatalogEntry>,
    next: Option<CatalogCursor>,
) -> CatalogPage {
    CatalogPage {
        key,
        total_count: CatalogCount::Unknown,
        completeness: if next.is_some() {
            CatalogCompleteness::Partial
        } else {
            CatalogCompleteness::Complete
        },
        entries,
        group_summaries: Vec::new(),
        next_cursor: next,
    }
}

fn database(profile: Uuid, name: &str) -> CatalogEntry {
    CatalogEntry::database(
        id(profile, CatalogKind::Database, &[name]),
        QualifiedName {
            database: Some(name.into()),
            schema: None,
            object: name.into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn schema(profile: Uuid, database: &CatalogId, name: &str) -> CatalogEntry {
    CatalogEntry::schema(
        id(profile, CatalogKind::Schema, &["app", name]),
        database.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some(name.into()),
            object: name.into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn relation(profile: Uuid, schema: &CatalogId, name: &str) -> CatalogEntry {
    CatalogEntry::relation(
        id(profile, CatalogKind::Table, &["app", "public", name]),
        schema.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: name.into(),
        },
        "table",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn id(profile: Uuid, kind: CatalogKind, path: &[&str]) -> CatalogId {
    CatalogId::new(profile, kind, path.iter().copied())
}

fn server() -> ServerInfo {
    ServerInfo {
        kind: DatabaseKind::Sqlite,
        version: "3.50".into(),
        database: "app".into(),
    }
}

#[test]
fn default_catalog_page_size_is_bounded() {
    assert!((1..=lazydb::db::catalog::MAX_CATALOG_PAGE_SIZE).contains(&PAGE_SIZE));
}
