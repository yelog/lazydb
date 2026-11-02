//! Workspace view for database users and roles.
//!
//! A principal tab has a compact overview and a read-only DDL view.
//! Rendering is delegated to [`super::read_only_sql::ReadOnlySqlEditor`] so the
//! syntax highlighting, scrollbars, mouse handling and Vim bindings match the
//! existing relation DDL view exactly.

use super::{loading, panel_block, read_only_sql::ReadOnlySqlEditor, theme::Theme};
use crate::{
    app::App,
    model::{
        editor::EditorViewport, principal::PrincipalView, tab::WorkspaceTab, workspace::Focus,
    },
    ui::{HitRegion, HitTarget},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

/// Vertical split used by the principal DDL view.
///
/// The first chunk contains the Overview/DDL selector, the second is an
/// optional status row, and the third owns the editor.
pub(crate) fn principal_ddl_layout(area: Rect, has_status: bool) -> [Rect; 3] {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_status {
            vec![
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Min(1),
            ]
        } else {
            vec![
                Constraint::Length(2),
                Constraint::Length(0),
                Constraint::Min(1),
            ]
        })
        .split(area);
    [chunks[0], chunks[1], chunks[2]]
}

fn render_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    view: PrincipalView,
    theme: Theme,
    state: &mut super::UiState,
) {
    let regions = super::render_tab_selectors(
        frame,
        area,
        &["OVERVIEW", "DDL"],
        usize::from(view == PrincipalView::Ddl),
        theme,
    );
    for (region, view) in regions
        .into_iter()
        .zip([PrincipalView::Overview, PrincipalView::Ddl])
    {
        state.hit_regions.push(HitRegion {
            area: region,
            target: HitTarget::PrincipalView(view),
        });
    }
}

fn ddl_block(focused: bool, theme: Theme) -> ratatui::widgets::Block<'static> {
    panel_block("", focused, theme)
}

