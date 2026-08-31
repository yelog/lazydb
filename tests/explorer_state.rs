use lazydb::{
    db::catalog::{
        CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry, CatalogId, CatalogKind,
        CatalogMetadata, CatalogSearchHit, ColumnMetadata, ObjectGroup, OptionalMetadata,
        QualifiedName,
    },
    model::explorer::{
        CatalogGroupState, CatalogTree, CatalogTreeError, ExplorerConnectionStatus,
        ExplorerLoadState, ExplorerNodeAlignment, ExplorerNodeId, ExplorerNodeTarget,
        ExplorerOwnerId, ExplorerScrollAmount, ExplorerTreeState, ProfilePlacement,
        ProfileProvenance, StatusRowKind,
    },
};
use uuid::Uuid;

#[test]
fn profile_order_controls_roots_independently_of_map_order() {
    let first = profile_id(1);
    let second = profile_id(2);
    let mut explorer = ExplorerTreeState::default();

    explorer.add_profile(second);
    explorer.add_profile(first);
    explorer.profile_order = vec![first, second];

    assert_eq!(
        visible_ids(&explorer),
        vec![
            ExplorerNodeId::Profile(first),
            ExplorerNodeId::Profile(second),
        ]
    );
}

#[test]
fn other_profiles_are_hidden_under_a_collapsed_group() {
    let current = profile_id(1);
    let other = profile_id(2);
    let mut explorer = ExplorerTreeState::default();
    explorer.add_profile_with_placement(
        current,
        "current".into(),
        lazydb::profile::DatabaseKind::Sqlite,
        String::new(),
        ProfileProvenance::Saved,
        ProfilePlacement::CurrentProject,
    );
    explorer.add_profile_with_placement(
        other,
        "other".into(),
        lazydb::profile::DatabaseKind::Sqlite,
        String::new(),
        ProfileProvenance::Saved,
        ProfilePlacement::OtherProject,
    );

    assert_eq!(
        visible_ids(&explorer),
        vec![ExplorerNodeId::Profile(current), ExplorerNodeId::Others,]
    );

    explorer.select(ExplorerNodeId::Others);
    explorer.expand();
    assert_eq!(
        visible_ids(&explorer),
        vec![
            ExplorerNodeId::Profile(current),
            ExplorerNodeId::Others,
            ExplorerNodeId::Profile(other),
        ]
    );
    assert_eq!(explorer.visible().last().unwrap().depth, 1);
}

#[test]
fn catalog_tree_validates_profiles_parents_and_duplicate_ids() {
    let profile = profile_id(1);
    let other_profile = profile_id(2);
    let fixture = fixture(profile);
    let mut tree = CatalogTree::new(profile);

    tree.insert(fixture.database.clone()).unwrap();
    assert!(matches!(
        tree.insert(fixture.database.clone()),
        Err(CatalogTreeError::DuplicateId { .. })
    ));
    assert!(matches!(
        tree.insert(database_entry(other_profile, "other")),
        Err(CatalogTreeError::ProfileMismatch { .. })
    ));
    assert!(matches!(
        tree.remove_subtree(&id(other_profile, CatalogKind::Database, "other")),
        Err(CatalogTreeError::ProfileMismatch { .. })
    ));
    assert!(matches!(tree.insert(fixture.schema.clone()), Ok(())));

    let missing_database = id(profile, CatalogKind::Database, "missing");
    let orphan = schema_entry(profile, &missing_database, "orphan");
    assert!(matches!(
        tree.insert(orphan),
        Err(CatalogTreeError::MissingParent { .. })
    ));

    assert_eq!(tree.roots(), std::slice::from_ref(&fixture.database.id));
    assert_eq!(
        tree.children(&fixture.database.id),
        std::slice::from_ref(&fixture.schema.id)
    );
    assert_eq!(tree.parent(&fixture.schema.id), Some(&fixture.database.id));
    assert!(matches!(
        tree.set_group_state(&fixture.database.id, ObjectGroup::Tables, complete_group(1),),
        Err(CatalogTreeError::InvalidGroupParent { .. })
    ));
}

