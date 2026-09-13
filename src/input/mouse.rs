use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::time::Instant;

use crate::{
    action::Action,
    app::App,
    model::{
        explorer::ExplorerScrollAmount,
        relation::RelationView,
        tab::{GridScrollAmount, WorkspaceTab},
        workspace::{Focus, Overlay, PaneSplit},
    },
    ui::{HitTarget, PaneResizeDrag, ProfileButton, UiState},
};

fn editor_position(
    position: crate::ui::text_selection::TextPosition,
) -> crate::model::editor::EditorPosition {
    crate::model::editor::EditorPosition {
        line: position.line,
        column: position.column,
    }
}

fn input_position_clamped(
    maps: &[(
        &crate::ui::text_selection::InputSelectionTarget,
        &crate::ui::text_selection::InputHitMap,
    )],
    column: u16,
    row: u16,
) -> Option<usize> {
    let first = maps
        .iter()
        .map(|(_, map)| *map)
        .min_by_key(|map| map.area.y)?;
    let last = maps
        .iter()
        .map(|(_, map)| *map)
        .max_by_key(|map| map.area.bottom())?;
    let line = maps
        .iter()
        .find(|(_, map)| row >= map.area.y && row < map.area.bottom())
        .map(|(_, map)| *map)
        .or({
            if row < first.area.y {
                Some(first)
            } else {
                Some(last)
            }
        })?;
    line.source_at_horizontal_clamped(column)
}

fn text_position_clamped(
    maps: &[crate::ui::text_selection::TextHitMap],
    column: u16,
    row: u16,
) -> Option<crate::ui::text_selection::TextPosition> {
    let first = maps.iter().min_by_key(|map| map.area.y)?;
    let last = maps.iter().max_by_key(|map| map.area.bottom())?;
    let line = maps
        .iter()
        .find(|map| row >= map.area.y && row < map.area.bottom())
        .unwrap_or(if row < first.area.y { first } else { last });
    Some(line.source_at_horizontal_clamped(column))
}