/// Workspace view for a single database user or role.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
) {
    let Some(WorkspaceTab::PrincipalDdl(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let status = tab.load.status();
    let has_status = status.is_some();
    if tab.view == PrincipalView::Overview {
        render_overview(frame, area, app, theme, state);
        return;
    }
    let layout = principal_ddl_layout(area, has_status);
    render_tabs(frame, layout[0], tab.view, theme, state);
    state.hit_regions.push(HitRegion {
        area: layout[2],
        target: HitTarget::Focus(Focus::Results),
    });
    let mut block = ddl_block(app.focus == Focus::Results, theme);
    if let Some(snapshot) = tab.load.snapshot() {
        let provenance = provenance_label(app, snapshot.connection, tab.entry.id.profile_id);
        block = block.title_top(Line::raw(format!(" {provenance} ")).right_aligned());
    }
    if let Some((message, is_error)) = status {
        let inner = panel_block("", false, theme).inner(layout[1]);
        render_status(
            frame,
            inner,
            &message,
            is_error,
            app.focus == Focus::Results,
            theme,
        );
    }
    render_ddl_editor(frame, layout[2], app, theme, state, block);
}

fn render_overview(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
) {
    let Some(WorkspaceTab::PrincipalDdl(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(area);
    render_tabs(frame, chunks[0], tab.view, theme, state);
    let identity = match tab.entry.kind {
        crate::db::principal::PrincipalKind::User => "USER",
        crate::db::principal::PrincipalKind::Role => "ROLE",
    };
    let summary = format!(
        " {identity}  {}   native: {}   scope: {}",
        tab.entry.name,
        tab.entry.native_kind,
        principal_scope(&tab.entry.id.scope),
    );
    frame.render_widget(
        Paragraph::new(summary)
            .block(panel_block(
                " PRINCIPAL ",
                app.focus == Focus::Results,
                theme,
            ))
            .style(Style::new().fg(theme.text).bg(theme.surface)),
        chunks[1],
    );
    let focused = app.focus == Focus::Results && app.overlay.is_none();
    let block = panel_block(" ACCESS ", focused, theme);
    let inner = block.inner(chunks[2]);
    state.hit_regions.push(HitRegion {
        area: chunks[2],
        target: HitTarget::Focus(Focus::Results),
    });
    let sections = crate::model::principal::PrincipalAccessSection::ALL;
    let mut lines = Vec::new();
    let mut section_x = inner.x;
    let mut section_line = Line::default();
    for section in sections {
        let (label, count) = access_section_label(section, tab.details.snapshot());
        let selected = section == tab.access_section;
        let text = format!(" {label} ({count}) ");
        let width = text.width() as u16;
        section_line.spans.push(Span::styled(
            text.clone(),
            if selected {
                theme.title(focused)
            } else {
                Style::new().fg(theme.muted)
            },
        ));
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                section_x,
                inner.y,
                width.min(inner.right().saturating_sub(section_x)),
                1,
            ),
            target: HitTarget::PrincipalAccessSection(section),
        });
        section_x = section_x.saturating_add(width);
    }
    lines.push(section_line);
    let find_row = tab.access_find.is_some();
    let header_y = inner.y.saturating_add(1 + u16::from(find_row));
    let body_y = header_y.saturating_add(1);
    let footer_y = inner.bottom().saturating_sub(1);
    let scrollbar_width = u16::from(inner.width > 1);
    let body_width = inner.width.saturating_sub(scrollbar_width);
    lines.push(Line::styled(
        access_header(tab.access_section, body_width),
        Style::new()
            .fg(theme.grid_header_text)
            .bg(theme.grid_header),
    ));
    if let Some(find) = tab.access_find.as_ref() {
        let (current, total) = find.position();
        lines.insert(
            1,
            Line::styled(
                format!(
                    "/{}  {current}/{total}   Enter confirm  Esc close  n/N next/prev",
                    find.query.value()
                ),
                Style::new().fg(theme.action).bg(theme.surface),
            ),
        );
    }
    let visible = footer_y.saturating_sub(body_y) as usize;
    let body = Rect::new(inner.x, body_y, body_width, visible as u16);
    state.principal_access_viewport = Some((tab.id, visible, body));
    if !body.is_empty() {
        state.hit_regions.push(HitRegion {
            area: body,
            target: HitTarget::PrincipalAccessBody {
                tab_id: tab.id,
                section: tab.access_section,
            },
        });
    }
    if let Some(find) = tab.access_find.as_ref()
        && find.phase == crate::model::principal::PrincipalFindPhase::Editing
    {
        state.cursor = Some(super::CursorSpec {
            position: ratatui::layout::Position::new(
                inner
                    .x
                    .saturating_add(1)
                    .saturating_add(find.query.value().width().min(inner.width as usize) as u16),
                inner.y.saturating_add(1),
            ),
            style: super::CursorStyle::Bar,
        });
    }
    if let Some(details) = tab.details.snapshot() {
        let (count, stored_offset) = access_count_offset(tab, details);
        let offset = if visible == 0 {
            stored_offset
        } else {
            stored_offset.max(
                tab.access_selection()
                    .saturating_sub(visible.saturating_sub(1)),
            )
        };
        for row in 0..visible.min(count.saturating_sub(offset)) {
            let index = offset + row;
            let selected = index == tab.access_selection();
            let line = access_line(
                tab.access_section,
                details,
                index,
                body_width,
                selected,
                focused,
                theme,
            );
            let line = tab
                .access_find
                .as_ref()
                .filter(|find| find.matches.contains(&index))
                .map_or(line.clone(), |find| {
                    highlight_access_line(line, find.query.value(), selected, focused, theme)
                });
            lines.push(line);
            state.hit_regions.push(HitRegion {
                area: Rect::new(inner.x, body_y.saturating_add(row as u16), body_width, 1),
                target: HitTarget::PrincipalAccessItem(index),
            });
        }
        if count == 0 {
            lines.push(Line::styled(
                "No entries in the current snapshot.",
                Style::new().fg(theme.muted),
            ));
        }
        let coverage = match tab.access_section {
            crate::model::principal::PrincipalAccessSection::Permissions => {
                &details.permissions_coverage
            }
            _ => &details.membership_coverage,
        };
        lines.push(Line::styled(
            format!("Coverage: {}", coverage_label(coverage)),
            Style::new().fg(theme.muted),
        ));
    } else {
        let message = tab.details.status().map_or_else(
            || "No access snapshot available.".to_owned(),
            |(message, _)| message,
        );
        lines.push(Line::styled(message, Style::new().fg(theme.muted)));
    }
    let hint = match app.principal_capability(tab.entry.id.profile_id) {
        Some(crate::db::principal::PrincipalCapability::DetailsAndMutation)
            if tab.access_section
                == crate::model::principal::PrincipalAccessSection::Permissions =>
        {
            "↑/↓ select  ←/→ section  gg/G ends  y copy  v details  e edit  (only structured direct grants)  r refresh  o DDL"
        }
        Some(crate::db::principal::PrincipalCapability::Details) => {
            "↑/↓ select  ←/→ section  gg/G ends  y copy  v details  read-only  r refresh  o DDL"
        }
        _ => "↑/↓ select  ←/→ section  gg/G ends  y copy  v details  read-only  r refresh  o DDL",
    };
    lines.push(Line::styled(hint, Style::new().fg(theme.muted)));
    frame.render_widget(Paragraph::new(lines).block(block), chunks[2]);
    if let Some(details) = tab.details.snapshot() {
        let (count, offset) = access_count_offset(tab, details);
        let track = Rect::new(
            inner.right().saturating_sub(1),
            body_y,
            1,
            visible.min(u16::MAX as usize) as u16,
        );
        if let Some(geometry) = super::scrollbar::geometry(track, visible, count, offset) {
            super::scrollbar::render_vertical(frame, track, geometry, theme);
            let before = geometry.thumb_start;
            let after = geometry
                .rail
                .height
                .saturating_sub(before)
                .saturating_sub(geometry.thumb_length);
            if before > 0 {
                state.hit_regions.push(HitRegion {
                    area: Rect::new(track.x, track.y.saturating_add(1), 1, before),
                    target: HitTarget::PrincipalAccessScrollbarPage {
                        tab_id: tab.id,
                        section: tab.access_section,
                        offset: offset.saturating_sub(visible),
                    },
                });
            }
            state.hit_regions.push(HitRegion {
                area: geometry.thumb_area(),
                target: HitTarget::PrincipalAccessScrollbarThumb {
                    tab_id: tab.id,
                    section: tab.access_section,
                    track_start: geometry.rail.y,
                    track_length: geometry.rail.height,
                    thumb_start: geometry.thumb_area().y,
                    thumb_length: geometry.thumb_length,
                    max_offset: geometry.max_offset,
                },
            });
            if after > 0 {
                state.hit_regions.push(HitRegion {
                    area: Rect::new(track.x, geometry.thumb_area().bottom(), 1, after),
                    target: HitTarget::PrincipalAccessScrollbarPage {
                        tab_id: tab.id,
                        section: tab.access_section,
                        offset: offset.saturating_add(visible).min(geometry.max_offset),
                    },
                });
            }
        }
    }
}

fn access_section_label(
    section: crate::model::principal::PrincipalAccessSection,
    details: Option<&crate::db::principal::PrincipalDetails>,
) -> (&'static str, usize) {
    let count = details.map_or(0, |details| match section {
        crate::model::principal::PrincipalAccessSection::Permissions => details.permissions.len(),
        crate::model::principal::PrincipalAccessSection::MemberOf => details.member_of.len(),
        crate::model::principal::PrincipalAccessSection::Members => details.members.len(),
    });
    (
        match section {
            crate::model::principal::PrincipalAccessSection::Permissions => "Permissions",
            crate::model::principal::PrincipalAccessSection::MemberOf => "Member of",
            crate::model::principal::PrincipalAccessSection::Members => "Members",
        },
        count,
    )
}

fn access_count_offset(
    tab: &crate::model::principal::PrincipalDdlTab,
    details: &crate::db::principal::PrincipalDetails,
) -> (usize, usize) {
    (
        access_section_label(tab.access_section, Some(details)).1,
        tab.access_offset(),
    )
}

fn highlight_access_line(
    line: Line<'static>,
    query: &str,
    selected: bool,
    focused: bool,
    theme: Theme,
) -> Line<'static> {
    let selection_background = if selected && focused {
        theme.selection
    } else {
        theme.surface
    };
    let mut spans = Vec::new();
    for span in line.spans {
        let text = span.content.into_owned();
        let ranges = crate::db::catalog::search_text_match_ranges(&text, query);
        if ranges.is_empty() {
            spans.push(Span::styled(text, span.style));
            continue;
        }
        let mut cursor = 0;
        for (start, end) in ranges {
            if cursor < start {
                spans.push(Span::styled(text[cursor..start].to_owned(), span.style));
            }
            spans.push(Span::styled(
                text[start..end].to_owned(),
                Style::new()
                    .fg(theme.warning)
                    .bg(selection_background)
                    .add_modifier(Modifier::BOLD),
            ));
            cursor = end;
        }
        if cursor < text.len() {
            spans.push(Span::styled(text[cursor..].to_owned(), span.style));
        }
    }
    Line::from(spans)
}

