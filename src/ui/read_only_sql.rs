use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
    text::Line,
    widgets::{Block, Paragraph},
};
use uuid::Uuid;

use crate::model::editor::{EditorMode, EditorRenderSnapshot, EditorViewport};

use super::{
    CursorSpec, CursorStyle, UiState, editor_line_spans, mouse_selection_cells,
    register_text_selection_target, render_editor_scrollbars, theme::Theme,
};

fn editor_columns(
    inner: Rect,
    snapshot: &EditorRenderSnapshot,
    show_line_numbers: bool,
) -> [Rect; 2] {
    let gutter_width = if show_line_numbers {
        snapshot.logical_line_count.max(1).to_string().len() + 1
    } else {
        0
    } as u16;
    let columns =
        Layout::horizontal([Constraint::Length(gutter_width), Constraint::Min(1)]).split(inner);
    [columns[0], columns[1]]
}

/// The text cells available after the line-number gutter and prompt row.
/// Use the same layout as the renderer, including on very narrow panes.
pub(crate) fn editor_viewport(
    inner: Rect,
    snapshot: &EditorRenderSnapshot,
    show_line_numbers: bool,
) -> EditorViewport {
    let body = editor_columns(inner, snapshot, show_line_numbers)[1];
    EditorViewport {
        width: usize::from(body.width),
        height: usize::from(body.height).saturating_sub(usize::from(snapshot.prompt.is_some())),
    }
}

/// Render a read-only editor snapshot without assuming which workspace tab
/// owns the editor session. DDL and modal viewers can therefore share the
/// same selection, scrolling, and syntax rendering behavior.
pub(crate) struct ReadOnlySqlEditor<'a> {
    pub session_id: Uuid,
    pub snapshot: &'a EditorRenderSnapshot,
    pub block: Block<'a>,
    pub focused: bool,
    pub show_line_numbers: bool,
}

impl ReadOnlySqlEditor<'_> {
    pub(crate) fn render(
        self,
        frame: &mut Frame<'_>,
        area: Rect,
        theme: Theme,
        state: &mut UiState,
    ) {
        let inner = self.block.inner(area);
        let [gutter, body] = editor_columns(inner, self.snapshot, self.show_line_numbers);
        let gutter_width = self.snapshot.logical_line_count.max(1).to_string().len() + 1;
        let viewport_height = editor_viewport(inner, self.snapshot, self.show_line_numbers).height;
        let selection_body = Rect::new(body.x, body.y, body.width, viewport_height as u16);
        register_text_selection_target(state, self.session_id, selection_body, self.snapshot);
        frame.render_widget(self.block, area);
        if self.show_line_numbers {
            for (row, line) in self.snapshot.lines.iter().take(viewport_height).enumerate() {
                frame.render_widget(
                    Paragraph::new(format!(
                        "{:>width$} ",
                        if line.wrap_offset == 0 {
                            (line.line + 1).to_string()
                        } else {
                            String::new()
                        },
                        width = gutter_width - 1
                    ))
                    .style(Style::new().fg(theme.muted)),
                    Rect::new(
                        gutter.x,
                        gutter.y.saturating_add(row as u16),
                        gutter.width,
                        1,
                    ),
                );
            }
        }
        for (row, line) in self.snapshot.lines.iter().take(viewport_height).enumerate() {
            let selected = self
                .snapshot
                .selection_cells
                .iter()
                .any(|(selected_line, _, _)| *selected_line == line.line);
            let spans = editor_line_spans(
                line,
                self.snapshot,
                theme,
                true,
                None,
                &mouse_selection_cells(state, self.session_id, self.snapshot, line),
                None,
            );
            frame.render_widget(
                Paragraph::new(Line::from(spans))
                    .style(ratatui::style::Style::new().bg(if selected {
                        theme.selection
                    } else {
                        theme.surface
                    }))
                    .scroll((
                        0,
                        (self.snapshot.horizontal_offset + line.wrap_offset).min(u16::MAX as usize)
                            as u16,
                    )),
                Rect::new(body.x, body.y.saturating_add(row as u16), body.width, 1),
            );
        }
        render_editor_scrollbars(
            frame,
            area,
            Some(self.session_id),
            self.snapshot,
            theme,
            state,
            Some(Rect::new(
                area.x.saturating_add(1),
                area.bottom().saturating_sub(1),
                area.width.saturating_sub(2),
                1,
            )),
        );
        if let Some(prompt) = self.snapshot.prompt.as_ref() {
            let prompt_y = body.bottom().saturating_sub(1);
            let prompt_text = match prompt.error.as_deref() {
                Some(error) => format!("{}{}  [{}]", prompt.prefix, prompt.text, error),
                None => format!("{}{}", prompt.prefix, prompt.text),
            };
            frame.render_widget(
                Paragraph::new(prompt_text).style(Style::new().fg(theme.accent).bg(theme.surface)),
                Rect::new(body.x, prompt_y, body.width, 1),
            );
            if self.focused {
                let cursor_x = body
                    .x
                    .saturating_add(prompt.prefix.chars().count() as u16)
                    .saturating_add(prompt.cursor as u16)
                    .min(body.right().saturating_sub(1));
                state.cursor = Some(CursorSpec {
                    position: Position::new(cursor_x, prompt_y),
                    style: CursorStyle::Bar,
                });
            }
        } else if self.focused
            && let Some((x, y)) = self.snapshot.cursor_screen_cell
        {
            state.cursor = Some(CursorSpec {
                position: Position::new(body.x.saturating_add(x), body.y.saturating_add(y)),
                style: match self.snapshot.mode {
                    EditorMode::Insert => CursorStyle::Bar,
                    EditorMode::Replace => CursorStyle::Underline,
                    _ => CursorStyle::Block,
                },
            });
        }
    }
}
