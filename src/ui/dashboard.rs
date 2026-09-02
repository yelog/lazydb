use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Row, Table},
};

use crate::{
    app::App,
    model::dashboard::{MetricKey, downsample_series, process_matches},
};

use super::theme::Theme;

pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let Some(crate::model::tab::WorkspaceTab::Dashboard(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);
    let page = ["Overview", "Processes", "Charts"]
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let style = if index == tab.page as usize {
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.muted)
            };
            Line::styled(format!(" {} ", label), style)
        })
        .collect::<Vec<_>>();
    let status = match (&tab.error, tab.loading, tab.last_refresh_millis) {
        (Some(error), _, _) => format!("  ERROR: {error}"),
        (None, true, _) => "  Loading metrics...".into(),
        (None, false, Some(at)) => {
            format!("  Last sample: {}", format_timestamp(at as f64 / 1_000.0))
        }
        (None, false, None) => "  Waiting for metrics".into(),
    };
    let header = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(vertical[0]);
    frame.render_widget(Paragraph::new(page), header[0]);
    frame.render_widget(
        Paragraph::new(Line::styled(status, Style::new().fg(theme.muted))),
        header[1],
    );

    match tab.page {
        crate::model::dashboard::DashboardPage::Processes => {
            render_processes(frame, vertical[1], theme, tab)
        }
        crate::model::dashboard::DashboardPage::Charts => {
            render_charts(frame, vertical[1], theme, tab)
        }
        crate::model::dashboard::DashboardPage::Overview => {
            render_overview(frame, vertical[1], theme, tab)
        }
    }
}

fn render_overview(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    tab: &crate::model::dashboard::DashboardTab,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(1),
        ])
        .split(area);
    let cards = [
        (
            "Transactions",
            metric_value(tab, crate::model::dashboard::MetricKey::Transactions),
        ),
        (
            "Connections",
            metric_value(tab, crate::model::dashboard::MetricKey::Connections),
        ),
        (
            "Active",
            metric_value(tab, crate::model::dashboard::MetricKey::ActiveConnections),
        ),
        (
            "Cache hit",
            metric_value(tab, crate::model::dashboard::MetricKey::CacheHitRatio),
        ),
        (
            "Deadlocks",
            metric_value(tab, crate::model::dashboard::MetricKey::Deadlocks),
        ),
        (
            "Temp files",
            metric_value(tab, crate::model::dashboard::MetricKey::TempFiles),
        ),
        (
            "Bytes in/s",
            metric_value(tab, crate::model::dashboard::MetricKey::BytesRead),
        ),
        (
            "Bytes out/s",
            metric_value(tab, crate::model::dashboard::MetricKey::BytesWritten),
        ),
    ];
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25); 4])
        .split(rows[0]);
    for (index, (label, value)) in cards.iter().take(4).enumerate() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(*label),
                Line::styled(value.as_str(), Style::new().fg(theme.accent)),
            ])
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(theme.border)),
            ),
            columns[index],
        );
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25); 4])
        .split(rows[1]);
    for (index, (label, value)) in cards.iter().skip(4).enumerate() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(*label),
                Line::styled(value.as_str(), Style::new().fg(theme.warning)),
            ])
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(theme.border)),
            ),
            columns[index],
        );
    }
    let metadata = match (
        tab.metadata.version.as_deref(),
        tab.metadata.max_connections,
    ) {
        (Some(version), Some(max)) => format!("Server {version}  ·  max connections {max}"),
        (Some(version), None) => format!("Server {version}"),
        _ => "Waiting for the first monitoring sample...".into(),
    };
    frame.render_widget(
        Paragraph::new(metadata).style(Style::new().fg(theme.muted)),
        rows[2],
    );
}

fn metric_value(
    tab: &crate::model::dashboard::DashboardTab,
    key: crate::model::dashboard::MetricKey,
) -> String {
    if key == MetricKey::CacheHitRatio {
        let hits = tab
            .latest
            .as_ref()
            .and_then(|sample| sample.values.get(&MetricKey::BlockHits))
            .copied();
        let reads = tab
            .latest
            .as_ref()
            .and_then(|sample| sample.values.get(&MetricKey::BlockReads))
            .copied();
        return match (hits, reads) {
            (Some(hits), Some(reads)) if hits + reads > 0.0 => {
                format_value(hits / (hits + reads) * 100.0)
            }
            _ => "--".into(),
        };
    }
    tab.latest
        .as_ref()
        .and_then(|sample| sample.values.get(&key))
        .map_or_else(|| "--".into(), |value| format_value(*value))
}

