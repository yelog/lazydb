use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::model::editor::EditorMode;
use crate::model::redis_browser::RedisValuePageState;
use crate::{
    app::App,
    db::catalog::ObjectGroup,
    model::workspace::Focus,
    model::{
        redis_browser::{RedisBrowserFocus, RedisPreviewState},
        redis_key_tree::VisibleKeyTreeRow,
    },
    ui::{Theme, icons::IconSet},
};

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    ui: &mut crate::ui::UiState,
    theme: Theme,
    icons: IconSet,
) {
    let layout =
        crate::ui::layout::RedisBrowserLayout::calculate(area, app.pane_sizes.redis_keys_width);
    let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) = app.tabs.get(app.active_tab)
    else {
        return;
    };
    let keys_focused = app.focus == Focus::Results && tab.focus == RedisBrowserFocus::Keys;
    let preview_focused = app.focus == Focus::Results && tab.focus == RedisBrowserFocus::Preview;
    let keys_block = super::panel_block("", keys_focused, theme);
    let keys_area = keys_block.inner(layout.keys);
    let preview_area = layout.preview;
    ui.hit_regions.push(crate::ui::HitRegion {
        area: layout.keys,
        target: crate::ui::HitTarget::RedisKeysFocus(tab.id),
    });
    frame.render_widget(
        keys_block
            .title(keys_title(tab))
            .border_style(Style::new().fg(if keys_focused {
                theme.accent
            } else {
                theme.border
            })),
        layout.keys,
    );
    let rows = tab.visible_rows();
    let (rows, selected_id, query, phase, matches) = if let Some(find) = tab.find.as_ref() {
        (
            rows,
            tab.tree.selected.clone(),
            Some(find.query.value().to_owned()),
            Some(find.phase),
            Some(find.matches.len()),
        )
    } else {
        (rows, tab.tree.selected.clone(), None, None, None)
    };
    let status = if query.is_some()
        && phase == Some(crate::model::redis_browser::RedisFindPhase::Confirmed)
        && matches == Some(0)
    {
        Some("No matching loaded keys".into())
    } else if rows.is_empty() {
        Some(keyspace_empty_text(&tab.keyspace.status))
    } else {
        match &tab.keyspace.status {
            crate::model::keyspace::KeyspaceStatus::Partial => {
                Some("Continue scanning: press r".into())
            }
            crate::model::keyspace::KeyspaceStatus::Failed(message) => {
                Some(format!("{message} - r=Retry"))
            }
            crate::model::keyspace::KeyspaceStatus::Paused { .. } => {
                Some("Scan paused at client cache limit".into())
            }
            crate::model::keyspace::KeyspaceStatus::Stale => {
                Some("Showing stale keys; press r to refresh".into())
            }
            _ => None,
        }
    };
    let status_rows = if matches!(
        tab.keyspace.status,
        crate::model::keyspace::KeyspaceStatus::Failed(_)
    ) {
        2
    } else {
        u16::from(status.is_some())
    };
    let search_rows = u16::from(query.is_some());
    let row_area = Rect::new(
        keys_area.x,
        keys_area.y.saturating_add(search_rows),
        keys_area.width,
        keys_area.height.saturating_sub(status_rows + search_rows),
    );
    ui.redis_keys_viewport_rows = Some((tab.id, row_area.height as usize));
    let visible_rows = rows
        .iter()
        .skip(tab.scroll)
        .take(row_area.height as usize)
        .collect::<Vec<_>>();
    let keys_scroll_track = Rect::new(
        keys_area.right().saturating_sub(1),
        row_area.y,
        1,
        row_area.height,
    );
    let keys = visible_rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            render_row(
                row,
                selected_id.as_ref() == Some(&row.id),
                ui,
                tab.id,
                row_area,
                index,
                theme,
                icons,
            )
        })
        .collect::<Vec<_>>();
    if let Some(status) = status {
        let footer = Rect::new(
            keys_area.x,
            keys_area.bottom().saturating_sub(status_rows),
            keys_area.width,
            status_rows,
        );
        frame.render_widget(Paragraph::new(keys), row_area);
        frame.render_widget(
            Paragraph::new(status)
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .wrap(Wrap { trim: true }),
            footer,
        );
    } else {
        frame.render_widget(Paragraph::new(keys), row_area);
    }
    if let Some(query) = query {
        let search_area = Rect::new(keys_area.x, keys_area.y, keys_area.width, 1);
        ui.hit_regions.push(crate::ui::HitRegion {
            area: search_area,
            target: crate::ui::HitTarget::RedisFindInput(tab.id),
        });
        let prefix = if phase == Some(crate::model::redis_browser::RedisFindPhase::Editing) {
            "/ "
        } else {
            "? "
        };
        let input = format!("{prefix}{query}");
        frame.render_widget(
            Paragraph::new(input).style(Style::new().fg(theme.action).bg(theme.surface)),
            search_area,
        );
        if phase == Some(crate::model::redis_browser::RedisFindPhase::Editing) {
            let prefix_width = prefix.cell_width();
            let query_width = query.cell_width();
            let cursor_x = search_area.x.saturating_add(
                prefix_width + query_width.min(search_area.width.saturating_sub(prefix_width)),
            );
            ui.cursor = Some(crate::ui::CursorSpec {
                position: ratatui::layout::Position::new(
                    cursor_x.min(search_area.right().saturating_sub(1)),
                    search_area.y,
                ),
                style: crate::ui::CursorStyle::Bar,
            });
        }
    }
    if let Some(geometry) = crate::ui::scrollbar::geometry(
        keys_scroll_track,
        row_area.height as usize,
        rows.len(),
        tab.scroll,
    ) {
        crate::ui::scrollbar::render_vertical(frame, keys_scroll_track, geometry, theme);
    }
    let preview_session = tab.preview_editor_id;
    let (value_area, header_area) = if preview_area.height >= 3 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(preview_area);
        (chunks[1], chunks[0])
    } else {
        (preview_area, Rect::default())
    };
    if header_area.height > 0 {
        let metadata = match &tab.value_page {
            RedisValuePageState::Ready(page) => Some(&page.metadata),
            _ => None,
        };
        let key = metadata.map_or_else(
            || {
                tab.opened_key
                    .as_ref()
                    .map(|key| display_bytes(&key.key))
                    .unwrap_or_else(|| "No key opened".into())
            },
            |metadata| display_bytes(&metadata.key.key),
        );
        let (kind, size, ttl) =
            metadata.map_or(("—".into(), "—".into(), "—".into()), |metadata| {
                (
                    format!("{:?}", metadata.value_type),
                    crate::ui::redis_value::format_bytes(metadata.memory_usage_bytes),
                    crate::ui::redis_value::format_ttl(&metadata.ttl),
                )
            });
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    key,
                    Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled(
                        format!("[ {kind} ] "),
                        Style::new().fg(theme.action).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("[ Size {size} ] "), Style::new().fg(theme.warning)),
                    Span::styled(format!("[ TTL {ttl} ]"), Style::new().fg(theme.muted)),
                ]),
            ]),
            header_area,
        );
    }
    let table_view = tab.format.view() == crate::value_preview::ValueView::Table;
    let value_block = if table_view {
        super::panel_block(" VALUE ", preview_focused, theme)
    } else {
        let value_mode = app
            .active_read_only_editor_mode()
            .unwrap_or(EditorMode::Normal);
        let mode_label = match value_mode {
            EditorMode::Normal => "NORMAL",
            EditorMode::Insert => "INSERT",
            EditorMode::Replace => "REPLACE",
            EditorMode::VisualChar => "VISUAL",
            EditorMode::VisualLine => "VISUAL LINE",
            EditorMode::VisualBlock => "VISUAL BLOCK",
        };
        super::panel_block("", preview_focused, theme)
            .title_top(Line::raw(format!(" VALUE {mode_label} ")).left_aligned())
    };
    let value_outer = value_area;
    ui.hit_regions.push(crate::ui::HitRegion {
        area: value_outer,
        target: crate::ui::HitTarget::RedisPreviewFocus(tab.id),
    });
    let value_area = value_block.inner(value_outer);
    frame.render_widget(value_block.clone(), value_outer);
    let format_label = format!(
        " f:{} ▾ ",
        preview_format_label(tab.format.selected, tab.format.automatic)
    );
    let wrap_label = if table_view {
        ""
    } else if tab.preview_wrap {
        " W:Wrap ON "
    } else {
        " W:Wrap OFF "
    };
    if value_outer.width > format_label.cell_width() + wrap_label.cell_width() + 10 {
        let format_area = Rect::new(
            value_outer
                .right()
                .saturating_sub(format_label.cell_width() + 1),
            value_outer.y,
            format_label.cell_width(),
            1,
        );
        ui.hit_regions.push(crate::ui::HitRegion {
            area: format_area,
            target: crate::ui::HitTarget::RedisPreviewFormat(tab.id),
        });
        frame.render_widget(
            Paragraph::new(format_label).style(Style::new().fg(theme.action)),
            format_area,
        );
        let wrap_area = Rect::new(
            format_area.x.saturating_sub(wrap_label.cell_width()),
            value_outer.y,
            wrap_label.cell_width(),
            1,
        );
        ui.hit_regions.push(crate::ui::HitRegion {
            area: wrap_area,
            target: crate::ui::HitTarget::RedisPreviewWrap(tab.id),
        });
        frame.render_widget(
            Paragraph::new(wrap_label).style(Style::new().fg(theme.action)),
            wrap_area,
        );
    }
    let table_view = tab.format.view() == crate::value_preview::ValueView::Table;
    if table_view {
        render_table_filter(frame, value_area, tab, ui, theme);
    }
    if table_view && let RedisValuePageState::Ready(page) = &tab.value_page {
        let table = tab
            .preview_table()
            .unwrap_or_else(|| crate::value_preview::table::from_page(&page.value));
        let result = redis_table_result(&table);
        let grid_area = Rect::new(
            value_area.x,
            value_area.y.saturating_add(2),
            value_area.width,
            value_area.height.saturating_sub(3),
        );
        super::data_grid::render(
            frame,
            grid_area,
            tab.id,
            &result,
            0,
            tab.preview_grid.clone(),
            &tab.preview_grid.column_widths,
            theme,
            ratatui::widgets::Block::default().style(Style::new().bg(theme.surface)),
            ui,
            None,
            ui.activity_icons,
            None,
            false,
        );
        let status_area = Rect::new(
            grid_area.x,
            value_area.bottom().saturating_sub(1),
            grid_area.width,
            1,
        );
        let loaded_rows = crate::value_preview::table::from_page(&page.value)
            .rows
            .len();
        render_value_page_status(
            frame,
            status_area,
            tab,
            ui,
            theme,
            result.rows.len(),
            loaded_rows,
        );
        ui.redis_preview_viewport_rows = Some((
            tab.id,
            grid_area.height.saturating_sub(2) as usize,
            result.rows.len() + 1,
        ));
        return;
    }
    if table_view {
        let body = Rect::new(
            value_area.x,
            value_area.y.saturating_add(2),
            value_area.width,
            value_area.height.saturating_sub(2),
        );
        let message = match &tab.value_page {
            RedisValuePageState::Loading { key } => {
                format!("Loading {}", display_bytes(&key.key))
            }
            RedisValuePageState::Failed { key, message } => {
                format!("{}\n{}", display_bytes(&key.key), message)
            }
            RedisValuePageState::Empty => "Select a key to preview".to_owned(),
            RedisValuePageState::Ready(_) => "No rows".to_owned(),
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::new().fg(theme.muted).bg(theme.surface)),
            body,
        );
        ui.redis_preview_viewport_rows = Some((tab.id, body.height as usize, 1));
        return;
    }
    let gutter = app
        .redis_preview_logical_line_count(tab.id)
        .map_or(1, |count| count.max(1).to_string().len() + 1);
    if let Ok(snapshot) = app.redis_preview_snapshot(
        tab.id,
        crate::model::editor::EditorViewport {
            width: (value_area.width as usize).saturating_sub(gutter),
            height: value_area.height as usize,
        },
    ) {
        // Preserve the border controls drawn above while sharing DDL's body renderer.
        let controls = (value_outer.x..value_outer.right())
            .map(|x| frame.buffer_mut()[(x, value_outer.y)].clone())
            .collect::<Vec<_>>();
        crate::ui::read_only_sql::ReadOnlySqlEditor {
            session_id: preview_session,
            snapshot: &snapshot,
            block: value_block,
            focused: preview_focused && app.overlay.is_none(),
            show_line_numbers: true,
        }
        .render(frame, value_outer, theme, ui);
        for (index, cell) in controls.into_iter().enumerate() {
            frame.buffer_mut()[(value_outer.x + index as u16, value_outer.y)] = cell;
        }
        ui.redis_editor_viewport = Some((preview_session, snapshot.viewport));
        ui.redis_preview_viewport_rows =
            Some((tab.id, snapshot.viewport.height, snapshot.total_lines));
    } else {
        let preview_lines = match &tab.value_page {
            RedisValuePageState::Ready(page) => crate::ui::redis_value::page_lines(page),
            RedisValuePageState::Loading { key } => {
                vec![Line::from(format!("Loading {}", display_bytes(&key.key)))]
            }
            RedisValuePageState::Failed { key, message } => vec![
                Line::from(display_bytes(&key.key)),
                Line::from(message.as_str()),
            ],
            RedisValuePageState::Empty => match &tab.preview {
                RedisPreviewState::Empty => vec![Line::from("Select a key to preview")],
                RedisPreviewState::Loading { key } => {
                    vec![Line::from(format!("Loading {}", display_bytes(&key.key)))]
                }
                RedisPreviewState::Ready { key, content } => vec![
                    Line::from(display_bytes(&key.key)),
                    Line::from(content.as_str()),
                ],
                RedisPreviewState::Failed { key, message } => vec![
                    Line::from(display_bytes(&key.key)),
                    Line::from(message.as_str()),
                ],
            },
        };
        let preview_content_rows = preview_lines.len();
        frame.render_widget(
            Paragraph::new(preview_lines)
                .style(Style::new().bg(theme.surface))
                .scroll((tab.preview_scroll.min(u16::MAX as usize) as u16, 0)),
            value_area,
        );
        ui.redis_preview_viewport_rows =
            Some((tab.id, preview_area.height as usize, preview_content_rows));
    }
}

