use std::time::Duration;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, Paragraph, Widget},
};

use super::{icons::IconSet, theme::Theme};
use crate::{cli::MotionMode, ui::animation::spinner_frame};

pub(crate) fn activity_text(
    label: &str,
    detail: Option<&str>,
    cancellable: bool,
    elapsed: Duration,
) -> String {
    activity_text_with_separator(label, detail, cancellable, elapsed, " · ")
}

fn activity_text_with_separator(
    label: &str,
    detail: Option<&str>,
    cancellable: bool,
    elapsed: Duration,
    separator: &str,
) -> String {
    let mut parts = vec![label.to_owned()];
    if let Some(detail) = detail {
        parts.push(detail.to_owned());
    }
    if elapsed >= Duration::from_secs(1) {
        let seconds = elapsed.as_secs_f64();
        if seconds < 10.0 {
            parts.push(format!("{seconds:.1}s"));
        } else {
            parts.push(format!("{}s", elapsed.as_secs()));
        }
    }
    if cancellable {
        parts.push("Ctrl-C cancel".to_owned());
    }
    parts.join(separator)
}

pub(crate) struct ActivityIndicator<'a> {
    pub mode: MotionMode,
    pub icons: IconSet,
    pub elapsed: Duration,
    pub label: &'a str,
    pub detail: Option<&'a str>,
    pub cancellable: bool,
    pub style: Style,
}

impl Widget for ActivityIndicator<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let frames = self.icons.activity_frames();
        let index = spinner_frame(self.mode, self.elapsed, frames.len());
        let marker = if self.mode == MotionMode::Off {
            "*"
        } else {
            frames[index]
        };
        let separator = if self.icons.activity_frames()[0].is_ascii() {
            " - "
        } else {
            " · "
        };
        let label = if separator == " · " {
            activity_text(self.label, self.detail, self.cancellable, self.elapsed)
        } else {
            activity_text_with_separator(
                self.label,
                self.detail,
                self.cancellable,
                self.elapsed,
                separator,
            )
        };
        let text = format!("{marker} {label}");
        Paragraph::new(text).style(self.style).render(area, buf);
    }
}

pub(crate) struct LoadingViewport<'a> {
    pub mode: MotionMode,
    pub icons: IconSet,
    pub elapsed: Duration,
    pub label: &'a str,
    pub helper: Option<&'a str>,
    pub cancellable: bool,
    pub theme: Theme,
    pub block: Block<'static>,
}

impl Widget for LoadingViewport<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = self.block.inner(area);
        self.block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        ActivityIndicator {
            mode: self.mode,
            icons: self.icons,
            elapsed: self.elapsed,
            label: self.label,
            detail: None,
            cancellable: self.cancellable,
            style: Style::new().fg(self.theme.action).bg(self.theme.surface),
        }
        .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
        let Some(helper) = self.helper else {
            return;
        };
        if inner.height < 3 {
            return;
        }
        let indent = inner.width.min(2);
        Paragraph::new(helper)
            .style(Style::new().fg(self.theme.muted).bg(self.theme.surface))
            .render(
                Rect::new(
                    inner.x.saturating_add(indent),
                    inner.y.saturating_add(2),
                    inner.width.saturating_sub(indent),
                    1,
                ),
                buf,
            );
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivityIndicator, LoadingViewport, activity_text};
    use crate::{
        cli::MotionMode,
        ui::{
            icons::{IconMode, IconSet},
            theme::Theme,
        },
    };
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        style::Style,
        widgets::{Block, Widget},
    };
    use std::time::Duration;

    #[test]
    fn activity_line_includes_elapsed_after_one_second() {
        assert_eq!(
            activity_text(
                "Executing query",
                Some("showing previous result"),
                true,
                Duration::from_millis(1_240)
            ),
            "Executing query · showing previous result · 1.2s · Ctrl-C cancel"
        );
    }

    #[test]
    fn activity_line_omits_subsecond_elapsed() {
        assert_eq!(
            activity_text(
                "Loading relation data",
                None,
                false,
                Duration::from_millis(900)
            ),
            "Loading relation data"
        );
    }

    #[test]
    fn loading_viewport_never_renders_dense_shading() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 12));
        LoadingViewport {
            mode: MotionMode::Off,
            icons: IconSet::default(),
            elapsed: Duration::from_secs(2),
            label: "Executing query",
            helper: Some("Waiting for the first result set..."),
            cancellable: true,
            theme: Theme::default(),
            block: Block::new(),
        }
        .render(Rect::new(0, 0, 80, 12), &mut buffer);
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| { !matches!(cell.symbol(), "░" | "▒" | "▓" | "█") })
        );
    }

    #[test]
    fn loading_viewport_is_ascii_safe_in_ascii_mode() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 6));
        LoadingViewport {
            mode: MotionMode::Off,
            icons: IconSet::new(IconMode::Ascii),
            elapsed: Duration::from_secs(2),
            label: "Executing query",
            helper: Some("Waiting for the first result set..."),
            cancellable: true,
            theme: Theme::default(),
            block: Block::new(),
        }
        .render(Rect::new(0, 0, 40, 6), &mut buffer);
        assert!(buffer.content().iter().all(|cell| cell.symbol().is_ascii()));
    }

    #[test]
    fn loading_viewport_omits_helper_when_height_is_tight() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 2));
        LoadingViewport {
            mode: MotionMode::Off,
            icons: IconSet::new(IconMode::Ascii),
            elapsed: Duration::from_secs(2),
            label: "Executing query",
            helper: Some("Waiting for the first result set..."),
            cancellable: true,
            theme: Theme::default(),
            block: Block::new(),
        }
        .render(Rect::new(0, 0, 40, 2), &mut buffer);
        let output = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(output.contains("Executing query"));
        assert!(!output.contains("Waiting for the first result set..."));
    }

    #[test]
    fn activity_indicator_renders_static_off_marker() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 30, 1));
        ActivityIndicator {
            mode: MotionMode::Off,
            icons: IconSet::default(),
            elapsed: Duration::ZERO,
            label: "Loading",
            detail: None,
            cancellable: false,
            style: Style::default(),
        }
        .render(Rect::new(0, 0, 30, 1), &mut buffer);
        assert_eq!(buffer[(0, 0)].symbol(), "*");
    }
}