#[test]
fn search_hit_merge_preserves_partial_group_and_real_path() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut tree = CatalogTree::new(profile);
    tree.merge_search_hit(&CatalogSearchHit {
        entry: fixture.column.clone(),
        ancestors: vec![
            fixture.database.clone(),
            fixture.schema.clone(),
            fixture.table.clone(),
        ],
    })
    .unwrap();

    assert_eq!(tree.get(&fixture.column.id), Some(&fixture.column));
    assert_eq!(
        tree.group_state(&fixture.schema.id, ObjectGroup::Tables),
        Some(&CatalogGroupState::default())
    );
}

#[test]
fn page_replacement_and_subtree_removal_keep_indexes_consistent() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let replacement = relation_entry(profile, &fixture.schema.id, CatalogKind::Table, "accounts");
    let mut tree = fixture.tree();

    tree.replace_page(
        &ExplorerOwnerId::Group {
            parent: fixture.schema.id.clone(),
            group: ObjectGroup::Tables,
        },
        vec![replacement.clone()],
    )
    .unwrap();

    assert!(tree.get(&fixture.table.id).is_none());
    assert!(tree.get(&fixture.column.id).is_none());
    assert!(tree.get(&fixture.view.id).is_some());
    assert_eq!(
        tree.group_children(&fixture.schema.id, ObjectGroup::Tables),
        std::slice::from_ref(&replacement.id)
    );
    assert_eq!(
        tree.group_children(&fixture.schema.id, ObjectGroup::Views),
        std::slice::from_ref(&fixture.view.id)
    );

    let removed = tree.remove_subtree(&fixture.schema.id).unwrap();
    assert!(removed.contains(&fixture.schema.id));
    assert!(removed.contains(&replacement.id));
    assert!(removed.contains(&fixture.view.id));
    assert!(tree.children(&fixture.database.id).is_empty());
    assert!(
        tree.group_state(&fixture.schema.id, ObjectGroup::Tables)
            .is_none()
    );
}

#[test]
fn refresh_preserves_stable_selection_and_expansion() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    let selected = ExplorerNodeId::Catalog(fixture.column.id.clone());
    let expanded = expanded_path(&fixture);

    explorer.expanded.extend(expanded.clone());
    assert!(explorer.select(selected.clone()));
    explorer
        .replace_page(
            ExplorerOwnerId::Catalog(fixture.table.id.clone()),
            vec![fixture.column.clone()],
        )
        .unwrap();

    assert_eq!(explorer.selected, Some(selected));
    for id in expanded {
        assert!(explorer.expanded.contains(&id));
    }
}

#[test]
fn removed_selection_falls_back_through_existing_catalog_ancestors() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    explorer.expanded.extend(expanded_path(&fixture));
    assert!(explorer.select(ExplorerNodeId::Catalog(fixture.column.id.clone())));

    explorer.remove_subtree(&fixture.column.id).unwrap();
    assert_eq!(
        explorer.selected,
        Some(ExplorerNodeId::Catalog(fixture.table.id.clone()))
    );

    explorer.remove_subtree(&fixture.table.id).unwrap();
    assert_eq!(
        explorer.selected,
        Some(ExplorerNodeId::Catalog(fixture.schema.id.clone()))
    );

    explorer.remove_subtree(&fixture.database.id).unwrap();
    assert_eq!(explorer.selected, Some(ExplorerNodeId::Profile(profile)));
}

#[test]
fn refresh_falls_from_disappearing_empty_row_to_its_group_owner() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let owner = ExplorerOwnerId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Tables,
    };
    let group = owner.node_id();
    let mut explorer = explorer_with_fixture(&fixture);
    explorer.replace_page(owner.clone(), Vec::new()).unwrap();
    explorer
        .profiles
        .get_mut(&profile)
        .unwrap()
        .load_states
        .insert(
            owner.clone(),
            ExplorerLoadState::Loaded { next_cursor: None },
        );
    assert!(explorer.select(ExplorerNodeId::Empty {
        owner: owner.clone(),
    }));

    explorer
        .replace_page(
            owner,
            vec![relation_entry(
                profile,
                &fixture.schema.id,
                CatalogKind::Table,
                "accounts",
            )],
        )
        .unwrap();

    assert_eq!(explorer.selected, Some(group));
}

