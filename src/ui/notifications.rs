use ratatui::layout::Position;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::App,
    model::notification::{
        HistorySearchPhase, Notification, NotificationHistoryState, NotificationLevel,
    },
    model::text_detail::TextDetailRequest,
    security::sanitize_terminal_text,
    ui::{HitRegion, HitTarget, UiState, icons::IconSet, theme::Theme},
};

const MAX_CARDS: usize = 4;
const MIN_WIDTH: u16 = 24;
const MAX_WIDTH: u16 = 56;

pub(crate) fn render(
    frame: &mut Frame<'_>,
    viewport: Rect,
    app: &App,
    theme: Theme,
    state: &mut UiState,
    icons: IconSet,
) {
    if viewport.width < MIN_WIDTH || viewport.height < 4 {
        return;
    }

    let entries = app
        .notifications
        .live()
        .iter()
        .rev()
        .take(MAX_CARDS)
        .filter_map(|live| app.notifications.get(live.notification_id))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return;
    }

    let width = card_width(viewport.width, &entries);
    let mut y = viewport.y.saturating_add(1);
    for notification in entries {
        if y >= viewport.bottom() {
            break;
        }
        let height = card_height(notification, width, icons);
        let height = height.min(viewport.bottom().saturating_sub(y));
        if height == 0 {
            break;
        }
        let area = Rect::new(viewport.right().saturating_sub(width), y, width, height);
        draw_card(frame, area, notification, theme, icons);
        let close_width = icons.close().width().max(1) as u16 + 2;
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                area.x,
                area.y.saturating_add(1),
                area.width.saturating_sub(close_width),
                area.height.saturating_sub(1),
            ),
            target: HitTarget::OpenTextDetail(notification_detail_request(notification)),
        });
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                area.right().saturating_sub(close_width),
                area.y,
                close_width.min(area.width),
                1,
            ),
            target: HitTarget::DismissNotification(notification.id),
        });
        y = y.saturating_add(height).saturating_add(1);
    }
}

pub(crate) fn render_history(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    history: &NotificationHistoryState,
    theme: Theme,
    state: &mut UiState,
) {
    let narrow = area.width < 72;
    let popup = centered_history(area, narrow);
    frame.render_widget(ratatui::widgets::Clear, popup);
    let block = Block::default()
        .title(" NOTIFICATION HISTORY ")
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.border))
        .style(Style::new().bg(theme.surface_raised));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.is_empty() {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    let body = if narrow {
        vec![chunks[1]]
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(chunks[1])
            .to_vec()
    };
    let entries = app.notifications.history().cloned().collect::<Vec<_>>();
    let matches = history.matching_indices(&entries).collect::<Vec<_>>();
    let search = if history.phase == HistorySearchPhase::Editing {
        "/"
    } else {
        ""
    };
    frame.render_widget(
        Paragraph::new(format!("{search}{}", history.query.value()))
            .style(Style::new().fg(theme.action)),
        chunks[0],
    );
    let selected = history.selected.min(entries.len().saturating_sub(1));
    let mut lines = Vec::new();
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "No notifications",
            Style::new().fg(theme.muted),
        )));
    } else {
        for (index, notification) in entries.iter().enumerate().take(usize::from(body[0].height)) {
            let marker = if index == selected { ">" } else { " " };
            let style = if index == selected {
                Style::new().fg(theme.text).bg(theme.selection)
            } else {
                Style::new().fg(theme.text)
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "{marker} {}  {:<7} {}",
                    notification.created_at.format("%H:%M:%S"),
                    notification.level,
                    sanitize_terminal_text(&notification.title)
                ),
                style,
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), body[0]);
    if narrow && let Some(notification) = entries.get(selected) {
        state.hit_regions.push(HitRegion {
            area: body[0],
            target: HitTarget::OpenTextDetail(notification_detail_request(notification)),
        });
    }
    if !narrow && let Some(notification) = entries.get(selected) {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(&notification.title, theme.title(true))),
                Line::raw(format!(
                    "{}  {}",
                    notification.level,
                    notification.created_at.format("%Y-%m-%d %H:%M:%S")
                )),
                Line::raw(format!(
                    "source: {}",
                    notification
                        .source
                        .map_or("unknown".to_owned(), |source| source.to_string())
                )),
                Line::raw(""),
                Line::raw(sanitize_terminal_text(&notification.body)),
            ])
            .wrap(Wrap { trim: true }),
            body[1],
        );
    }
    let footer = if history.clear_confirm {
        "Clear all notifications? y confirm  n/Esc cancel"
    } else {
        "j/k select  / search  n/N next/previous  c clear  Esc/q close"
    };
    frame.render_widget(
        Paragraph::new(footer).style(Style::new().fg(if history.clear_confirm {
            theme.warning
        } else {
            theme.muted
        })),
        chunks[2],
    );
    if history.phase == HistorySearchPhase::Editing {
        state.cursor_style = Some(crate::ui::CursorStyle::Bar);
        let x = chunks[0]
            .x
            .saturating_add(history.query.value().width() as u16)
            .min(chunks[0].right().saturating_sub(1));
        frame.set_cursor_position(Position::new(x, chunks[0].y));
    }
    let _ = matches;
}

