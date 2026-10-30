use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Cell, HighlightSpacing, List, ListItem, Paragraph, Row, Table},
};

use crate::model::{
    mongodb_browser::{MongoPageState, document_cell},
    tab::WorkspaceTab,
};

/// Width of the document counter that prefixes every grid row.
const ROW_NUMBER_WIDTH: u16 = 3;

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &crate::app::App,
    theme: crate::ui::theme::Theme,
    ui: &mut crate::ui::UiState,
) {
    let Some(WorkspaceTab::MongoBrowser(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let region = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    // The filter bar owns its own row, otherwise it paints over the pane titles.
    let query_bar = region[0];
    // Reserve the bottom row for the status line, then give the inspector a
    // column only when it has to be on screen.
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(region[1]);
    let body_area = body[0];
    let status_area = body[1];
    let inspector_area = tab.inspector_visible().then(|| {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(body_area)[1]
    });
    let documents_area = inspector_area.map_or(body_area, |inspector| {
        Rect::new(
            body_area.x,
            body_area.y,
            body_area.width.saturating_sub(inspector.width),
            body_area.height,
        )
    });
    let document_rows: Vec<ListItem<'_>> = match &tab.page {
        MongoPageState::Empty => vec![ListItem::new("Choose a collection to load documents")],
        MongoPageState::Loading => vec![ListItem::new("Loading documents…")],
        MongoPageState::Failed(error) => vec![ListItem::new(format!("Load failed: {error}"))],
        MongoPageState::Ready(page) if page.documents.is_empty() => {
            vec![ListItem::new("No documents")]
        }
        MongoPageState::Ready(_) => Vec::new(),
    };
    let block = Block::default().title("Documents").borders(Borders::ALL);
    if let MongoPageState::Ready(page) = &tab.page
        && !page.documents.is_empty()
    {
        let (fields, widths) = crate::model::mongodb_browser::page_grid_columns(
            page,
            documents_area.width.saturating_sub(2),
        );
        let header = Row::new(
            std::iter::once("#".to_owned())
                .chain(fields.iter().cloned())
                .map(Cell::from)
                .collect::<Vec<_>>(),
        )
        .style(Style::new().fg(theme.muted).add_modifier(Modifier::BOLD));
        let rows =
            page.documents
                .iter()
                .enumerate()
                .map(|(index, document)| {
                    let cells = std::iter::once((index + 1).to_string())
                        .chain(fields.iter().zip(&widths).map(|(field, width)| {
                            clip_cell(document_cell(document, field), *width)
                        }))
                        .map(Cell::from)
                        .collect::<Vec<_>>();
                    let row = Row::new(cells);
                    if index == tab.selected_document {
                        row.style(Style::new().bg(theme.selection))
                    } else {
                        row
                    }
                })
                .collect::<Vec<_>>();
        frame.render_widget(
            Table::new(rows, column_widths(&widths))
                .header(header)
                .column_spacing(1)
                .block(block)
                .highlight_spacing(HighlightSpacing::Never),
            documents_area,
        );
    } else {
        frame.render_widget(List::new(document_rows).block(block), documents_area);
    };
    let detail = if tab.query_field.is_some() {
        format!(
            "{}{}",
            tab.query_text,
            tab.query_error
                .as_ref()
                .map(|error| format!("\n\nError: {error}"))
                .unwrap_or_default()
        )
    } else if tab.editing {
        format!(
            "{}{}",
            tab.edit_text,
            tab.edit_error
                .as_ref()
                .map(|error| format!("\n\nError: {error}"))
                .unwrap_or_default()
        )
    } else if let Some(draft) = &tab.import_draft {
        format!(
            "Import draft · {} document(s)\n{}",
            draft.documents.len(),
            draft.documents.join("\n")
        )
    } else {
        tab.selected_document_inspector().map(|(text, _)| text).unwrap_or_else(|| match &tab.page {
        MongoPageState::Ready(page) if page.oversized_document => "Selected document exceeds the configured byte budget. Raw BSON is retained by the reader.".to_owned(),
        MongoPageState::Ready(page) if page.truncated => "Page reached the configured byte budget. Load the next page or reduce the result size.".to_owned(),
        _ => "Select a document to inspect Canonical Extended JSON.".to_owned(),
    })
    };
    let status = match &tab.page {
        MongoPageState::Ready(page) => format!(
            "{} documents · {} BSON bytes{}",
            page.documents.len(),
            page.bytes,
            if page.truncated { " · truncated" } else { "" }
        ),
        MongoPageState::Failed(error) => format!("{} · press r to retry", error),
        _ => tab
            .namespace
            .collection
            .as_deref()
            .unwrap_or(&tab.namespace.database)
            .to_owned(),
    };
    let status = tab.mutation_message.as_deref().unwrap_or(&status);
    let status_style =
        if status.contains("unknown") || status.contains("Partial") || status.contains("partial") {
            Style::new().fg(ratatui::style::Color::Yellow)
        } else {
            Style::new().fg(theme.muted)
        };
    let title = if let Some(query_field) = tab.query_field {
        format!("Edit {query_field:?} · Enter apply · Esc cancel")
    } else if tab.editing {
        "Edit Canonical EJSON · Ctrl+Enter save · Esc cancel".to_owned()
    } else {
        if tab
            .selected_document_inspector()
            .is_some_and(|(_, raw_only)| raw_only)
        {
            "Raw BSON inspector · read only".to_owned()
        } else {
            "Canonical EJSON · e edit · y copy".to_owned()
        }
    };
    if let Some(inspector_area) = inspector_area {
        frame.render_widget(
            Paragraph::new(Text::from(
                detail.lines().map(Line::from).collect::<Vec<_>>(),
            ))
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(ratatui::widgets::Wrap { trim: false }),
            inspector_area,
        );
    }
    if let MongoPageState::Ready(page) = &tab.page
        && page.next_offset.is_some()
    {
        let more = Rect::new(
            documents_area.x + 1,
            documents_area.y + documents_area.height.saturating_sub(1),
            documents_area.width.saturating_sub(2),
            1,
        );
        frame.render_widget(
            Paragraph::new("Load next page").style(Style::new().fg(theme.accent)),
            more,
        );
        ui.hit_regions.push(crate::ui::HitRegion {
            area: more,
            target: crate::ui::HitTarget::MongoPageLoadMore(tab.id),
        });
    }
    let controls = Rect::new(
        documents_area.x,
        documents_area.y + documents_area.height.saturating_sub(2),
        documents_area.width,
        1,
    );
    let control_text = if tab.import_draft.is_some() {
        "Import draft ready · I insert · Esc discard"
    } else {
        "paste import · i insert · d delete · y copy · E export loaded EJSON"
    };
    frame.render_widget(
        Paragraph::new(control_text).style(Style::new().fg(theme.accent)),
        controls,
    );
    let labels = [
        (
            "Filter",
            crate::model::mongodb_browser::MongoQueryField::Filter,
        ),
        ("Sort", crate::model::mongodb_browser::MongoQueryField::Sort),
        (
            "Projection",
            crate::model::mongodb_browser::MongoQueryField::Projection,
        ),
    ];
    let width = (query_bar.width / 3).max(1);
    for (index, (label, field)) in labels.into_iter().enumerate() {
        let rect = Rect::new(
            query_bar.x + (index as u16 * width),
            query_bar.y,
            if index == 2 {
                query_bar.width.saturating_sub(width * 2)
            } else {
                width
            },
            1,
        );
        let text = match field {
            crate::model::mongodb_browser::MongoQueryField::Filter => tab.spec.filter.to_string(),
            crate::model::mongodb_browser::MongoQueryField::Sort => tab
                .spec
                .sort
                .as_ref()
                .map_or("{}".into(), ToString::to_string),
            crate::model::mongodb_browser::MongoQueryField::Projection => tab
                .spec
                .projection
                .as_ref()
                .map_or("{}".into(), ToString::to_string),
        };
        frame.render_widget(
            Paragraph::new(format!("{label}: {text}")).style(Style::new().fg(theme.muted)),
            rect,
        );
        ui.hit_regions.push(crate::ui::HitRegion {
            area: rect,
            target: crate::ui::HitTarget::MongoQueryField(tab.id, field),
        });
    }
    frame.render_widget(Paragraph::new(status).style(status_style), status_area);
}

/// Clip a cell to its column and say so, instead of letting it run into the
/// next column.
fn clip_cell(text: String, width: u16) -> String {
    let width = width as usize;
    if text.chars().count() <= width {
        return text;
    }
    let mut clipped = text
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    clipped.push('…');
    clipped
}

fn column_widths(widths: &[u16]) -> Vec<Constraint> {
    let mut constraints = vec![Constraint::Length(ROW_NUMBER_WIDTH)];
    constraints.extend(widths.iter().map(|width| Constraint::Length(*width)));
    constraints
}
