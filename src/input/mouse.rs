use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::time::Instant;

use crate::{
    action::Action,
    app::App,
    model::{
        explorer::ExplorerScrollAmount,
        relation::RelationView,
        tab::{GridScrollAmount, WorkspaceTab},
        workspace::{Focus, Overlay},
    },
    ui::{HitTarget, ProfileButton, UiState},
};

pub fn map_mouse(event: MouseEvent, ui: &UiState, app: &App) -> Option<Action> {
    match event.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.overlay.is_some() {
                ui.relation_resize.borrow_mut().take();
                ui.grid_scrollbar_drag.borrow_mut().take();
                return None;
            }
            if let Some(drag) = *ui.grid_scrollbar_drag.borrow() {
                let travel = drag.track_width.saturating_sub(drag.thumb_width);
                let pointer = event
                    .column
                    .saturating_sub(drag.track_x)
                    .saturating_sub(drag.pointer_offset)
                    .min(travel);
                let offset = if travel == 0 {
                    0
                } else {
                    (pointer as usize * drag.max_offset + travel as usize / 2) / travel as usize
                };
                return Some(Action::GridSetColumnOffset { offset });
            }
            let (column, start_width, start_x) = (*ui.relation_resize.borrow())?;
            Some(Action::GridSetColumnWidth {
                column,
                width: start_width.saturating_add_signed(event.column as i16 - start_x as i16),
            })
        }
        MouseEventKind::Up(MouseButton::Left) => {
            ui.relation_resize.borrow_mut().take();
            ui.grid_scrollbar_drag.borrow_mut().take();
            Some(Action::GridEndColumnResize)
        }
        MouseEventKind::Down(MouseButton::Left) => {
            ui.relation_resize.borrow_mut().take();
            ui.grid_scrollbar_drag.borrow_mut().take();
            let Some(target) = ui.target_at(event.column, event.row).cloned() else {
                ui.clear_click_tracker();
                return None;
            };
            if !matches!(target, HitTarget::ExplorerRow(_)) {
                ui.clear_click_tracker();
            }
            if let Some(overlay) = &app.overlay
                && (overlay != &Overlay::ProfileManager
                    && overlay != &Overlay::CatalogEditor
                    && !matches!(overlay, Overlay::TargetSelector { .. })
                    && !matches!(overlay, Overlay::TransactionMenu { .. })
                    && !matches!(overlay, Overlay::TransactionExitConfirm { .. })
                    && !matches!(overlay, Overlay::Update(_))
                    || !matches!(
                        target,
                        HitTarget::ProfileField(_)
                            | HitTarget::ProfileDriver(_)
                            | HitTarget::ProfileToggle(_)
                            | HitTarget::ProfileScopeRow(_)
                            | HitTarget::ProfileButton(_)
                            | HitTarget::ProfileGroupOption(_)
                            | HitTarget::ProfileGroupConfirm
                            | HitTarget::ProfileGroupCancel
                            | HitTarget::ExplorerAddOption(_)
                            | HitTarget::CatalogEditorField(_)
                            | HitTarget::CatalogEditorFormField(_)
                            | HitTarget::CatalogEditorTableField(_)
                            | HitTarget::CatalogEditorTableColumn(_)
                            | HitTarget::CatalogEditorAddTableColumn
                            | HitTarget::CatalogEditorRemoveTableColumn
                            | HitTarget::CatalogEditorReview
                            | HitTarget::CatalogEditorCancel
                            | HitTarget::CatalogEditorColumnDetailsConfirm
                            | HitTarget::CatalogEditorColumnDetailsCancel
                            | HitTarget::CatalogOwnerChoice(_)
                            | HitTarget::TargetSelectorRow(_)
                            | HitTarget::TargetSelectorCancel
                            | HitTarget::EditorExecutionTarget
                            | HitTarget::EditorTransactionMenu
                            | HitTarget::TransactionMenuItem(_)
                            | HitTarget::TransactionMenuCancel
                            | HitTarget::TransactionExitChoice(_)
                            | HitTarget::TransactionExitCancel
                            | HitTarget::UpdateButton { .. }
                    ))
            {
                return None;
            }
            let details_open = app
                .catalog_editor
                .as_ref()
                .and_then(|editor| editor.draft.as_ref())
                .is_some_and(|draft| {
                    matches!(
                        draft,
                        crate::model::catalog_editor::CatalogDraft::Table(table)
                            if table.column_editor.is_some()
                    )
                });
            if details_open
                && !matches!(
                    target,
                    HitTarget::CatalogEditorTableField(
                        crate::model::catalog_editor::TableEditorFocus::ColumnDetails(_)
                    ) | HitTarget::CatalogEditorColumnDetailsConfirm
                        | HitTarget::CatalogEditorColumnDetailsCancel
                )
            {
                return None;
            }
            match target {
                HitTarget::Focus(focus) => Some(Action::Focus(focus)),
                HitTarget::Tab(index) => Some(Action::ActivateTab(index)),
                HitTarget::TabScrollLeft(index) | HitTarget::TabScrollRight(index) => {
                    Some(Action::ActivateTab(index))
                }
                HitTarget::CloseTab(id) => Some(Action::CloseTab(id)),
                HitTarget::DismissNotification(id) => Some(Action::DismissNotification(id)),
                HitTarget::ExplorerRow(id) => {
                    if ui.track_explorer_click(&id, Instant::now()) {
                        Some(Action::ExplorerPrimary)
                    } else {
                        Some(Action::ExplorerSelect(id))
                    }
                }
                HitTarget::ResultCell { row, column } => Some(Action::GridSelect { row, column }),
                HitTarget::Help => Some(Action::ShowHelp),
                HitTarget::UpdateCenter => Some(Action::OpenUpdateCenter),
                HitTarget::UpdateButton { primary } => {
                    if primary {
                        Some(Action::UpdateOverlayConfirm)
                    } else {
                        Some(Action::DismissOverlay)
                    }
                }
                HitTarget::ToggleResultView => Some(Action::ToggleResultView),
                HitTarget::ResultView(view) => Some(Action::SetResultView(view)),
                HitTarget::RelationView(view) => Some(Action::SetRelationView(view)),
                HitTarget::DashboardView(page) => Some(Action::DashboardSetPage(page)),
                HitTarget::RelationRetry => Some(Action::RefreshActiveRelation),
                HitTarget::RelationCancel => Some(Action::CancelActiveRelationRequest),
                HitTarget::DataQueryInput(input) => Some(Action::FocusDataQueryInput(input)),
                HitTarget::RelationColumnResize { column, width } => {
                    *ui.relation_resize.borrow_mut() = Some((column, width, event.column));
                    Some(Action::GridStartColumnResize { column, width })
                }
                HitTarget::GridScrollbarThumb {
                    track_x,
                    track_width,
                    thumb_x,
                    thumb_width,
                    offset,
                    max_offset,
                } => {
                    *ui.grid_scrollbar_drag.borrow_mut() = Some(crate::ui::GridScrollbarDrag {
                        track_x,
                        track_width,
                        thumb_width,
                        pointer_offset: event.column.saturating_sub(thumb_x),
                        max_offset,
                    });
                    Some(Action::GridSetColumnOffset { offset })
                }
                HitTarget::GridScrollbarPage { offset } => {
                    Some(Action::GridSetColumnOffset { offset })
                }
                HitTarget::HeaderProfile => app.connection.profile_id.map_or(
                    Some(Action::Focus(Focus::Explorer)),
                    |profile_id| {
                        Some(Action::ExplorerSelect(
                            crate::model::explorer::ExplorerNodeId::Profile(profile_id),
                        ))
                    },
                ),
                HitTarget::ProfileField(field) => Some(Action::ProfileFocusField(field)),
                HitTarget::ProfileDriver(kind) => Some(Action::ProfileSelectDriver(kind)),
                HitTarget::ProfileToggle(field) => Some(Action::ProfileToggleField(field)),
                HitTarget::ProfileScopeRow(id) => Some(Action::ProfileToggleScopeRow(id)),
                HitTarget::ProfileButton(button) => Some(profile_button_action(button)),
                HitTarget::ProfileGroupOption(index) => Some(Action::ProfileGroupSelect(index)),
                HitTarget::ProfileGroupConfirm => Some(Action::ProfileGroupConfirm),
                HitTarget::ProfileGroupCancel => Some(Action::ProfileGroupCancel),
                HitTarget::ExplorerAddOption(index) => Some(Action::ExplorerAddSelect(index)),
                HitTarget::CatalogEditorField(index) => {
                    Some(Action::CatalogEditorFocusField(index))
                }
                HitTarget::CatalogEditorFormField(field) => {
                    Some(Action::CatalogEditorFocusFormField(field))
                }
                HitTarget::CatalogEditorTableField(field) => {
                    Some(Action::CatalogEditorFocusTableField(field))
                }
                HitTarget::CatalogEditorTableColumn(index) => {
                    Some(Action::CatalogEditorSelectTableColumn(index))
                }
                HitTarget::CatalogEditorAddTableColumn => Some(Action::CatalogEditorAddTableColumn),
                HitTarget::CatalogEditorRemoveTableColumn => {
                    Some(Action::CatalogEditorRemoveTableColumn)
                }
                HitTarget::CatalogEditorReview => Some(Action::CatalogEditorPreview),
                HitTarget::CatalogEditorCancel => Some(Action::CatalogEditorCancel),
                HitTarget::CatalogEditorColumnDetailsConfirm => {
                    Some(Action::CatalogEditorConfirmTableColumnDetails)
                }
                HitTarget::CatalogEditorColumnDetailsCancel => {
                    Some(Action::CatalogEditorCancelTableColumnDetails)
                }
                HitTarget::CatalogOwnerChoice(name) => Some(Action::CatalogOwnerPickerChoose(name)),
                HitTarget::TargetSelectorRow(index) => Some(Action::SelectTargetSelector(index)),
                HitTarget::TargetSelectorCancel => Some(Action::CancelTargetSelector),
                HitTarget::EditorExecutionTarget => Some(Action::OpenTargetSelector),
                HitTarget::EditorTransactionMenu => Some(Action::OpenTransactionMenu),
                HitTarget::TransactionMenuItem(index) => Some(Action::SelectTransactionMenu(index)),
                HitTarget::TransactionMenuCancel => Some(Action::CancelTransactionMenu),
                HitTarget::TransactionExitChoice(choice) => {
                    Some(Action::ConfirmTransactionExitChoice(choice))
                }
                HitTarget::TransactionExitCancel => Some(Action::CancelTransactionExit),
                HitTarget::RelationFirstPage => Some(Action::RelationFirstPage),
                HitTarget::RelationPreviousPage => Some(Action::RelationPreviousPage),
                HitTarget::RelationPageSize => {
                    Some(Action::OpenPageSizeSelector { relation: true })
                }
                HitTarget::RelationNextPage => Some(Action::RelationNextPage),
                HitTarget::RelationLastPage => Some(Action::RelationLastPage),
                HitTarget::ResultFirstPage => Some(Action::ResultFirstPage),
                HitTarget::ResultPreviousPage => Some(Action::ResultPreviousPage),
                HitTarget::ResultPageSize => Some(Action::OpenPageSizeSelector { relation: false }),
                HitTarget::ResultNextPage => Some(Action::ResultNextPage),
                HitTarget::ResultLastPage => Some(Action::ResultLastPage),
            }
        }
        MouseEventKind::Down(MouseButton::Right) => {
            if app.overlay.is_some() {
                return None;
            }
            match ui.target_at(event.column, event.row)?.clone() {
                HitTarget::ExplorerRow(crate::model::explorer::ExplorerNodeId::Profile(_))
                | HitTarget::ExplorerRow(crate::model::explorer::ExplorerNodeId::Catalog(_)) => {
                    Some(Action::OpenCatalogEdit)
                }
                _ => None,
            }
        }
        MouseEventKind::ScrollDown => {
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerScrollNodes {
                    direction: 1,
                    amount: ExplorerScrollAmount::Lines(3),
                }),
                Focus::Results if is_relation_ddl_focus(app) => ddl_scroll_action(app, 3),
                Focus::Results => Some(Action::GridScrollRows {
                    direction: 1,
                    amount: GridScrollAmount::Lines(3),
                }),
                Focus::Editor => Some(Action::EditorScroll {
                    rows: 3,
                    columns: 0,
                }),
            }
        }
        MouseEventKind::ScrollUp => {
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerScrollNodes {
                    direction: -1,
                    amount: ExplorerScrollAmount::Lines(3),
                }),
                Focus::Results if is_relation_ddl_focus(app) => ddl_scroll_action(app, -3),
                Focus::Results => Some(Action::GridScrollRows {
                    direction: -1,
                    amount: GridScrollAmount::Lines(3),
                }),
                Focus::Editor => Some(Action::EditorScroll {
                    rows: -3,
                    columns: 0,
                }),
            }
        }
        MouseEventKind::ScrollLeft => {
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Results if is_relation_ddl_focus(app) => {
                    ddl_horizontal_scroll_action(app, -3)
                }
                Focus::Results => grid_horizontal_scroll_action(ui, false),
                Focus::Editor => Some(Action::EditorScroll {
                    rows: 0,
                    columns: -3,
                }),
                Focus::Explorer => None,
            }
        }
        MouseEventKind::ScrollRight => {
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Results if is_relation_ddl_focus(app) => {
                    ddl_horizontal_scroll_action(app, 3)
                }
                Focus::Results => grid_horizontal_scroll_action(ui, true),
                Focus::Editor => Some(Action::EditorScroll {
                    rows: 0,
                    columns: 3,
                }),
                Focus::Explorer => None,
            }
        }
        _ => None,
    }
}

