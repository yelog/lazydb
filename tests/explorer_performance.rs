use lazydb::{
    db::catalog::{
        CatalogCompleteness, CatalogCount, CatalogEntry, CatalogId, CatalogKind, ObjectGroup,
        OptionalMetadata, QualifiedName,
    },
    model::explorer::{CatalogGroupState, CatalogTree, ExplorerNodeId, ExplorerTreeState},
};
use uuid::Uuid;

#[test]
fn projection_visits_only_expanded_subtrees_with_ten_thousand_objects() {
    let profile = Uuid::from_u128(1);
    let database = database_entry(profile);
    let mut entries = vec![database.clone()];
    let mut schemas = Vec::with_capacity(100);

    for schema_index in 0..100 {
        let schema = schema_entry(profile, &database.id, schema_index);
        for relation_index in 0..100 {
            entries.push(relation_entry(
                profile,
                &schema.id,
                schema_index,
                relation_index,
            ));
        }
        schemas.push(schema.clone());
        entries.push(schema);
    }

    // Parents may appear anywhere in a replacement batch; identity and adjacency
    // validation must not require a pre-sorted database response.
    entries[1..].rotate_right(1);

    let mut tree = CatalogTree::new(profile);
    tree.insert_subtree(entries).unwrap();
    for schema in &schemas {
        tree.set_group_state(
            &schema.id,
            ObjectGroup::Tables,
            CatalogGroupState {
                count: CatalogCount::Exact(100),
                completeness: CatalogCompleteness::Complete,
            },
        )
        .unwrap();
    }

    let mut explorer = ExplorerTreeState::default();
    explorer.add_profile(profile);
    explorer.profiles.get_mut(&profile).unwrap().catalog = tree;
    explorer.expanded.extend([
        ExplorerNodeId::Profile(profile),
        ExplorerNodeId::Catalog(database.id.clone()),
        ExplorerNodeId::Catalog(schemas[0].id.clone()),
        ExplorerNodeId::Group {
            parent: schemas[0].id.clone(),
            group: ObjectGroup::Tables,
        },
    ]);

    let (rows, visited_catalog_entries) = explorer.visible_with_visit_count();
    assert_eq!(visited_catalog_entries, 201);
    assert_eq!(rows.len(), 203);

    for schema in &schemas {
        explorer
            .expanded
            .insert(ExplorerNodeId::Catalog(schema.id.clone()));
        explorer.expanded.insert(ExplorerNodeId::Group {
            parent: schema.id.clone(),
            group: ObjectGroup::Tables,
        });
    }

    let (rows, visited_catalog_entries) = explorer.visible_with_visit_count();
    assert_eq!(visited_catalog_entries, 10_101);
    assert_eq!(rows.len(), 10_202);
}

fn database_entry(profile: Uuid) -> CatalogEntry {
    CatalogEntry::database(
        CatalogId::new(profile, CatalogKind::Database, ["app"]),
        QualifiedName {
            database: Some("app".to_owned()),
            schema: None,
            object: "app".to_owned(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn schema_entry(profile: Uuid, database: &CatalogId, index: usize) -> CatalogEntry {
    let name = format!("schema_{index:03}");
    CatalogEntry::schema(
        CatalogId::new(profile, CatalogKind::Schema, [name.as_str()]),
        database.clone(),
        QualifiedName {
            database: Some("app".to_owned()),
            schema: Some(name.clone()),
            object: name,
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
    schema_index: usize,
    relation_index: usize,
) -> CatalogEntry {
    let name = format!("table_{schema_index:03}_{relation_index:03}");
    CatalogEntry::relation(
        CatalogId::new(profile, CatalogKind::Table, [name.as_str()]),
        schema.clone(),
        QualifiedName {
            database: Some("app".to_owned()),
            schema: Some(format!("schema_{schema_index:03}")),
            object: name,
        },
        "table",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap()
}
