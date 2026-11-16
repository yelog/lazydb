use ratatui::{
    Frame,
    layout::{Alignment, Position, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthChar;

use super::{
    CursorSpec, CursorStyle, HitRegion, HitTarget, UiState, centered, dialog, dialog_footer,
    panel_block,
    shortcut_hints::{self, ShortcutHint},
    theme::Theme,
};
use crate::{
    app::App,
    data_ops::{DataEndpoint, DataJobPlan, DataJobRecord, DataJobStatus, DataOperation},
    model::{
        data_ops::{DataField, DataOperationAction as A, DataOperationDraft},
        workspace::Overlay,
    },
    security::sanitize_terminal_text,
};

pub(super) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    let tasks = app.overlay == Some(Overlay::DataTasks);
    let draft = app.data_operations.draft.as_ref();
    if !tasks && draft.is_none() {
        return;
    }
    let title = if tasks {
        " DATA TASKS ".to_owned()
    } else {
        let draft = draft.unwrap();
        format!(
            " {} DATABASE · {} ",
            draft.operation.label().to_uppercase(),
            if draft.plan.is_some() {
                "REVIEW"
            } else {
                "DRAFT"
            }
        )
    };
    let popup = centered(area, 100, 30);
    frame.render_widget(Clear, popup);
    let block = panel_block(&title, true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.is_empty() {
        return;
    }
    let mut actions = Vec::new();
    let focused;
    let mut hints;
    if tasks {
        let selected = app.data_operations.selected_record();
        let can_cancel = selected.is_some_and(|record| {
            !record.status.is_terminal()
                && app.data_operations.runs.contains_key(&record.id)
                && !app.data_operations.cancelling.contains(&record.id)
        });
        actions.push(("Close", A::Close, true, false));
        actions.push((
            "Rerun Draft",
            A::RerunTask,
            selected.is_some_and(|record| record.status.is_terminal()),
            false,
        ));
        actions.push(("Cancel Task", A::CancelTask, can_cancel, false));
        actions.push((
            "Refresh",
            A::RefreshTasks,
            app.data_operations.history_request.is_none(),
            false,
        ));
        focused = None;
        hints = vec![
            ShortcutHint::new("Esc", "close"),
            ShortcutHint::new("↑/↓", "select"),
            ShortcutHint::new("r", "new draft"),
            ShortcutHint::new("c", "cancel task"),
            ShortcutHint::new("R", "refresh"),
            ShortcutHint::new("PgUp/PgDn", "details"),
        ];
    } else if draft.unwrap().plan.is_some() {
        actions.push(("Edit Draft", A::Back, true, false));
        actions.push((
            "Execute",
            A::Execute,
            true,
            draft.unwrap().operation.writes_database(),
        ));
        focused = Some(1);
        hints = vec![
            ShortcutHint::new("Esc", "edit draft"),
            ShortcutHint::new("Enter", "execute reviewed plan"),
            ShortcutHint::new("↑/↓", "scroll"),
        ];
    } else {
        let draft = draft.unwrap();
        actions.push(("Cancel", A::Close, true, false));
        actions.push((
            "Review Plan",
            A::Review,
            draft.pending_plan.is_none(),
            false,
        ));
        focused = match draft.focus {
            DataField::Cancel => Some(0),
            DataField::Review => Some(1),
            _ => None,
        };
        hints = vec![
            ShortcutHint::new("Esc", "cancel"),
            ShortcutHint::new("Tab/⇧Tab", "focus"),
            ShortcutHint::new(
                "Enter",
                if draft.focus == DataField::Cancel {
                    "cancel"
                } else {
                    "review plan"
                },
            ),
        ];
        if matches!(
            draft.focus,
            DataField::SourceProfile | DataField::TargetProfile
        ) {
            hints.push(ShortcutHint::new("←/→", "choose profile"));
        }
    }
    let labels = actions
        .iter()
        .map(|(label, _, _, _)| *label)
        .collect::<Vec<_>>();
    let action_width =
        dialog_footer::action_width(&labels, 2).saturating_add(actions.len() as u16 * 6);
    let rows = if action_width > inner.width {
        actions.len() as u16
    } else {
        1
    };
    let footer = dialog_footer::measure(
        inner,
        action_width,
        1,
        dialog_footer::FooterDensity::Compact,
    );
    let action_y = footer
        .status_and_actions
        .y
        .saturating_sub(rows - 1)
        .max(inner.y);
    let action_area = Rect::new(
        inner.x,
        action_y,
        inner.width,
        footer.help.y.saturating_sub(action_y),
    );
    let body = Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(2),
        action_y.saturating_sub(inner.y).saturating_sub(1),
    );
    if tasks {
        render_tasks(frame, body, app, state, theme);
    } else {
        let draft = draft.unwrap();
        if let Some(plan) = &draft.plan {
            let mut lines = plan_lines(plan, theme);
            if let Some(error) = &draft.error {
                lines.insert(
                    0,
                    Line::styled(sanitize_terminal_text(error), Style::new().fg(theme.error)),
                );
            }
            render_scrolled(frame, body, lines, draft.scroll, theme);
        } else {
            render_form(frame, body, draft, app, state, theme);
        }
    }
    let buttons = actions
        .iter()
        .map(|(label, _, enabled, danger)| dialog::DialogButton {
            label,
            tone: if *danger {
                dialog::DialogTone::Danger
            } else {
                dialog::DialogTone::Normal
            },
            emphasis: if *label == "Execute" || *label == "Review Plan" {
                dialog::DialogEmphasis::Primary
            } else {
                dialog::DialogEmphasis::Secondary
            },
            enabled: *enabled,
        })
        .collect::<Vec<_>>();
    let regions = match focused {
        Some(index) => dialog::render_actions(frame, action_area, &buttons, index, theme),
        None => dialog::render_actions_without_focus(frame, action_area, &buttons, theme),
    };
    for region in regions {
        state.hit_regions.push(HitRegion {
            area: region.area,
            target: HitTarget::DataOperation(Box::new(actions[region.index].1.clone())),
        });
    }
    shortcut_hints::render(
        frame,
        footer.help,
        &hints,
        theme,
        theme.surface,
        Alignment::Left,
    );
}

fn render_form(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &DataOperationDraft,
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }
    let status = draft
        .error
        .as_deref()
        .unwrap_or(if draft.pending_plan.is_some() {
            "Checking tools, scope and paths… You can keep editing or cancel."
        } else {
            "Full database only. Existing artifacts and target databases / files are refused."
        });
    frame.render_widget(
        Paragraph::new(sanitize_terminal_text(status)).style(Style::new().fg(
            if draft.error.is_some() {
                theme.error
            } else {
                theme.muted
            },
        )),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let fields = draft
        .fields()
        .into_iter()
        .filter(|field| !matches!(field, DataField::Review | DataField::Cancel))
        .collect::<Vec<_>>();
    let viewport = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    let visible = (viewport.height / 2).max(1) as usize;
    let selected = fields
        .iter()
        .position(|field| *field == draft.focus)
        .unwrap_or(fields.len().saturating_sub(1));
    let start = selected.saturating_sub(visible.saturating_sub(1));
    for (row, field) in fields.iter().skip(start).take(visible).enumerate() {
        let y = viewport.y + row as u16 * 2;
        if y >= viewport.bottom() {
            break;
        }
        let focused = draft.focus == *field;
        let label = format!("{} {}", if focused { ">" } else { " " }, field.label());
        frame.render_widget(
            Paragraph::new(label).style(Style::new().fg(if focused {
                theme.accent
            } else {
                theme.muted
            })),
            Rect::new(area.x, y, area.width, 1),
        );
        if y + 1 >= viewport.bottom() {
            continue;
        }
        let value_area = Rect::new(area.x + 2, y + 1, area.width.saturating_sub(2), 1);
        let style = if focused {
            Style::new()
                .fg(theme.text)
                .bg(theme.surface_raised)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new().fg(theme.text)
        };
        let value = if let Some(input) = draft.input(*field) {
            let clean = sanitize_terminal_text(input.value());
            let prefix_width = clean
                .chars()
                .take(input.cursor())
                .map(|character| character.width().unwrap_or_default())
                .sum::<usize>();
            let skip_cells = if focused {
                prefix_width.saturating_sub(value_area.width.saturating_sub(1) as usize)
            } else {
                0
            };
            let mut skipped = 0;
            let shown = clean
                .chars()
                .skip_while(|character| {
                    if skipped >= skip_cells {
                        return false;
                    }
                    skipped += character.width().unwrap_or_default();
                    true
                })
                .collect::<String>();
            if focused && value_area.width > 0 {
                state.cursor = Some(CursorSpec {
                    position: Position::new(
                        value_area.x
                            + prefix_width
                                .saturating_sub(skipped)
                                .min(value_area.width.saturating_sub(1) as usize)
                                as u16,
                        value_area.y,
                    ),
                    style: CursorStyle::Bar,
                });
            }
            if shown.is_empty() {
                match field {
                    DataField::Artifact if draft.operation == DataOperation::Restore => {
                        "Path to an existing native backup".into()
                    }
                    DataField::Artifact => "Path to a NEW backup artifact".into(),
                    DataField::TargetDatabase => "NEW database name or SQLite file path".into(),
                    DataField::DumpTool | DataField::RestoreTool => "Automatic discovery".into(),
                    _ => String::new(),
                }
            } else {
                shown
            }
        } else {
            let id = if *field == DataField::SourceProfile {
                draft.source_profile_id
            } else {
                draft.target_profile_id
            };
            id.and_then(|id| app.profiles.iter().find(|profile| profile.id == id))
                .map(|profile| {
                    format!(
                        "‹ {} · {:?}{} ›",
                        sanitize_terminal_text(&profile.name),
                        profile.kind,
                        if profile.read_only {
                            " · read-only"
                        } else {
                            ""
                        }
                    )
                })
                .unwrap_or_else(|| "‹ Select profile with ←/→ ›".into())
        };
        frame.render_widget(Paragraph::new(value).style(style), value_area);
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                area.x,
                y,
                area.width,
                2.min(viewport.bottom().saturating_sub(y)),
            ),
            target: HitTarget::DataOperation(Box::new(A::FocusField(*field))),
        });
    }
}