#[test]
fn directional_expand_collapse_and_parent_are_distinct() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    let schema = ExplorerNodeId::Catalog(fixture.schema.id.clone());

    assert!(explorer.select(schema.clone()));
    assert!(explorer.expand());
    assert!(explorer.expanded.contains(&schema));
    assert!(explorer.move_to_parent());
    assert_eq!(
        explorer.selected,
        Some(ExplorerNodeId::Catalog(fixture.database.id.clone()))
    );
    assert!(explorer.expanded.contains(&schema));

    assert!(explorer.select(schema.clone()));
    assert!(explorer.collapse());
    assert_eq!(explorer.selected, Some(schema.clone()));
    assert!(!explorer.expanded.contains(&schema));
    assert!(!explorer.collapse());
    assert!(explorer.move_to_parent());
}

#[test]
fn moving_to_parent_scrolls_the_new_selection_into_the_body() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    explorer.expanded.extend(expanded_path(&fixture));
    explorer.set_viewport_height(3);

    let table = ExplorerNodeId::Catalog(fixture.table.id.clone());
    let group = ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Tables,
    };
    assert!(explorer.select(table));
    explorer.scroll = 5;

    assert!(explorer.move_to_parent());
    assert_eq!(explorer.selected, Some(group.clone()));

    let viewport = explorer.viewport(3);
    assert!(viewport.rows.iter().any(|row| row.id == group));
}

#[test]
fn reveal_node_expands_the_complete_visible_parent_path() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    let table = ExplorerNodeId::Catalog(fixture.table.id.clone());
    explorer.expanded.clear();

    assert!(explorer.reveal_node(table.clone()));
    assert_eq!(explorer.selected, Some(table.clone()));
    assert!(
        explorer
            .expanded
            .contains(&ExplorerNodeId::Profile(profile))
    );
    assert!(
        explorer
            .expanded
            .contains(&ExplorerNodeId::Catalog(fixture.database.id.clone()))
    );
    assert!(
        explorer
            .expanded
            .contains(&ExplorerNodeId::Catalog(fixture.schema.id.clone()))
    );
    assert!(explorer.expanded.contains(&ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Tables,
    }));
    assert!(!explorer.expanded.contains(&table));
    assert!(visible_ids(&explorer).contains(&table));
}

#[test]
fn projection_inserts_groups_and_skips_collapsed_catalog_subtrees() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    let profile_node = ExplorerNodeId::Profile(profile);
    let database_node = ExplorerNodeId::Catalog(fixture.database.id.clone());
    let schema_node = ExplorerNodeId::Catalog(fixture.schema.id.clone());
    let table_group = ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Tables,
    };
    let table_node = ExplorerNodeId::Catalog(fixture.table.id.clone());

    explorer.expanded.extend([
        profile_node,
        database_node,
        schema_node.clone(),
        table_group.clone(),
    ]);
    let (rows, visits) = explorer.visible_with_visit_count();
    let ids: Vec<_> = rows.into_iter().map(|row| row.id).collect();
    assert!(ids.contains(&table_group));
    assert!(ids.contains(&table_node));
    assert!(!ids.contains(&ExplorerNodeId::Catalog(fixture.column.id.clone())));
    assert_eq!(visits, 3);

    explorer.expanded.insert(table_node);
    let (rows, visits) = explorer.visible_with_visit_count();
    assert!(
        rows.iter()
            .any(|row| row.id == ExplorerNodeId::Catalog(fixture.column.id.clone()))
    );
    assert_eq!(visits, 4);

    explorer.expanded.remove(&schema_node);
    let (rows, visits) = explorer.visible_with_visit_count();
    assert!(!rows.iter().any(|row| row.id == table_group));
    assert_eq!(visits, 2);
}

