use lazydb::{
    action::Action,
    app::App,
    db::descriptor::{DRIVERS, DatabaseCategory, drivers_in},
    model::profile_manager::{ProfileField, ProfileManagerState},
    profile::DatabaseKind,
};

#[test]
fn categories_partition_the_registry_without_changing_machine_names() {
    assert_eq!(drivers_in(DatabaseCategory::Relational).count(), 11);
    assert_eq!(
        drivers_in(DatabaseCategory::NonRelational)
            .map(|d| d.kind)
            .collect::<Vec<_>>(),
        vec![
            DatabaseKind::Redis,
            DatabaseKind::Valkey,
            DatabaseKind::MongoDb,
            DatabaseKind::Elasticsearch
        ]
    );
    assert_eq!(
        DRIVERS.iter().map(|d| d.name).collect::<Vec<_>>(),
        vec![
            "postgres",
            "mysql",
            "mariadb",
            "oracle",
            "sqlserver",
            "sqlite",
            "duckdb",
            "redis",
            "valkey",
            "mongodb",
            "clickhouse",
            "tidb",
            "cockroachdb",
            "snowflake",
            "elasticsearch"
        ]
    );
}

#[test]
fn category_round_trip_remembers_driver_and_automatic_ports_cross_sqlite() {
    let mut manager = ProfileManagerState::new(false);
    manager.start_new(DatabaseKind::MySql);
    manager.selected_field = ProfileField::DatabaseCategory;
    manager.cycle(1);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Redis);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "6379");
    manager.select_driver(DatabaseKind::MongoDb);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::MongoDb);
    manager.select_driver(DatabaseKind::Redis);
    manager.select_driver(DatabaseKind::MySql);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::MySql);
    manager.select_driver(DatabaseKind::SqlServer);
    manager.select_driver(DatabaseKind::Sqlite);
    manager.cycle(1);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "6379");
    manager.select_driver(DatabaseKind::Valkey);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Valkey);
}

#[test]
fn custom_port_survives_category_switch_and_single_driver_cycle_is_a_noop() {
    let mut manager = ProfileManagerState::new(false);
    manager.start_new(DatabaseKind::Postgres);
    manager.draft.as_mut().unwrap().port.set("6380");
    manager.selected_field = ProfileField::DatabaseCategory;
    manager.cycle(1);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "6380");
    manager.select_driver(DatabaseKind::Redis);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Redis);
    manager.select_driver(DatabaseKind::Valkey);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Valkey);
    manager.select_driver(DatabaseKind::MongoDb);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::MongoDb);
    manager.select_driver(DatabaseKind::MongoDb);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "6380");
}

#[test]
fn category_action_updates_the_real_form_and_driver_list() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileSelectCategory(
        DatabaseCategory::NonRelational,
    ));
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.selected_field, ProfileField::DatabaseCategory);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Redis);
    assert!(!manager.visible_fields().contains(&ProfileField::Schema));
    app.update(Action::ProfileSelectDriver(DatabaseKind::Valkey));
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Valkey);
    assert!(!manager.visible_fields().contains(&ProfileField::Schema));
}

#[test]
fn tidb_category_round_trip_restores_driver_defaults_and_preserves_custom_ports() {
    let mut manager = ProfileManagerState::new(false);
    manager.start_new(DatabaseKind::TiDb);
    assert_eq!(manager.draft.as_ref().unwrap().user.value(), "root");
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "4000");
    manager.selected_field = ProfileField::DatabaseCategory;
    manager.cycle(1);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Redis);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "6379");
    manager.cycle(-1);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::TiDb);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "4000");
    manager.select_driver(DatabaseKind::MySql);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "3306");
    manager.select_driver(DatabaseKind::TiDb);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "4000");
    manager.draft.as_mut().unwrap().port.set("4400");
    manager.select_driver(DatabaseKind::MySql);
    manager.select_driver(DatabaseKind::TiDb);
    assert_eq!(manager.draft.as_ref().unwrap().port.value(), "4400");
}