fn shorten(value: &str, width: usize) -> String {
    let value = crate::security::sanitize_terminal_text(value).replace(['\n', '\t'], " ");
    let mut result = String::new();
    if width == 0 {
        return result;
    }
    let value_width = value.width();
    let truncated = value_width > width;
    let content_width = if truncated {
        width.saturating_sub(1)
    } else {
        width
    };
    let mut used = 0_usize;
    for ch in value.chars() {
        let ch_width = ch.to_string().width();
        if used.saturating_add(ch_width) > content_width {
            break;
        }
        result.push(ch);
        used += ch_width;
    }
    if truncated && width > 0 {
        result.push('…');
        used += 1;
    }
    result.push_str(&" ".repeat(width.saturating_sub(used)));
    result
}

#[derive(Clone, Copy)]
struct PermissionColumnWidths {
    target: usize,
    privilege: usize,
    origin: usize,
}

fn permission_column_widths(width: u16) -> PermissionColumnWidths {
    PermissionColumnWidths {
        target: usize::from(width.saturating_sub(30)),
        privilege: 15,
        origin: 9,
    }
}

fn access_header(section: crate::model::principal::PrincipalAccessSection, width: u16) -> String {
    match section {
        crate::model::principal::PrincipalAccessSection::Permissions if width >= 56 => {
            let columns = permission_column_widths(width);
            format!(
                "  {}  {}  {}",
                shorten("Target", columns.target),
                shorten("Privilege", columns.privilege),
                shorten("Origin", columns.origin)
            )
        }
        crate::model::principal::PrincipalAccessSection::Permissions => "  Entry".to_owned(),
        crate::model::principal::PrincipalAccessSection::MemberOf => {
            "  Role                                           Admin option".to_owned()
        }
        crate::model::principal::PrincipalAccessSection::Members => {
            "  Member                                         Admin option".to_owned()
        }
    }
}