fn render_value_page_status(
    frame: &mut Frame<'_>,
    area: Rect,
    tab: &crate::model::redis_browser::RedisBrowserTab,
    ui: &mut crate::ui::UiState,
    theme: Theme,
    row_count: usize,
    loaded_rows: usize,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let state = match &tab.value_page {
        RedisValuePageState::Ready(_) if tab.value_page_loading => "Loading…".to_owned(),
        RedisValuePageState::Ready(page) if page.complete => "Complete".to_owned(),
        RedisValuePageState::Ready(_) => "More available".to_owned(),
        RedisValuePageState::Failed { .. } => "Load failed · Retry".to_owned(),
        _ => "Not loaded".to_owned(),
    };
    let start = tab.preview_grid.row_offset.saturating_add(1);
    let end = (start + tab.preview_grid.viewport_rows.max(1)).min(row_count);
    let text = if tab.value_filter.applied.is_empty() {
        if loaded_rows == 0 {
            format!("0 loaded · {state}")
        } else {
            format!("Rows {start}–{end} · {loaded_rows} loaded · {state}")
        }
    } else if row_count == 0 {
        format!("0 matched / {loaded_rows} loaded · {state}")
    } else {
        format!("Rows {start}–{end} · {row_count} matched / {loaded_rows} loaded · {state}")
    };
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(theme.muted).bg(theme.surface)),
        area,
    );
    if !tab.value_page_loading
        && matches!(tab.value_page, RedisValuePageState::Ready(ref page) if !page.complete)
    {
        ui.hit_regions.push(crate::ui::HitRegion {
            area,
            target: crate::ui::HitTarget::RedisPreviewLoadMore(tab.id),
        });
    }
}

