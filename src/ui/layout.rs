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
    pub tabs: Option<Rect>,
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
                tabs: None,
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
                Constraint::Min(8),
                Constraint::Length(2),
            ])
            .split(area);
        let header = vertical[0];
        let body = vertical[1];
        let footer = vertical[2];

        if area.width < 100 {
            return match focus {
                Focus::Explorer => Self {
                    mode: LayoutMode::Focus,
                    header,
                    tabs: None,
                    body,
                    explorer: Some(body),
                    editor: None,
                    result_tabs: None,
                    results: None,
                    relation: None,
                    footer,
                },
                Focus::Editor => {
                    let (tabs, content) = split_main_content(body);
                    Self {
                        mode: LayoutMode::Focus,
                        header,
                        tabs: Some(tabs),
                        body,
                        explorer: None,
                        editor: (!is_relation).then_some(content),
                        result_tabs: None,
                        results: None,
                        relation: is_relation.then_some(content),
                        footer,
                    }
                }
                Focus::Results => {
                    if is_relation {
                        let (tabs, content) = split_main_content(body);
                        return Self {
                            mode: LayoutMode::Focus,
                            header,
                            tabs: Some(tabs),
                            body,
                            explorer: None,
                            editor: None,
                            result_tabs: None,
                            results: None,
                            relation: Some(content),
                            footer,
                        };
                    }
                    let (tabs, content) = split_main_content(body);
                    let result = split_results(content);
                    Self {
                        mode: LayoutMode::Focus,
                        header,
                        tabs: Some(tabs),
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
            .constraints([Constraint::Length(2), Constraint::Min(7)])
            .split(horizontal[1]);
        let content = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(46),
                Constraint::Length(2),
                Constraint::Min(7),
            ])
            .split(main[1]);
        Self {
            mode,
            header,
            tabs: Some(main[0]),
            body,
            explorer: Some(horizontal[0]),
            editor: (!is_relation).then_some(content[0]),
            result_tabs: (!is_relation).then_some(content[1]),
            results: (!is_relation).then_some(content[2]),
            relation: is_relation.then_some(main[1]),
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

fn split_main_content(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(area);
    (chunks[0], chunks[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_workspace_tabs_above_main_content() {
        let layout = AppLayout::calculate(Rect::new(0, 0, 180, 50), Focus::Editor, false);
        let explorer = layout.explorer.unwrap();
        let tabs = layout.tabs.unwrap();
        let editor = layout.editor.unwrap();

        assert_eq!(tabs.x, explorer.right());
        assert_eq!(tabs.y, explorer.y);
        assert_eq!(tabs.width, editor.width);
        assert_eq!(editor.y, tabs.bottom());
        assert_eq!(explorer.bottom(), layout.body.bottom());
    }

    #[test]
    fn relation_content_starts_below_workspace_tabs() {
        let layout = AppLayout::calculate(Rect::new(0, 0, 180, 50), Focus::Results, true);

        assert_eq!(layout.relation.unwrap().y, layout.tabs.unwrap().bottom());
        assert_eq!(layout.relation.unwrap().x, layout.tabs.unwrap().x);
    }

    #[test]
    fn narrow_explorer_hides_workspace_tabs() {
        let layout = AppLayout::calculate(Rect::new(0, 0, 90, 40), Focus::Explorer, false);

        assert_eq!(layout.tabs, None);
        assert_eq!(layout.explorer, Some(layout.body));
    }

    #[test]
    fn narrow_main_focus_keeps_workspace_tabs_visible() {
        let editor = AppLayout::calculate(Rect::new(0, 0, 90, 40), Focus::Editor, false);
        let results = AppLayout::calculate(Rect::new(0, 0, 90, 40), Focus::Results, false);

        assert_eq!(editor.editor.unwrap().y, editor.tabs.unwrap().bottom());
        assert_eq!(
            results.result_tabs.unwrap().y,
            results.tabs.unwrap().bottom()
        );
    }
}
