use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthChar;

use crate::{app::App, security::sanitize_terminal_text};

use super::{CursorSpec, CursorStyle, HitRegion, HitTarget, UiState, theme::Theme};

pub(super) fn render(frame: &mut Frame<'_>, app: &App, state: &mut UiState, theme: Theme) {
    let Some(omni) = app.omni.as_ref() else {
        return;
    };
    let area = frame.area();
    let width = area.width.min(88).saturating_sub(2);
    let height = area.height.min(22).saturating_sub(2);
    if width < 10 || height < 5 {
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new("Terminal too small for Omni. Resize or press Esc.")
                .style(Style::new().fg(theme.text).bg(theme.surface)),
            area,
        );
        return;
    }
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 3,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.accent))
        .style(Style::new().fg(theme.text).bg(theme.surface_raised))
        .title(" OMNI ")
        .title_bottom(" Type to search  Enter open  Tab actions  Esc close ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let query = sanitize_terminal_text(omni.query());
    let input = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "> ",
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(query, Style::new().fg(theme.text)),
        ]))
        .style(Style::new().bg(theme.surface_raised)),
        input,
    );
    let cursor_column = omni
        .query()
        .chars()
        .take(usize::from(inner.width.saturating_sub(2)))
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>()
        .min(usize::from(inner.width.saturating_sub(1)));
    state.cursor = Some(CursorSpec {
        position: Position::new(
            input
                .x
                .saturating_add(2)
                .saturating_add(cursor_column as u16),
            input.y,
        ),
        style: CursorStyle::Bar,
    });
    state.hit_regions.push(HitRegion {
        area: popup,
        target: HitTarget::Omni,
    });

    let rows = height.saturating_sub(4) as usize;
    let visible = omni.visible_items();
    let start = omni.scroll.min(visible.len().saturating_sub(rows));
    let end = start.saturating_add(rows).min(visible.len());
    let items = visible[start..end]
        .iter()
        .enumerate()
        .map(|(offset, item)| {
            let selected = Some(&item.id) == omni.selected.as_ref();
            let mut title = sanitize_terminal_text(&item.title);
            if item.opened {
                title.push_str("  [open]");
            }
            let subtitle = sanitize_terminal_text(&item.subtitle);
            let width = usize::from(inner.width);
            let title_width = usize::from(title.cell_width());
            let subtitle_width = usize::from(subtitle.cell_width());
            let line = if subtitle.is_empty() || width <= title_width.saturating_add(2) {
                truncate_cells(&title, width)
            } else {
                let available = width.saturating_sub(subtitle_width.saturating_add(3));
                format!("{}  {}", truncate_cells(&title, available), subtitle)
            };
            let style = if selected {
                Style::new()
                    .fg(theme.background)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            let row = inner.y.saturating_add(2).saturating_add(offset as u16);
            state.hit_regions.push(HitRegion {
                area: Rect::new(inner.x, row, inner.width, 1),
                target: HitTarget::OmniItem(start + offset),
            });
            ListItem::new(line).style(style)
        })
        .collect::<Vec<_>>();
    let results = Rect::new(inner.x, inner.y.saturating_add(2), inner.width, rows as u16);
    frame.render_widget(
        List::new(items).style(Style::new().bg(theme.surface_raised)),
        results,
    );

    let status = omni.status.as_deref().unwrap_or({
        if visible.is_empty() {
            "No matching actions or objects"
        } else {
            ""
        }
    });
    if !status.is_empty() && inner.height > 1 {
        frame.render_widget(
            Paragraph::new(sanitize_terminal_text(status))
                .style(Style::new().fg(theme.muted).bg(theme.surface_raised)),
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
    }
}

fn truncate_cells(value: &str, width: usize) -> String {
    let mut used = 0;
    value
        .chars()
        .take_while(|character| {
            let next = character.width().unwrap_or(0);
            if used + next > width {
                false
            } else {
                used += next;
                true
            }
        })
        .collect()
}
