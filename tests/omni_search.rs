use lazydb::{
    action::{Action, CatalogSearchOwner, Command},
    app::App,
    db::catalog::{CatalogSearchPage, CatalogSearchRequest},
    model::{text_input::TextInputEdit, workspace::ConnectionIdentity},
    profile::import_connection_url,
};
use uuid::Uuid;

fn connected_app() -> (App, Uuid, ConnectionIdentity) {
    let profile = import_connection_url("sqlite::memory:", Some("local"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "test".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    (
        app,
        profile_id,
        ConnectionIdentity {
            profile_id,
            generation: 1,
        },
    )
}

#[test]
fn omni_query_emits_owner_scoped_search_only_for_nonempty_catalog_query() {
    let (mut app, _, _) = connected_app();
    app.update(Action::OpenOmni);
    let command = app.update(Action::OmniEdit(TextInputEdit::Insert('u')));
    assert!(matches!(
        command.as_slice(),
        [Command::SearchCatalog { owner: CatalogSearchOwner::Omni, request }]
            if request.query == "u"
                && request.session_id == app.omni.as_ref().unwrap().session_id
    ));
}

#[test]
fn closed_omni_ignores_late_search_failure() {
    let (mut app, _, connection) = connected_app();
    app.update(Action::OpenOmni);
    let session_id = app.omni.as_ref().unwrap().session_id;
    app.update(Action::OmniEdit(TextInputEdit::Insert('u')));
    let generation = app.omni.as_ref().unwrap().query_generation;
    app.update(Action::OmniDismiss);

    app.update(Action::CatalogSearchFailed {
        owner: CatalogSearchOwner::Omni,
        connection,
        session_id,
        generation,
        message: "late failure".into(),
    });
    assert!(app.omni.is_none());
}

#[test]
fn explorer_search_failure_does_not_write_omni_status() {
    let (mut app, _, connection) = connected_app();
    app.update(Action::OpenOmni);
    let session_id = app.omni.as_ref().unwrap().session_id;
    app.update(Action::CatalogSearchFailed {
        owner: CatalogSearchOwner::Explorer,
        connection,
        session_id,
        generation: 1,
        message: "explorer failure".into(),
    });
    assert_eq!(app.omni.as_ref().unwrap().status, None);
}

#[test]
fn search_result_carries_owner_and_the_catalog_protocol_identity() {
    let (_, profile_id, connection) = connected_app();
    let request = CatalogSearchRequest {
        connection,
        session_id: 5,
        generation: 8,
        query: "users".into(),
        scope: lazydb::profile::CatalogScope::for_profile(
            lazydb::profile::DatabaseKind::Sqlite,
            ":memory:",
            None,
        ),
        limit: 10,
    };
    assert_eq!(request.connection.profile_id, profile_id);
    assert_eq!(request.session_id, 5);
    assert_eq!(request.generation, 8);
    let page = CatalogSearchPage::new(&request, Vec::new(), Some(0), false).unwrap();
    let action = Action::CatalogSearchSucceeded {
        owner: CatalogSearchOwner::Omni,
        page,
    };
    assert!(matches!(
        action,
        Action::CatalogSearchSucceeded {
            owner: CatalogSearchOwner::Omni,
            page,
        } if page.session_id == 5 && page.generation == 8
    ));
}