#[test]
fn synthetic_child_rows_have_deterministic_non_recursive_ids() {
    let profile = profile_id(1);
    let owner = ExplorerOwnerId::Profile(profile);
    let mut explorer = ExplorerTreeState::default();
    explorer.add_profile(profile);
    explorer.expanded.insert(ExplorerNodeId::Profile(profile));

    let profile_state = explorer.profiles.get_mut(&profile).unwrap();
    profile_state
        .load_states
        .insert(owner.clone(), ExplorerLoadState::Loading { request_id: 1 });
    let loading = ExplorerNodeId::Status {
        owner: owner.clone(),
        kind: StatusRowKind::Loading,
    };
    assert!(visible_ids(&explorer).contains(&loading));

    explorer
        .profiles
        .get_mut(&profile)
        .unwrap()
        .load_states
        .insert(owner.clone(), ExplorerLoadState::Loading { request_id: 99 });
    assert!(visible_ids(&explorer).contains(&loading));

    explorer
        .profiles
        .get_mut(&profile)
        .unwrap()
        .load_states
        .insert(owner.clone(), ExplorerLoadState::Failed { request_id: 100 });
    assert!(visible_ids(&explorer).contains(&ExplorerNodeId::Status {
        owner: owner.clone(),
        kind: StatusRowKind::Retry,
    }));

    explorer
        .profiles
        .get_mut(&profile)
        .unwrap()
        .load_states
        .insert(
            owner.clone(),
            ExplorerLoadState::Loaded { next_cursor: None },
        );
    assert!(visible_ids(&explorer).contains(&ExplorerNodeId::Empty {
        owner: owner.clone(),
    }));

    let cursor = CatalogCursor::new("page-2");
    explorer
        .profiles
        .get_mut(&profile)
        .unwrap()
        .load_states
        .insert(
            owner.clone(),
            ExplorerLoadState::Loaded {
                next_cursor: Some(cursor.clone()),
            },
        );
    assert!(visible_ids(&explorer).contains(&ExplorerNodeId::LoadMore {
        parent: owner,
        cursor,
    }));
}

#[test]
fn viewport_scroll_keeps_selection_visible() {
    let profiles: Vec<_> = (1..=8).map(profile_id).collect();
    let mut explorer = ExplorerTreeState::default();
    for profile in &profiles {
        explorer.add_profile(*profile);
    }

    explorer.move_selection(7, 3);
    assert_eq!(
        explorer.selected,
        Some(ExplorerNodeId::Profile(profiles[7]))
    );
    assert_eq!(explorer.selected_visible_index(), Some(7));
    assert_eq!(explorer.scroll, 5);

    explorer.move_selection(-6, 3);
    assert_eq!(
        explorer.selected,
        Some(ExplorerNodeId::Profile(profiles[1]))
    );
    assert_eq!(explorer.scroll, 1);
}

#[test]
fn viewport_pins_offscreen_ancestors_without_duplicating_the_body() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    explorer.expanded.extend(expanded_path(&fixture));
    let selected = ExplorerNodeId::Catalog(fixture.column.id.clone());
    assert!(explorer.select(selected.clone()));
    explorer.scroll = 5;

    let viewport = explorer.viewport(4);

    assert_eq!(
        viewport
            .pinned
            .iter()
            .map(|row| &row.id)
            .collect::<Vec<_>>(),
        vec![
            &ExplorerNodeId::Group {
                parent: fixture.schema.id.clone(),
                group: ObjectGroup::Tables,
            },
            &ExplorerNodeId::Catalog(fixture.table.id.clone()),
        ]
    );
    assert_eq!(viewport.rows.len(), 1);
    assert_eq!(viewport.rows[0].id, selected);
    assert!(
        !viewport
            .rows
            .iter()
            .any(|row| { viewport.pinned.iter().any(|pinned| pinned.id == row.id) })
    );
}

#[test]
fn viewport_compacts_to_nearest_ancestor_at_two_rows() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut explorer = explorer_with_fixture(&fixture);
    explorer.expanded.extend(expanded_path(&fixture));
    let selected = ExplorerNodeId::Catalog(fixture.column.id.clone());
    assert!(explorer.select(selected.clone()));
    explorer.scroll = 5;

    let viewport = explorer.viewport(2);

    assert_eq!(viewport.pinned.len(), 1);
    assert_eq!(
        viewport.pinned[0].id,
        ExplorerNodeId::Catalog(fixture.table.id)
    );
    assert_eq!(viewport.rows[0].id, selected);
    assert_eq!(viewport.hidden_ancestor_count, 4);
}

