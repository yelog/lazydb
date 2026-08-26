use lazydb::profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope};
use serde_json::json;

#[test]
fn scope_defaults_to_current_database_and_default_schema() {
    let scope = CatalogScope::for_profile(DatabaseKind::Postgres, "App", Some("Public"));

    assert_eq!(
        scope,
        CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: "App".to_owned(),
                schemas: CatalogSelection::Selected(vec!["Public".to_owned()]),
            }]),
        }
    );
}

#[test]
fn missing_default_schema_selects_all_postgres_schemas() {
    let scope = CatalogScope::for_profile(DatabaseKind::Postgres, "app", None);

    assert_eq!(
        scope,
        CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: "app".to_owned(),
                schemas: CatalogSelection::All,
            }]),
        }
    );
}

#[test]
fn mysql_uses_canonical_all_schema_selection() {
    let scope = CatalogScope::for_profile(DatabaseKind::MySql, "Sales", Some("Sales"));

    assert_eq!(
        scope,
        CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: "Sales".to_owned(),
                schemas: CatalogSelection::All,
            }]),
        }
    );
}

#[test]
fn sqlite_without_default_schema_selects_all_discovered_schemas() {
    let scope = CatalogScope::for_profile(DatabaseKind::Sqlite, "/tmp/Case.db", None);

    assert_eq!(
        scope,
        CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: "/tmp/Case.db".to_owned(),
                schemas: CatalogSelection::All,
            }]),
        }
    );
}

#[test]
fn databases_can_have_independent_schema_selections() {
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "primary".to_owned(),
                schemas: CatalogSelection::Selected(vec!["public".to_owned()]),
            },
            DatabaseScope {
                name: "analytics".to_owned(),
                schemas: CatalogSelection::Selected(vec!["reporting".to_owned()]),
            },
        ]),
    };

    assert!(scope.validate("primary", Some("public")).is_ok());
    assert!(scope.allows_schema("primary", "public"));
    assert!(!scope.allows_schema("primary", "reporting"));
    assert!(scope.allows_schema("analytics", "reporting"));
    assert!(!scope.allows_schema("analytics", "public"));
}

#[test]
fn selections_serialize_with_explicit_tags() {
    let all = serde_json::to_value(CatalogSelection::<String>::All).unwrap();
    let selected =
        serde_json::to_value(CatalogSelection::Selected(vec!["Public".to_owned()])).unwrap();

    assert_eq!(all, json!({ "mode": "all" }));
    assert_eq!(selected, json!({ "mode": "selected", "items": ["Public"] }));
}

#[test]
fn exact_case_sensitive_names_round_trip_unchanged() {
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "SalesDB".to_owned(),
            schemas: CatalogSelection::Selected(vec!["CamelCase".to_owned()]),
        }]),
    };

    let serialized = serde_json::to_string(&scope).unwrap();
    let deserialized: CatalogScope = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized, scope);
    assert!(deserialized.allows_database("SalesDB"));
    assert!(!deserialized.allows_database("salesdb"));
    assert!(deserialized.allows_schema("SalesDB", "CamelCase"));
    assert!(!deserialized.allows_schema("SalesDB", "camelcase"));
}

#[test]
fn validation_rejects_empty_selected_at_either_level() {
    let empty_databases = CatalogScope {
        databases: CatalogSelection::Selected(Vec::new()),
    };
    let empty_schemas = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".to_owned(),
            schemas: CatalogSelection::Selected(Vec::new()),
        }]),
    };

    assert!(empty_databases.validate("app", None).is_err());
    assert!(empty_schemas.validate("app", None).is_err());
}

#[test]
fn validation_rejects_empty_names() {
    let empty_database = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: String::new(),
            schemas: CatalogSelection::All,
        }]),
    };
    let empty_schema = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".to_owned(),
            schemas: CatalogSelection::Selected(vec![String::new()]),
        }]),
    };

    assert!(empty_database.validate("app", None).is_err());
    assert!(empty_schema.validate("app", None).is_err());
}

#[test]
fn validation_rejects_exact_duplicates_but_allows_case_variants() {
    let duplicate_databases = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "App".to_owned(),
                schemas: CatalogSelection::All,
            },
            DatabaseScope {
                name: "App".to_owned(),
                schemas: CatalogSelection::All,
            },
        ]),
    };
    let duplicate_schemas = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "App".to_owned(),
            schemas: CatalogSelection::Selected(vec!["Public".to_owned(), "Public".to_owned()]),
        }]),
    };
    let case_variants = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "App".to_owned(),
                schemas: CatalogSelection::Selected(vec!["Public".to_owned(), "public".to_owned()]),
            },
            DatabaseScope {
                name: "app".to_owned(),
                schemas: CatalogSelection::All,
            },
        ]),
    };

    assert!(duplicate_databases.validate("App", None).is_err());
    assert!(duplicate_schemas.validate("App", None).is_err());
    assert!(case_variants.validate("App", Some("Public")).is_ok());
}

#[test]
fn validation_rejects_an_excluded_default_schema() {
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".to_owned(),
            schemas: CatalogSelection::Selected(vec!["private".to_owned()]),
        }]),
    };

    assert!(scope.validate("app", Some("public")).is_err());
    assert!(scope.validate("app", Some("private")).is_ok());
}

#[test]
fn allows_schema_never_bypasses_database_selection() {
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".to_owned(),
            schemas: CatalogSelection::All,
        }]),
    };

    assert!(scope.allows_database("app"));
    assert!(!scope.allows_database("archive"));
    assert!(scope.allows_schema("app", "public"));
    assert!(!scope.allows_schema("archive", "public"));
}
