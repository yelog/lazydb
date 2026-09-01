use crate::model::pagination::{PageSize, ResultPagination, TotalRows};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{HitRegion, HitTarget, UiState, theme::Theme};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaginationKind {
    Relation,
    Result,
}

#[derive(Clone, Copy)]
enum PageAction {
    First,
    Previous,
    Next,
    Last,
    Size,
}

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    pagination: ResultPagination,
    kind: PaginationKind,
    theme: Theme,
    state: &mut UiState,
) {
    if area.is_empty() {
        return;
    }
    let (start, end) = pagination
        .range()
        .map_or((0, 0), |range| (*range.start(), *range.end()));
    let total = match pagination.total {
        TotalRows::Exact(value) => format_number(value),
        TotalRows::LowerBound(value) => format!("{}+", format_number(value)),
    };
    let labels = [
        (
            "|<".to_owned(),
            page_target(
                kind,
                PageAction::First,
                pagination.first_request().is_some(),
            ),
        ),
        (
            "<".to_owned(),
            page_target(
                kind,
                PageAction::Previous,
                pagination.previous_request().is_some(),
            ),
        ),
        (
            format!("{}-{}", format_number(start), format_number(end)),
            page_target(kind, PageAction::Size, true),
        ),
        ("of".to_owned(), None),
        (total, None),
        (
            ">".to_owned(),
            page_target(kind, PageAction::Next, pagination.next_request().is_some()),
        ),
        (
            ">|".to_owned(),
            page_target(kind, PageAction::Last, pagination.last_request().is_some()),
        ),
    ];
    let mut spans = Vec::new();
    let mut x = area.x;
    for (index, (label, target)) in labels.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
            x = x.saturating_add(1);
        }
        let width = label.chars().count() as u16;
        let enabled = target.is_some();
        if let Some(target) = target {
            state.hit_regions.push(HitRegion {
                area: Rect::new(x, area.y, width.min(area.right().saturating_sub(x)), 1),
                target,
            });
        }
        spans.push(Span::styled(
            label,
            Style::new()
                .fg(if enabled { theme.action } else { theme.muted })
                .bg(theme.surface),
        ));
        x = x.saturating_add(width);
    }
    spans.push(Span::styled(
        format!("  {} rows", pagination.page_size.get()),
        Style::new().fg(theme.muted).bg(theme.surface),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.base()), area);
}

fn page_target(kind: PaginationKind, action: PageAction, enabled: bool) -> Option<HitTarget> {
    if !enabled {
        return None;
    }
    Some(match (kind, action) {
        (PaginationKind::Relation, PageAction::First) => HitTarget::RelationFirstPage,
        (PaginationKind::Relation, PageAction::Previous) => HitTarget::RelationPreviousPage,
        (PaginationKind::Relation, PageAction::Next) => HitTarget::RelationNextPage,
        (PaginationKind::Relation, PageAction::Last) => HitTarget::RelationLastPage,
        (PaginationKind::Relation, PageAction::Size) => HitTarget::RelationPageSize,
        (PaginationKind::Result, PageAction::First) => HitTarget::ResultFirstPage,
        (PaginationKind::Result, PageAction::Previous) => HitTarget::ResultPreviousPage,
        (PaginationKind::Result, PageAction::Next) => HitTarget::ResultNextPage,
        (PaginationKind::Result, PageAction::Last) => HitTarget::ResultLastPage,
        (PaginationKind::Result, PageAction::Size) => HitTarget::ResultPageSize,
    })
}

fn format_number(value: u64) -> String {
    let text = value.to_string();
    let mut result = String::new();
    for (index, character) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(character);
    }
    result
}

pub(crate) fn selector_items() -> &'static [PageSize] {
    &PageSize::ALL
}
