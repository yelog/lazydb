use lazydb::{
    action::Action,
    app::App,
    db::{
        catalog::{CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName},
        mongodb::types::{
            MongoCollectionInfo, MongoCollectionType, MongoExplorerNodeId, MongoNamespace,
        },
        principal::{PrincipalEntry, PrincipalId, PrincipalKind, PrincipalPage, PrincipalScope},
        redis::{
            discovery::{RedisDatabaseDiscovery, RedisDatabaseInfo, RedisDiscoveryCompleteness},
            types::RedisTarget,
        },
    },
    identity::ConnectionIdentity,
    model::{
        dashboard::{DashboardTab, MongoDashboardTab},
        explorer::{ExplorerConnectionStatus, ExplorerNodeId},
        mongodb_browser::MongoBrowserTab,
        mongodb_console::MongoConsoleTab,
        principal::PrincipalDdlTab,
        redis_browser::{RedisBrowserFocus, RedisBrowserTab},
        relation::{RelationDescriptor, RelationTab, RelationView},
        tab::WorkspaceTab,
        workspace::Focus,
    },
    profile::import_connection_url,
};
use uuid::Uuid;

const VIEWPORT_HEIGHT: usize = 5;
const REDIS_DATABASE: u32 = 31;
const MONGO_DATABASE: &str = "database_23";
const COLLECTION: &str = "events.archive";

fn app_with_matching_profiles(url: &str) -> (App, [Uuid; 2]) {
    let profile_ids = [Uuid::from_u128(1), Uuid::from_u128(2)];
    let profiles = profile_ids
        .map(|profile_id| {
            let mut profile = import_connection_url(url, Some("shared")).unwrap().profile;
            profile.id = profile_id;
            profile
        })
        .into_iter()
        .collect();
    let mut app = App::new(profiles);
    for state in app.explorer.normalized.profiles.values_mut() {
        state.status = ExplorerConnectionStatus::Online;
    }
    (app, profile_ids)
}

fn assert_revealed(app: &App, expected: &ExplorerNodeId) {
    assert_eq!(app.explorer.selected_id(), Some(expected));
    let viewport = app.explorer.viewport(VIEWPORT_HEIGHT);
    assert!(
        viewport
            .rows
            .iter()
            .chain(&viewport.pinned)
            .any(|row| &row.id == expected),
        "expected {expected:?} in the viewport"
    );
}

fn assert_tab_reveals(
    app: &mut App,
    tab: WorkspaceTab,
    expected: ExplorerNodeId,
    ancestors: &[ExplorerNodeId],
) {
    let other_profile = app
        .profiles
        .iter()
        .find(|profile| Some(profile.id) != expected.profile_id())
        .unwrap()
        .id;
    app.tabs = vec![WorkspaceTab::Dashboard(DashboardTab::new()), tab];
    for focus in [Focus::Results, Focus::Explorer] {
        for action in [Action::ActivateTab(1), Action::NextTab, Action::PreviousTab] {
            app.active_tab = 0;
            app.focus = focus;
            app.explorer.normalized.expanded.clear();
            app.explorer.set_viewport_height(VIEWPORT_HEIGHT);
            assert!(
                app.explorer
                    .select_id(ExplorerNodeId::Profile(other_profile))
            );

            app.update(action);

            assert_eq!(app.active_tab, 1);
            assert_revealed(app, &expected);
            for ancestor in ancestors {
                assert!(
                    app.explorer.normalized.expanded.contains(ancestor),
                    "expected expanded ancestor {ancestor:?} for {expected:?}"
                );
            }
            assert_eq!(app.focus, focus);
            if let WorkspaceTab::RedisBrowser(browser) = &app.tabs[1] {
                assert_eq!(browser.focus, RedisBrowserFocus::Preview);
            }
        }
    }
}

#[test]
fn redis_browser_and_dashboard_reveal_the_database_on_the_correct_profile() {
    let (mut app, profile_ids) = app_with_matching_profiles("redis://localhost:6379/31");
    for state in app.explorer.normalized.profiles.values_mut() {
        state.set_redis_databases(RedisDatabaseDiscovery {
            databases: (0..=REDIS_DATABASE)
                .map(|database| RedisDatabaseInfo {
                    database,
                    keys: None,
                    expires: None,
                })
                .collect(),
            completeness: RedisDiscoveryCompleteness::Complete,
            warnings: Vec::new(),
        });
    }
    for profile_id in profile_ids {
        let mut browser = RedisBrowserTab::new(
            Uuid::new_v4(),
            RedisTarget {
                profile_id,
                database: REDIS_DATABASE,
            },
        );
        browser.focus = RedisBrowserFocus::Preview;
        let mut dashboard = DashboardTab::new();
        dashboard.profile_id = Some(profile_id);
        dashboard.redis_database = Some(REDIS_DATABASE);
        for tab in [
            WorkspaceTab::RedisBrowser(browser),
            WorkspaceTab::Dashboard(dashboard),
        ] {
            assert_tab_reveals(
                &mut app,
                tab,
                ExplorerNodeId::RedisDatabase {
                    profile_id,
                    database: REDIS_DATABASE,
                },
                &[ExplorerNodeId::Profile(profile_id)],
            );
        }
    }
}