fn endpoint_lines(label: &str, endpoint: Option<&DataEndpoint>) -> Vec<Line<'static>> {
    endpoint
        .map(|endpoint| {
            vec![
                Line::from(format!("{label} profile: {}", endpoint.profile_id)),
                Line::from(format!(
                    "{label} database / file: {}",
                    sanitize_terminal_text(&endpoint.database)
                )),
            ]
        })
        .unwrap_or_default()
}

fn plan_lines(plan: &DataJobPlan, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled(
            format!(
                "{} · immutable plan {}",
                plan.spec.operation.label(),
                plan.id
            ),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Line::from("Full database scope"),
    ];
    lines.extend(endpoint_lines("Source", plan.spec.source.as_ref()));
    lines.extend(endpoint_lines("NEW target", plan.spec.destination.as_ref()));
    lines.push(Line::from(format!(
        "{} artifact: {}",
        if plan.spec.operation == DataOperation::Restore {
            "Input"
        } else {
            "NEW output"
        },
        sanitize_terminal_text(&plan.spec.artifact.to_string_lossy())
    )));
    lines.push(Line::styled(
        "Overwrite refused: existing output files and target databases are never replaced.",
        Style::new().fg(theme.warning),
    ));
    lines.push(Line::from(""));
    lines.push(Line::styled("STEPS", Style::new().fg(theme.accent)));
    for (index, step) in plan.native.steps.iter().enumerate() {
        lines.push(Line::from(format!(
            "{}. {}",
            index + 1,
            sanitize_terminal_text(step)
        )));
    }
    for tool in &plan.native.tools {
        lines.push(Line::from(format!(
            "Tool: {} · {} · {}",
            sanitize_terminal_text(&tool.name),
            sanitize_terminal_text(&tool.version),
            sanitize_terminal_text(&tool.path.to_string_lossy())
        )));
    }
    for warning in &plan.native.warnings {
        lines.push(Line::styled(
            format!("Warning: {}", sanitize_terminal_text(warning)),
            Style::new().fg(theme.warning),
        ));
    }
    lines.push(Line::styled(
        "Completion reports the actual verification level; it does not imply data parity.",
        Style::new().fg(theme.muted),
    ));
    if plan.spec.operation.writes_database() {
        lines.push(Line::styled("Failure or cancellation can leave a partial target. Review task details before cleanup.",
            Style::new().fg(theme.warning)));
    }
    lines
}