fn render_table_filter(
    frame: &mut Frame<'_>,
    value_area: Rect,
    tab: &crate::model::redis_browser::RedisBrowserTab,
    ui: &mut super::UiState,
    theme: Theme,
) {
    if value_area.height < 2 || value_area.width == 0 {
        return;
    }
    let area = Rect::new(value_area.x, value_area.y, value_area.width, 2);
    ui.hit_regions.push(crate::ui::HitRegion {
        area,
        target: crate::ui::HitTarget::RedisValueFilter(tab.id),
    });
    let field = Rect::new(area.x, area.y, area.width, 1);
    let prefix = "FILTER  ";
    let offset = super::text_input_horizontal_offset(field, prefix, &tab.value_filter.draft);
    super::register_input_selection_target(
        ui,
        super::text_selection::InputSelectionTarget::RedisValueFilter(tab.id),
        field,
        prefix,
        &tab.value_filter.draft,
        offset,
    );
    super::query_bar::render_query_field(
        frame,
        field,
        "FILTER",
        &tab.value_filter.draft,
        &[],
        true,
        tab.value_filter.editing,
        offset,
        theme,
        ui,
    );
    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize)).style(
            Style::new()
                .fg(if tab.value_filter.editing {
                    theme.accent
                } else {
                    theme.border
                })
                .bg(theme.surface),
        ),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
}

