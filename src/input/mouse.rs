use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::{
    action::Action,
    app::App,
    model::workspace::{Focus, Overlay},
    ui::{HitTarget, ProfileButton, UiState},
};

pub fn map_mouse(event: MouseEvent, ui: &UiState, app: &App) -> Option<Action> {
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let target = ui.target_at(event.column, event.row)?.clone();
            if let Some(overlay) = &app.overlay
                && (overlay != &Overlay::ProfileManager
                    || !matches!(
                        target,
                        HitTarget::ProfileRow(_)
                            | HitTarget::ProfileField(_)
                            | HitTarget::ProfileToggle(_)
                            | HitTarget::ProfileButton(_)
                    ))
            {
                return None;
            }
            match target {
                HitTarget::Focus(focus) => Some(Action::Focus(focus)),
                HitTarget::Tab(index) => Some(Action::ActivateTab(index)),
                HitTarget::ExplorerRow(index) => Some(Action::ExplorerSelect(index)),
                HitTarget::ResultCell { row, column } => Some(Action::GridSelect { row, column }),
                HitTarget::Help => Some(Action::ShowHelp),
                HitTarget::ToggleResultView => Some(Action::ToggleResultView),
                HitTarget::HeaderProfile => Some(Action::OpenProfileManager),
                HitTarget::ProfileRow(index) => profile_row_action(index, app),
                HitTarget::ProfileField(field) => Some(Action::ProfileFocusField(field)),
                HitTarget::ProfileToggle(field) => Some(Action::ProfileToggleField(field)),
                HitTarget::ProfileButton(button) => Some(profile_button_action(button)),
            }
        }
        MouseEventKind::ScrollDown => {
            if let Some(overlay) = &app.overlay {
                return (overlay == &Overlay::ProfileManager
                    && matches!(
                        ui.target_at(event.column, event.row),
                        Some(HitTarget::ProfileRow(_))
                    ))
                .then_some(Action::ProfileMove(3));
            }
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
            if let Some(overlay) = &app.overlay {
                return (overlay == &Overlay::ProfileManager
                    && matches!(
                        ui.target_at(event.column, event.row),
                        Some(HitTarget::ProfileRow(_))
                    ))
                .then_some(Action::ProfileMove(-3));
            }
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

fn profile_row_action(index: usize, app: &App) -> Option<Action> {
    let selected = app.profile_manager.as_ref()?.selected;
    let delta = if index >= selected {
        isize::try_from(index - selected).unwrap_or(isize::MAX)
    } else {
        -isize::try_from(selected - index).unwrap_or(isize::MAX)
    };
    Some(Action::ProfileMove(delta))
}

fn profile_button_action(button: ProfileButton) -> Action {
    match button {
        ProfileButton::New => Action::ProfileStartNew,
        ProfileButton::Edit => Action::ProfileStartEdit,
        ProfileButton::Delete => Action::ProfileRequestDelete,
        ProfileButton::Connect => Action::ProfileConnectSelected,
        ProfileButton::Close | ProfileButton::Cancel => Action::CloseProfileManager,
        ProfileButton::Test => Action::ProfileTest,
        ProfileButton::Save => Action::ProfileSave { connect: false },
        ProfileButton::SaveAndConnect => Action::ProfileSave { connect: true },
        ProfileButton::ConfirmDelete => Action::ProfileConfirmDelete,
        ProfileButton::CancelDelete => Action::ProfileCancelDelete,
    }
}

fn focus_at(ui: &UiState, column: u16, row: u16) -> Option<Focus> {
    match ui.target_at(column, row)? {
        HitTarget::Focus(focus) => Some(*focus),
        HitTarget::ExplorerRow(_) => Some(Focus::Explorer),
        HitTarget::ResultCell { .. } | HitTarget::ToggleResultView => Some(Focus::Results),
        HitTarget::Tab(_)
        | HitTarget::Help
        | HitTarget::HeaderProfile
        | HitTarget::ProfileRow(_)
        | HitTarget::ProfileField(_)
        | HitTarget::ProfileToggle(_)
        | HitTarget::ProfileButton(_) => None,
    }
}
