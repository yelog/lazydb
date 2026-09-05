use std::{hint::black_box, time::Instant};

use lazydb::{
    action::Action,
    app::App,
    db::{ServerInfo, query::QueryOutcome, value::CellValue},
    model::tab::WorkspaceTab,
    profile::{DatabaseKind, import_connection_url},
    ui::{
        self, UiState,
        icons::{IconMode, IconSet},
        theme::Theme,
    },
};
use ratatui::{Terminal, backend::TestBackend};

const RESULT_ROWS: usize = 500;
const RESULT_COLUMNS: usize = 20;

fn fixture() -> App {
    let profile = import_connection_url("sqlite::memory:", Some("performance"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Sqlite,
            version: "3.50.0".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ReplaceEditor(large_sql()));
    let tab_id = app.active_console().id;
    let generation = app.active_console().generation;
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection: app.connection.active_identity().unwrap(),
        outcome: result_outcome(),
    });
    app
}

fn large_sql() -> String {
    (0..4_000)
        .map(|index| format!("SELECT id, name FROM users_{index} WHERE active = true;\n"))
        .collect()
}

fn result_outcome() -> QueryOutcome {
    QueryOutcome {
        result_sets: vec![lazydb::db::query::ResultSet {
            columns: (0..RESULT_COLUMNS)
                .map(|index| lazydb::db::query::ColumnMeta {
                    name: format!("column_{index}"),
                    type_name: "TEXT".into(),
                })
                .collect(),
            rows: (0..RESULT_ROWS)
                .map(|row| {
                    (0..RESULT_COLUMNS)
                        .map(|column| CellValue::Text(format!("value_{row}_{column}")))
                        .collect()
                })
                .collect(),
            affected_rows: 0,
        }],
        stats: lazydb::db::query::QueryStats::new(
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            RESULT_ROWS,
        ),
    }
}

fn render_once(app: &mut App, terminal: &mut Terminal<TestBackend>, state: &mut UiState) {
    let icons = IconSet::new(IconMode::Ascii);
    let theme = Theme::for_color_mode(lazydb::cli::ColorMode::Never);
    terminal
        .draw(|frame| {
            ui::render_with_state_using_icons_sequence_and_theme(
                frame, app, state, icons, None, theme,
            );
        })
        .unwrap();
}

#[test]
fn performance_fixtures_are_deterministic_and_renderable() {
    let mut app = fixture();
    let tab = app.active_console();
    assert_eq!(
        tab.outcome.as_ref().unwrap().result_sets[0].rows.len(),
        RESULT_ROWS
    );
    assert_eq!(
        tab.outcome.as_ref().unwrap().result_sets[0].columns.len(),
        RESULT_COLUMNS
    );
    assert!(app.active_editor_text().unwrap().len() > 100_000);

    let mut terminal = Terminal::new(TestBackend::new(160, 48)).unwrap();
    let mut state = UiState::new();
    render_once(&mut app, &mut terminal, &mut state);
    assert!(!terminal.backend().buffer().content().is_empty());
}

#[test]
#[ignore = "run manually in release mode to compare elapsed time"]
fn performance_baseline() {
    let mut app = fixture();
    let mut terminal = Terminal::new(TestBackend::new(160, 48)).unwrap();
    let mut state = UiState::new();

    let start = Instant::now();
    for _ in 0..20 {
        render_once(&mut app, &mut terminal, &mut state);
    }
    let elapsed = start.elapsed();
    black_box(&app);
    println!("baseline: 20 full renders in {elapsed:?}");

    let result_rows = match app.tabs.first() {
        Some(WorkspaceTab::Sql(tab)) => tab.outcome.as_ref().unwrap().result_sets[0].rows.len(),
        _ => 0,
    };
    assert_eq!(result_rows, RESULT_ROWS);
}