fn render_tasks(
    frame: &mut Frame<'_>,
    mut area: Rect,
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }
    let data = &app.data_operations;
    if !data.runs.is_empty() {
        let guidance = "Before quitting: wait for tasks to finish, or cancel with c and wait.";
        let rows = (guidance.len() as u16)
            .div_ceil(area.width)
            .min(area.height);
        frame.render_widget(
            Paragraph::new(guidance)
                .wrap(Wrap { trim: false })
                .style(Style::new().fg(theme.warning)),
            Rect::new(area.x, area.y, area.width, rows),
        );
        area.y += rows;
        area.height -= rows;
        if area.is_empty() {
            return;
        }
    }
    if data.records.is_empty() {
        let text = data
            .history_error
            .as_deref()
            .unwrap_or(if data.history_request.is_some() {
                "Loading data tasks…"
            } else {
                "No data tasks yet. F2 → Backup Database, Restore Database or Copy Database."
            });
        frame.render_widget(
            Paragraph::new(sanitize_terminal_text(text)).wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let list_rows = 5.min((area.height / 3).max(1)) as usize;
    let start = data.selected.saturating_sub(list_rows.saturating_sub(1));
    for (row, record) in data.records.iter().enumerate().skip(start).take(list_rows) {
        let row_area = Rect::new(area.x, area.y + (row - start) as u16, area.width, 1);
        let selected = row == data.selected;
        let status = if data.cancelling.contains(&record.id) {
            "Cancelling".into()
        } else {
            format!("{:?}", record.status)
        };
        let target = record
            .plan
            .spec
            .destination
            .as_ref()
            .or(record.plan.spec.source.as_ref())
            .map(|endpoint| endpoint.database.as_str())
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(format!(
                "{} {}  {}  {}",
                if selected { ">" } else { " " },
                record.plan.spec.operation.label(),
                status,
                sanitize_terminal_text(target)
            ))
            .style(if selected {
                Style::new()
                    .fg(theme.text)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.muted)
            }),
            row_area,
        );
        state.hit_regions.push(HitRegion {
            area: row_area,
            target: HitTarget::DataOperation(Box::new(A::TaskSelect(row))),
        });
    }
    let y = area.y + list_rows as u16 + 1;
    if y >= area.bottom() {
        return;
    }
    let mut details = data
        .selected_record()
        .map(|record| record_lines(record, theme))
        .unwrap_or_default();
    if let Some(error) = &data.history_error {
        details.insert(
            0,
            Line::styled(sanitize_terminal_text(error), Style::new().fg(theme.error)),
        );
    }
    render_scrolled(
        frame,
        Rect::new(area.x, y, area.width, area.bottom() - y),
        details,
        data.scroll,
        theme,
    );
}