#[test]
fn sql_dashboards_reveal_the_profile_even_when_a_database_is_loaded() {
    let (mut app, profile_ids) = app_with_matching_profiles("postgres://user@localhost/app");
    for profile_id in profile_ids {
        app.explorer
            .normalized
            .profiles
            .get_mut(&profile_id)
            .unwrap()
            .catalog
            .insert(
                CatalogEntry::database(
                    CatalogId::new(profile_id, CatalogKind::Database, ["opaque-native-id"]),
                    QualifiedName {
                        database: Some("app".into()),
                        schema: None,
                        object: "app".into(),
                    },
                    "database",
                    OptionalMetadata::Unsupported,
                    true,
                )
                .unwrap(),
            )
            .unwrap();
        let mut dashboard = DashboardTab::new();
        dashboard.profile_id = Some(profile_id);

        assert_tab_reveals(
            &mut app,
            WorkspaceTab::Dashboard(dashboard),
            ExplorerNodeId::Profile(profile_id),
            &[],
        );
    }
}

#[test]
fn mongo_native_tabs_reveal_namespaces_without_sql_catalog_entries() {
    let (mut app, profile_ids) =
        app_with_matching_profiles("mongodb://localhost:27017/database_23");
    for state in app.explorer.normalized.profiles.values_mut() {
        state.mongodb_databases = (0..24)
            .map(|index| format!("database_{index:02}"))
            .collect();
        for database in ["database_00", MONGO_DATABASE] {
            let mut collections = (0..24)
                .map(|index| MongoCollectionInfo {
                    namespace: MongoNamespace::collection(database, format!("events_{index:02}")),
                    collection_type: MongoCollectionType::Collection,
                    options_ejson: None,
                })
                .collect::<Vec<_>>();
            collections.push(MongoCollectionInfo {
                namespace: MongoNamespace::collection(database, COLLECTION),
                collection_type: MongoCollectionType::Collection,
                options_ejson: None,
            });
            state
                .mongodb_collections
                .insert(database.to_owned(), collections);
        }
    }
    for profile_id in profile_ids {
        for database in ["database_00", MONGO_DATABASE] {
            let namespace = MongoNamespace::collection(database, COLLECTION);
            let profile_node = ExplorerNodeId::Profile(profile_id);
            let database_node = ExplorerNodeId::Mongo {
                profile_id,
                node: MongoExplorerNodeId::Database(database.to_owned()),
            };
            let collection_node = ExplorerNodeId::Mongo {
                profile_id,
                node: MongoExplorerNodeId::Collection(namespace.clone()),
            };
            let descriptor =
                RelationDescriptor::mongodb_collection(profile_id, &namespace).unwrap();
            assert!(
                app.explorer.normalized.profiles[&profile_id]
                    .catalog
                    .get(&descriptor.key.object_id)
                    .is_none()
            );
            for tab in [
                WorkspaceTab::Relation(RelationTab::with_descriptor(
                    descriptor,
                    RelationView::Data,
                )),
                WorkspaceTab::MongoBrowser(MongoBrowserTab::new(profile_id, namespace)),
            ] {
                assert_tab_reveals(
                    &mut app,
                    tab,
                    collection_node.clone(),
                    &[profile_node.clone(), database_node.clone()],
                );
            }
            for tab in [
                WorkspaceTab::MongoConsole(MongoConsoleTab::new(profile_id, database)),
                WorkspaceTab::MongoDashboard(Box::new(MongoDashboardTab::new(
                    profile_id, database,
                ))),
            ] {
                assert_tab_reveals(
                    &mut app,
                    tab,
                    database_node.clone(),
                    std::slice::from_ref(&profile_node),
                );
            }
        }
    }
}

fn principal_entry(profile_id: Uuid, index: u32) -> PrincipalEntry {
    PrincipalEntry {
        id: PrincipalId {
            profile_id,
            scope: PrincipalScope::Cluster,
            native_id: format!("principal-{index}"),
            host: None,
        },
        kind: PrincipalKind::User,
        name: format!("user_{index:02}"),
        native_kind: "login_role".into(),
        system: false,
    }
}

fn app_with_principals() -> (App, [Uuid; 2]) {
    let (mut app, profile_ids) = app_with_matching_profiles("postgres://user@localhost/app");
    for profile_id in profile_ids {
        app.explorer
            .normalized
            .profiles
            .get_mut(&profile_id)
            .unwrap()
            .set_principals(PrincipalPage {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                entries: (0..32)
                    .map(|index| principal_entry(profile_id, index))
                    .collect(),
                complete: true,
            });
    }
    (app, profile_ids)
}

#[test]
fn principal_tabs_expand_users_and_roles_on_the_correct_profile() {
    let (mut app, profile_ids) = app_with_principals();
    for profile_id in profile_ids {
        let entry = principal_entry(profile_id, 31);
        let expected = ExplorerNodeId::Principal {
            entry: entry.id.clone(),
        };

        assert_tab_reveals(
            &mut app,
            WorkspaceTab::PrincipalDdl(PrincipalDdlTab::new(entry)),
            expected,
            &[
                ExplorerNodeId::Profile(profile_id),
                ExplorerNodeId::PrincipalGroup { profile_id },
            ],
        );
    }
}

#[test]
fn offline_principal_tabs_reveal_the_profile_instead_of_a_hidden_principal() {
    let (mut app, profile_ids) = app_with_principals();
    for profile_id in profile_ids {
        app.explorer
            .normalized
            .profiles
            .get_mut(&profile_id)
            .unwrap()
            .status = ExplorerConnectionStatus::Offline;
        let entry = principal_entry(profile_id, 31);
        let hidden = ExplorerNodeId::Principal {
            entry: entry.id.clone(),
        };

        assert_tab_reveals(
            &mut app,
            WorkspaceTab::PrincipalDdl(PrincipalDdlTab::new(entry)),
            ExplorerNodeId::Profile(profile_id),
            &[],
        );

        assert!(
            !app.explorer
                .normalized
                .visible()
                .iter()
                .any(|row| row.id == hidden)
        );
    }
}
