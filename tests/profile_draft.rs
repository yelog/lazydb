use lazydb::{
    model::{
        profile_manager::{
            CredentialUpdate, ProfileDraft, ProfileField, ProfileManagerPage, ProfileManagerState,
        },
        text_input::TextInput,
    },
    profile::{ConnectionProfile, DatabaseKind, Environment, SslMode},
};
use secrecy::ExposeSecret;

fn valid_postgres_draft() -> ProfileDraft {
    let mut draft = ProfileDraft::new(DatabaseKind::Postgres);
    draft.name.set("primary");
    draft.database.set("lazydb");
    draft
}

fn saved_postgres_profile() -> ConnectionProfile {
    valid_postgres_draft().validate(&[]).unwrap().profile
}

#[test]
fn text_input_edits_unicode_by_character_position() {
    let mut input = TextInput::from("数据");
    input.move_end();
    input.move_left();
    input.insert('库');

    assert_eq!(input.value(), "数库据");
    assert_eq!(input.cursor(), 2);

    input.backspace();
    assert_eq!(input.value(), "数据");
    assert_eq!(input.cursor(), 1);
}

#[test]
fn text_input_supports_unicode_paste_delete_and_bounded_movement() {
    let mut input = TextInput::from("a界z");
    input.move_home();
    input.move_left();
    input.move_right();
    input.paste("数据");

    assert_eq!(input.value(), "a数据界z");
    assert_eq!(input.cursor(), 3);

    input.delete();
    assert_eq!(input.value(), "a数据z");
    input.move_end();
    input.move_right();
    input.delete();
    assert_eq!(input.cursor(), 4);
    assert_eq!(input.value(), "a数据z");

    input.set("é🙂");
    assert_eq!(input.cursor(), 2);
    input.backspace();
    assert_eq!(input.value(), "é");
}

#[test]
fn new_postgres_uses_server_defaults() {
    let draft = ProfileDraft::new(DatabaseKind::Postgres);

    assert_eq!(draft.host.value(), "localhost");
    assert_eq!(draft.port.value(), "5432");
    assert_eq!(draft.schema.value(), "public");
    assert_eq!(draft.ssl_mode, SslMode::Prefer);
    assert!(!draft.sqlite_memory);
}

#[test]
fn new_mysql_and_sqlite_use_driver_defaults() {
    let mysql = ProfileDraft::new(DatabaseKind::MySql);
    assert_eq!(mysql.host.value(), "localhost");
    assert_eq!(mysql.port.value(), "3306");

    let sqlite = ProfileDraft::new(DatabaseKind::Sqlite);
    assert!(!sqlite.sqlite_memory);
    assert!(sqlite.sqlite_path.value().is_empty());
    assert_eq!(sqlite.ssl_mode, SslMode::Disable);
}

#[test]
fn sqlite_accepts_memory_or_a_non_empty_path() {
    let mut draft = ProfileDraft::new(DatabaseKind::Sqlite);
    draft.name.set("scratch");

    let error = draft.validate(&[]).unwrap_err();
    assert_eq!(error.field, ProfileField::SqlitePath);

    draft.sqlite_path.set(" ./data/lazy.db ");
    let file = draft.validate(&[]).unwrap().profile;
    assert_eq!(
        file.sqlite_path.unwrap().to_string_lossy(),
        "./data/lazy.db"
    );
    assert_eq!(file.database.as_deref(), Some("./data/lazy.db"));

    draft.sqlite_memory = true;
    let memory = draft.validate(&[]).unwrap().profile;
    assert!(memory.sqlite_path.is_none());
    assert_eq!(memory.database.as_deref(), Some(":memory:"));
}

#[test]
fn name_is_required_and_unique_case_insensitively() {
    let existing = saved_postgres_profile();
    let mut draft = valid_postgres_draft();
    draft.name.set("  ");

    assert_eq!(draft.validate(&[]).unwrap_err().field, ProfileField::Name);

    draft.name.set(" PRIMARY ");
    assert_eq!(
        draft.validate(&[existing]).unwrap_err().field,
        ProfileField::Name
    );
}

#[test]
fn editing_excludes_its_own_name_from_duplicate_validation() {
    let mut profile = saved_postgres_profile();
    profile.name = "Primary".into();
    let draft = ProfileDraft::edit(&profile, false);

    let submission = draft.validate(&[profile.clone()]).unwrap();
    assert_eq!(submission.profile.id, profile.id);
    assert_eq!(submission.profile.name, "Primary");
}

#[test]
fn server_host_and_database_are_required() {
    let mut draft = valid_postgres_draft();
    draft.host.set(" ");
    assert_eq!(draft.validate(&[]).unwrap_err().field, ProfileField::Host);

    draft.host.set("localhost");
    draft.database.set("");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::Database
    );
}

#[test]
fn port_must_be_an_integer_in_the_tcp_range() {
    let mut draft = valid_postgres_draft();

    for invalid in ["", "0", "65536", "abc"] {
        draft.port.set(invalid);
        assert_eq!(draft.validate(&[]).unwrap_err().field, ProfileField::Port);
    }

    for valid in ["1", "65535"] {
        draft.port.set(valid);
        assert_eq!(
            draft.validate(&[]).unwrap().profile.port,
            valid.parse().ok()
        );
    }
}

