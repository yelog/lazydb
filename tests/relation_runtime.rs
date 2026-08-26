use lazydb::{
    action::Action,
    app::App,
    db::catalog::{CatalogId, CatalogKind},
    identity::ConnectionIdentity,
    model::{
        relation::{
            OwnedSnapshot, RelationDescriptor, RelationKey, RelationLoad, RelationRequest,
            RelationRequestKind, RelationSnapshot, RelationTab, RelationView,
        },
        tab::WorkspaceTab,
    },
    persistence::{profiles::ProfileStore, secrets::NativeSecretStore},
    profile::{CatalogScope, DatabaseKind},
    runtime::Runtime,
};
use std::collections::HashSet;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};
use uuid::Uuid;

fn request() -> RelationRequest {
    let profile_id = Uuid::new_v4();
    RelationRequest {
        tab_id: Uuid::new_v4(),
        tab_generation: 3,
        request_id: 9,
        connection: ConnectionIdentity {
            profile_id,
            generation: 4,
        },
        relation: RelationKey {
            profile_id,
            object_id: CatalogId::new(profile_id, CatalogKind::Table, ["users"]),
        },
        kind: RelationRequestKind::Preview,
        scope: CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
    }
}

#[test]
fn every_mutated_request_identity_is_stale() {
    let valid = request();
    let mutations = [
        RelationRequest {
            connection: ConnectionIdentity {
                profile_id: Uuid::new_v4(),
                ..valid.connection
            },
            ..valid.clone()
        },
        RelationRequest {
            connection: ConnectionIdentity {
                generation: valid.connection.generation + 1,
                ..valid.connection
            },
            ..valid.clone()
        },
        RelationRequest {
            tab_id: Uuid::new_v4(),
            ..valid.clone()
        },
        RelationRequest {
            tab_generation: valid.tab_generation + 1,
            ..valid.clone()
        },
        RelationRequest {
            request_id: valid.request_id + 1,
            ..valid.clone()
        },
        RelationRequest {
            relation: RelationKey {
                object_id: CatalogId::new(
                    valid.connection.profile_id,
                    CatalogKind::Table,
                    ["other"],
                ),
                ..valid.relation.clone()
            },
            ..valid.clone()
        },
        RelationRequest {
            kind: RelationRequestKind::Structure,
            ..valid.clone()
        },
    ];
    assert!(mutations.iter().all(|candidate| candidate != &valid));
}

#[test]
fn unchanged_relation_response_is_the_only_accepted_identity() {
    let request = request();
    let mut app = App::new(Vec::new());
    let mut profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("test"))
        .unwrap()
        .profile;
    profile.id = request.connection.profile_id;
    app.profiles.push(profile);
    app.connection.profile_id = Some(request.connection.profile_id);
    app.connection.generation = request.connection.generation;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    let tab = RelationTab::with_descriptor(
        RelationDescriptor {
            key: request.relation.clone(),
            qualified_name: lazydb::db::catalog::QualifiedName {
                database: None,
                schema: None,
                object: "users".into(),
            },
            kind: CatalogKind::Table,
            title: "users".into(),
        },
        RelationView::Data,
    );
    let tab_id = tab.id;
    let mut tab = tab;
    let mut request = request;
    request.tab_id = tab_id;
    request.tab_generation = tab.generation;
    let previous = OwnedSnapshot {
        value: lazydb::db::RelationPreview {
            sql: "previous".into(),
            result: empty_outcome(),
        },
        attribution: lazydb::model::relation::SnapshotAttribution {
            connection: request.connection,
            profile_id: request.connection.profile_id,
            scope: lazydb::profile::CatalogScope::for_profile(
                lazydb::profile::DatabaseKind::Sqlite,
                "db",
                None,
            ),
        },
    };
    tab.data = RelationLoad::Loading {
        request: request.clone(),
        previous: Some(previous.clone()),
    };
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = 1;
    let _ = app.update(Action::RelationSucceeded {
        request,
        snapshot: Box::new(RelationSnapshot::Preview(lazydb::db::RelationPreview {
            sql: "select 1".into(),
            result: lazydb::db::query::QueryOutcome {
                result_sets: Vec::new(),
                stats: lazydb::db::query::QueryStats::new(
                    std::time::Duration::ZERO,
                    std::time::Duration::ZERO,
                    0,
                ),
            },
        })),
    });
    assert!(matches!(
        app.tabs[1],
        WorkspaceTab::Relation(RelationTab {
            data: RelationLoad::Ready(OwnedSnapshot { .. }),
            ..
        })
    ));
}

#[test]
fn every_stale_field_preserves_pending_and_previous_snapshot() {
    let valid = request();
    for stale in stale_requests(&valid) {
        let mut app = relation_app(&valid);
        let before = app.tabs[1].clone();
        app.update(Action::RelationSucceeded {
            request: stale,
            snapshot: Box::new(RelationSnapshot::Preview(lazydb::db::RelationPreview {
                sql: "stale".into(),
                result: empty_outcome(),
            })),
        });
        assert_eq!(app.tabs[1], before);
    }
}

#[test]
fn stale_failure_also_preserves_pending_and_previous_snapshot() {
    let valid = request();
    let mut app = relation_app(&valid);
    let before = app.tabs[1].clone();
    app.update(Action::RelationFailed {
        request: RelationRequest {
            request_id: valid.request_id + 1,
            ..valid
        },
        message: "stale".into(),
    });
    assert_eq!(app.tabs[1], before);
}

