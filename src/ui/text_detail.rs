use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::{
    HitRegion, HitTarget, UiState, editor_line_spans, panel_block, register_text_selection_target,
    theme::Theme,
};
use crate::{
    app::App,
    model::{editor::EditorViewport, text_detail::TextDetailState},
    security::sanitize_terminal_text,
};

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    view: &TextDetailState,
    theme: Theme,
    state: &mut UiState,
) {
    let popup = super::centered(
        area,
        area.width.saturating_sub(6).max(30),
        area.height.saturating_sub(4).max(8),
    );
    frame.render_widget(Clear, popup);
    let title = format!(" {} ", sanitize_terminal_text(&view.title));
    let block = panel_block(&title, true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);
    let viewport = EditorViewport {
        width: chunks[0].width.saturating_sub(1) as usize,
        height: chunks[0].height as usize,
    };
    let Ok(snapshot) = app.text_detail_snapshot(view.session_id, viewport) else {
        return;
    };
    register_text_selection_target(state, view.session_id, chunks[0], &snapshot);
    for (row, line) in snapshot.lines.iter().take(viewport.height).enumerate() {
        frame.render_widget(
            Paragraph::new(
                Line::from(editor_line_spans(line, &snapshot, theme, true, None))
                    .style(Style::new().bg(theme.surface_raised)),
            )
            .scroll((0, snapshot.horizontal_offset as u16)),
            Rect::new(chunks[0].x, chunks[0].y + row as u16, chunks[0].width, 1),
        );
    }
    let button_style = Style::new().fg(theme.action).add_modifier(Modifier::BOLD);
    let footer = Line::from(vec![
        Span::styled(" Copy selection ", button_style),
        Span::raw("  "),
        Span::styled(" Copy all ", button_style),
        Span::raw("  "),
        Span::styled(" Close ", button_style),
        Span::styled(
            "   Drag to select; arrows/page keys scroll; Esc close",
            Style::new().fg(theme.muted),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(footer).block(
            Block::new()
                .borders(Borders::TOP)
                .border_style(Style::new().fg(theme.border))
                .style(Style::new().bg(theme.surface_raised)),
        ),
        chunks[1],
    );
    let y = chunks[1].y;
    let x = chunks[1].x;
    state.hit_regions.extend([
        HitRegion {
            area: Rect::new(x, y, 18.min(chunks[1].width), 1),
            target: HitTarget::TextDetailCopySelection,
        },
        HitRegion {
            area: Rect::new(
                x.saturating_add(20),
                y,
                12.min(chunks[1].width.saturating_sub(20)),
                1,
            ),
            target: HitTarget::TextDetailCopyAll,
        },
        HitRegion {
            area: Rect::new(
                x.saturating_add(34),
                y,
                8.min(chunks[1].width.saturating_sub(34)),
                1,
            ),
            target: HitTarget::TextDetailClose,
        },
    ]);
}
