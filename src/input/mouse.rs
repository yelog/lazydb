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
            if matches!(
                app.overlay,
                Some(
                    Overlay::TextDetail(_)
                        | Overlay::SqlHistory(_)
                        | Overlay::RelationTransactionConfirm(_),
                )
            ) && *ui.mouse_gesture.borrow()
                == Some(crate::ui::text_selection::GestureOwner::Text)
            {
                let gesture = ui.text_gesture.borrow().as_ref().copied()?;
                let target = ui
                    .text_selection_targets
                    .iter()
                    .rev()
                    .find(|target| target.session_id == gesture.session_id)?;
                if !matches!(
                    gesture.source,
                    crate::ui::text_selection::TextGestureSource::TextDetail
                        | crate::ui::text_selection::TextGestureSource::SqlHistory
                        | crate::ui::text_selection::TextGestureSource::TransactionReview
                ) || gesture.session_id != target.session_id
                {
                    ui.cancel_mouse_gesture();
                    return None;
                }
                let end = target.position_clamped(event.column, event.row)?;
                ui.update_text_gesture(end);
                return None;
            }
            if let Some(drag) = *ui.panel_scrollbar_drag.borrow() {
                let offset = crate::ui::scrollbar::ScrollbarGeometry {
                    rail: ratatui::layout::Rect::new(0, drag.track_start, 1, drag.track_length),
                    thumb_start: 0,
                    thumb_length: drag.thumb_length,
                    max_offset: drag.max_offset,
                }
                .offset_at(event.row, drag.pointer_offset);
                return Some(if drag.help {
                    Action::HelpSetScroll(offset)
                } else {
                    Action::OmniSetScroll(offset)
                });
            }
            if app.overlay.is_some()
                && !matches!(app.overlay, Some(Overlay::RelationTransactionConfirm(_)))
            {
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
                    .rev()
                    .find(|target| target.session_id == gesture.session_id)?;
                if gesture.source != crate::ui::text_selection::TextGestureSource::Editor
                    || gesture.session_id != target.session_id
                {
                    ui.cancel_mouse_gesture();
                    return None;
                }
                // A drag that leaves the visible text keeps extending the
                // selection while the editor scrolls after the pointer, so one
                // gesture can reach text that is off screen.
                ui.track_text_drag(event.column, event.row);
                return ui.follow_text_drag().map(|(session_id, rows)| {
                    Action::ReadOnlyEditorScroll {
                        session_id,
                        rows,
                        columns: 0,
                    }
                });
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
            let principal_access_drag = *ui.principal_access_scrollbar_drag.borrow();
            if let Some(drag) = principal_access_drag {
                if !principal_access_drag_is_current(drag, app) || app.overlay.is_some() {
                    ui.principal_access_scrollbar_drag.borrow_mut().take();
                    ui.mouse_gesture.borrow_mut().take();
                    return None;
                }
                let current_metrics = ui
                    .principal_access_viewport
                    .filter(|(tab_id, _, _)| *tab_id == drag.tab_id)
                    .and_then(|(_, rows, body)| {
                        app.tabs.get(app.active_tab).and_then(|tab| match tab {
                            WorkspaceTab::PrincipalDdl(tab) => tab.details.snapshot().map(|details| {
                                let count = match drag.section {
                                    crate::model::principal::PrincipalAccessSection::Permissions => details.permissions.len(),
                                    crate::model::principal::PrincipalAccessSection::MemberOf => details.member_of.len(),
                                    crate::model::principal::PrincipalAccessSection::Members => details.members.len(),
                                };
                                (count.saturating_sub(rows), body.height.saturating_sub(2))
                            }),
                            _ => None,
                        })
                    });
                let Some((max_offset, current_track_length)) = current_metrics else {
                    ui.principal_access_scrollbar_drag.borrow_mut().take();
                    ui.mouse_gesture.borrow_mut().take();
                    return None;
                };
                if current_track_length != drag.track_length {
                    ui.principal_access_scrollbar_drag.borrow_mut().take();
                    ui.mouse_gesture.borrow_mut().take();
                    return None;
                }
                let offset = crate::ui::scrollbar::ScrollbarGeometry {
                    rail: ratatui::layout::Rect::new(0, drag.track_start, 1, drag.track_length),
                    thumb_start: 0,
                    thumb_length: drag.thumb_length,
                    max_offset,
                }
                .offset_at(event.row, drag.pointer_offset);
                return Some(Action::SetPrincipalAccessOffset(offset));
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
                    crate::ui::text_selection::TextGestureSource::SqlHistory => {
                        matches!(
                            &app.overlay,
                            Some(Overlay::SqlHistory(view))
                                if view.mode == crate::model::sql_history_view::SqlHistoryMode::Sql
                        )
                    }
                    crate::ui::text_selection::TextGestureSource::TransactionReview => {
                        app.review_preview_session_id() == Some(gesture.session_id)
                    }
                };
                let target_matches = target_is_current
                    && ui
                        .text_selection_targets
                        .iter()
                        .any(|target| target.session_id == gesture.session_id);
                if !target_matches {
                    ui.end_mouse_gesture();
                    return None;
                }
                if let Some(target) = ui
                    .text_selection_targets
                    .iter()
                    .rev()
                    .find(|target| target.session_id == gesture.session_id)
                    && let Some(end) = target.position_clamped(event.column, event.row)
                {
                    gesture.has_dragged |= end != gesture.start;
                    gesture.end = end;
                }
                ui.end_mouse_gesture();
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
            let was_principal_access_scrollbar_drag = ui
                .principal_access_scrollbar_drag
                .borrow_mut()
                .take()
                .is_some();
            let was_panel_scrollbar_drag = ui.panel_scrollbar_drag.borrow_mut().take().is_some();
            if was_column_resize
                || was_scrollbar_drag
                || was_editor_scrollbar_drag
                || was_explorer_scrollbar_drag
                || was_principal_access_scrollbar_drag
                || was_panel_scrollbar_drag
            {
                ui.mouse_gesture.borrow_mut().take();
                if was_panel_scrollbar_drag {
                    None
                } else {
                    Some(Action::GridEndColumnResize)
                }
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
            ui.principal_access_scrollbar_drag.borrow_mut().take();
            ui.pane_resize_drag.borrow_mut().take();
            if let Some(overlay) = app.overlay.as_ref()
                && let Some((session_id, revision, source)) = match overlay {
                    Overlay::TextDetail(view) => Some((
                        view.session_id,
                        view.revision,
                        crate::ui::text_selection::TextGestureSource::TextDetail,
                    )),
                    Overlay::SqlHistory(view)
                        if view.mode == crate::model::sql_history_view::SqlHistoryMode::Sql =>
                    {
                        Some((
                            view.editor_session_id,
                            app.sql_history_editor_revision().unwrap_or_default(),
                            crate::ui::text_selection::TextGestureSource::SqlHistory,
                        ))
                    }
                    Overlay::RelationTransactionConfirm(review) => Some((
                        review.editor_session_id,
                        app.editor_revision(review.editor_session_id),
                        crate::ui::text_selection::TextGestureSource::TransactionReview,
                    )),
                    _ => None,
                }
            {
                if source != crate::ui::text_selection::TextGestureSource::TransactionReview
                    && let Some(target) = ui.target_at(event.column, event.row).cloned()
                {
                    return match target {
                        HitTarget::TextDetailCopyAll
                            if source
                                == crate::ui::text_selection::TextGestureSource::TextDetail =>
                        {
                            Some(Action::CopyTextDetailAll { session_id })
                        }
                        HitTarget::TextDetailClose
                            if source
                                == crate::ui::text_selection::TextGestureSource::TextDetail =>
                        {
                            Some(Action::CloseTextDetail)
                        }
                        HitTarget::OpenTextDetail(request) => Some(Action::OpenTextDetail(request)),
                        _ => None,
                    };
                }
                let position = ui
                    .text_selection_target_at(event.column, event.row)
                    .filter(|(target, _)| target.session_id == session_id)
                    .map(|(_, position)| position);
                return position.map(|position| {
                    ui.begin_text_gesture(crate::ui::text_selection::TextGesture {
                        session_id,
                        source,
                        start: position,
                        end: position,
                        revision,
                        has_dragged: false,
                    });
                    // Read-only modal editors keep the gesture as their preview state.
                    let _ = position;
                    Action::SetEditorMouseCursor {
                        session_id,
                        position: editor_position(position),
                        revision,
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
            match target {
                HitTarget::HelpItem(index) => return Some(Action::HelpSelect(index)),
                HitTarget::HelpTogglePanel | HitTarget::OmniTogglePanel => {
                    return Some(Action::ToggleHelpPanel);
                }
                HitTarget::OmniItem(index) => return Some(Action::OmniSelect(index)),
                _ => {}
            }
            if let HitTarget::OpenTextDetail(request) = target {
                return Some(Action::OpenTextDetail(request));
            }
            if let HitTarget::Shortcut(keys) = target {
                return crate::input::keymap::map_shortcut(&keys, app);
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
            let review_interactive =
                matches!(app.overlay, Some(Overlay::RelationTransactionConfirm(_)))
                    && (ui
                        .text_selection_target_at(event.column, event.row)
                        .is_some()
                        || matches!(
                            target,
                            HitTarget::EditorScrollbarPage { session_id, .. }
                                | HitTarget::EditorScrollbarThumb { session_id, .. }
                        if app.review_preview_session_id() == Some(session_id)
                        ));
            if let Some(overlay) = &app.overlay
                && !matches!(
                    overlay,
                    Overlay::NotificationHistory(_) | Overlay::NotificationDetail(_)
                )
                && (overlay != &Overlay::ProfileManager
                    && overlay != &Overlay::CatalogEditor
                    && !matches!(overlay, Overlay::RelationTransactionConfirm(_))
                    && !matches!(overlay, Overlay::TargetSelector { .. })
                    && !matches!(overlay, Overlay::DatabaseSelector(_))
                    && !matches!(overlay, Overlay::TransactionMenu { .. })
                    && !matches!(overlay, Overlay::TransactionExitConfirm { .. })
                    && !matches!(overlay, Overlay::CatalogEditorDiscardConfirm { .. })
                    && !matches!(overlay, Overlay::CatalogDropConfirm { .. })
                    && !matches!(overlay, Overlay::Update(_))
                    || (!review_interactive
                        && !matches!(
                            target,
                            HitTarget::ProfileField(_)
                                | HitTarget::ProfileDriver(_)
                                | HitTarget::ProfileCategory(_)
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
                                | HitTarget::CatalogEditorAddTableConstraintPrimaryKey
                                | HitTarget::CatalogEditorAddTableConstraintUnique
                                | HitTarget::CatalogEditorAddTableConstraintCheck
                                | HitTarget::CatalogEditorAddTableConstraintForeignKey
                                | HitTarget::CatalogEditorFocusTableConstraint(_)
                                | HitTarget::CatalogEditorRemoveTableConstraint(_)
                                | HitTarget::CatalogEditorConstraintField { .. }
                                | HitTarget::CatalogEditorRemoveTableColumn
                                | HitTarget::CatalogEditorRemoveTableColumnRow(_)
                                | HitTarget::CatalogEditorRestoreTableColumnRow(_)
                                | HitTarget::CatalogEditorReview
                                | HitTarget::CatalogEditorCancel
                                | HitTarget::CatalogEditorDiscardKeepEditing
                                | HitTarget::CatalogEditorDiscardChanges
                                | HitTarget::CatalogDropCancel
                                | HitTarget::CatalogDropConfirm
                                | HitTarget::PrincipalDropCancel
                                | HitTarget::PrincipalDropConfirm
                                | HitTarget::CatalogEditorColumnDetailsConfirm
                                | HitTarget::CatalogEditorColumnDetailsCancel
                                | HitTarget::CatalogOwnerChoice(_)
                                | HitTarget::TargetSelectorRow(_)
                                | HitTarget::TargetSelectorCancel
                                | HitTarget::DatabaseSelectorRow(_)
                                | HitTarget::EditorExecutionTarget
                                | HitTarget::EditorTransactionMenu
                                | HitTarget::TransactionMenuItem(_)
                                | HitTarget::TransactionMenuCancel
                                | HitTarget::TransactionExitChoice(_)
                                | HitTarget::TransactionExitCancel
                                | HitTarget::UpdateButton { .. }
                                | HitTarget::OpenTextDetail(_)
                                | HitTarget::HelpItem(_)
                                | HitTarget::HelpTogglePanel
                                | HitTarget::HelpScrollbarPage { .. }
                                | HitTarget::HelpScrollbarThumb { .. }
                                | HitTarget::OmniTogglePanel
                                | HitTarget::OmniItem(_)
                                | HitTarget::OmniScrollbarPage { .. }
                                | HitTarget::OmniScrollbarThumb { .. }
                                | HitTarget::Shortcut(_)
                        )))
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
            let review_text_target =
                matches!(app.overlay, Some(Overlay::RelationTransactionConfirm(_)));
            if (app.overlay.is_none() || review_text_target)
                && let Some((text_target, position)) =
                    ui.text_selection_target_at(event.column, event.row)
            {
                let revision = app.editor_revision(text_target.session_id);
                let source = if review_text_target
                    && app.review_preview_session_id() == Some(text_target.session_id)
                {
                    crate::ui::text_selection::TextGestureSource::TransactionReview
                } else if app.overlay.is_none() {
                    crate::ui::text_selection::TextGestureSource::Editor
                } else {
                    return None;
                };
                ui.begin_text_gesture(crate::ui::text_selection::TextGesture {
                    session_id: text_target.session_id,
                    source,
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
                HitTarget::Shortcut(_) => None,
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
                HitTarget::RedisKeyNode { tab_id, node } => {
                    let double = ui.track_redis_click(tab_id, &node, Instant::now());
                    match (&node, double) {
                        (crate::model::redis_key_tree::KeyTreeNodeId::Prefix(_), true) => {
                            Some(Action::SelectRedisNode {
                                tab_id,
                                node: Some(node),
                            })
                        }
                        (crate::model::redis_key_tree::KeyTreeNodeId::Prefix(_), false) => {
                            Some(Action::RedisToggleNode { tab_id, node })
                        }
                        (crate::model::redis_key_tree::KeyTreeNodeId::Key(_), true) => {
                            Some(Action::OpenRedisKey { tab_id, node })
                        }
                        (crate::model::redis_key_tree::KeyTreeNodeId::Key(_), false) => {
                            Some(Action::SelectRedisNode {
                                tab_id,
                                node: Some(node),
                            })
                        }
                    }
                }
                HitTarget::RedisKeyToggle { tab_id, node } => {
                    Some(Action::RedisToggleNode { tab_id, node })
                }
                HitTarget::RedisFindInput(_) => Some(Action::RedisFocusPane(
                    crate::model::redis_browser::RedisBrowserFocus::Keys,
                )),
                HitTarget::RedisValueFilter(tab_id) => {
                    Some(Action::RedisValueFilterFocus { tab_id })
                }
                HitTarget::RedisPreviewFormat(_) => Some(Action::RedisPreviewCycleFormat),
                HitTarget::RedisPreviewWrap(_) => Some(Action::RedisPreviewToggleWrap),
                HitTarget::RedisPreviewFocus(_) => Some(Action::RedisFocusPane(
                    crate::model::redis_browser::RedisBrowserFocus::Preview,
                )),
                HitTarget::RedisPreviewTableCell {
                    tab_id,
                    row,
                    column,
                } => Some(Action::RedisPreviewCellDetail {
                    tab_id,
                    row,
                    column,
                }),
                HitTarget::RedisPreviewLoadMore(_) => Some(Action::RedisPreviewLoadNext),
                HitTarget::MongoPageLoadMore(_) => Some(Action::MongoPageNext),
                HitTarget::MongoQueryField(_, field) => Some(Action::MongoQueryField(field)),
                HitTarget::RedisValueSaveAction(index) => {
                    Some(Action::RedisValueSaveActivate(index))
                }
                HitTarget::RedisUnsavedValueAction(index) => {
                    Some(Action::RedisUnsavedValueActivate(index))
                }
                HitTarget::WorkspaceSaveAction(index) => {
                    Some(Action::WorkspaceSaveActivateAt(index))
                }
                HitTarget::ResultCell { row, column } => Some(Action::GridSelect { row, column }),
                HitTarget::Help => Some(Action::ShowHelp),
                HitTarget::Omni => None,
                HitTarget::OmniItem(index) => Some(Action::OmniSelect(index)),
                HitTarget::HelpItem(index) => Some(Action::HelpSelect(index)),
                HitTarget::HelpTogglePanel | HitTarget::OmniTogglePanel => {
                    Some(Action::ToggleHelpPanel)
                }
                HitTarget::HelpScrollbarPage { offset } => Some(Action::HelpSetScroll(offset)),
                HitTarget::OmniScrollbarPage { offset } => Some(Action::OmniSetScroll(offset)),
                HitTarget::HelpScrollbarThumb {
                    track_start,
                    track_length,
                    thumb_start,
                    thumb_length,
                    max_offset,
                } => {
                    *ui.panel_scrollbar_drag.borrow_mut() = Some(crate::ui::PanelScrollbarDrag {
                        help: true,
                        track_start,
                        track_length,
                        thumb_length,
                        pointer_offset: event.row.saturating_sub(thumb_start),
                        max_offset,
                    });
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::GridScrollbar);
                    Some(Action::HelpSetScroll(
                        crate::ui::scrollbar::ScrollbarGeometry {
                            rail: ratatui::layout::Rect::new(0, track_start, 1, track_length),
                            thumb_start: 0,
                            thumb_length,
                            max_offset,
                        }
                        .offset_at(event.row, event.row.saturating_sub(thumb_start)),
                    ))
                }
                HitTarget::OmniScrollbarThumb {
                    track_start,
                    track_length,
                    thumb_start,
                    thumb_length,
                    max_offset,
                } => {
                    *ui.panel_scrollbar_drag.borrow_mut() = Some(crate::ui::PanelScrollbarDrag {
                        help: false,
                        track_start,
                        track_length,
                        thumb_length,
                        pointer_offset: event.row.saturating_sub(thumb_start),
                        max_offset,
                    });
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::GridScrollbar);
                    Some(Action::OmniSetScroll(
                        crate::ui::scrollbar::ScrollbarGeometry {
                            rail: ratatui::layout::Rect::new(0, track_start, 1, track_length),
                            thumb_start: 0,
                            thumb_length,
                            max_offset,
                        }
                        .offset_at(event.row, event.row.saturating_sub(thumb_start)),
                    ))
                }
                HitTarget::UpdateCenter => Some(Action::OpenUpdateCenter),
                HitTarget::UpdateButton { action } => Some(Action::UpdateOverlayActivate(action)),
                HitTarget::ToggleResultView => Some(Action::ToggleResultView),
                HitTarget::ResultView(view) => Some(Action::SetResultView(view)),
                HitTarget::RelationView(view) => Some(Action::SetRelationView(view)),
                HitTarget::PrincipalView(view) => Some(Action::SetPrincipalView(view)),
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
                        crate::model::workspace::PaneSplit::RedisKeysWidth => {
                            ui.pane_layout.redis_keys_width
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
                HitTarget::PrincipalAccessScrollbarPage {
                    tab_id,
                    section,
                    offset,
                } if principal_access_target_is_current(tab_id, section, app) => {
                    Some(Action::SetPrincipalAccessOffset(offset))
                }
                HitTarget::PrincipalAccessBody { tab_id, section }
                    if principal_access_target_is_current(tab_id, section, app) =>
                {
                    Some(Action::Focus(Focus::Results))
                }
                HitTarget::PrincipalAccessScrollbarThumb {
                    tab_id,
                    section,
                    track_start,
                    track_length,
                    thumb_start,
                    thumb_length,
                    max_offset,
                } if principal_access_target_is_current(tab_id, section, app) => {
                    *ui.principal_access_scrollbar_drag.borrow_mut() =
                        Some(crate::ui::PrincipalAccessScrollbarDrag {
                            tab_id,
                            section,
                            track_start,
                            track_length,
                            thumb_length,
                            pointer_offset: event.row.saturating_sub(thumb_start),
                            max_offset,
                        });
                    *ui.mouse_gesture.borrow_mut() =
                        Some(crate::ui::text_selection::GestureOwner::GridScrollbar);
                    Some(Action::SetPrincipalAccessOffset(
                        crate::ui::scrollbar::ScrollbarGeometry {
                            rail: ratatui::layout::Rect::new(0, track_start, 1, track_length),
                            thumb_start: 0,
                            thumb_length,
                            max_offset,
                        }
                        .offset_at(event.row, event.row.saturating_sub(thumb_start)),
                    ))
                }
                HitTarget::PrincipalAccessBody { .. }
                | HitTarget::PrincipalAccessScrollbarPage { .. } => None,
                HitTarget::PrincipalAccessScrollbarThumb { .. } => None,
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
                HitTarget::ProfileField(field) => Some(Action::ProfileFocusField(field)),
                HitTarget::ProfileDriver(kind) => Some(Action::ProfileSelectDriver(kind)),
                HitTarget::ProfileCategory(category) => {
                    Some(Action::ProfileSelectCategory(category))
                }
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
                HitTarget::CatalogEditorAddTableConstraintPrimaryKey => {
                    Some(Action::CatalogEditorAddTableConstraintPrimaryKey)
                }
                HitTarget::CatalogEditorAddTableConstraintUnique => {
                    Some(Action::CatalogEditorAddTableConstraintUnique)
                }
                HitTarget::CatalogEditorAddTableConstraintCheck => {
                    Some(Action::CatalogEditorAddTableConstraintCheck)
                }
                HitTarget::CatalogEditorAddTableConstraintForeignKey => {
                    Some(Action::CatalogEditorAddTableConstraintForeignKey)
                }
                HitTarget::CatalogEditorFocusTableConstraint(index) => {
                    Some(Action::CatalogEditorFocusTableConstraint(index))
                }
                HitTarget::CatalogEditorRemoveTableConstraint(index) => {
                    Some(Action::CatalogEditorRemoveTableConstraint(index))
                }
                HitTarget::CatalogEditorConstraintField { constraint, field } => {
                    Some(Action::CatalogEditorFocusConstraintField { constraint, field })
                }
                HitTarget::CatalogEditorRemoveTableColumn => {
                    Some(Action::CatalogEditorRemoveTableColumn)
                }
                HitTarget::CatalogEditorRemoveTableColumnRow(row_id) => {
                    Some(Action::CatalogEditorRemoveTableColumnRow(row_id))
                }
                HitTarget::CatalogEditorRestoreTableColumnRow(row_id) => {
                    Some(Action::CatalogEditorRestoreTableColumnRow(row_id))
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
                HitTarget::PrincipalMutationCancel => Some(Action::CancelPrincipalMutation),
                HitTarget::PrincipalMutationApply => Some(Action::ConfirmPrincipalMutation),
                HitTarget::PrincipalPermission(index) => {
                    Some(Action::SelectPrincipalPermission(index))
                }
                HitTarget::PrincipalAccessSection(section) => {
                    Some(Action::SelectPrincipalAccess(section))
                }
                HitTarget::PrincipalAccessItem(index) => {
                    Some(Action::SelectPrincipalAccessItem(index))
                }
                HitTarget::PrincipalAccessDetails => Some(Action::OpenPrincipalAccessDetails),
                HitTarget::PrincipalMutationForm => None,
                HitTarget::ExecutionCancel => Some(Action::CancelExecution),
                HitTarget::ClearTransactionConfirm => Some(Action::ConfirmClearTransactionOutcome),
                HitTarget::ClearTransactionCancel => Some(Action::CancelClearTransactionOutcome),
                HitTarget::DeleteConsoleConfirm => Some(Action::ActivateDeleteConsole),
                HitTarget::DeleteConsoleCancel => Some(Action::CancelDeleteConsole),
                HitTarget::RedisDeleteConfirm => Some(Action::RedisDeleteConfirm),
                HitTarget::RedisDeleteCancel => Some(Action::RedisDeleteCancel),
                HitTarget::SqlEditorListDeleteConfirm => Some(Action::SqlEditorListDeleteActivate),
                HitTarget::CatalogDropCancel => Some(Action::CatalogDropCancel),
                HitTarget::CatalogDropConfirm => Some(Action::ActivateCatalogDrop),
                HitTarget::PrincipalDropCancel => Some(Action::PrincipalDropCancel),
                HitTarget::PrincipalDropConfirm => Some(Action::PrincipalDropConfirm),
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
            if let Some(Overlay::RelationTransactionConfirm(review)) = app.overlay.as_ref()
                && ui.text_selection_targets.iter().any(|target| {
                    target.session_id == review.editor_session_id
                        && target.hit_maps.iter().any(|map| {
                            map.area
                                .contains(ratatui::layout::Position::new(event.column, event.row))
                        })
                })
            {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: review.editor_session_id,
                    rows: 3,
                    columns: 0,
                });
            }
            if app.overlay.is_none()
                && let Some(target) = ui.text_selection_targets.iter().find(|target| {
                    target.hit_maps.iter().any(|map| {
                        map.area
                            .contains(ratatui::layout::Position::new(event.column, event.row))
                    })
                })
                && matches!(app.tabs.get(app.active_tab), Some(WorkspaceTab::RedisBrowser(tab)) if tab.preview_editor_id == target.session_id)
            {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: target.session_id,
                    rows: 3,
                    columns: 0,
                });
            }
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
            if app.omni.is_some()
                && ui.target_at(event.column, event.row).is_some_and(|target| {
                    matches!(
                        target,
                        HitTarget::Omni
                            | HitTarget::OmniItem(_)
                            | HitTarget::OmniTogglePanel
                            | HitTarget::OmniScrollbarPage { .. }
                            | HitTarget::OmniScrollbarThumb { .. }
                    )
                })
            {
                return Some(Action::OmniScroll(3));
            }
            if matches!(app.overlay, Some(Overlay::Help(_)))
                && ui.target_at(event.column, event.row).is_some_and(|target| {
                    matches!(
                        target,
                        HitTarget::Help
                            | HitTarget::HelpItem(_)
                            | HitTarget::HelpTogglePanel
                            | HitTarget::HelpScrollbarPage { .. }
                            | HitTarget::HelpScrollbarThumb { .. }
                    )
                })
            {
                return Some(Action::HelpScroll(3));
            }
            if app.overlay.is_some() {
                return None;
            }
            if matches!(
                app.tabs.get(app.active_tab),
                Some(WorkspaceTab::PrincipalDdl(tab))
                    if tab.view == crate::model::principal::PrincipalView::Overview
            ) && ui.principal_access_viewport.is_some_and(|(_, _, _)| {
                principal_access_body_contains(ui, event.column, event.row)
            }) {
                return Some(Action::ScrollPrincipalAccess(3));
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerScrollNodes {
                    direction: 1,
                    amount: ExplorerScrollAmount::Lines(3),
                }),
                Focus::Results if is_ddl_only_focus(app) => ddl_scroll_action(app, 3),
                Focus::Results if is_output_focus(app) => output_scroll_action(app, 3, 0),
                Focus::Results
                    if matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                            if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                    ) =>
                {
                    Some(Action::RedisPreviewScroll(3))
                }
                Focus::Results
                    if matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                            if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Keys
                    ) =>
                {
                    Some(Action::RedisKeysScroll(3))
                }
                Focus::Results => app
                    .active_grid_can_scroll_rows(1, GridScrollAmount::Lines(3))
                    .then_some(Action::GridScrollRows {
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
            if let Some(Overlay::RelationTransactionConfirm(review)) = app.overlay.as_ref()
                && ui.text_selection_targets.iter().any(|target| {
                    target.session_id == review.editor_session_id
                        && target.hit_maps.iter().any(|map| {
                            map.area
                                .contains(ratatui::layout::Position::new(event.column, event.row))
                        })
                })
            {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: review.editor_session_id,
                    rows: -3,
                    columns: 0,
                });
            }
            if app.overlay.is_none()
                && let Some(target) = ui.text_selection_targets.iter().find(|target| {
                    target.hit_maps.iter().any(|map| {
                        map.area
                            .contains(ratatui::layout::Position::new(event.column, event.row))
                    })
                })
                && matches!(app.tabs.get(app.active_tab), Some(WorkspaceTab::RedisBrowser(tab)) if tab.preview_editor_id == target.session_id)
            {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: target.session_id,
                    rows: -3,
                    columns: 0,
                });
            }
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
            if app.omni.is_some()
                && ui.target_at(event.column, event.row).is_some_and(|target| {
                    matches!(
                        target,
                        HitTarget::Omni
                            | HitTarget::OmniItem(_)
                            | HitTarget::OmniTogglePanel
                            | HitTarget::OmniScrollbarPage { .. }
                            | HitTarget::OmniScrollbarThumb { .. }
                    )
                })
            {
                return Some(Action::OmniScroll(-3));
            }
            if matches!(app.overlay, Some(Overlay::Help(_)))
                && ui.target_at(event.column, event.row).is_some_and(|target| {
                    matches!(
                        target,
                        HitTarget::Help
                            | HitTarget::HelpItem(_)
                            | HitTarget::HelpTogglePanel
                            | HitTarget::HelpScrollbarPage { .. }
                            | HitTarget::HelpScrollbarThumb { .. }
                    )
                })
            {
                return Some(Action::HelpScroll(-3));
            }
            if app.overlay.is_some() {
                return None;
            }
            if matches!(
                app.tabs.get(app.active_tab),
                Some(WorkspaceTab::PrincipalDdl(tab))
                    if tab.view == crate::model::principal::PrincipalView::Overview
            ) && principal_access_body_contains(ui, event.column, event.row)
            {
                return Some(Action::ScrollPrincipalAccess(-3));
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Explorer => Some(Action::ExplorerScrollNodes {
                    direction: -1,
                    amount: ExplorerScrollAmount::Lines(3),
                }),
                Focus::Results if is_ddl_only_focus(app) => ddl_scroll_action(app, -3),
                Focus::Results if is_output_focus(app) => output_scroll_action(app, -3, 0),
                Focus::Results
                    if matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                            if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                    ) =>
                {
                    Some(Action::RedisPreviewScroll(-3))
                }
                Focus::Results
                    if matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                            if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Keys
                    ) =>
                {
                    Some(Action::RedisKeysScroll(-3))
                }
                Focus::Results => app
                    .active_grid_can_scroll_rows(-1, GridScrollAmount::Lines(3))
                    .then_some(Action::GridScrollRows {
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
            if let Some(Overlay::RelationTransactionConfirm(review)) = app.overlay.as_ref()
                && ui.text_selection_targets.iter().any(|target| {
                    target.session_id == review.editor_session_id
                        && target.hit_maps.iter().any(|map| {
                            map.area
                                .contains(ratatui::layout::Position::new(event.column, event.row))
                        })
                })
            {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: review.editor_session_id,
                    rows: 0,
                    columns: -3,
                });
            }
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Results if is_ddl_only_focus(app) => ddl_horizontal_scroll_action(app, -3),
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
            if let Some(Overlay::RelationTransactionConfirm(review)) = app.overlay.as_ref()
                && ui.text_selection_targets.iter().any(|target| {
                    target.session_id == review.editor_session_id
                        && target.hit_maps.iter().any(|map| {
                            map.area
                                .contains(ratatui::layout::Position::new(event.column, event.row))
                        })
                })
            {
                return Some(Action::ReadOnlyEditorScroll {
                    session_id: review.editor_session_id,
                    rows: 0,
                    columns: 3,
                });
            }
            if app.overlay.is_some() {
                return None;
            }
            match focus_at(ui, event.column, event.row).unwrap_or(app.focus) {
                Focus::Results if is_ddl_only_focus(app) => ddl_horizontal_scroll_action(app, 3),
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
        crate::model::workspace::PaneSplit::RedisKeysWidth => app.pane_sizes.redis_keys_width,
    }
    .or(match drag.split {
        crate::model::workspace::PaneSplit::ExplorerWidth => ui.pane_layout.explorer_width,
        crate::model::workspace::PaneSplit::EditorHeight => ui.pane_layout.editor_height,
        crate::model::workspace::PaneSplit::RedisKeysWidth => ui.pane_layout.redis_keys_width,
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
        crate::model::workspace::PaneSplit::RedisKeysWidth => event.column,
    }
}

fn profile_button_action(button: ProfileButton) -> Action {
    match button {
        ProfileButton::Cancel => Action::CloseProfileManager,
        ProfileButton::Test => Action::ProfileTest,
        ProfileButton::Save => Action::ProfileSave { connect: true },
        ProfileButton::ConfirmDelete => Action::ActivateProfileDelete,
        ProfileButton::CancelDelete => Action::ProfileCancelDelete,
    }
}

fn focus_at(ui: &UiState, column: u16, row: u16) -> Option<Focus> {
    match ui.target_at(column, row)? {
        HitTarget::PaneResize(PaneSplit::RedisKeysWidth) => Some(Focus::Results),
        HitTarget::Focus(focus) => Some(*focus),
        HitTarget::ExplorerRow(_) => Some(Focus::Explorer),
        HitTarget::ExplorerToggle(_) => Some(Focus::Explorer),
        HitTarget::RedisKeyNode { .. } => Some(Focus::Results),
        HitTarget::RedisKeyToggle { .. } => Some(Focus::Results),
        HitTarget::RedisFindInput(_) => Some(Focus::Results),
        HitTarget::RedisValueFilter(_) => Some(Focus::Results),
        HitTarget::RedisPreviewFormat(_) => Some(Focus::Results),
        HitTarget::RedisPreviewWrap(_) => Some(Focus::Results),
        HitTarget::RedisPreviewFocus(_) => Some(Focus::Results),
        HitTarget::RedisPreviewTableCell { .. } => Some(Focus::Results),
        HitTarget::RedisPreviewLoadMore(_) => Some(Focus::Results),
        HitTarget::MongoPageLoadMore(_) => Some(Focus::Results),
        HitTarget::MongoQueryField(_, _) => Some(Focus::Results),
        HitTarget::RedisValueSaveAction(_)
        | HitTarget::RedisUnsavedValueAction(_)
        | HitTarget::WorkspaceSaveAction(_) => None,
        HitTarget::ResultCell { .. }
        | HitTarget::ToggleResultView
        | HitTarget::ResultView(_)
        | HitTarget::RelationView(_)
        | HitTarget::PrincipalView(_)
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
        HitTarget::HelpItem(_)
        | HitTarget::HelpTogglePanel
        | HitTarget::HelpScrollbarPage { .. }
        | HitTarget::HelpScrollbarThumb { .. }
        | HitTarget::Omni
        | HitTarget::OmniItem(_)
        | HitTarget::OmniTogglePanel
        | HitTarget::OmniScrollbarPage { .. }
        | HitTarget::OmniScrollbarThumb { .. } => None,
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
        | HitTarget::ProfileField(_)
        | HitTarget::ProfileDriver(_)
        | HitTarget::ProfileCategory(_)
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
        | HitTarget::CatalogEditorAddTableConstraintPrimaryKey
        | HitTarget::CatalogEditorAddTableConstraintUnique
        | HitTarget::CatalogEditorAddTableConstraintCheck
        | HitTarget::CatalogEditorAddTableConstraintForeignKey
        | HitTarget::CatalogEditorFocusTableConstraint(_)
        | HitTarget::CatalogEditorRemoveTableConstraint(_)
        | HitTarget::CatalogEditorConstraintField { .. }
        | HitTarget::CatalogEditorRemoveTableColumn
        | HitTarget::CatalogEditorRestoreTableColumn
        | HitTarget::CatalogEditorRemoveTableColumnRow(_)
        | HitTarget::CatalogEditorRestoreTableColumnRow(_)
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
        | HitTarget::RedisDeleteConfirm
        | HitTarget::RedisDeleteCancel
        | HitTarget::DeleteConsoleCancel
        | HitTarget::SqlEditorListDeleteConfirm
        | HitTarget::CatalogDropCancel
        | HitTarget::CatalogDropConfirm
        | HitTarget::PrincipalDropCancel
        | HitTarget::PrincipalDropConfirm
        | HitTarget::SqlEditorListDeleteCancel => None,
        HitTarget::ManualCancellationKeepRunning | HitTarget::ManualCancellationConfirm => None,
        HitTarget::ExecutionConfirm | HitTarget::ExecutionCancel => None,
        HitTarget::PrincipalMutationCancel | HitTarget::PrincipalMutationApply => None,
        HitTarget::PrincipalPermission(_) => Some(Focus::Results),
        HitTarget::PrincipalAccessSection(_)
        | HitTarget::PrincipalAccessItem(_)
        | HitTarget::PrincipalAccessBody { .. }
        | HitTarget::PrincipalAccessScrollbarPage { .. }
        | HitTarget::PrincipalAccessScrollbarThumb { .. }
        | HitTarget::PrincipalAccessDetails => Some(Focus::Results),
        HitTarget::PrincipalMutationForm => None,
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
        HitTarget::Shortcut(_) => None,
    }
}

fn principal_access_target_is_current(
    tab_id: uuid::Uuid,
    section: crate::model::principal::PrincipalAccessSection,
    app: &App,
) -> bool {
    matches!(app.tabs.get(app.active_tab), Some(WorkspaceTab::PrincipalDdl(tab))
        if tab.id == tab_id
            && tab.view == crate::model::principal::PrincipalView::Overview
            && tab.access_section == section)
}

fn principal_access_drag_is_current(
    drag: crate::ui::PrincipalAccessScrollbarDrag,
    app: &App,
) -> bool {
    principal_access_target_is_current(drag.tab_id, drag.section, app)
}

fn principal_access_body_contains(ui: &UiState, column: u16, row: u16) -> bool {
    ui.principal_access_viewport
        .is_some_and(|(_, _, body)| body.contains(ratatui::layout::Position::new(column, row)))
}

fn is_relation_ddl_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && matches!(
            app.tabs.get(app.active_tab),
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Ddl
        )
}

/// DDL-only read-only focus shared by the relation DDL view and the
/// principal (user/role) DDL tab.
fn is_ddl_only_focus(app: &App) -> bool {
    is_relation_ddl_focus(app)
        || (app.focus == Focus::Results
            && matches!(
                app.tabs.get(app.active_tab),
                Some(WorkspaceTab::PrincipalDdl(_))
            ))
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
    let session_id = match app.tabs.get(app.active_tab) {
        Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Ddl => tab.ddl_editor_id,
        Some(WorkspaceTab::PrincipalDdl(tab)) => tab.editor_id,
        _ => return None,
    };
    Some(Action::ReadOnlyEditorScroll {
        session_id,
        rows,
        columns: 0,
    })
}

fn ddl_horizontal_scroll_action(app: &App, columns: isize) -> Option<Action> {
    let session_id = match app.tabs.get(app.active_tab) {
        Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Ddl => tab.ddl_editor_id,
        Some(WorkspaceTab::PrincipalDdl(tab)) => tab.editor_id,
        _ => return None,
    };
    Some(Action::ReadOnlyEditorScroll {
        session_id,
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