fn notification_detail_request(notification: &Notification) -> TextDetailRequest {
    let body = sanitize_terminal_text(&notification.body);
    TextDetailRequest::new(
        notification.title.clone(),
        uuid::Uuid::nil(),
        0,
        body.clone(),
        body,
        None,
    )
}

fn centered_history(area: Rect, narrow: bool) -> Rect {
    let width = if narrow {
        area.width.saturating_sub(2)
    } else {
        100.min(area.width.saturating_sub(4))
    };
    let height = 18.min(area.height.saturating_sub(2)).max(6);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn card_width(viewport_width: u16, entries: &[&Notification]) -> u16 {
    if viewport_width == 0 {
        return 0;
    }
    let wanted = entries
        .iter()
        .map(|notification| notification.title.width().max(notification.body.width()) as u16 + 8)
        .max()
        .unwrap_or(MIN_WIDTH);
    wanted
        .min(MAX_WIDTH)
        .min(viewport_width)
        .max(MIN_WIDTH.min(viewport_width))
}

fn card_height(notification: &Notification, width: u16, icons: IconSet) -> u16 {
    let content_width = usize::from(width.saturating_sub(4));
    let body_lines = wrapped_line_count(&sanitize_terminal_text(&notification.body), content_width);
    let title_lines = wrapped_line_count(
        &sanitize_terminal_text(&notification.title),
        content_width.saturating_sub(4),
    );
    (body_lines + title_lines.max(1) + 3).clamp(4, 8) as u16
        + u16::from(icons.notification(notification.level).width() == 0)
}

fn draw_card(
    frame: &mut Frame<'_>,
    area: Rect,
    notification: &Notification,
    theme: Theme,
    icons: IconSet,
) {
    let color = level_color(notification.level, theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(color))
        .style(Style::new().bg(theme.surface_raised));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let icon = icons.notification(notification.level);
    let title = sanitize_terminal_text(&notification.title);
    let body = sanitize_terminal_text(&notification.body);
    let time = notification.created_at.format("%H:%M:%S").to_string();
    let close = icons.close();
    let level = notification.level.to_string().to_uppercase();
    let title_width =
        usize::from(inner.width).saturating_sub(icon.width() + close.width() + level.width() + 6);
    let title = truncate_cells(&title, title_width);
    let line = Line::from(vec![
        Span::styled(
            format!("{icon} "),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{level} "),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title,
            Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {close}"), Style::new().fg(theme.muted)),
    ]);
    let text = vec![
        line,
        Line::from(Span::styled(body, Style::new().fg(theme.text))),
        Line::from(Span::styled(time, Style::new().fg(theme.muted))),
    ];
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
}

fn level_color(level: NotificationLevel, theme: Theme) -> ratatui::style::Color {
    match level {
        NotificationLevel::Info => theme.action,
        NotificationLevel::Success => theme.accent,
        NotificationLevel::Warning => theme.warning,
        NotificationLevel::Error => theme.error,
    }
}

fn wrapped_line_count(value: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    value
        .lines()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum::<usize>()
        .max(1)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::CellWidth;
    use std::time::Instant;

    #[test]
    fn width_and_text_helpers_use_display_cells() {
        assert_eq!("界".cell_width(), 2);
        assert_eq!(truncate_cells("界abc", 3), "界a");
        assert_eq!(wrapped_line_count("界界a", 4), 2);
    }

    #[test]
    fn card_width_is_clamped_for_small_and_large_terminals() {
        let notification = Notification {
            id: 1,
            level: NotificationLevel::Info,
            title: "title".into(),
            body: "body".into(),
            created_at: chrono::Local::now(),
            source: None,
        };
        assert_eq!(card_width(10, &[&notification]), 10);
        assert_eq!(card_width(100, &[&notification]), MIN_WIDTH);
    }

    #[test]
    fn history_popup_falls_back_to_full_width_on_narrow_terminals() {
        let popup = centered_history(Rect::new(0, 0, 60, 24), true);
        assert_eq!(popup.width, 58);
        assert!(popup.height <= 22);
    }

    #[test]
    fn notification_detail_request_copies_complete_sanitized_body() {
        let body = "first\nsecond\u{1b}[31m";
        let request = notification_detail_request(&Notification {
            id: 1,
            level: NotificationLevel::Error,
            title: "Error".into(),
            body: body.into(),
            created_at: chrono::Local::now(),
            source: None,
        });

        assert_eq!(request.display_text, sanitize_terminal_text(body));
        assert_eq!(request.copy_text, sanitize_terminal_text(body));
        assert_eq!(request.source_session_id, uuid::Uuid::nil());
        assert_eq!(request.source_revision, 0);
    }

    #[test]
    fn narrow_history_registers_an_explicit_detail_target_for_selected_row() {
        let mut app = App::new(Vec::new());
        app.notifications.push(
            NotificationLevel::Info,
            "Title",
            "Complete body",
            Instant::now(),
        );
        let history = NotificationHistoryState::new();
        let mut ui = UiState::new();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20))
            .expect("test terminal");
        terminal
            .draw(|frame| {
                render_history(
                    frame,
                    frame.area(),
                    &app,
                    &history,
                    Theme::default(),
                    &mut ui,
                );
            })
            .expect("render history");

        assert!(
            ui.hit_regions
                .iter()
                .any(|region| { matches!(region.target, HitTarget::OpenTextDetail(_)) })
        );
    }
}
