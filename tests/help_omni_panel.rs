use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use lazydb::{
    action::Action,
    app::App,
    cli::MotionMode,
    config::HelpPanelView,
    input::keymap::Keymap,
    input::mouse::map_mouse,
    model::workspace::Overlay,
    ui::{self, HitTarget, UiState, theme::Theme},
};
use ratatui::{Terminal, backend::TestBackend};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn help_shortcut_opens_help_and_tab_switches_to_omni() {
    let mut app = App::new(Vec::new());
    let mut keymap = Keymap::default();

    app.update(Action::ShowHelp);
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::Help(_))
    ));

    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::ToggleHelpPanel)
    );
    app.update(Action::ToggleHelpPanel);
    assert!(app.overlay.is_none());
    assert!(app.omni.is_some());

    app.update(Action::ToggleHelpPanel);
    assert!(app.omni.is_none());
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::Help(_))
    ));
}

#[test]
fn configured_default_panel_opens_omni() {
    let mut app = App::new(Vec::new());
    app.set_help_panel_view(HelpPanelView::Omni);

    app.update(Action::ShowHelp);

    assert!(app.overlay.is_none());
    assert!(app.omni.is_some());
}

#[test]
fn dismissing_help_does_not_retain_a_hidden_omni_session() {
    let mut app = App::new(Vec::new());
    app.update(Action::ShowHelp);
    app.update(Action::ToggleHelpPanel);
    assert!(app.omni.is_some());
    app.update(Action::ToggleHelpPanel);
    assert!(app.overlay.is_some());
    app.update(Action::DismissOverlay);

    app.update(Action::ShowHelp);
    assert!(app.omni.is_none());
    assert!(app.overlay.is_some());
}

#[test]
fn unified_panel_restores_the_real_overlay_after_closing() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::Message {
        title: "source".into(),
        body: "body".into(),
    });
    app.update(Action::ShowHelp);
    assert!(matches!(app.overlay, Some(Overlay::Help(_))));

    app.update(Action::ToggleHelpPanel);
    app.update(Action::OmniDismiss);

    assert!(matches!(app.overlay, Some(Overlay::Message { .. })));
}

#[test]
fn help_and_omni_render_toggle_targets_and_help_rows_are_selectable() {
    let mut app = App::new(Vec::new());
    app.update(Action::ShowHelp);
    let mut state = UiState::new();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    let toggle = state
        .hit_regions
        .iter()
        .find(|region| region.target == HitTarget::HelpTogglePanel)
        .unwrap();
    assert_eq!(
        map_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: toggle.area.x,
                row: toggle.area.y,
                modifiers: crossterm::event::KeyModifiers::NONE
            },
            &state,
            &app
        ),
        Some(Action::ToggleHelpPanel)
    );
    let row = state
        .hit_regions
        .iter()
        .find(|region| matches!(region.target, HitTarget::HelpItem(_)))
        .unwrap();
    assert!(matches!(
        map_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: row.area.x,
                row: row.area.y,
                modifiers: crossterm::event::KeyModifiers::NONE
            },
            &state,
            &app
        ),
        Some(Action::HelpSelect(_))
    ));
}

#[test]
fn help_shortcut_columns_stay_fixed_when_selection_moves() {
    let mut app = App::new(Vec::new());
    app.update(Action::ShowHelp);
    let mut state = UiState::with_motion(MotionMode::Off);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    for (delta, selected) in [(0, 0), (1, 1), (-1, 0)] {
        app.update(Action::HelpMove(delta));
        terminal
            .draw(|frame| ui::render_with_state(frame, &app, &mut state))
            .unwrap();

        for (index, shortcut, description) in [
            (0, "F1, ?", "open this help panel"),
            (1, "F2", "open Omni search"),
        ] {
            let row = state
                .hit_regions
                .iter()
                .find(|region| region.target == HitTarget::HelpItem(index))
                .unwrap()
                .area;
            let buffer = terminal.backend().buffer();
            let text = (row.x..row.right())
                .map(|column| buffer[(column, row.y)].symbol())
                .collect::<String>();

            assert_eq!(text.find(shortcut), Some(2), "{text:?}");
            assert_eq!(text.find(description), Some(20), "{text:?}");
            assert_eq!(
                buffer[(row.x, row.y)].symbol(),
                if index == selected { ">" } else { " " }
            );
            assert_eq!(buffer[(row.x + 1, row.y)].symbol(), " ");
        }
    }
}

#[test]
fn help_selection_highlight_covers_the_whole_row_and_moves_with_selection() {
    let mut app = App::new(Vec::new());
    app.update(Action::ShowHelp);
    let mut state = UiState::with_motion(MotionMode::Off);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let theme = Theme::default();

    for selected in [0, 1] {
        app.update(Action::HelpSelect(selected));
        terminal
            .draw(|frame| ui::render_with_state(frame, &app, &mut state))
            .unwrap();

        for index in 0..2 {
            let row = state
                .hit_regions
                .iter()
                .find(|region| region.target == HitTarget::HelpItem(index))
                .unwrap()
                .area;
            let buffer = terminal.backend().buffer();
            let background = if index == selected {
                theme.selection
            } else {
                theme.surface_raised
            };

            for column in row.x..row.right() {
                assert_eq!(
                    buffer[(column, row.y)].bg,
                    background,
                    "help row {index}, column {column}, selected {selected}"
                );
            }
            if index == selected {
                assert_eq!(buffer[(row.x, row.y)].fg, theme.accent);
            }
            assert_eq!(buffer[(row.x + 2, row.y)].fg, theme.action);
        }
    }
}

#[test]
fn rendered_panel_scrollbar_targets_map_to_panel_actions() {
    let mut app = App::new(Vec::new());
    app.update(Action::ShowHelp);
    let mut state = UiState::new();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    let help_bar = state.hit_regions.iter().find(|region| {
        matches!(
            region.target,
            HitTarget::HelpScrollbarPage { .. } | HitTarget::HelpScrollbarThumb { .. }
        )
    });
    if let Some(region) = help_bar {
        let action = map_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: region.area.x,
                row: region.area.y,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
            &state,
            &app,
        );
        assert!(matches!(action, Some(Action::HelpSetScroll(_))));
    }
}
