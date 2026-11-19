use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazydb::{
    action::Action,
    app::App,
    cli::MotionMode,
    config::AppConfig,
    db::redis::types::{RedisKeyId, RedisTarget},
    input::{keymap::Keymap, mouse::map_mouse},
    model::{
        keyspace::KeyspaceStatus,
        redis_browser::{RedisBrowserFocus, RedisBrowserTab},
        redis_key_tree::KeyTreeNodeId,
        tab::{ConsoleTab, WorkspaceTab},
        workspace::Focus,
    },
    ui::{
        self, HitTarget, UiState,
        layout::{AppLayout, RedisBrowserLayout},
    },
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use uuid::Uuid;

fn key(character: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)
}

fn redis_app() -> App {
    let mut app = App::new(Vec::new());
    let target = RedisTarget {
        profile_id: Uuid::from_u128(2),
        database: 0,
    };
    let mut tab = RedisBrowserTab::new(Uuid::from_u128(1), target.clone());
    tab.keyspace.keys = ["plain", "group:first", "group:second"]
        .into_iter()
        .map(|name| RedisKeyId {
            target: target.clone(),
            key: name.as_bytes().to_vec(),
        })
        .collect();
    tab.keyspace.status = KeyspaceStatus::Complete;
    tab.tree.rebuild(&tab.keyspace.keys);
    app.tabs.push(WorkspaceTab::RedisBrowser(tab));
    app.tabs.push(WorkspaceTab::Sql(ConsoleTab::new("third")));
    app.active_tab = 1;
    app.focus = Focus::Results;
    app
}

fn redis_tab(app: &App) -> &RedisBrowserTab {
    let WorkspaceTab::RedisBrowser(tab) = &app.tabs[1] else {
        panic!("Redis tab must remain at index 1");
    };
    tab
}

fn render(app: &App) -> (UiState, Rect) {
    let area = Rect::new(0, 0, 180, 40);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let mut state = UiState::with_motion(MotionMode::Off);
    terminal
        .draw(|frame| ui::render_with_state(frame, app, &mut state))
        .unwrap();
    let content = AppLayout::calculate(area, app.focus, true, app.pane_sizes, app.pane_maximized)
        .relation
        .unwrap();
    let keys = RedisBrowserLayout::calculate(content, app.pane_sizes.redis_keys_width).keys;
    (state, keys)
}

fn click(app: &mut App, state: &UiState, column: u16, row: u16) {
    let action = map_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        state,
        app,
    )
    .expect("click should map to an action");
    app.update(action);
}

#[test]
fn redis_keys_mouse_clicks_focus_the_panel_without_opening_values() {
    for origin in [Focus::Explorer, Focus::Results] {
        for target in ["leaf", "group", "toggle", "title", "blank", "empty"] {
            let mut app = redis_app();
            app.focus = origin;
            if let WorkspaceTab::RedisBrowser(tab) = &mut app.tabs[1] {
                tab.focus = RedisBrowserFocus::Preview;
                if target == "empty" {
                    tab.keyspace.keys.clear();
                    tab.tree.rebuild(&[]);
                    tab.keyspace.status = KeyspaceStatus::CompleteEmpty;
                }
            }
            let (state, keys) = render(&app);
            let (column, row) = match target {
                "title" => (keys.x + 2, keys.y),
                "blank" | "empty" => (keys.x + 2, keys.bottom() - 3),
                _ => {
                    let region = state
                        .hit_regions
                        .iter()
                        .find(|region| match (&region.target, target) {
                            (HitTarget::RedisKeyNode { node, .. }, "leaf") => {
                                *node == KeyTreeNodeId::Key(b"plain".to_vec())
                            }
                            (HitTarget::RedisKeyNode { node, .. }, "group") => {
                                *node == KeyTreeNodeId::Prefix(b"group:".to_vec())
                            }
                            (HitTarget::RedisKeyToggle { .. }, "toggle") => true,
                            _ => false,
                        })
                        .unwrap();
                    (
                        region.area.x + u16::from(target != "toggle") * 3,
                        region.area.y,
                    )
                }
            };

            click(&mut app, &state, column, row);

            assert_eq!(app.focus, Focus::Results, "{origin:?}, {target}");
            assert_eq!(
                redis_tab(&app).focus,
                RedisBrowserFocus::Keys,
                "{origin:?}, {target}"
            );
            assert!(redis_tab(&app).opened_key.is_none(), "{origin:?}, {target}");
        }
    }
}

#[test]
fn redis_key_double_click_still_opens_and_focuses_value() {
    let mut app = redis_app();
    let (state, _) = render(&app);
    let region = state
        .hit_regions
        .iter()
        .find(|region| matches!(&region.target,
            HitTarget::RedisKeyNode { node, .. } if *node == KeyTreeNodeId::Key(b"plain".to_vec())
        ))
        .unwrap();

    click(&mut app, &state, region.area.x + 3, region.area.y);
    assert_eq!(redis_tab(&app).focus, RedisBrowserFocus::Keys);
    assert!(redis_tab(&app).opened_key.is_none());
    click(&mut app, &state, region.area.x + 3, region.area.y);

    assert_eq!(app.focus, Focus::Results);
    assert_eq!(redis_tab(&app).focus, RedisBrowserFocus::Preview);
    assert_eq!(redis_tab(&app).opened_key.as_ref().unwrap().key, b"plain");
}

