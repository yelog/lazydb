use lazydb::{
    action::Action,
    app::App,
    db::descriptor::{DRIVERS, DatabaseCategory, drivers_in},
    model::profile_manager::{ProfileField, ProfileManagerState},
    profile::DatabaseKind,
};

#[test]
fn categories_partition_the_registry_without_changing_machine_names() {
    assert_eq!(drivers_in(DatabaseCategory::Relational).count(), 9);
    assert_eq!(
        drivers_in(DatabaseCategory::NonRelational)
            .map(|d| d.kind)
            .collect::<Vec<_>>(),
        vec![
            DatabaseKind::Redis,
            DatabaseKind::Valkey,
            DatabaseKind::MongoDb
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
            "snowflake"
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