fn record_lines(record: &DataJobRecord, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled(
            format!("{:?} · {}", record.status, record.id),
            Style::new().fg(if record.status.is_success() {
                theme.success
            } else if matches!(
                record.status,
                DataJobStatus::Failed | DataJobStatus::Interrupted
            ) {
                theme.error
            } else {
                theme.accent
            }),
        ),
        Line::from(sanitize_terminal_text(&record.message)),
    ];
    if let Some(error) = &record.error {
        lines.push(Line::styled(
            sanitize_terminal_text(error),
            Style::new().fg(theme.error),
        ));
    }
    if record.target_may_be_partial {
        lines.push(Line::styled(
            "WARNING: target may be partial; no automatic target removal was attempted.",
            Style::new().fg(theme.warning),
        ));
    }
    if let Some(outcome) = &record.outcome {
        for verification in &outcome.verification {
            lines.push(Line::from(format!(
                "Verified: {}",
                sanitize_terminal_text(verification)
            )));
        }
        for warning in &outcome.warnings {
            lines.push(Line::styled(
                format!("Warning: {}", sanitize_terminal_text(warning)),
                Style::new().fg(theme.warning),
            ));
        }
        if let Some(bytes) = outcome.bytes {
            lines.push(Line::from(format!("Artifact bytes: {bytes}")));
        }
        if let Some(checksum) = &outcome.sha256 {
            lines.push(Line::from(format!(
                "SHA-256: {}",
                sanitize_terminal_text(checksum)
            )));
        }
    } else if let Some(bytes) = record.bytes {
        lines.push(Line::from(format!("Processed bytes: {bytes}")));
    }
    lines.extend(plan_lines(&record.plan, theme));
    if !record.log.is_empty() {
        lines.push(Line::styled("LOG", Style::new().fg(theme.accent)));
        lines.extend(
            record
                .log
                .iter()
                .map(|message| Line::from(sanitize_terminal_text(message))),
        );
    }
    lines
}

fn render_scrolled(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
    scroll: usize,
    theme: Theme,
) {
    let rows = lines
        .iter()
        .map(|line| (line.width().max(1) as u16).div_ceil(area.width.max(1)) as usize)
        .sum::<usize>();
    let max_scroll = rows.saturating_sub(area.height as usize);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll.min(max_scroll).min(u16::MAX as usize) as u16, 0))
            .style(Style::new().fg(theme.text).bg(theme.surface)),
        area,
    );
}