#[test]
fn redis_background_node_actions_preserve_active_focus() {
    let mut app = redis_app();
    app.active_tab = 0;
    app.focus = Focus::Editor;
    if let WorkspaceTab::RedisBrowser(tab) = &mut app.tabs[1] {
        tab.focus = RedisBrowserFocus::Preview;
    }
    for action in [
        Action::SelectRedisNode {
            tab_id: Uuid::from_u128(1),
            node: Some(KeyTreeNodeId::Key(b"plain".to_vec())),
        },
        Action::RedisToggleNode {
            tab_id: Uuid::from_u128(1),
            node: KeyTreeNodeId::Prefix(b"group:".to_vec()),
        },
    ] {
        app.update(action);
        assert_eq!(app.active_tab, 0);
        assert_eq!(app.focus, Focus::Editor);
        assert_eq!(redis_tab(&app).focus, RedisBrowserFocus::Preview);
    }
}

#[test]
fn redis_keys_tab_sequences_switch_workspace_tabs() {
    for confirmed_find in [false, true] {
        for (sequence, expected_action, expected_tab) in [
            ("gt", Action::NextTab, 2),
            ("gT", Action::PreviousTab, 0),
            ("]t", Action::NextTab, 2),
            ("[t", Action::PreviousTab, 0),
        ] {
            let mut app = redis_app();
            if confirmed_find {
                app.update(Action::RedisFindOpen);
                app.update(Action::RedisFindConfirm);
            }
            let mut keymap = Keymap::default();
            let mut action = None;
            for character in sequence.chars() {
                let event = if character == 'T' {
                    KeyEvent::new(KeyCode::Char(character), KeyModifiers::SHIFT)
                } else {
                    key(character)
                };
                action = keymap.map(event, &app);
            }
            assert_eq!(
                action,
                Some(expected_action),
                "{sequence}, find={confirmed_find}"
            );
            app.update(action.unwrap());
            assert_eq!(app.active_tab, expected_tab);
            assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        }
    }
}

#[test]
fn redis_keys_tab_sequences_follow_custom_and_disabled_bindings() {
    for (sequence, expected) in [
        ("gn", Some(Action::NextTab)),
        ("gp", Some(Action::PreviousTab)),
        ("gt", None),
        ("gT", None),
        ("]t", None),
        ("[t", None),
    ] {
        let app = redis_app();
        let mut config = AppConfig::default();
        config
            .keybindings
            .global
            .insert("next-tab".into(), vec!["g n".into()]);
        config
            .keybindings
            .global
            .insert("previous-tab".into(), vec!["g p".into()]);
        let bindings = config.keybindings.key_bindings().unwrap();
        let mut keymap =
            Keymap::with_sequence_timeout_and_bindings(Duration::from_millis(750), bindings);
        let mut action = None;
        for character in sequence.chars() {
            action = keymap.map(key(character), &app);
        }
        assert_eq!(action, expected, "{sequence}");
    }

    let app = redis_app();
    let mut config = AppConfig::default();
    for command in ["next-tab", "previous-tab"] {
        config.keybindings.global.insert(command.into(), Vec::new());
    }
    let bindings = config.keybindings.key_bindings().unwrap();
    let mut keymap =
        Keymap::with_sequence_timeout_and_bindings(Duration::from_millis(750), bindings);
    for character in ['g', '[', ']'] {
        assert_eq!(keymap.map(key(character), &app), None);
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
    }
}

#[test]
fn redis_keys_search_keeps_tab_sequence_characters_as_text() {
    let mut app = redis_app();
    app.update(Action::RedisFindOpen);
    let mut keymap = Keymap::default();
    for character in "gtgT[t]t".chars() {
        let action = keymap.map(key(character), &app);
        assert_eq!(action, Some(Action::RedisFindInsert(character)));
        app.update(action.unwrap());
    }
    assert_eq!(app.active_tab, 1);
    assert_eq!(
        redis_tab(&app).find.as_ref().unwrap().query.value(),
        "gtgT[t]t"
    );
}

#[test]
fn redis_tab_key_cycles_explorer_keys_and_value() {
    let mut app = redis_app();
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();
    for (code, focus, pane) in [
        (KeyCode::Tab, Focus::Results, RedisBrowserFocus::Keys),
        (KeyCode::Tab, Focus::Results, RedisBrowserFocus::Preview),
        (KeyCode::Tab, Focus::Explorer, RedisBrowserFocus::Preview),
        (KeyCode::BackTab, Focus::Results, RedisBrowserFocus::Preview),
        (KeyCode::BackTab, Focus::Results, RedisBrowserFocus::Keys),
        (KeyCode::BackTab, Focus::Explorer, RedisBrowserFocus::Keys),
    ] {
        let action = keymap
            .map(KeyEvent::new(code, KeyModifiers::NONE), &app)
            .expect("Tab should move pane focus");
        app.update(action);
        assert_eq!(app.active_tab, 1);
        assert_eq!(app.focus, focus);
        assert_eq!(redis_tab(&app).focus, pane);
    }
}