#[test]
fn vim_targets_page_moves_and_alignment_use_the_measured_viewport() {
    let profiles: Vec<_> = (1..=12).map(profile_id).collect();
    let mut explorer = ExplorerTreeState::default();
    for profile in &profiles {
        explorer.add_profile(*profile);
    }
    explorer.set_viewport_height(5);

    for _ in 0..4 {
        explorer.move_selection(1, 5);
    }
    assert_eq!(explorer.selected_visible_index(), Some(4));
    assert_eq!(explorer.scroll, 0);
    explorer.move_selection(1, 5);
    assert_eq!(explorer.selected_visible_index(), Some(5));
    assert_eq!(explorer.scroll, 1);

    explorer.select_target(ExplorerNodeTarget::First);
    assert_eq!(explorer.selected_visible_index(), Some(0));
    assert_eq!(explorer.scroll, 0);
    explorer.select_target(ExplorerNodeTarget::ViewBottom);
    assert_eq!(explorer.selected_visible_index(), Some(4));
    explorer.select_target(ExplorerNodeTarget::ViewMiddle);
    assert_eq!(explorer.selected_visible_index(), Some(2));
    explorer.select_target(ExplorerNodeTarget::Last);
    assert_eq!(explorer.selected_visible_index(), Some(11));
    assert_eq!(explorer.scroll, 7);

    explorer.scroll_nodes(-1, ExplorerScrollAmount::Page);
    assert_eq!(explorer.selected_visible_index(), Some(6));
    assert_eq!(explorer.scroll, 2);
    explorer.scroll_nodes(1, ExplorerScrollAmount::HalfPage);
    assert_eq!(explorer.selected_visible_index(), Some(8));
    assert_eq!(explorer.scroll, 4);

    explorer.align_selected(ExplorerNodeAlignment::Top);
    assert_eq!(explorer.scroll, 7);
    explorer.align_selected(ExplorerNodeAlignment::Bottom);
    assert_eq!(explorer.scroll, 4);
}

#[test]
fn compatibility_movement_updates_normalized_scroll() {
    let profiles: Vec<_> = (1..=20).map(profile_id).collect();
    let mut explorer = ExplorerTreeState::default();
    for profile in profiles {
        explorer.add_profile(profile);
    }
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer,
        ..Default::default()
    };
    state.move_selection(19);
    assert_eq!(state.normalized.selected_visible_index(), Some(19));
    assert!(state.normalized.scroll > 0);
    let selected = state.normalized.selected_visible_index().unwrap();
    assert!(selected >= state.normalized.scroll);
    assert!(selected < state.normalized.scroll + 8);
}

#[test]
fn visible_find_snapshots_only_currently_visible_primary_labels() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.normalized.expanded.extend([
        ExplorerNodeId::Profile(profile),
        ExplorerNodeId::Catalog(fixture.database.id.clone()),
        ExplorerNodeId::Catalog(fixture.schema.id.clone()),
        ExplorerNodeId::Group {
            parent: fixture.schema.id.clone(),
            group: ObjectGroup::Tables,
        },
    ]);

    state.open_find();
    state.edit_find(|query| query.push_str("USER"));

    let find = state.find.as_ref().unwrap();
    assert_eq!(
        find.matches,
        vec![ExplorerNodeId::Catalog(fixture.table.id.clone())]
    );
    assert_eq!(state.find_match_position(), (1, 1));
    assert!(find.rows.iter().any(|row| row.label == "users"));
    assert!(!find.rows.iter().any(|row| row.label == "id"));
}

#[test]
fn visible_find_matches_primary_labels_not_metadata_and_empty_queries_are_idle() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.normalized.expanded.extend(expanded_path(&fixture));
    state.normalized.expanded.insert(ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Views,
    });
    state.open_find();

    state.edit_find(|query| query.push_str("BIGINT"));
    assert!(state.find.as_ref().unwrap().matches.is_empty());
    assert_eq!(state.find_match_position(), (0, 0));

    state.edit_find(String::clear);
    assert!(state.find.as_ref().unwrap().matches.is_empty());
    assert_eq!(state.find_match_position(), (0, 0));
}

#[test]
fn visible_find_confirms_and_cycles_selection_with_wraparound() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.normalized.expanded.extend(expanded_path(&fixture));
    state.normalized.expanded.insert(ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Views,
    });
    state.open_find();
    state.edit_find(|query| query.push_str("user"));

    assert!(state.confirm_find());
    assert_eq!(
        state.selected_id(),
        Some(&ExplorerNodeId::Catalog(fixture.table.id.clone()))
    );
    assert!(state.move_find_match(1));
    assert_eq!(
        state.selected_id(),
        Some(&ExplorerNodeId::Catalog(fixture.view.id.clone()))
    );
    assert!(state.move_find_match(1));
    assert_eq!(
        state.selected_id(),
        Some(&ExplorerNodeId::Catalog(fixture.table.id.clone()))
    );
    assert!(state.move_find_match(-1));
    assert_eq!(
        state.selected_id(),
        Some(&ExplorerNodeId::Catalog(fixture.view.id.clone()))
    );
}