fn redis_table_result(
    table: &crate::value_preview::table::RedisTable,
) -> crate::db::query::ResultSet {
    crate::db::query::ResultSet {
        columns: table
            .columns
            .iter()
            .map(|name| crate::db::query::ColumnMeta {
                name: name.clone(),
                type_name: "REDIS".into(),
            })
            .collect(),
        rows: table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .cloned()
                    .map(crate::db::value::CellValue::Text)
                    .collect()
            })
            .collect(),
        affected_rows: 0,
    }
}

fn preview_format_label(format: crate::value_preview::PreviewFormat, automatic: bool) -> String {
    let label = match format.encoding {
        crate::value_preview::ValueEncoding::Java => "Java",
        crate::value_preview::ValueEncoding::Php => "PHP",
        crate::value_preview::ValueEncoding::Pickle => "Pickle",
        _ => match format.view {
            crate::value_preview::ValueView::Raw => "RAW",
            crate::value_preview::ValueView::Json => "JSON",
            crate::value_preview::ValueView::Yaml => "YAML",
            crate::value_preview::ValueView::Table => "Table",
            crate::value_preview::ValueView::Hex => "Hex",
        },
    };
    if automatic {
        format!("Auto · {label}")
    } else {
        label.to_string()
    }
}

fn keys_title(tab: &crate::model::redis_browser::RedisBrowserTab) -> String {
    format!(
        "Keys · DB {} · {} loaded · {}",
        tab.target.database,
        tab.keyspace.keys.len(),
        keyspace_status_label(&tab.keyspace.status)
    )
}

