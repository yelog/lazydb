use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::model::workspace::{Focus, PaneLayoutMetrics, PaneSizePreferences};

const MIN_EXPLORER_WIDTH: u16 = 34;
const MAX_DEFAULT_EXPLORER_WIDTH: u16 = 56;
const MIN_RIGHT_WIDTH: u16 = 60;
const HEADER_HEIGHT: u16 = 1;
const FOOTER_HEIGHT: u16 = 1;
const WORKSPACE_TABS_HEIGHT: u16 = 2;
const RESULT_TABS_HEIGHT: u16 = 2;
const MIN_EDITOR_HEIGHT: u16 = 1;
const MIN_RESULTS_HEIGHT: u16 = 7;

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
    pub pane_metrics: PaneLayoutMetrics,
}

impl AppLayout {
    pub fn calculate(
        area: Rect,
        focus: Focus,
        is_relation: bool,
        preferences: PaneSizePreferences,
        pane_maximized: bool,
    ) -> Self {
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
                pane_metrics: PaneLayoutMetrics::default(),
            };
        }

        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(HEADER_HEIGHT),
                Constraint::Min(8),
                Constraint::Length(FOOTER_HEIGHT),
            ])
            .split(area);
        let header = vertical[0];
        let body = vertical[1];
        let footer = vertical[2];

        if pane_maximized || area.width < 100 {
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
                    pane_metrics: PaneLayoutMetrics::default(),
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
                        pane_metrics: PaneLayoutMetrics::default(),
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
                            pane_metrics: PaneLayoutMetrics::default(),
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
                        pane_metrics: PaneLayoutMetrics::default(),
                    }
                }
            };
        }

        let mode = if area.width >= 160 {
            LayoutMode::Wide
        } else {
            LayoutMode::Standard
        };
        let explorer_width = explorer_width(area.width, preferences.explorer_width);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(explorer_width),
                Constraint::Min(MIN_RIGHT_WIDTH),
            ])
            .split(body);
        let main = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(WORKSPACE_TABS_HEIGHT),
                Constraint::Min(MIN_EDITOR_HEIGHT + RESULT_TABS_HEIGHT + MIN_RESULTS_HEIGHT),
            ])
            .split(horizontal[1]);
        let content_area = main[1];
        let content = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(editor_height(
                    content_area.height,
                    preferences.editor_height,
                )),
                Constraint::Length(RESULT_TABS_HEIGHT),
                Constraint::Min(MIN_RESULTS_HEIGHT),
            ])
            .split(content_area);
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
            pane_metrics: PaneLayoutMetrics {
                explorer_width: Some(horizontal[0].width),
                editor_height: (!is_relation).then_some(content[0].height),
            },
        }
    }
}

fn explorer_width(area_width: u16, preference: Option<u16>) -> u16 {
    let maximum = area_width.saturating_sub(MIN_RIGHT_WIDTH);
    preference
        .unwrap_or((area_width / 3).clamp(MIN_EXPLORER_WIDTH, MAX_DEFAULT_EXPLORER_WIDTH))
        .clamp(MIN_EXPLORER_WIDTH.min(maximum), maximum)
}

