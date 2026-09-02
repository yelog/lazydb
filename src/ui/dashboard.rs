use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
};

use crate::{
    app::App,
    model::dashboard::{MetricKey, downsample_series},
};

use super::{
    icons::{DashboardMetric, IconSet},
    theme::Theme,
};

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
) {
    let Some(crate::model::tab::WorkspaceTab::Dashboard(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let block = super::panel_block(
        " DASHBOARD ",
        app.focus == crate::model::workspace::Focus::Results
            && tab.page != crate::model::dashboard::DashboardPage::Processes,
        theme,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let area = inner;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    let status = match (&tab.error, tab.loading) {
        (Some(error), _) => format!("  ERROR  {error}"),
        (None, true) => format!("  AUTO  ·  refresh {}s  ·  loading", app.dashboard_refresh_interval_seconds()),
        (None, false) => format!("  AUTO  ·  refresh {}s", app.dashboard_refresh_interval_seconds()),
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            status,
            Style::new().fg(if tab.error.is_some() {
                theme.error
            } else {
                theme.muted
            }),
        )),
        vertical[0],
    );

    match tab.page {
        crate::model::dashboard::DashboardPage::Processes => render_processes(
            frame,
            vertical[1],
            theme,
            tab,
            state,
            app.focus == crate::model::workspace::Focus::Results,
        ),
        crate::model::dashboard::DashboardPage::Charts => {
            render_charts(frame, vertical[1], theme, tab)
        }
        crate::model::dashboard::DashboardPage::Overview => {
            render_overview(frame, vertical[1], theme, tab, state.activity_icons)
        }
    }
}

fn render_overview(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    tab: &crate::model::dashboard::DashboardTab,
    icons: IconSet,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(area);
    let cards = [
        (
            DashboardMetric::Transactions,
            "Transactions/s",
            transactions_rate(tab),
            theme.accent,
        ),
        (
            DashboardMetric::Connections,
            "Connections",
            connections_value(tab),
            theme.action,
        ),
        (
            DashboardMetric::ActiveConnections,
            "Active",
            current_metric_value(tab, MetricKey::ActiveConnections),
            theme.success,
        ),
        (
            DashboardMetric::CacheHit,
            "Cache hit rate",
            cache_hit_value(tab),
            theme.success,
        ),
        (
            DashboardMetric::Deadlocks,
            "Deadlocks",
            current_metric_value(tab, MetricKey::Deadlocks),
            theme.error,
        ),
        (
            DashboardMetric::TempFiles,
            "Temp files",
            current_metric_value(tab, MetricKey::TempFiles),
            theme.warning,
        ),
        (
            DashboardMetric::Uptime,
            "Uptime",
            uptime_value(tab),
            theme.action,
        ),
        (
            DashboardMetric::WriteRate,
            write_rate_label(tab),
            write_rate_value(tab),
            theme.accent,
        ),
    ];
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25); 4])
        .split(rows[0]);
    for (index, (metric, label, value, color)) in cards.iter().take(4).enumerate() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(icons.dashboard(*metric), Style::new().fg(*color)),
                    Span::raw(" "),
                    Span::styled(*label, Style::new().fg(theme.muted)),
                ]),
                Line::from(""),
                Line::styled(value.as_str(), Style::new().fg(*color)),
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
    for (index, (metric, label, value, color)) in cards.iter().skip(4).enumerate() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(icons.dashboard(*metric), Style::new().fg(*color)),
                    Span::raw(" "),
                    Span::styled(*label, Style::new().fg(theme.muted)),
                ]),
                Line::from(""),
                Line::styled(value.as_str(), Style::new().fg(*color)),
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
    render_history(frame, rows[3], theme, tab);
}

fn metric_rate_value(tab: &crate::model::dashboard::DashboardTab, key: MetricKey) -> String {
    tab.history
        .points(key)
        .last()
        .and_then(|point| point.value)
        .map_or_else(|| "--".into(), format_value)
}

fn current_metric_value(tab: &crate::model::dashboard::DashboardTab, key: MetricKey) -> String {
    tab.latest
        .as_ref()
        .and_then(|sample| sample.values.get(&key))
        .map_or_else(|| "--".into(), |value| format_value(*value))
}