#[test]
fn draft_uuid_is_stable_across_validation() {
    let draft = valid_postgres_draft();

    let first = draft.validate(&[]).unwrap().profile.id;
    let second = draft.validate(&[]).unwrap().profile.id;

    assert_eq!(first, second);
}

#[test]
fn editing_preserves_uuid_and_unedited_profile_metadata() {
    let mut profile = saved_postgres_profile();
    profile.environment = Environment::Production;
    profile.read_only = true;
    profile.include_databases = vec!["app".into()];
    profile.include_schemas = vec!["public".into()];

    let mut draft = ProfileDraft::edit(&profile, false);
    draft.name.set("renamed");
    let edited = draft.validate(&[profile.clone()]).unwrap().profile;

    assert_eq!(edited.id, profile.id);
    assert_eq!(edited.environment, Environment::Production);
    assert!(edited.read_only);
    assert_eq!(edited.include_databases, vec!["app"]);
    assert_eq!(edited.include_schemas, vec!["public"]);
}

#[test]
fn credential_intent_preserves_forgets_or_replaces_stored_passwords() {
    let mut profile = saved_postgres_profile();
    profile.secret_ref = Some(format!("keyring:test/{}", profile.id));

    let draft = ProfileDraft::edit(&profile, true);
    assert!(matches!(
        draft.validate(&[profile.clone()]).unwrap().credential,
        CredentialUpdate::Preserve
    ));

    let mut forget = draft.clone();
    forget.remember_password = false;
    let submission = forget.validate(&[profile.clone()]).unwrap();
    assert!(matches!(submission.credential, CredentialUpdate::Forget));
    assert!(submission.profile.secret_ref.is_none());

    let mut session = draft.clone();
    session.remember_password = false;
    session.set_password("session-password");
    let submission = session.validate(&[profile.clone()]).unwrap();
    match submission.credential {
        CredentialUpdate::Session(secret) => {
            assert_eq!(secret.expose_secret(), "session-password");
        }
        other => panic!("expected session credential, got {other:?}"),
    }
    assert!(submission.profile.secret_ref.is_none());

    let mut remember = draft;
    remember.set_password("remembered-password");
    match remember.validate(&[profile]).unwrap().credential {
        CredentialUpdate::Remember(secret) => {
            assert_eq!(secret.expose_secret(), "remembered-password");
        }
        other => panic!("expected remembered credential, got {other:?}"),
    }
}

#[test]
fn new_password_uses_session_or_remember_intent() {
    let mut draft = valid_postgres_draft();
    draft.set_password("temporary");

    assert!(matches!(
        draft.validate(&[]).unwrap().credential,
        CredentialUpdate::Session(_)
    ));

    draft.remember_password = true;
    assert!(matches!(
        draft.validate(&[]).unwrap().credential,
        CredentialUpdate::Remember(_)
    ));
}

#[test]
fn debug_output_redacts_passwords() {
    let mut draft = valid_postgres_draft();
    draft.set_password("never-print-this-password");
    draft.remember_password = true;
    let submission = draft.validate(&[]).unwrap();

    assert!(!format!("{draft:?}").contains("never-print-this-password"));
    assert!(!format!("{submission:?}").contains("never-print-this-password"));
    assert!(!format!("{:?}", submission.credential).contains("never-print-this-password"));
}

#[test]
fn visible_fields_follow_the_selected_driver_and_sqlite_mode() {
    let postgres = ProfileDraft::new(DatabaseKind::Postgres);
    assert_eq!(
        postgres.visible_fields(),
        &[
            ProfileField::Kind,
            ProfileField::Name,
            ProfileField::Host,
            ProfileField::Port,
            ProfileField::User,
            ProfileField::Password,
            ProfileField::Database,
            ProfileField::Schema,
            ProfileField::SslMode,
            ProfileField::Environment,
            ProfileField::ReadOnly,
            ProfileField::RememberPassword,
            ProfileField::Test,
            ProfileField::Save,
            ProfileField::SaveAndConnect,
            ProfileField::Cancel,
        ]
    );

    let mut sqlite = ProfileDraft::new(DatabaseKind::Sqlite);
    assert!(sqlite.visible_fields().contains(&ProfileField::SqlitePath));
    sqlite.sqlite_memory = true;
    assert!(!sqlite.visible_fields().contains(&ProfileField::SqlitePath));
    assert!(!sqlite.visible_fields().contains(&ProfileField::Password));
}

#[test]
fn manager_state_initializes_new_and_edit_forms() {
    let mut state = ProfileManagerState::new(true);
    assert_eq!(state.page, ProfileManagerPage::List);
    assert!(state.opened_automatically);

    state.start_new(DatabaseKind::MySql);
    assert_eq!(state.page, ProfileManagerPage::Form);
    assert_eq!(state.selected_field, ProfileField::Kind);
    assert_eq!(state.draft.as_ref().unwrap().port.value(), "3306");

    let profile = saved_postgres_profile();
    state.start_edit(&profile, false);
    assert_eq!(state.draft.as_ref().unwrap().profile_id(), profile.id);
    assert_eq!(
        state.visible_fields(),
        state.draft.as_ref().unwrap().visible_fields()
    );
}