fn access_line(
    section: crate::model::principal::PrincipalAccessSection,
    details: &crate::db::principal::PrincipalDetails,
    index: usize,
    width: u16,
    selected: bool,
    focused: bool,
    theme: Theme,
) -> Line<'static> {
    let prefix = if selected { ">" } else { " " };
    let row_style = if selected && focused {
        Style::new().fg(theme.text).bg(theme.selection)
    } else if selected {
        Style::new().fg(theme.action).bg(theme.surface)
    } else {
        Style::new().fg(theme.text).bg(theme.surface)
    };
    if section == crate::model::principal::PrincipalAccessSection::Permissions && width >= 56 {
        let permission = &details.permissions[index];
        let columns = permission_column_widths(width);
        let source = source_label(permission.source_kind);
        let source_style = match permission.source_kind {
            crate::db::principal::PrincipalPermissionSource::Direct => {
                Style::new().fg(theme.success)
            }
            crate::db::principal::PrincipalPermissionSource::Owner => Style::new().fg(theme.accent),
            crate::db::principal::PrincipalPermissionSource::Default => {
                Style::new().fg(theme.warning)
            }
            _ => Style::new().fg(theme.muted),
        }
        .bg(if selected && focused {
            theme.selection
        } else {
            theme.surface
        });
        return Line::from(vec![
            Span::styled(format!("{prefix} "), row_style),
            Span::styled(shorten(&permission.target, columns.target), row_style),
            Span::styled("  ", row_style),
            Span::styled(
                shorten(&permission.privilege, columns.privilege),
                Style::new()
                    .fg(theme.action)
                    .bg(row_style.bg.unwrap_or(theme.surface)),
            ),
            Span::styled("  ", row_style),
            Span::styled(shorten(source, columns.origin), source_style),
        ]);
    }
    let base = match section {
        crate::model::principal::PrincipalAccessSection::Permissions => {
            let p = &details.permissions[index];
            let available = usize::from(width.saturating_sub(1));
            let target_width = available.min(p.target.width());
            format!("{prefix} {}", shorten(&p.target, target_width))
        }
        crate::model::principal::PrincipalAccessSection::MemberOf => {
            let m = &details.member_of[index];
            format!(
                "{prefix} {}  {}",
                shorten(&m.role, 42),
                if m.admin_option { "ADMIN" } else { "-" }
            )
        }
        crate::model::principal::PrincipalAccessSection::Members => {
            let m = &details.members[index];
            format!(
                "{prefix} {}  {}",
                shorten(&m.member, 42),
                if m.admin_option { "ADMIN" } else { "-" }
            )
        }
    };
    let fg = if selected && focused {
        theme.text
    } else if selected {
        theme.action
    } else {
        theme.text
    };
    let bg = if selected && focused {
        theme.selection
    } else {
        theme.surface
    };
    Line::styled(
        shorten(&base, width.saturating_sub(1) as usize),
        Style::new().fg(fg).bg(bg),
    )
}