#[test]
fn visible_find_starts_at_current_selection_and_wraps_downward() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.normalized.expanded.extend(expanded_path(&fixture));
    state.normalized.expanded.insert(ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Views,
    });
    assert!(
        state
            .normalized
            .select(ExplorerNodeId::Catalog(fixture.table.id.clone()))
    );
    state.open_find();
    state.edit_find(|query| query.push_str("user"));

    assert!(state.confirm_find());
    assert_eq!(
        state.selected_id(),
        Some(&ExplorerNodeId::Catalog(fixture.table.id.clone()))
    );
}

#[test]
fn visible_find_centers_each_current_match_in_the_viewport() {
    let profiles: Vec<_> = (1..=8).map(profile_id).collect();
    let mut normalized = ExplorerTreeState::default();
    for (index, profile) in profiles.iter().enumerate() {
        normalized.add_profile_with_metadata(
            *profile,
            format!("profile-{index}"),
            lazydb::profile::DatabaseKind::Sqlite,
            String::new(),
            ProfileProvenance::Saved,
        );
    }
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized,
        ..Default::default()
    };
    state.set_viewport_height(5);
    state.open_find();
    state.edit_find(|query| query.push_str("profile"));

    assert!(state.confirm_find());
    assert_eq!(state.normalized.selected_visible_index(), Some(0));
    assert_eq!(state.normalized.scroll, 0);

    assert!(state.move_find_match(1));
    assert_eq!(state.normalized.selected_visible_index(), Some(1));
    assert_eq!(state.normalized.scroll, 0);
    assert!(state.move_find_match(1));
    assert_eq!(state.normalized.selected_visible_index(), Some(2));
    assert_eq!(state.normalized.scroll, 0);
    assert!(state.move_find_match(1));
    assert_eq!(state.normalized.selected_visible_index(), Some(3));
    assert_eq!(state.normalized.scroll, 1);
    assert!(state.move_find_match(1));
    assert_eq!(state.normalized.selected_visible_index(), Some(4));
    assert_eq!(state.normalized.scroll, 2);
    assert!(state.move_find_match(-1));
    assert_eq!(state.normalized.selected_visible_index(), Some(3));
    assert_eq!(state.normalized.scroll, 1);
}

#[test]
fn visible_find_close_can_restore_original_selection() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.normalized.selected = Some(ExplorerNodeId::Profile(profile));
    state.normalized.expanded.extend(expanded_path(&fixture));
    state.open_find();
    state.edit_find(|query| query.push_str("user"));
    state.confirm_find();
    state.close_find(true);

    assert!(state.find.is_none());
    assert_eq!(state.selected_id(), Some(&ExplorerNodeId::Profile(profile)));
}

#[test]
fn frontend_search_locates_the_current_match_instead_of_the_first() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.open_search(
        Some(lazydb::identity::ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        }),
        1,
    );
    state.edit_search(|query| query.push_str("user"));
    state.refresh_frontend_search();

    let matches = state.search.as_ref().unwrap().frontend_match_rows.clone();
    assert_eq!(matches.len(), 2);
    let selected = state.search.as_ref().unwrap().frontend_rows[matches[1]]
        .id
        .clone();
    state.search.as_mut().unwrap().selected = matches[1];

    assert_eq!(state.locate_search_hit(), Ok(true));
    assert!(state.search.is_none());
    assert_eq!(state.selected_id(), Some(&selected));
}

#[test]
fn frontend_search_starts_after_current_explorer_selection() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.normalized.expanded.extend(expanded_path(&fixture));
    assert!(
        state
            .normalized
            .select(ExplorerNodeId::Catalog(fixture.table.id.clone()))
    );
    state.open_search(
        Some(lazydb::identity::ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        }),
        1,
    );
    state.edit_search(|query| query.push_str("user"));
    state.refresh_frontend_search();

    let search = state.search.as_ref().unwrap();
    let selected = &search.frontend_rows[search.selected].id;
    assert_eq!(selected, &ExplorerNodeId::Catalog(fixture.table.id.clone()));
}