pub fn map_mouse(event: MouseEvent, ui: &UiState, app: &App) -> Option<Action> {
    // Toasts own their visible cells, even above modal inputs and selection targets.
    if let Some(
        target @ (HitTarget::DismissNotification(_) | HitTarget::OpenNotificationHistoryAt(_)),
    ) = ui.target_at(event.column, event.row)
    {
        let input_target = ui
            .input_gesture
            .borrow()
            .as_ref()
            .map(|gesture| gesture.target.clone());
        ui.cancel_mouse_gesture();
        if let Some(target) = input_target {
            return Some(Action::CancelMouseInputSelection { target });
        }
        return if event.kind == MouseEventKind::Down(MouseButton::Left) {
            match target {
                HitTarget::DismissNotification(id) => Some(Action::DismissNotification(*id)),
                HitTarget::OpenNotificationHistoryAt(id) if app.overlay.is_none() => {
                    Some(Action::OpenNotificationHistoryAt(*id))
                }
                _ => None,
            }
        } else {
            None
        };
    }
    match event.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            if *ui.mouse_gesture.borrow() == Some(crate::ui::text_selection::GestureOwner::Input) {
                let target_id = ui
                    .input_gesture
                    .borrow()
                    .as_ref()
                    .map(|gesture| gesture.target.clone());
                let target_id = target_id?;
                let position = ui
                    .input_selection_targets
                    .iter()
                    .rev()
                    .filter(|(target, _)| *target == target_id)
                    .map(|(target, map)| (target, map))
                    .collect::<Vec<_>>();
                let position = input_position_clamped(&position, event.column, event.row);
                let Some(position) = position else {
                    let target = ui
                        .input_gesture
                        .borrow()
                        .as_ref()
                        .map(|gesture| gesture.target.clone());
                    ui.cancel_mouse_gesture();
                    if let Some(target) = target {
                        return Some(Action::CancelMouseInputSelection { target });
                    }
                    return None;
                };
                let mut gesture = ui.input_gesture.borrow_mut();
                let gesture = gesture.as_mut()?;
                gesture.end = position;
                gesture.has_dragged |= position != gesture.start;
                return Some(Action::UpdateMouseInputSelection {
                    target: gesture.target.clone(),
                    cursor: position,
                });
            }
            if matches!(app.overlay, Some(Overlay::TextDetail(_)))
                && *ui.mouse_gesture.borrow() == Some(crate::ui::text_selection::GestureOwner::Text)
            {
                let gesture = ui.text_gesture.borrow().as_ref().copied()?;
                let target = ui
                    .text_selection_targets
                    .iter()
                    .find(|target| target.session_id == gesture.session_id)?;
                if gesture.source != crate::ui::text_selection::TextGestureSource::TextDetail
                    || gesture.session_id != target.session_id
                {
                    ui.cancel_mouse_gesture();
                    return None;
                }
                let end = text_position_clamped(&target.hit_maps, event.column, event.row)?;
                ui.update_text_gesture(end);
                return None;
            }
            if app.overlay.is_some() {
                ui.relation_resize.borrow_mut().take();
                ui.grid_scrollbar_drag.borrow_mut().take();
                ui.pane_resize_drag.borrow_mut().take();
                let input_target = ui
                    .input_gesture
                    .borrow()
                    .as_ref()
                    .map(|gesture| gesture.target.clone());
                ui.cancel_mouse_gesture();
                if let Some(target) = input_target {
                    return Some(Action::CancelMouseInputSelection { target });
                }
                return None;
            }
            if *ui.mouse_gesture.borrow() == Some(crate::ui::text_selection::GestureOwner::Text) {
                let gesture = ui.text_gesture.borrow().as_ref().copied()?;
                let target = ui
                    .text_selection_targets
                    .iter()
                    .find(|target| target.session_id == gesture.session_id)?;
                if gesture.source != crate::ui::text_selection::TextGestureSource::Editor
                    || gesture.session_id != target.session_id
                {
                    ui.cancel_mouse_gesture();
                    return None;
                }
                let end = text_position_clamped(&target.hit_maps, event.column, event.row)?;
                ui.update_text_gesture(end);
                return None;
            }
            if let Some(drag) = *ui.pane_resize_drag.borrow() {
                return pane_resize_action(drag, pane_resize_pointer(drag.split, event), ui, app);
            }
            if let Some(drag) = *ui.grid_scrollbar_drag.borrow() {
                let pointer_position = match drag.axis {
                    crate::ui::GridScrollAxis::Horizontal => event.column,
                    crate::ui::GridScrollAxis::Vertical => event.row,
                };
                let rail = match drag.axis {
                    crate::ui::GridScrollAxis::Horizontal => {
                        ratatui::layout::Rect::new(drag.track_start, 0, drag.track_length, 1)
                    }
                    crate::ui::GridScrollAxis::Vertical => {
                        ratatui::layout::Rect::new(0, drag.track_start, 1, drag.track_length)
                    }
                };
                let offset = crate::ui::scrollbar::ScrollbarGeometry {
                    rail,
                    thumb_start: 0,
                    thumb_length: drag.thumb_length,
                    max_offset: drag.max_offset,
                }
                .offset_at(pointer_position, drag.pointer_offset);
                return Some(match drag.axis {
                    crate::ui::GridScrollAxis::Horizontal => Action::GridSetColumnOffset { offset },
                    crate::ui::GridScrollAxis::Vertical => Action::GridSetRowOffset { offset },
                });
            }
            if let Some(drag) = *ui.explorer_scrollbar_drag.borrow() {
                let offset = crate::ui::scrollbar::ScrollbarGeometry {
                    rail: ratatui::layout::Rect::new(0, drag.track_start, 1, drag.track_length),
                    thumb_start: 0,
                    thumb_length: drag.thumb_length,
                    max_offset: drag.max_offset,
                }
                .offset_at(event.row, drag.pointer_offset);
                return Some(Action::ExplorerSetScrollOffset(offset));
            }
            if let Some(drag) = *ui.editor_scrollbar_drag.borrow() {
                let pointer_position = if drag.vertical {
                    event.row
                } else {
                    event.column
                };
                let rail = if drag.vertical {
                    ratatui::layout::Rect::new(0, drag.track_start, 1, drag.track_length)
                } else {
                    ratatui::layout::Rect::new(drag.track_start, 0, drag.track_length, 1)
                };
                let offset = crate::ui::scrollbar::ScrollbarGeometry {
                    rail,
                    thumb_start: 0,
                    thumb_length: drag.thumb_length,
                    max_offset: drag.max_offset,
                }
                .offset_at(pointer_position, drag.pointer_offset);
                return Some(Action::EditorSetScrollAxis {
                    session_id: drag.session_id,
                    vertical: drag.vertical,
                    offset,
                });
            }
            let (column, start_width, start_x) = (*ui.relation_resize.borrow())?;
            Some(Action::GridSetColumnWidth {
                column,
                width: start_width.saturating_add_signed(event.column as i16 - start_x as i16),
            })
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if *ui.mouse_gesture.borrow() == Some(crate::ui::text_selection::GestureOwner::Input) {
                let Some(gesture) = ui.input_gesture.borrow_mut().take() else {
                    ui.mouse_gesture.borrow_mut().take();
                    return None;
                };
                ui.mouse_gesture.borrow_mut().take();
                if !ui.input_selection_is_current(&gesture) {
                    return None;
                }
                if !gesture.has_dragged {
                    return Some(Action::CancelMouseInputSelection {
                        target: gesture.target,
                    });
                }
                return Some(Action::CompleteMouseInputSelection {
                    target: gesture.target,
                    start: gesture.start,
                    end: gesture.end,
                });
            }
            if *ui.mouse_gesture.borrow() == Some(crate::ui::text_selection::GestureOwner::Text) {
                let Some(mut gesture) = ui.text_gesture.borrow_mut().take() else {
                    ui.end_mouse_gesture();
                    return None;
                };
                let target_is_current = match gesture.source {
                    crate::ui::text_selection::TextGestureSource::Editor => app.overlay.is_none(),
                    crate::ui::text_selection::TextGestureSource::TextDetail => {
                        matches!(app.overlay, Some(Overlay::TextDetail(_)))
                    }
                };
                let target_matches = target_is_current
                    && ui
                        .text_selection_targets
                        .iter()
                        .any(|target| target.session_id == gesture.session_id);
                if !target_matches {
                    ui.mouse_gesture.borrow_mut().take();
                    return None;
                }
                if let Some(target) = ui
                    .text_selection_targets
                    .iter()
                    .find(|target| target.session_id == gesture.session_id)
                    && let Some(end) =
                        text_position_clamped(&target.hit_maps, event.column, event.row)
                {
                    gesture.has_dragged |= end != gesture.start;
                    gesture.end = end;
                }
                ui.mouse_gesture.borrow_mut().take();
                if !gesture.has_dragged {
                    return None;
                }
                return Some(Action::CompleteMouseTextSelection {
                    source: gesture.source,
                    session_id: gesture.session_id,
                    start: editor_position(gesture.start),
                    end: editor_position(gesture.end),
                    revision: gesture.revision,
                });
            }
            if let Some(drag) = ui.pane_resize_drag.borrow_mut().take() {
                ui.mouse_gesture.borrow_mut().take();
                return pane_resize_action(drag, pane_resize_pointer(drag.split, event), ui, app);
            }
            let was_column_resize = ui.relation_resize.borrow_mut().take().is_some();
            let was_scrollbar_drag = ui.grid_scrollbar_drag.borrow_mut().take().is_some();
            let was_editor_scrollbar_drag = ui.editor_scrollbar_drag.borrow_mut().take().is_some();
            let was_explorer_scrollbar_drag =
                ui.explorer_scrollbar_drag.borrow_mut().take().is_some();
            if was_column_resize
                || was_scrollbar_drag
                || was_editor_scrollbar_drag
                || was_explorer_scrollbar_drag
            {
                ui.mouse_gesture.borrow_mut().take();
                Some(Action::GridEndColumnResize)
            } else {
                None
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if ui.mouse_gesture.borrow().is_some() {
                let input_target = ui
                    .input_gesture
                    .borrow()
                    .as_ref()
                    .map(|gesture| gesture.target.clone());
                ui.cancel_mouse_gesture();
                if let Some(target) = input_target {
                    return Some(Action::CancelMouseInputSelection { target });
                }
                return None;
            }
            ui.relation_resize.borrow_mut().take();
            ui.grid_scrollbar_drag.borrow_mut().take();
            ui.editor_scrollbar_drag.borrow_mut().take();
            ui.explorer_scrollbar_drag.borrow_mut().take();
            ui.pane_resize_drag.borrow_mut().take();
            if let Some(Overlay::TextDetail(view)) = app.overlay.as_ref() {
                if let Some(target) = ui.target_at(event.column, event.row).cloned() {
                    return match target {
                        HitTarget::TextDetailCopyAll => Some(Action::CopyTextDetailAll {
                            session_id: view.session_id,
                        }),
                        HitTarget::TextDetailClose => Some(Action::CloseTextDetail),
                        HitTarget::OpenTextDetail(request) => Some(Action::OpenTextDetail(request)),
                        _ => None,
                    };
                }
                let position = ui
                    .text_selection_target_at(event.column, event.row)
                    .filter(|(target, _)| target.session_id == view.session_id)
                    .map(|(_, position)| position);
                return position.map(|position| {
                    ui.begin_text_gesture(crate::ui::text_selection::TextGesture {
                        session_id: view.session_id,
                        source: crate::ui::text_selection::TextGestureSource::TextDetail,
                        start: position,
                        end: position,
                        revision: view.revision,
                        has_dragged: false,
                    });
                    // Text detail is read-only; the gesture itself is the preview state.
                    let _ = position;
                    Action::SetEditorMouseCursor {
                        session_id: view.session_id,
                        position: editor_position(position),
                        revision: view.revision,
                    }
                });
            }
            let Some(target) = ui.target_at(event.column, event.row).cloned() else {
                ui.clear_click_tracker();
                return None;
            };
            if !matches!(target, HitTarget::ExplorerRow(_)) {
                ui.clear_click_tracker();
            }
            if let HitTarget::OpenTextDetail(request) = target {
                return Some(Action::OpenTextDetail(request));
            }
            if let Some((input_target, cursor)) =
                ui.input_selection_target_at(event.column, event.row)
            {
                let input_target = input_target.clone();
                ui.input_gesture
                    .borrow_mut()
                    .replace(crate::ui::text_selection::InputGesture {
                        target: input_target.clone(),
                        hit_map: ui
                            .input_selection_targets
                            .iter()
                            .find(|(target, _)| target == &input_target)
                            .map(|(_, map)| map.clone())?,
                        start: cursor,
                        end: cursor,
                        has_dragged: false,
                    });
                ui.mouse_gesture
                    .borrow_mut()
                    .replace(crate::ui::text_selection::GestureOwner::Input);
                return Some(Action::BeginMouseInputSelection {
                    target: input_target,
                    cursor,
                });
            }
            if let HitTarget::ProfileField(field) = target
                && let Some((_, cursor)) = ui.profile_input_at(event.column, event.row)
            {
                if field == crate::model::profile_manager::ProfileField::Password {
                    return None;
                }
                if let Some((input_target, cursor)) = ui
                    .input_selection_target_at(event.column, event.row)
                    .filter(|(target, _)| {
                        **target == crate::ui::text_selection::InputSelectionTarget::Profile(field)
                    })
                {
                    ui.input_gesture.borrow_mut().replace(
                        crate::ui::text_selection::InputGesture {
                            target: input_target.clone(),
                            hit_map: ui
                                .input_selection_targets
                                .iter()
                                .find(|entry| entry.0 == *input_target)
                                .map(|(_, map)| map.clone())?,
                            start: cursor,
                            end: cursor,
                            has_dragged: false,
                        },
                    );
                    ui.mouse_gesture
                        .borrow_mut()
                        .replace(crate::ui::text_selection::GestureOwner::Input);
                    return Some(Action::BeginMouseInputSelection {
                        target: input_target.clone(),
                        cursor,
                    });
                }
                return Some(Action::ProfileSetCursor { field, cursor });
            }
            if let Some((target, cursor)) = ui.catalog_input_at(event.column, event.row) {
                let input_target =
                    crate::ui::text_selection::InputSelectionTarget::Catalog(target.clone());
                if let Some((_, position)) = ui
                    .input_selection_target_at(event.column, event.row)
                    .filter(|(candidate, _)| **candidate == input_target)
                {
                    ui.input_gesture.borrow_mut().replace(
                        crate::ui::text_selection::InputGesture {
                            target: input_target.clone(),
                            hit_map: ui
                                .input_selection_targets
                                .iter()
                                .find(|entry| entry.0 == input_target)
                                .map(|(_, map)| map.clone())?,
                            start: position,
                            end: position,
                            has_dragged: false,
                        },
                    );
                    ui.mouse_gesture
                        .borrow_mut()
                        .replace(crate::ui::text_selection::GestureOwner::Input);
                    return Some(Action::BeginMouseInputSelection {
                        target: input_target,
                        cursor: position,
                    });
                }
                return Some(Action::CatalogEditorSetCursor { target, cursor });
            }
            if let Some(overlay) = &app.overlay
                && !matches!(
                    overlay,
                    Overlay::NotificationHistory(_) | Overlay::NotificationDetail(_)
                )
                && (overlay != &Overlay::ProfileManager
                    && overlay != &Overlay::CatalogEditor
                    && !matches!(overlay, Overlay::TargetSelector { .. })
                    && !matches!(overlay, Overlay::DatabaseSelector(_))
                    && !matches!(overlay, Overlay::TransactionMenu { .. })
                    && !matches!(overlay, Overlay::TransactionExitConfirm { .. })
                    && !matches!(overlay, Overlay::CatalogEditorDiscardConfirm { .. })
                    && !matches!(overlay, Overlay::CatalogDropConfirm { .. })
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
                            | HitTarget::SqlEditorListSearch
                            | HitTarget::SqlEditorListRename
                            | HitTarget::HelpSearch
                            | HitTarget::ProfileGroupName
                            | HitTarget::ExplorerFind
                            | HitTarget::ExplorerSearch
                            | HitTarget::KeySequencePopup
                            | HitTarget::ExplorerAddOption(_)
                            | HitTarget::CatalogEditorField(_)
                            | HitTarget::CatalogEditorFormField(_)
                            | HitTarget::CatalogEditorTableField(_)
                            | HitTarget::CatalogEditorTableColumn(_)
                            | HitTarget::CatalogEditorAddTableColumn
                            | HitTarget::CatalogEditorRemoveTableColumn
                            | HitTarget::CatalogEditorReview
                            | HitTarget::CatalogEditorCancel
                            | HitTarget::CatalogEditorDiscardKeepEditing
                            | HitTarget::CatalogEditorDiscardChanges
                            | HitTarget::CatalogDropCancel
                            | HitTarget::CatalogDropConfirm
                            | HitTarget::CatalogEditorColumnDetailsConfirm
                            | HitTarget::CatalogEditorColumnDetailsCancel
                            | HitTarget::CatalogOwnerChoice(_)
                            | HitTarget::TargetSelectorRow(_)
                            | HitTarget::TargetSelectorCancel
                            | HitTarget::DatabaseSelectorRow(_)
                            | HitTarget::HeaderDatabase
                            | HitTarget::EditorExecutionTarget
                            | HitTarget::EditorTransactionMenu
                            | HitTarget::TransactionMenuItem(_)
                            | HitTarget::TransactionMenuCancel
                            | HitTarget::TransactionExitChoice(_)
                            | HitTarget::TransactionExitCancel
                            | HitTarget::UpdateButton { .. }
                            | HitTarget::OpenTextDetail(_)
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
            if app.overlay.is_none()
                && let Some((text_target, position)) =
                    ui.text_selection_target_at(event.column, event.row)
            {
                let revision = app.editor_revision(text_target.session_id);
                ui.begin_text_gesture(crate::ui::text_selection::TextGesture {
                    session_id: text_target.session_id,
                    source: crate::ui::text_selection::TextGestureSource::Editor,
                    start: position,
                    end: position,
                    revision,
                    has_dragged: false,
                });
                return Some(Action::SetEditorMouseCursor {
                    session_id: text_target.session_id,
                    position: editor_position(position),
                    revision,
                });
            }
            match target {
                HitTarget::Focus(focus) => Some(Action::Focus(focus)),
                HitTarget::Tab(index) => Some(Action::ActivateTab(index)),
                HitTarget::TabScrollLeft(index) | HitTarget::TabScrollRight(index) => {
                    Some(Action::ActivateTab(index))
                }
                HitTarget::CloseTab(id) => Some(Action::CloseTab(id)),
                HitTarget::DismissNotification(id) => Some(Action::DismissNotification(id)),
                HitTarget::OpenNotificationHistoryAt(id) => {
                    Some(Action::OpenNotificationHistoryAt(id))
                }
                HitTarget::OpenTextDetail(request) => Some(Action::OpenTextDetail(request)),
                HitTarget::NotificationHistoryRow(index) => {
                    Some(Action::NotificationHistorySelect(index))
                }
                HitTarget::SqlHistoryRow(index) => Some(Action::SqlHistorySelect(index)),
                HitTarget::ExplorerRow(id) => {
                    if ui.track_explorer_click(&id, Instant::now()) {
                        Some(Action::ExplorerPrimary)
                    } else {
                        Some(Action::ExplorerSelect(id))
                    }
                }
                HitTarget::ExplorerToggle(id) => Some(Action::ExplorerToggleNode(id)),
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
                HitTarget::PaneResize(split) => {
                    let start_size = match split {
                        crate::model::workspace::PaneSplit::ExplorerWidth => {
                            ui.pane_layout.explorer_width
                        }
                        crate::model::workspace::PaneSplit::EditorHeight => {
                            ui.pane_layout.editor_height
                        }
                    }?;
                    *ui.pane_resize_drag.borrow_mut() = Some(PaneResizeDrag {
                        split,
                        start_pointer: pane_resize_pointer(split, event),
                        start_size,
                    });
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::PaneResize);
                    None
                }
                HitTarget::RelationColumnResize { column, width } => {
                    *ui.relation_resize.borrow_mut() = Some((column, width, event.column));
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::RelationColumnResize);
                    Some(Action::GridStartColumnResize { column, width })
                }
                HitTarget::GridColumnSort(column) => Some(Action::CycleDataColumnSort(column)),
                HitTarget::GridScrollbarThumb {
                    axis,
                    track_start,
                    track_length,
                    thumb_start,
                    thumb_length,
                    offset,
                    max_offset,
                } => {
                    *ui.grid_scrollbar_drag.borrow_mut() = Some(crate::ui::GridScrollbarDrag {
                        axis,
                        track_start,
                        track_length,
                        thumb_length,
                        pointer_offset: match axis {
                            crate::ui::GridScrollAxis::Horizontal => {
                                event.column.saturating_sub(thumb_start)
                            }
                            crate::ui::GridScrollAxis::Vertical => {
                                event.row.saturating_sub(thumb_start)
                            }
                        },
                        max_offset,
                    });
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::GridScrollbar);
                    Some(match axis {
                        crate::ui::GridScrollAxis::Horizontal => {
                            Action::GridSetColumnOffset { offset }
                        }
                        crate::ui::GridScrollAxis::Vertical => Action::GridSetRowOffset { offset },
                    })
                }
                HitTarget::GridScrollbarPage { axis, offset } => Some(match axis {
                    crate::ui::GridScrollAxis::Horizontal => Action::GridSetColumnOffset { offset },
                    crate::ui::GridScrollAxis::Vertical => Action::GridSetRowOffset { offset },
                }),
                HitTarget::ExplorerScrollbarPage { offset } => {
                    Some(Action::ExplorerSetScrollOffset(offset))
                }
                HitTarget::ExplorerScrollbarThumb {
                    track_start,
                    track_length,
                    thumb_start,
                    thumb_length,
                    max_offset,
                } => {
                    *ui.explorer_scrollbar_drag.borrow_mut() =
                        Some(crate::ui::ExplorerScrollbarDrag {
                            track_start,
                            track_length,
                            thumb_length,
                            pointer_offset: event.row.saturating_sub(thumb_start),
                            max_offset,
                        });
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::GridScrollbar);
                    Some(Action::ExplorerSetScrollOffset(
                        crate::ui::scrollbar::ScrollbarGeometry {
                            rail: ratatui::layout::Rect::new(0, track_start, 1, track_length),
                            thumb_start: 0,
                            thumb_length,
                            max_offset,
                        }
                        .offset_at(event.row, event.row.saturating_sub(thumb_start)),
                    ))
                }
                HitTarget::EditorScrollbarPage {
                    session_id,
                    rows,
                    columns,
                } => Some(Action::EditorScrollBy {
                    session_id,
                    rows,
                    columns,
                }),
                HitTarget::EditorScrollbarThumb {
                    session_id,
                    vertical,
                    track_start,
                    track_length,
                    thumb_start,
                    thumb_length,
                    offset,
                    max_offset,
                } => {
                    *ui.editor_scrollbar_drag.borrow_mut() = Some(crate::ui::EditorScrollbarDrag {
                        session_id,
                        vertical,
                        track_start,
                        track_length,
                        thumb_length,
                        pointer_offset: if vertical {
                            event.row.saturating_sub(thumb_start)
                        } else {
                            event.column.saturating_sub(thumb_start)
                        },
                        max_offset,
                    });
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::GridScrollbar);
                    Some(Action::EditorSetScrollAxis {
                        session_id,
                        vertical,
                        offset,
                    })
                }
                HitTarget::HeaderProfile => app.connection.profile_id.map_or(
                    Some(Action::Focus(Focus::Explorer)),
                    |profile_id| {
                        Some(Action::ExplorerSelect(
                            crate::model::explorer::ExplorerNodeId::Profile(profile_id),
                        ))
                    },
                ),
                HitTarget::HeaderDatabase => Some(Action::OpenDatabaseSelector),
                HitTarget::ProfileField(field) => Some(Action::ProfileFocusField(field)),
                HitTarget::ProfileDriver(kind) => Some(Action::ProfileSelectDriver(kind)),
                HitTarget::ProfileToggle(field) => Some(Action::ProfileToggleField(field)),
                HitTarget::ProfileScopeRow(id) => Some(Action::ProfileToggleScopeRow(id)),
                HitTarget::ProfileButton(button) => Some(profile_button_action(button)),
                HitTarget::ProfileGroupOption(index) => Some(Action::ProfileGroupSelect(index)),
                HitTarget::ProfileGroupConfirm => Some(Action::ProfileGroupConfirm),
                HitTarget::ProfileGroupCancel => Some(Action::ProfileGroupCancel),
                HitTarget::SqlEditorListSearch
                | HitTarget::SqlEditorListRename
                | HitTarget::HelpSearch
                | HitTarget::ProfileGroupName
                | HitTarget::ExplorerFind
                | HitTarget::ExplorerSearch
                | HitTarget::KeySequencePopup => None,
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
                HitTarget::CatalogEditorRestoreTableColumn => {
                    Some(Action::CatalogEditorRestoreTableColumn)
                }
                HitTarget::CatalogEditorReview => Some(Action::CatalogEditorPreview),
                HitTarget::CatalogEditorCancel => Some(Action::CatalogEditorCancel),
                HitTarget::CatalogEditorDiscardKeepEditing => {
                    Some(Action::CatalogEditorDiscardKeepEditing)
                }
                HitTarget::CatalogEditorDiscardChanges => Some(Action::CatalogEditorDiscardChanges),
                HitTarget::CatalogEditorColumnDetailsConfirm => {
                    Some(Action::CatalogEditorConfirmTableColumnDetails)
                }
                HitTarget::CatalogEditorColumnDetailsCancel => {
                    Some(Action::CatalogEditorCancelTableColumnDetails)
                }
                HitTarget::CatalogOwnerChoice(name) => Some(Action::CatalogOwnerPickerChoose(name)),
                HitTarget::TargetSelectorRow(index) => Some(Action::SelectTargetSelector(index)),
                HitTarget::TargetSelectorCancel => Some(Action::CancelTargetSelector),
                HitTarget::DatabaseSelectorRow(index) => {
                    Some(Action::SelectDatabaseSelector(index))
                }
                HitTarget::EditorExecutionTarget => Some(Action::OpenTargetSelector),
                HitTarget::EditorTransactionMenu => Some(Action::ActivateEditorTransaction),
                HitTarget::TransactionMenuItem(index) => Some(Action::SelectTransactionMenu(index)),
                HitTarget::TransactionMenuCancel => Some(Action::CancelTransactionMenu),
                HitTarget::TransactionExitChoice(choice) => {
                    Some(Action::ConfirmTransactionExitChoice(choice))
                }
                HitTarget::TransactionExitCancel => Some(Action::CancelTransactionExit),
                HitTarget::ManualCancellationKeepRunning => {
                    Some(Action::ActivateManualCancellationKeepRunning)
                }
                HitTarget::ManualCancellationConfirm => Some(Action::ConfirmManualCancellation),
                HitTarget::ExecutionConfirm => Some(Action::ConfirmExecution),
                HitTarget::ExecutionCancel => Some(Action::CancelExecution),
                HitTarget::ClearTransactionConfirm => Some(Action::ConfirmClearTransactionOutcome),
                HitTarget::ClearTransactionCancel => Some(Action::CancelClearTransactionOutcome),
                HitTarget::DeleteConsoleConfirm => Some(Action::ActivateDeleteConsole),
                HitTarget::DeleteConsoleCancel => Some(Action::CancelDeleteConsole),
                HitTarget::SqlEditorListDeleteConfirm => Some(Action::SqlEditorListDeleteActivate),
                HitTarget::CatalogDropCancel => Some(Action::CatalogDropCancel),
                HitTarget::CatalogDropConfirm => Some(Action::ActivateCatalogDrop),
                HitTarget::SqlEditorListDeleteCancel => Some(Action::SqlEditorListDeleteCancel),
                HitTarget::TextDetailCopyAll => None,
                HitTarget::TextDetailClose => None,
                HitTarget::RecordViewCopyCell => Some(Action::CopyRecordViewCell),
                HitTarget::RecordViewCopyRow => Some(Action::CopyRecordViewRow {
                    include_headers: false,
                }),
                HitTarget::RecordViewViewValue => Some(Action::ViewRecordViewValue),
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
                | HitTarget::ExplorerRow(crate::model::explorer::ExplorerNodeId::Catalog(_))
                | HitTarget::ExplorerToggle(crate::model::explorer::ExplorerNodeId::Profile(_))
                | HitTarget::ExplorerToggle(crate::model::explorer::ExplorerNodeId::Catalog(_)) => {
                    Some(Action::OpenCatalogEdit)
                }
                _ => None,
            }
        }
        MouseEventKind::ScrollDown => {
            if let Some(Overlay::TextDetail(view)) = app.overlay.as_ref() {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: view.session_id,
                    rows: 3,
                    columns: 0,
                });
            }
            if matches!(app.overlay, Some(Overlay::NotificationDetail(_))) {
                return Some(Action::NotificationDetailMove(3));
            }
            if matches!(app.overlay, Some(Overlay::NotificationHistory(_))) {
                return Some(Action::NotificationHistoryMove(3));
            }
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerScrollNodes {
                    direction: 1,
                    amount: ExplorerScrollAmount::Lines(3),
                }),
                Focus::Results if is_relation_ddl_focus(app) => ddl_scroll_action(app, 3),
                Focus::Results if is_output_focus(app) => output_scroll_action(app, 3, 0),
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
            if let Some(Overlay::TextDetail(view)) = app.overlay.as_ref() {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: view.session_id,
                    rows: -3,
                    columns: 0,
                });
            }
            if matches!(app.overlay, Some(Overlay::NotificationDetail(_))) {
                return Some(Action::NotificationDetailMove(-3));
            }
            if matches!(app.overlay, Some(Overlay::NotificationHistory(_))) {
                return Some(Action::NotificationHistoryMove(-3));
            }
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerScrollNodes {
                    direction: -1,
                    amount: ExplorerScrollAmount::Lines(3),
                }),
                Focus::Results if is_relation_ddl_focus(app) => ddl_scroll_action(app, -3),
                Focus::Results if is_output_focus(app) => output_scroll_action(app, -3, 0),
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
                Focus::Results if is_output_focus(app) => output_scroll_action(app, 0, -3),
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
                Focus::Results if is_output_focus(app) => output_scroll_action(app, 0, 3),
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