fn format_value(value: f64) -> String {
    if value.abs() >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value.abs() >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn render_processes(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    tab: &crate::model::dashboard::DashboardTab,
) {
    let filter = format!(
        "Filter: {}{}",
        if tab.process_filter.is_empty() {
            "<all>"
        } else {
            &tab.process_filter
        },
        if tab.process_truncated {
            "  (truncated)"
        } else {
            ""
        }
    );
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(filter).style(Style::new().fg(theme.muted)),
        sections[0],
    );
    let area = sections[1];
    let mut rows = tab
        .processes
        .iter()
        .filter(|row| process_matches(row, &tab.process_filter))
        .map(|row| {
            Row::new(vec![
                row.id.to_string(),
                row.user.clone(),
                row.database.clone().unwrap_or_else(|| "-".into()),
                row.state.clone().unwrap_or_else(|| "-".into()),
                row.query.clone().unwrap_or_else(|| "-".into()),
            ])
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        rows.push(Row::new(vec![
            if tab.process_loading {
                "Loading..."
            } else {
                "No process samples yet"
            },
            "",
            "",
            "",
            "",
        ]));
    }
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(24),
                Constraint::Length(16),
                Constraint::Length(16),
                Constraint::Length(16),
                Constraint::Min(1),
            ],
        )
        .header(
            Row::new(vec!["PID", "USER", "DB", "STATE", "QUERY"])
                .style(Style::new().fg(theme.grid_header_text)),
        )
        .block(Block::new().borders(Borders::ALL).title("Process List")),
        area,
    );
}

fn render_charts(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    tab: &crate::model::dashboard::DashboardTab,
) {
    let chart_area = area;
    let inner = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.border))
        .title("History · last 10 minutes")
        .inner(chart_area);
    let latest = tab.latest.as_ref();
    let mut series_data = Vec::new();
    for (key, label, color) in [
        (MetricKey::Commits, "commits/s", theme.accent),
        (MetricKey::Rollbacks, "rollbacks/s", theme.error),
        (MetricKey::Connections, "connections", theme.action),
    ] {
        let points = downsample_series(tab.history.points(key), inner.width as usize * 2);
        if !points.is_empty() {
            series_data.push((label, color, points));
        }
    }
    if series_data.is_empty() {
        frame.render_widget(
            Paragraph::new("Charts will appear after two valid samples.")
                .style(Style::new().fg(theme.muted))
                .block(Block::new().borders(Borders::ALL).title("History")),
            area,
        );
        return;
    }
    let datasets = series_data
        .iter()
        .map(|(label, color, points)| {
            Dataset::default()
                .name(*label)
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(*color))
                .data(points)
        })
        .collect::<Vec<_>>();
    let mut max_y: f64 = 1.0;
    for key in [
        MetricKey::Commits,
        MetricKey::Rollbacks,
        MetricKey::Connections,
    ] {
        max_y = max_y.max(
            tab.history
                .points(key)
                .iter()
                .filter_map(|point| point.value)
                .fold(0.0, f64::max),
        );
    }
    let x_end = latest
        .map(|sample| sample.at_millis as f64 / 1_000.0)
        .unwrap_or(1.0);
    let x_start = tab
        .history
        .samples()
        .first()
        .map(|sample| sample.at_millis as f64 / 1_000.0)
        .unwrap_or(x_end - 1.0);
    let x_mid = (x_start + x_end) / 2.0;
    let labels = [
        Span::styled(format_timestamp(x_start), Style::new().fg(theme.muted)),
        Span::styled(format_timestamp(x_mid), Style::new().fg(theme.muted)),
        Span::styled(format_timestamp(x_end), Style::new().fg(theme.muted)),
    ];
    frame.render_widget(
        Chart::new(datasets)
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .title("History · last 10 minutes"),
            )
            .x_axis(
                Axis::default()
                    .bounds([x_start, x_end.max(x_start + 1.0)])
                    .labels(labels),
            )
            .y_axis(Axis::default().bounds([0.0, max_y * 1.05]).labels([
                Span::styled("0", Style::new().fg(theme.muted)),
                Span::styled(format_value(max_y / 2.0), Style::new().fg(theme.muted)),
                Span::styled(format_value(max_y), Style::new().fg(theme.muted)),
            ])),
        chart_area,
    );
}

fn format_timestamp(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        (seconds / 3_600) % 24,
        (seconds / 60) % 60,
        seconds % 60
    )
}