#[test]
fn frontend_search_wraps_to_first_match_after_current_position() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.normalized.expanded.extend(expanded_path(&fixture));
    assert!(
        state
            .normalized
            .select(ExplorerNodeId::Catalog(fixture.view.id.clone()))
    );
    state.open_search(
        Some(lazydb::identity::ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        }),
        1,
    );
    state.edit_search(|query| query.push_str("user"));
    state.refresh_frontend_search();

    let search = state.search.as_ref().unwrap();
    let selected = &search.frontend_rows[search.selected].id;
    assert_eq!(selected, &ExplorerNodeId::Catalog(fixture.table.id.clone()));
}

#[test]
fn catalog_search_projection_deduplicates_ancestors_and_places_groups_before_hits() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let connection = lazydb::identity::ConnectionIdentity {
        profile_id: profile,
        generation: 1,
    };
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.open_search(Some(connection), 1);
    state.edit_search(|query| query.push('u'));
    let page = lazydb::db::catalog::CatalogSearchPage {
        connection,
        session_id: 1,
        generation: 1,
        hits: vec![
            CatalogSearchHit {
                entry: fixture.table.clone(),
                ancestors: vec![fixture.database.clone(), fixture.schema.clone()],
            },
            CatalogSearchHit {
                entry: fixture.view.clone(),
                ancestors: vec![fixture.database.clone(), fixture.schema.clone()],
            },
        ],
        total_count: Some(2),
        truncated: false,
    };

    assert!(state.accept_search_page(page));
    let rows = &state.search.as_ref().unwrap().rows;
    let schema = ExplorerNodeId::Catalog(fixture.schema.id.clone());
    let tables = ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Tables,
    };
    let table = ExplorerNodeId::Catalog(fixture.table.id.clone());
    let views = ExplorerNodeId::Group {
        parent: fixture.schema.id.clone(),
        group: ObjectGroup::Views,
    };
    let view = ExplorerNodeId::Catalog(fixture.view.id.clone());
    assert_eq!(rows.iter().filter(|row| row.id == schema).count(), 1);
    assert!(
        rows.iter().position(|row| row.id == tables).unwrap()
            < rows.iter().position(|row| row.id == table).unwrap()
    );
    assert!(
        rows.iter().position(|row| row.id == views).unwrap()
            < rows.iter().position(|row| row.id == view).unwrap()
    );
    assert_eq!(rows.iter().filter(|row| row.is_match).count(), 2);
    state.search.as_mut().unwrap().selected = rows.iter().position(|row| row.id == schema).unwrap();
    assert!(!state.locate_search_hit().unwrap());
    assert!(state.move_search_match(1));
    let selected = state.search.as_ref().unwrap().selected;
    assert!(state.search.as_ref().unwrap().rows[selected].is_match);
}

#[test]
fn rebuilding_projection_preserves_user_collapse_state() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let mut state = lazydb::model::workspace::ExplorerState {
        normalized: explorer_with_fixture(&fixture),
        ..Default::default()
    };
    state.rebuild_projection(profile);
    state.normalized.expanded.clear();
    state.normalized.selected = Some(ExplorerNodeId::Profile(profile));
    state.rebuild_projection(profile);
    assert!(
        !state
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(profile))
    );
    assert!(
        !state
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Catalog(fixture.database.id))
    );
}

#[test]
fn profile_state_tracks_status_generations_requests_loads_and_errors() {
    let profile = profile_id(1);
    let mut explorer = ExplorerTreeState::default();
    explorer.add_profile(profile);
    let state = explorer.profiles.get_mut(&profile).unwrap();

    assert_eq!(state.status, ExplorerConnectionStatus::Offline);
    assert_eq!(state.catalog_epoch, 0);
    assert_eq!(state.next_request_id, 1);
    assert!(state.load_states.is_empty());
    assert_eq!(state.last_error, None);
    assert!(!state.expand_after_connect);
    assert_eq!(state.allocate_request_id(), Some(1));
    assert_eq!(state.allocate_request_id(), Some(2));
    assert_eq!(state.advance_catalog_epoch(), Some(1));
}