fn source_label(source: crate::db::principal::PrincipalPermissionSource) -> &'static str {
    match source {
        crate::db::principal::PrincipalPermissionSource::Direct => "Direct",
        crate::db::principal::PrincipalPermissionSource::Public => "Public",
        crate::db::principal::PrincipalPermissionSource::Owner => "Owner",
        crate::db::principal::PrincipalPermissionSource::Default => "Default",
        crate::db::principal::PrincipalPermissionSource::Inherited => "Inherited",
    }
}

fn coverage_label(coverage: &crate::db::principal::PrincipalCoverage) -> String {
    match coverage {
        crate::db::principal::PrincipalCoverage::Complete => "Complete".to_owned(),
        crate::db::principal::PrincipalCoverage::Partial(reason) => format!("Partial: {reason}"),
        crate::db::principal::PrincipalCoverage::Unavailable(reason) => {
            format!("Unavailable: {reason}")
        }
        crate::db::principal::PrincipalCoverage::Unsupported(reason) => {
            format!("Unsupported: {reason}")
        }
    }
}

fn principal_scope(scope: &crate::db::principal::PrincipalScope) -> String {
    match scope {
        crate::db::principal::PrincipalScope::Cluster => "cluster".to_owned(),
        crate::db::principal::PrincipalScope::Server => "server".to_owned(),
        crate::db::principal::PrincipalScope::Database(database) => format!("database:{database}"),
    }
}

fn render_status(
    frame: &mut Frame<'_>,
    area: Rect,
    message: &str,
    is_error: bool,
    focused: bool,
    theme: Theme,
) {
    let style = if is_error {
        Style::new().fg(theme.error)
    } else {
        Style::new().fg(theme.action)
    }
    .bg(theme.surface)
    .add_modifier(if focused {
        Modifier::empty()
    } else {
        Modifier::DIM
    });
    let indicator = loading::activity_text(message, None, false, std::time::Duration::ZERO);
    frame.render_widget(Paragraph::new(indicator).style(style), area);
}

fn render_ddl_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
    block: ratatui::widgets::Block<'_>,
) {
    let inner = block.inner(area);
    let viewport = EditorViewport {
        width: inner.width as usize,
        height: inner.height as usize,
    };
    let Some(WorkspaceTab::PrincipalDdl(tab)) = app.tabs.get(app.active_tab) else {
        frame.render_widget(block, area);
        return;
    };
    let editor_id = tab.editor_id;
    let Ok(preliminary_snapshot) = app.active_principal_editor_snapshot(viewport) else {
        frame.render_widget(block, area);
        return;
    };
    let text_viewport = super::read_only_sql::editor_viewport(inner, &preliminary_snapshot, true);
    state.ddl_editor_viewport = Some((editor_id, text_viewport));
    let Ok(snapshot) = (if text_viewport == viewport {
        Ok(preliminary_snapshot)
    } else {
        app.active_principal_editor_snapshot(text_viewport)
    }) else {
        frame.render_widget(block, area);
        return;
    };
    super::register_text_selection_target(
        state,
        editor_id,
        Rect::new(inner.x, inner.y, inner.width, inner.height),
        &snapshot,
    );
    ReadOnlySqlEditor {
        session_id: editor_id,
        snapshot: &snapshot,
        block,
        focused: app.focus == Focus::Results && app.overlay.is_none(),
        show_line_numbers: true,
    }
    .render(frame, area, theme, state);
}

fn provenance_label(
    app: &App,
    snapshot: crate::identity::ConnectionIdentity,
    profile_id: uuid::Uuid,
) -> &'static str {
    if app.profiles.iter().all(|profile| profile.id != profile_id) {
        return "PROFILE DELETED";
    }
    if app.connection.active_identity() == Some(snapshot) {
        "LIVE"
    } else {
        "OFFLINE"
    }
}