fn pane_resize_action(
    drag: PaneResizeDrag,
    pointer: u16,
    ui: &UiState,
    app: &App,
) -> Option<Action> {
    let size = (i32::from(drag.start_size) + i32::from(pointer) - i32::from(drag.start_pointer))
        .clamp(0, i32::from(u16::MAX)) as u16;
    let current = match drag.split {
        crate::model::workspace::PaneSplit::ExplorerWidth => app.pane_sizes.explorer_width,
        crate::model::workspace::PaneSplit::EditorHeight => app.pane_sizes.editor_height,
    }
    .or(match drag.split {
        crate::model::workspace::PaneSplit::ExplorerWidth => ui.pane_layout.explorer_width,
        crate::model::workspace::PaneSplit::EditorHeight => ui.pane_layout.editor_height,
    });
    (current != Some(size)).then_some(Action::SetPaneSize {
        split: drag.split,
        size,
    })
}

fn pane_resize_pointer(split: crate::model::workspace::PaneSplit, event: MouseEvent) -> u16 {
    match split {
        crate::model::workspace::PaneSplit::ExplorerWidth => event.column,
        crate::model::workspace::PaneSplit::EditorHeight => event.row,
    }
}

fn profile_button_action(button: ProfileButton) -> Action {
    match button {
        ProfileButton::Cancel => Action::CloseProfileManager,
        ProfileButton::Test => Action::ProfileTest,
        ProfileButton::Save => Action::ProfileSave { connect: false },
        ProfileButton::SaveAndConnect => Action::ProfileSave { connect: true },
        ProfileButton::ConfirmDelete => Action::ActivateProfileDelete,
        ProfileButton::CancelDelete => Action::ProfileCancelDelete,
    }
}