#[test]
fn relation_lookup_uses_catalog_identity_not_path_offsets() {
    let profile = profile_id(1);
    let fixture = fixture(profile);
    let tree = fixture.tree();

    assert_eq!(
        tree.owning_relation_id(&fixture.table.id),
        Some(&fixture.table.id)
    );
    assert_eq!(
        tree.owning_relation_id(&fixture.column.id),
        Some(&fixture.table.id)
    );
    assert_eq!(
        tree.owning_relation(&fixture.column.id),
        Some(&fixture.table)
    );
    assert_eq!(
        tree.owning_relation_id(&fixture.schema.id),
        None,
        "schemas are not relations"
    );
}

#[derive(Clone)]
struct Fixture {
    profile: Uuid,
    database: CatalogEntry,
    schema: CatalogEntry,
    table: CatalogEntry,
    view: CatalogEntry,
    column: CatalogEntry,
}

impl Fixture {
    fn tree(&self) -> CatalogTree {
        let mut tree = CatalogTree::new(self.profile);
        tree.insert_subtree(vec![
            self.database.clone(),
            self.schema.clone(),
            self.table.clone(),
            self.view.clone(),
            self.column.clone(),
        ])
        .unwrap();
        tree.set_group_state(&self.schema.id, ObjectGroup::Tables, complete_group(1))
            .unwrap();
        tree.set_group_state(&self.schema.id, ObjectGroup::Views, complete_group(1))
            .unwrap();
        tree
    }
}

fn fixture(profile: Uuid) -> Fixture {
    let database = database_entry(profile, "app");
    let schema = schema_entry(profile, &database.id, "public");
    let table = relation_entry(profile, &schema.id, CatalogKind::Table, "users");
    let view = relation_entry(profile, &schema.id, CatalogKind::View, "active_users");
    let column = CatalogEntry::relation_child(
        id(profile, CatalogKind::Column, "users.id"),
        table.id.clone(),
        qualified("id"),
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
    )
    .unwrap();
    Fixture {
        profile,
        database,
        schema,
        table,
        view,
        column,
    }
}

fn explorer_with_fixture(fixture: &Fixture) -> ExplorerTreeState {
    let mut explorer = ExplorerTreeState::default();
    explorer.add_profile(fixture.profile);
    explorer.profiles.get_mut(&fixture.profile).unwrap().catalog = fixture.tree();
    explorer
}

fn expanded_path(fixture: &Fixture) -> Vec<ExplorerNodeId> {
    vec![
        ExplorerNodeId::Profile(fixture.profile),
        ExplorerNodeId::Catalog(fixture.database.id.clone()),
        ExplorerNodeId::Catalog(fixture.schema.id.clone()),
        ExplorerNodeId::Group {
            parent: fixture.schema.id.clone(),
            group: ObjectGroup::Tables,
        },
        ExplorerNodeId::Catalog(fixture.table.id.clone()),
    ]
}

fn database_entry(profile: Uuid, name: &str) -> CatalogEntry {
    CatalogEntry::database(
        id(profile, CatalogKind::Database, name),
        QualifiedName {
            database: Some(name.to_owned()),
            schema: None,
            object: name.to_owned(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn schema_entry(profile: Uuid, database: &CatalogId, name: &str) -> CatalogEntry {
    CatalogEntry::schema(
        id(profile, CatalogKind::Schema, name),
        database.clone(),
        QualifiedName {
            database: Some("app".to_owned()),
            schema: Some(name.to_owned()),
            object: name.to_owned(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn relation_entry(
    profile: Uuid,
    schema: &CatalogId,
    kind: CatalogKind,
    name: &str,
) -> CatalogEntry {
    CatalogEntry::relation(
        id(profile, kind, name),
        schema.clone(),
        qualified(name),
        match kind {
            CatalogKind::Table => "table",
            CatalogKind::View => "view",
            _ => "relation",
        },
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn complete_group(count: u64) -> CatalogGroupState {
    CatalogGroupState {
        count: CatalogCount::Exact(count),
        completeness: CatalogCompleteness::Complete,
    }
}

fn qualified(object: &str) -> QualifiedName {
    QualifiedName {
        database: Some("app".to_owned()),
        schema: Some("public".to_owned()),
        object: object.to_owned(),
    }
}

fn id(profile: Uuid, kind: CatalogKind, native_id: &str) -> CatalogId {
    CatalogId::new(profile, kind, [native_id])
}

fn profile_id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn visible_ids(explorer: &ExplorerTreeState) -> Vec<ExplorerNodeId> {
    explorer.visible().into_iter().map(|row| row.id).collect()
}
