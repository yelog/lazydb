use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::model::workspace::Focus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutMode {
    TooSmall,
    Focus,
    Standard,
    Wide,
}

#[derive(Clone, Copy, Debug)]
pub struct AppLayout {
    pub mode: LayoutMode,
    pub header: Rect,
    pub tabs: Rect,
    pub body: Rect,
    pub explorer: Option<Rect>,
    pub editor: Option<Rect>,
    pub result_tabs: Option<Rect>,
    pub results: Option<Rect>,
    pub relation: Option<Rect>,
    pub footer: Rect,
}

impl AppLayout {
    pub fn calculate(area: Rect, focus: Focus, is_relation: bool) -> Self {
        if area.width < 56 || area.height < 16 {
            return Self {
                mode: LayoutMode::TooSmall,
                header: area,
                tabs: Rect::default(),
                body: area,
                explorer: None,
                editor: None,
                result_tabs: None,
                results: None,
                relation: None,
                footer: Rect::default(),
            };
        }

        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(area);
        let header = vertical[0];
        let tabs = vertical[1];
        let body = vertical[2];
        let footer = vertical[3];

        if area.width < 100 {
            return match focus {
                Focus::Explorer => Self {
                    mode: LayoutMode::Focus,
                    header,
                    tabs,
                    body,
                    explorer: Some(body),
                    editor: None,
                    result_tabs: None,
                    results: None,
                    relation: None,
                    footer,
                },
                Focus::Editor => Self {
                    mode: LayoutMode::Focus,
                    header,
                    tabs,
                    body,
                    explorer: None,
                    editor: Some(body),
                    result_tabs: None,
                    results: None,
                    relation: is_relation.then_some(body),
                    footer,
                },
                Focus::Results => {
                    if is_relation {
                        return Self {
                            mode: LayoutMode::Focus,
                            header,
                            tabs,
                            body,
                            explorer: None,
                            editor: None,
                            result_tabs: None,
                            results: None,
                            relation: Some(body),
                            footer,
                        };
                    }
                    let result = split_results(body);
                    Self {
                        mode: LayoutMode::Focus,
                        header,
                        tabs,
                        body,
                        explorer: None,
                        editor: None,
                        result_tabs: Some(result.0),
                        results: Some(result.1),
                        relation: None,
                        footer,
                    }
                }
            };
        }

        let mode = if area.width >= 160 {
            LayoutMode::Wide
        } else {
            LayoutMode::Standard
        };
        let explorer_width = (area.width / 3).clamp(34, 56);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(explorer_width), Constraint::Min(60)])
            .split(body);
        let main = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(46),
                Constraint::Length(2),
                Constraint::Min(7),
            ])
            .split(horizontal[1]);
        Self {
            mode,
            header,
            tabs,
            body,
            explorer: Some(horizontal[0]),
            editor: Some(main[0]),
            result_tabs: Some(main[1]),
            results: Some(main[2]),
            relation: is_relation.then_some(horizontal[1]),
            footer,
        }
    }
}

fn split_results(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(5)])
        .split(area);
    (chunks[0], chunks[1])
}