fn focus_at(ui: &UiState, column: u16, row: u16) -> Option<Focus> {
    match ui.target_at(column, row)? {
        HitTarget::Focus(focus) => Some(*focus),
        HitTarget::ExplorerRow(_) => Some(Focus::Explorer),
        HitTarget::ExplorerToggle(_) => Some(Focus::Explorer),
        HitTarget::ResultCell { .. }
        | HitTarget::ToggleResultView
        | HitTarget::ResultView(_)
        | HitTarget::RelationView(_)
        | HitTarget::DashboardView(_)
        | HitTarget::RelationRetry
        | HitTarget::GridColumnSort(_)
        | HitTarget::DataQueryInput(_)
        | HitTarget::RelationColumnResize { .. }
        | HitTarget::SqlHistoryRow(_)
        | HitTarget::GridScrollbarThumb { .. }
        | HitTarget::GridScrollbarPage { .. } => Some(Focus::Results),
        HitTarget::ExplorerScrollbarPage { .. } | HitTarget::ExplorerScrollbarThumb { .. } => {
            Some(Focus::Explorer)
        }
        HitTarget::EditorScrollbarPage { .. } | HitTarget::EditorScrollbarThumb { .. } => {
            Some(Focus::Editor)
        }
        HitTarget::PaneResize(PaneSplit::ExplorerWidth) => Some(Focus::Explorer),
        HitTarget::PaneResize(PaneSplit::EditorHeight) => Some(Focus::Editor),
        HitTarget::RelationCancel => Some(Focus::Results),
        HitTarget::Tab(_)
        | HitTarget::TabScrollLeft(_)
        | HitTarget::TabScrollRight(_)
        | HitTarget::CloseTab(_)
        | HitTarget::DismissNotification(_)
        | HitTarget::OpenNotificationHistoryAt(_)
        | HitTarget::OpenTextDetail(_)
        | HitTarget::NotificationHistoryRow(_)
        | HitTarget::Help
        | HitTarget::UpdateCenter
        | HitTarget::UpdateButton { .. }
        | HitTarget::HeaderProfile
        | HitTarget::HeaderDatabase
        | HitTarget::ProfileField(_)
        | HitTarget::ProfileDriver(_)
        | HitTarget::ProfileToggle(_)
        | HitTarget::ProfileScopeRow(_)
        | HitTarget::ProfileButton(_)
        | HitTarget::ProfileGroupOption(_)
        | HitTarget::ProfileGroupConfirm
        | HitTarget::ProfileGroupCancel
        | HitTarget::SqlEditorListSearch
        | HitTarget::SqlEditorListRename
        | HitTarget::HelpSearch
        | HitTarget::ProfileGroupName
        | HitTarget::ExplorerFind
        | HitTarget::ExplorerSearch
        | HitTarget::KeySequencePopup
        | HitTarget::ExplorerAddOption(_)
        | HitTarget::CatalogEditorField(_)
        | HitTarget::CatalogEditorFormField(_)
        | HitTarget::CatalogEditorTableField(_)
        | HitTarget::CatalogEditorTableColumn(_)
        | HitTarget::CatalogEditorAddTableColumn
        | HitTarget::CatalogEditorRemoveTableColumn
        | HitTarget::CatalogEditorRestoreTableColumn
        | HitTarget::CatalogEditorReview
        | HitTarget::CatalogEditorCancel
        | HitTarget::CatalogEditorDiscardKeepEditing
        | HitTarget::CatalogEditorDiscardChanges
        | HitTarget::CatalogEditorColumnDetailsConfirm
        | HitTarget::CatalogEditorColumnDetailsCancel
        | HitTarget::CatalogOwnerChoice(_) => None,
        HitTarget::TargetSelectorRow(_)
        | HitTarget::TargetSelectorCancel
        | HitTarget::DatabaseSelectorRow(_)
        | HitTarget::EditorExecutionTarget
        | HitTarget::EditorTransactionMenu
        | HitTarget::TransactionMenuItem(_)
        | HitTarget::TransactionMenuCancel => None,
        HitTarget::TransactionExitChoice(_) | HitTarget::TransactionExitCancel => None,
        HitTarget::DeleteConsoleConfirm
        | HitTarget::DeleteConsoleCancel
        | HitTarget::SqlEditorListDeleteConfirm
        | HitTarget::CatalogDropCancel
        | HitTarget::CatalogDropConfirm
        | HitTarget::SqlEditorListDeleteCancel => None,
        HitTarget::ManualCancellationKeepRunning | HitTarget::ManualCancellationConfirm => None,
        HitTarget::ExecutionConfirm | HitTarget::ExecutionCancel => None,
        HitTarget::ClearTransactionConfirm | HitTarget::ClearTransactionCancel => None,
        HitTarget::TextDetailCopyAll
        | HitTarget::TextDetailClose
        | HitTarget::RecordViewCopyCell
        | HitTarget::RecordViewCopyRow
        | HitTarget::RecordViewViewValue => None,
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

fn is_output_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && app.active_console_opt().is_some_and(|tab| {
            matches!(
                tab.result_view,
                crate::model::tab::ResultView::Output | crate::model::tab::ResultView::Plan
            )
        })
}

fn output_scroll_action(app: &App, rows: isize, columns: isize) -> Option<Action> {
    app.active_console_opt().map(|tab| Action::EditorScrollBy {
        session_id: tab.output_editor_id,
        rows,
        columns,
    })
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