fn transactions_rate(tab: &crate::model::dashboard::DashboardTab) -> String {
    let commits = tab
        .history
        .points(MetricKey::Commits)
        .last()
        .and_then(|p| p.value);
    let rollbacks = tab
        .history
        .points(MetricKey::Rollbacks)
        .last()
        .and_then(|p| p.value);
    match (commits, rollbacks) {
        (Some(commits), Some(rollbacks)) => format_value(commits + rollbacks),
        _ => metric_rate_value(tab, MetricKey::Transactions),
    }
}

fn cache_hit_value(tab: &crate::model::dashboard::DashboardTab) -> String {
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
    match (hits, reads) {
        (Some(hits), Some(reads)) if hits + reads > 0.0 => {
            format!("{:.2}%", hits / (hits + reads) * 100.0)
        }
        _ => "--".into(),
    }
}

fn connections_value(tab: &crate::model::dashboard::DashboardTab) -> String {
    let current = tab
        .latest
        .as_ref()
        .and_then(|sample| sample.values.get(&MetricKey::Connections))
        .map(|value| format_value(*value))
        .unwrap_or_else(|| "--".into());
    let maximum = tab
        .metadata
        .max_connections
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".into());
    format!("{current}/{maximum}")
}

fn uptime_value(tab: &crate::model::dashboard::DashboardTab) -> String {
    let seconds = tab
        .latest
        .as_ref()
        .and_then(|sample| sample.values.get(&MetricKey::ServerUptime))
        .copied();
    let Some(seconds) = seconds.filter(|value| value.is_finite() && *value >= 0.0) else {
        return "--".into();
    };
    let seconds = seconds as u64;
    format!(
        "{}d {:02}:{:02}:{:02}",
        seconds / 86_400,
        (seconds / 3_600) % 24,
        (seconds / 60) % 60,
        seconds % 60
    )
}

fn write_rate_label(tab: &crate::model::dashboard::DashboardTab) -> &'static str {
    if tab
        .latest
        .as_ref()
        .is_some_and(|sample| sample.values.contains_key(&MetricKey::WalBytes))
    {
        "WAL rate"
    } else {
        "Network out/s"
    }
}

fn write_rate_value(tab: &crate::model::dashboard::DashboardTab) -> String {
    let key = if tab
        .latest
        .as_ref()
        .is_some_and(|sample| sample.values.contains_key(&MetricKey::WalBytes))
    {
        MetricKey::WalBytes
    } else {
        MetricKey::BytesWritten
    };
    tab.history
        .points(key)
        .last()
        .and_then(|point| point.value)
        .map_or_else(|| "--".into(), format_bytes_rate)
}

fn format_bytes_rate(value: f64) -> String {
    let units = ["B/s", "KiB/s", "MiB/s", "GiB/s"];
    let mut value = value.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 || value.fract() == 0.0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
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
    state: &mut super::UiState,
    focused: bool,
) {
    let filter = format!(
        "Filter{}: {}  ·  {} shown / {} total{}{}",
        if tab.process_filter_active {
            " (editing)"
        } else {
            ""
        },
        if tab.effective_process_filter().is_empty() {
            "<all>"
        } else {
            tab.effective_process_filter()
        },
        tab.process_result_set().rows.len(),
        tab.processes.len(),
        if tab.process_truncated {
            "  ·  truncated at 2000"
        } else {
            ""
        },
        tab.process_error
            .as_deref()
            .map_or_else(String::new, |error| format!("  ·  ERROR: {error}")),
    );
    let status = if tab.process_loading {
        "  Loading process snapshot..."
    } else if tab.processes.is_empty() {
        "  No process samples yet"
    } else {
        match tab.visibility {
            crate::db::monitor::MonitorVisibility::Full => "  Visibility: full",
            crate::db::monitor::MonitorVisibility::Restricted => "  Visibility: restricted",
            crate::db::monitor::MonitorVisibility::Unknown => "  Visibility: unknown",
        }
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(filter).style(Style::new().fg(theme.muted)),
        sections[0],
    );
    if sections[0].height > 1 {
        frame.render_widget(
            Paragraph::new(status).style(Style::new().fg(theme.muted)),
            Rect::new(
                sections[0].x,
                sections[0].y.saturating_add(1),
                sections[0].width,
                1,
            ),
        );
    }
    let area = sections[1];
    let result = tab.process_result_set();
    state.result_area = Some(area);
    super::data_grid::render(
        frame,
        area,
        tab.id,
        &result,
        tab.grid.clone(),
        &tab.grid.column_widths,
        theme,
        Block::new()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(if focused { theme.accent } else { theme.border }))
            .title("Process List"),
        state,
        None,
        state.activity_icons,
    );
}

