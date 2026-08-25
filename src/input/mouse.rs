use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::{
    action::Action,
    app::App,
    model::workspace::Focus,
    ui::{HitTarget, UiState},
};

pub fn map_mouse(event: MouseEvent, ui: &UiState, app: &App) -> Option<Action> {
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            match ui.target_at(event.column, event.row)?.clone() {
                HitTarget::Focus(focus) => Some(Action::Focus(focus)),
                HitTarget::Tab(index) => Some(Action::ActivateTab(index)),
                HitTarget::ExplorerRow(index) => Some(Action::ExplorerSelect(index)),
                HitTarget::ResultCell { row, column } => Some(Action::GridSelect { row, column }),
                HitTarget::Help => Some(Action::ShowHelp),
                HitTarget::ToggleResultView => Some(Action::ToggleResultView),
            }
        }
        MouseEventKind::ScrollDown => {
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerMove(3)),
                Focus::Results => Some(Action::GridMove {
                    rows: 3,
                    columns: 0,
                }),
                Focus::Editor => Some(Action::MoveDown),
            }
        }
        MouseEventKind::ScrollUp => {
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerMove(-3)),
                Focus::Results => Some(Action::GridMove {
                    rows: -3,
                    columns: 0,
                }),
                Focus::Editor => Some(Action::MoveUp),
            }
        }
        _ => None,
    }
}

fn focus_at(ui: &UiState, column: u16, row: u16) -> Option<Focus> {
    match ui.target_at(column, row)? {
        HitTarget::Focus(focus) => Some(*focus),
        HitTarget::ExplorerRow(_) => Some(Focus::Explorer),
        HitTarget::ResultCell { .. } | HitTarget::ToggleResultView => Some(Focus::Results),
        HitTarget::Tab(_) | HitTarget::Help => None,
    }
}