fn profile_button_action(button: ProfileButton) -> Action {
    match button {
        ProfileButton::Cancel => Action::CloseProfileManager,
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
        HitTarget::ResultCell { .. }
        | HitTarget::ToggleResultView
        | HitTarget::ResultView(_)
        | HitTarget::RelationView(_)
        | HitTarget::DashboardView(_)
        | HitTarget::RelationRetry
        | HitTarget::DataQueryInput(_)
        | HitTarget::RelationColumnResize { .. }
        | HitTarget::GridScrollbarThumb { .. }
        | HitTarget::GridScrollbarPage { .. } => Some(Focus::Results),
        HitTarget::RelationCancel => Some(Focus::Results),
        HitTarget::Tab(_)
        | HitTarget::TabScrollLeft(_)
        | HitTarget::TabScrollRight(_)
        | HitTarget::CloseTab(_)
        | HitTarget::DismissNotification(_)
        | HitTarget::Help
        | HitTarget::UpdateCenter
        | HitTarget::UpdateButton { .. }
        | HitTarget::HeaderProfile
        | HitTarget::ProfileField(_)
        | HitTarget::ProfileDriver(_)
        | HitTarget::ProfileToggle(_)
        | HitTarget::ProfileScopeRow(_)
        | HitTarget::ProfileButton(_)
        | HitTarget::ProfileGroupOption(_)
        | HitTarget::ProfileGroupConfirm
        | HitTarget::ProfileGroupCancel
        | HitTarget::ExplorerAddOption(_)
        | HitTarget::CatalogEditorField(_)
        | HitTarget::CatalogEditorFormField(_)
        | HitTarget::CatalogEditorTableField(_)
        | HitTarget::CatalogEditorTableColumn(_)
        | HitTarget::CatalogEditorAddTableColumn
        | HitTarget::CatalogEditorRemoveTableColumn
        | HitTarget::CatalogEditorReview
        | HitTarget::CatalogEditorCancel
        | HitTarget::CatalogEditorColumnDetailsConfirm
        | HitTarget::CatalogEditorColumnDetailsCancel
        | HitTarget::CatalogOwnerChoice(_) => None,
        HitTarget::TargetSelectorRow(_)
        | HitTarget::TargetSelectorCancel
        | HitTarget::EditorExecutionTarget
        | HitTarget::EditorTransactionMenu
        | HitTarget::TransactionMenuItem(_)
        | HitTarget::TransactionMenuCancel => None,
        HitTarget::TransactionExitChoice(_) | HitTarget::TransactionExitCancel => None,
        HitTarget::RelationFirstPage
        | HitTarget::RelationPreviousPage
        | HitTarget::RelationPageSize
        | HitTarget::RelationNextPage
        | HitTarget::RelationLastPage
        | HitTarget::ResultFirstPage
        | HitTarget::ResultPreviousPage
        | HitTarget::ResultPageSize
        | HitTarget::ResultNextPage
        | HitTarget::ResultLastPage => Some(Focus::Results),
    }
}

fn is_relation_ddl_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && matches!(
            app.tabs.get(app.active_tab),
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Ddl
        )
}

fn ddl_scroll_action(app: &App, rows: isize) -> Option<Action> {
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        return None;
    };
    Some(Action::ReadOnlyEditorScroll {
        session_id: tab.ddl_editor_id,
        rows,
        columns: 0,
    })
}

fn ddl_horizontal_scroll_action(app: &App, columns: isize) -> Option<Action> {
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        return None;
    };
    Some(Action::ReadOnlyEditorScroll {
        session_id: tab.ddl_editor_id,
        rows: 0,
        columns,
    })
}

fn grid_horizontal_scroll_action(ui: &UiState, right: bool) -> Option<Action> {
    let targets = ui.grid_horizontal_scroll?;
    let target = if right { targets.right } else { targets.left };
    Some(Action::GridScrollColumns {
        offset: target.offset,
        first_visible: target.first_visible,
        last_visible: target.last_visible,
    })
}