fn render_charts(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    tab: &crate::model::dashboard::DashboardTab,
) {
    render_history(frame, area, theme, tab);
}

fn render_history(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: Theme,
    tab: &crate::model::dashboard::DashboardTab,
) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let charts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(area);
    render_metric_chart(
        frame,
        charts[0],
        theme,
        tab,
        "Transactions and connections · last 10 minutes",
        &[
            (MetricKey::Commits, "commits/s", theme.accent),
            (MetricKey::Rollbacks, "rollbacks/s", theme.error),
            (MetricKey::Connections, "connections", theme.action),
        ],
    );
    render_metric_chart(
        frame,
        charts[1],
        theme,
        tab,
        "Statement activity · last 10 minutes",
        &[
            (MetricKey::Selects, "select activity/s", theme.action),
            (MetricKey::Inserts, "insert activity/s", theme.success),
            (MetricKey::Updates, "update activity/s", theme.warning),
            (MetricKey::Deletes, "delete activity/s", theme.error),
        ],
    );
}

fn render_metric_chart(
    frame: &mut Frame<'_>,
    chart_area: Rect,
    theme: Theme,
    tab: &crate::model::dashboard::DashboardTab,
    title: &'static str,
    series: &[(MetricKey, &'static str, ratatui::style::Color)],
) {
    if chart_area.width < 2 || chart_area.height < 2 {
        return;
    }
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.border))
        .title(title);
    let inner = block.inner(chart_area);
    let latest = tab.latest.as_ref();
    let mut series_data = Vec::new();
    for &(key, label, color) in series {
        let points = downsample_series(tab.history.points(key), inner.width as usize * 2);
        if !points.is_empty() {
            series_data.push((label, color, points));
        }
    }
    if series_data.is_empty() {
        frame.render_widget(
            Paragraph::new("Charts will appear after two valid samples.")
                .style(Style::new().fg(theme.muted))
                .block(Block::new().borders(Borders::ALL).title(title)),
            chart_area,
        );
        return;
    }
    frame.render_widget(block, chart_area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    let legend = series
        .iter()
        .enumerate()
        .flat_map(|(index, (key, label, color))| {
            let separator = (index > 0).then(|| Span::raw("  "));
            let value = tab
                .history
                .points(*key)
                .iter()
                .rev()
                .find_map(|point| point.value)
                .map_or_else(|| "--".into(), format_value);
            separator.into_iter().chain(std::iter::once(Span::styled(
                legend_value(label, &value),
                Style::new().fg(*color),
            )))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(Line::from(legend)).alignment(Alignment::Center),
        sections[0],
    );
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
    for &(key, _, _) in series {
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
        Span::styled(
            format!("┬ {}", format_timestamp(x_start)),
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!("┬ {}", format_timestamp(x_mid)),
            Style::new().fg(theme.muted),
        ),
        Span::styled(
            format!("{} ┬", format_timestamp(x_end)),
            Style::new().fg(theme.muted),
        ),
    ];
    let axis_style = Style::new().fg(theme.border).bg(theme.background);
    let label_style = Style::new().fg(theme.muted).bg(theme.background);
    frame.render_widget(
        Chart::new(datasets)
            .style(Style::new().bg(theme.background))
            .x_axis(
                Axis::default()
                    .style(axis_style)
                    .bounds([x_start, x_end.max(x_start + 1.0)])
                    .labels(labels)
                    .labels_alignment(Alignment::Right),
            )
            .y_axis(
                Axis::default()
                    .style(axis_style)
                    .bounds([0.0, max_y * 1.05])
                    .labels([
                        Span::styled("0 ─", label_style),
                        Span::styled(format!("{} ─", format_value(max_y / 2.0)), label_style),
                        Span::styled(format!("{} ─", format_value(max_y)), label_style),
                    ])
                    .labels_alignment(Alignment::Right),
            ),
        sections[1],
    );
}

fn legend_value(label: &str, value: &str) -> String {
    label.strip_suffix(" activity/s").map_or_else(
        || format!("{value} {label}"),
        |operation| format!("{operation} {value} activity/s"),
    )
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