fn keyspace_status_label(status: &crate::model::keyspace::KeyspaceStatus) -> &'static str {
    use crate::model::keyspace::KeyspaceStatus;
    match status {
        KeyspaceStatus::NotLoaded | KeyspaceStatus::Idle => "not loaded",
        KeyspaceStatus::Loading => "loading",
        KeyspaceStatus::Partial => "partial",
        KeyspaceStatus::Complete => "complete",
        KeyspaceStatus::CompleteEmpty => "empty",
        KeyspaceStatus::Paused { .. } => "paused",
        KeyspaceStatus::Stale => "stale",
        KeyspaceStatus::Failed(_) => "failed",
    }
}

fn keyspace_empty_text(status: &crate::model::keyspace::KeyspaceStatus) -> String {
    use crate::model::keyspace::KeyspaceStatus;
    match status {
        KeyspaceStatus::NotLoaded | KeyspaceStatus::Idle => "Not loaded — Enter to load".into(),
        KeyspaceStatus::Loading => "Loading keys…".into(),
        KeyspaceStatus::Partial => "No keys in batch — press r to continue".into(),
        KeyspaceStatus::CompleteEmpty => "No matching keys".into(),
        KeyspaceStatus::Complete => "No keys".into(),
        KeyspaceStatus::Failed(message) => format!("{message} - r=Retry"),
        KeyspaceStatus::Paused { loaded } => {
            format!("Client cache limit reached ({loaded} loaded)")
        }
        KeyspaceStatus::Stale => "Showing stale keys; refresh to reload".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    row: &VisibleKeyTreeRow,
    selected: bool,
    ui: &mut crate::ui::UiState,
    tab_id: uuid::Uuid,
    area: Rect,
    visible_index: usize,
    theme: Theme,
    icons: IconSet,
) -> Line<'static> {
    let marker = if !row.expandable {
        " "
    } else if row.expanded {
        "▾"
    } else {
        "▸"
    };
    let label = if row.is_key && row.expandable {
        format!("{} [key]", display_bytes(&row.label))
    } else {
        display_bytes(&row.label)
    };
    let background = if selected {
        theme.selection
    } else {
        theme.surface
    };
    let style = if selected {
        Style::new()
            .bg(background)
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().bg(background).fg(theme.text)
    };
    let icon_style = Style::new().bg(background).fg(if row.expandable {
        theme.muted
    } else {
        theme.text
    });
    ui.hit_regions.push(crate::ui::HitRegion {
        area: Rect::new(
            area.x + row.depth.saturating_mul(2) as u16,
            area.y + visible_index as u16,
            area.width.saturating_sub(2),
            1,
        ),
        target: crate::ui::HitTarget::RedisKeyNode {
            tab_id,
            node: row.id.clone(),
        },
    });
    if row.expandable {
        ui.hit_regions.push(crate::ui::HitRegion {
            area: Rect::new(
                area.x + row.depth.saturating_mul(2) as u16,
                area.y + visible_index as u16,
                2,
                1,
            ),
            target: crate::ui::HitTarget::RedisKeyToggle {
                tab_id,
                node: row.id.clone(),
            },
        });
    }
    let icon = if row.expandable {
        icons.group(ObjectGroup::Tables, row.expanded)
    } else {
        icons.redis_key()
    };
    let mut spans = vec![
        Span::styled(format!("{}{} ", "  ".repeat(row.depth), marker), style),
        Span::styled(format!("{} ", icon), icon_style),
        Span::styled(label, style),
    ];
    if row.expandable {
        spans.push(Span::styled(
            format!(" {}", row.total_keys),
            Style::new().fg(theme.muted).bg(background),
        ));
    }
    spans.push(Span::styled(
        " ".repeat(area.width.saturating_sub(1) as usize),
        Style::new().bg(background),
    ));
    Line::from(spans)
}

fn display_bytes(value: &[u8]) -> String {
    crate::model::redis_key_text::display_bytes(value)
}