fn stale_requests(valid: &RelationRequest) -> Vec<RelationRequest> {
    vec![
        RelationRequest {
            connection: ConnectionIdentity {
                profile_id: Uuid::new_v4(),
                ..valid.connection
            },
            ..valid.clone()
        },
        RelationRequest {
            connection: ConnectionIdentity {
                generation: valid.connection.generation + 1,
                ..valid.connection
            },
            ..valid.clone()
        },
        RelationRequest {
            tab_id: Uuid::new_v4(),
            ..valid.clone()
        },
        RelationRequest {
            tab_generation: valid.tab_generation + 1,
            ..valid.clone()
        },
        RelationRequest {
            request_id: valid.request_id + 1,
            ..valid.clone()
        },
        RelationRequest {
            relation: RelationKey {
                object_id: CatalogId::new(
                    valid.connection.profile_id,
                    CatalogKind::Table,
                    ["other"],
                ),
                ..valid.relation.clone()
            },
            ..valid.clone()
        },
        RelationRequest {
            kind: RelationRequestKind::Structure,
            ..valid.clone()
        },
    ]
}

fn relation_app(request: &RelationRequest) -> App {
    let mut app = App::new(Vec::new());
    app.connection.profile_id = Some(request.connection.profile_id);
    app.connection.generation = request.connection.generation;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    let mut tab = RelationTab::with_descriptor(
        RelationDescriptor {
            key: request.relation.clone(),
            qualified_name: lazydb::db::catalog::QualifiedName {
                database: None,
                schema: None,
                object: "users".into(),
            },
            kind: CatalogKind::Table,
            title: "users".into(),
        },
        RelationView::Data,
    );
    tab.generation = request.tab_generation;
    tab.data = RelationLoad::Loading {
        request: request.clone(),
        previous: Some(OwnedSnapshot {
            value: lazydb::db::RelationPreview {
                sql: "previous".into(),
                result: empty_outcome(),
            },
            attribution: lazydb::model::relation::SnapshotAttribution {
                connection: request.connection,
                profile_id: request.connection.profile_id,
                scope: lazydb::profile::CatalogScope::for_profile(
                    lazydb::profile::DatabaseKind::Sqlite,
                    "db",
                    None,
                ),
            },
        }),
    };
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = 1;
    app
}

fn empty_outcome() -> lazydb::db::query::QueryOutcome {
    lazydb::db::query::QueryOutcome {
        result_sets: Vec::new(),
        stats: lazydb::db::query::QueryStats::new(
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            0,
        ),
    }
}

#[tokio::test]
async fn runtime_rejects_relation_before_adapter_when_catalog_identity_is_unknown() {
    let temp = TempDir::new().unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        Vec::new(),
        HashSet::new(),
        std::collections::HashMap::new(),
        None,
        ProfileStore::new(temp.path().join("connections.toml")),
        Arc::new(NativeSecretStore),
        sender,
    );
    let request = request();
    runtime.dispatch(lazydb::action::Command::LoadRelationPreview(
        request.clone(),
    ));
    assert!(matches!(
        timeout(Duration::from_secs(1), receiver.recv()).await.unwrap().unwrap(),
        Action::RelationFailed { request: echoed, .. } if echoed == request
    ));
    runtime.shutdown().await;
}

#[test]
fn scope_mutation_does_not_change_request_snapshot_attribution() {
    let mut request = request();
    let mut app = relation_app(&request);
    request.tab_id = app.tabs[1].id();
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.data = RelationLoad::Loading {
            request: request.clone(),
            previous: None,
        };
    }
    let mut profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("test"))
        .unwrap()
        .profile;
    profile.id = request.connection.profile_id;
    profile.catalog_scope = CatalogScope::for_profile(DatabaseKind::Sqlite, "changed", None);
    app.profiles.push(profile.clone());
    app.update(Action::RelationSucceeded {
        request: request.clone(),
        snapshot: Box::new(RelationSnapshot::Preview(lazydb::db::RelationPreview {
            sql: "select 1".into(),
            result: empty_outcome(),
        })),
    });
    let WorkspaceTab::Relation(tab) = &app.tabs[1] else {
        panic!()
    };
    let RelationLoad::Ready(snapshot) = &tab.data else {
        panic!()
    };
    assert_eq!(snapshot.attribution.scope, request.scope);
}

#[test]
fn duplicate_request_identity_is_rejected_by_request_set() {
    let request = request();
    let mut set = HashSet::new();
    assert!(set.insert(request.clone()));
    assert!(!set.insert(request));
}

fn import_profile(id: Uuid) -> lazydb::profile::ConnectionProfile {
    let mut profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("test"))
        .unwrap()
        .profile;
    profile.id = id;
    profile
}

#[test]
fn refreshing_loading_relation_cancels_exact_previous_request_first() {
    let request = request();
    let mut app = relation_app(&request);
    let mut profile = import_profile(request.connection.profile_id);
    profile.catalog_scope = request.scope.clone();
    app.profiles.push(profile);
    let pending = match &app.tabs[1] {
        lazydb::model::tab::WorkspaceTab::Relation(tab) => match &tab.data {
            RelationLoad::Loading { request, .. } => request.clone(),
            _ => panic!(),
        },
        _ => panic!(),
    };
    let pending = RelationRequest {
        tab_id: app.tabs[1].id(),
        ..pending
    };
    if let lazydb::model::tab::WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.data = RelationLoad::Loading {
            request: pending.clone(),
            previous: match std::mem::replace(&mut tab.data, RelationLoad::Empty) {
                RelationLoad::Loading { previous, .. } => previous,
                _ => None,
            },
        };
    }
    let commands = app.update(Action::RefreshActiveRelation);
    assert!(matches!(&commands[..], [
        lazydb::action::Command::CancelRelationRequest(previous),
        lazydb::action::Command::LoadRelationPreview(next)
    ] if previous == &pending && next.request_id == 1));
}