fn editor_height(content_height: u16, preference: Option<u16>) -> u16 {
    let maximum = content_height.saturating_sub(RESULT_TABS_HEIGHT + MIN_RESULTS_HEIGHT);
    preference
        .unwrap_or((content_height * 46 / 100).max(MIN_EDITOR_HEIGHT))
        .clamp(MIN_EDITOR_HEIGHT.min(maximum), maximum)
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
    fn header_and_footer_each_use_one_row() {
        let area = Rect::new(0, 0, 120, 36);
        let layout = AppLayout::calculate(
            area,
            Focus::Editor,
            false,
            PaneSizePreferences::default(),
            false,
        );

        assert_eq!(layout.header.height, 1);
        assert_eq!(layout.footer.height, 1);
        assert_eq!(layout.header.y, area.y);
        assert_eq!(layout.body.y, area.y + 1);
        assert_eq!(layout.footer.y, area.bottom() - 1);
        assert_eq!(
            layout.header.height + layout.body.height + layout.footer.height,
            area.height
        );
    }

    #[test]
    fn places_workspace_tabs_above_main_content() {
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 180, 50),
            Focus::Editor,
            false,
            PaneSizePreferences::default(),
            false,
        );
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
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 180, 50),
            Focus::Results,
            true,
            PaneSizePreferences::default(),
            false,
        );

        assert_eq!(layout.relation.unwrap().y, layout.tabs.unwrap().bottom());
        assert_eq!(layout.relation.unwrap().x, layout.tabs.unwrap().x);
    }

    #[test]
    fn narrow_explorer_hides_workspace_tabs() {
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 90, 40),
            Focus::Explorer,
            false,
            PaneSizePreferences::default(),
            false,
        );

        assert_eq!(layout.tabs, None);
        assert_eq!(layout.explorer, Some(layout.body));
    }

    #[test]
    fn narrow_main_focus_keeps_workspace_tabs_visible() {
        let editor = AppLayout::calculate(
            Rect::new(0, 0, 90, 40),
            Focus::Editor,
            false,
            PaneSizePreferences::default(),
            false,
        );
        let results = AppLayout::calculate(
            Rect::new(0, 0, 90, 40),
            Focus::Results,
            false,
            PaneSizePreferences::default(),
            false,
        );

        assert_eq!(editor.editor.unwrap().y, editor.tabs.unwrap().bottom());
        assert_eq!(
            results.result_tabs.unwrap().y,
            results.tabs.unwrap().bottom()
        );
    }

    #[test]
    fn explicit_preferences_control_split_sizes() {
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 180, 50),
            Focus::Editor,
            false,
            PaneSizePreferences {
                explorer_width: Some(70),
                editor_height: Some(20),
            },
            false,
        );

        assert_eq!(layout.explorer.unwrap().width, 70);
        assert_eq!(layout.editor.unwrap().height, 20);
        assert_eq!(layout.pane_metrics.explorer_width, Some(70));
        assert_eq!(layout.pane_metrics.editor_height, Some(20));
    }

    #[test]
    fn explicit_preferences_are_clamped_to_available_space() {
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 120, 30),
            Focus::Editor,
            false,
            PaneSizePreferences {
                explorer_width: Some(u16::MAX),
                editor_height: Some(u16::MAX),
            },
            false,
        );

        assert_eq!(layout.explorer.unwrap().width, 60);
        assert_eq!(layout.editor.unwrap().height, 17);
        assert_eq!(layout.results.unwrap().height, 7);
    }

    #[test]
    fn relation_layout_has_no_editor_height_metric() {
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 180, 50),
            Focus::Results,
            true,
            PaneSizePreferences {
                explorer_width: Some(70),
                editor_height: Some(20),
            },
            false,
        );

        assert_eq!(layout.pane_metrics.explorer_width, Some(70));
        assert_eq!(layout.pane_metrics.editor_height, None);
        assert_eq!(layout.relation.unwrap().height, layout.body.height - 2);
    }

    #[test]
    fn maximized_sql_layout_only_exposes_the_focused_pane() {
        for focus in [Focus::Explorer, Focus::Editor, Focus::Results] {
            let layout = AppLayout::calculate(
                Rect::new(0, 0, 180, 50),
                focus,
                false,
                PaneSizePreferences::default(),
                true,
            );

            assert_eq!(layout.mode, LayoutMode::Focus);
            assert_eq!(layout.explorer.is_some(), focus == Focus::Explorer);
            assert_eq!(layout.editor.is_some(), focus == Focus::Editor);
            assert_eq!(layout.results.is_some(), focus == Focus::Results);
            assert_eq!(layout.pane_metrics, PaneLayoutMetrics::default());
        }
    }

    #[test]
    fn maximized_relation_layout_uses_results_as_the_main_pane() {
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 180, 50),
            Focus::Results,
            true,
            PaneSizePreferences::default(),
            true,
        );

        assert_eq!(layout.mode, LayoutMode::Focus);
        assert!(layout.explorer.is_none());
        assert!(layout.editor.is_none());
        assert!(layout.results.is_none());
        assert!(layout.relation.is_some());
        assert!(layout.tabs.is_some());
        assert_eq!(layout.pane_metrics, PaneLayoutMetrics::default());
    }
}
