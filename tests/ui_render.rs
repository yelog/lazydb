use std::time::Duration;

use lazydb::{
    action::Action,
    app::App,
    db::{
        ServerInfo,
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        value::CellValue,
    },
    model::workspace::{ConnectionStatus, Focus},
    profile::{DatabaseKind, import_connection_url},
    ui,
};
use ratatui::{Terminal, backend::TestBackend};

fn fixture() -> App {
    let profile = import_connection_url("sqlite::memory:", Some("orbital-lab"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.connection.profile_id = Some(profile.id);
    app.connection.status = ConnectionStatus::Connected;
    app.connection.server = Some(ServerInfo {
        kind: DatabaseKind::Sqlite,
        version: "3.50.0".into(),
        database: ":memory:".into(),
    });
    app.update(Action::ReplaceEditor(
        "SELECT id, name, active\nFROM users\nWHERE active = true;".into(),
    ));
    let tab_id = app.active_console().id;
    let generation = app.active_console().generation;
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        outcome: QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![
                    ColumnMeta {
                        name: "id".into(),
                        type_name: "INTEGER".into(),
                    },
                    ColumnMeta {
                        name: "name".into(),
                        type_name: "TEXT".into(),
                    },
                    ColumnMeta {
                        name: "active".into(),
                        type_name: "BOOLEAN".into(),
                    },
                ],
                rows: vec![vec![
                    CellValue::Integer(42),
                    CellValue::Text("Ada".into()),
                    CellValue::Boolean(true),
                ]],
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::from_millis(24), Duration::from_millis(352), 1),
        },
    });
    app
}

fn render(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut output = String::new();
    for y in 0..height {
        for x in 0..width {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    output
}

#[test]
fn standard_layout_shows_stable_workspace_regions() {
    let output = render(&fixture(), 120, 36);

    assert!(output.contains("LAZYDB"));
    assert!(output.contains("orbital-lab"));
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("console"));
    assert!(output.contains("SELECT"));
    assert!(output.contains("DATA"));
    assert!(output.contains("OUTPUT"));
    assert!(output.contains("Ada"));
    assert!(output.contains("F1 help"));
}

#[test]
fn compact_layout_uses_the_focused_panel() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    let output = render(&app, 80, 24);

    assert!(output.contains("LAZYDB"));
    assert!(output.contains("SQL EDITOR"));
    assert!(output.contains("WHERE"));
    assert!(!output.contains("EXPLORER"));
}

#[test]
fn wide_layout_remains_readable() {
    let mut app = fixture();
    app.update(Action::ToggleResultView);
    let output = render(&app, 180, 50);

    assert!(output.contains("EXPLORER"));
    assert!(output.contains("SQL EDITOR"));
    assert!(output.contains("1 row(s) retrieved in 376 ms"));
}

#[test]
fn help_overlay_is_contextual() {
    let mut app = fixture();
    app.focus = Focus::Explorer;
    app.update(Action::ShowHelp);
    let output = render(&app, 120, 36);

    assert!(output.contains("KEYMAP // EXPLORER"));
    assert!(output.contains("expand / open"));
    assert!(output.contains("Esc"));
}

#[test]
fn tiny_terminal_gets_an_actionable_message() {
    let output = render(&fixture(), 40, 10);

    assert!(output.contains("TERMINAL TOO SMALL"));
    assert!(output.contains("Resize"));
}
