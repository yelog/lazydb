use uuid::Uuid;

use crate::{
    action::{Action, Command},
    model::{
        editor::EditorMode,
        tab::{ConsoleTab, OutputEntry, OutputKind, ResultView},
        workspace::{
            ConnectionState, ConnectionStatus, ExplorerState, Focus, Overlay, QueryStatus,
        },
    },
    profile::ConnectionProfile,
};

#[derive(Clone, Debug)]
pub struct App {
    pub profiles: Vec<ConnectionProfile>,
    pub connection: ConnectionState,
    pub explorer: ExplorerState,
    pub tabs: Vec<ConsoleTab>,
    pub active_tab: usize,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    pub should_quit: bool,
    next_console_number: usize,
}

impl App {
    pub fn new(profiles: Vec<ConnectionProfile>) -> Self {
        Self {
            profiles,
            connection: ConnectionState::default(),
            explorer: ExplorerState::default(),
            tabs: vec![ConsoleTab::new("console")],
            active_tab: 0,
            focus: Focus::Editor,
            overlay: None,
            should_quit: false,
            next_console_number: 2,
        }
    }

    pub fn active_console(&self) -> &ConsoleTab {
        &self.tabs[self.active_tab]
    }

    pub fn active_console_mut(&mut self) -> &mut ConsoleTab {
        &mut self.tabs[self.active_tab]
    }

    pub fn active_profile(&self) -> Option<&ConnectionProfile> {
        let profile_id = self.connection.profile_id?;
        self.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    }

    pub fn update(&mut self, action: Action) -> Vec<Command> {
        match action {
            Action::NewConsole => {
                let name = format!("console_{}", self.next_console_number);
                self.next_console_number += 1;
                self.tabs.push(ConsoleTab::new(name));
                self.active_tab = self.tabs.len() - 1;
                self.focus = Focus::Editor;
                vec![Command::PersistWorkspace]
            }
            Action::CloseActiveTab => {
                if self.tabs.len() > 1 {
                    self.tabs.remove(self.active_tab);
                    self.active_tab = self.active_tab.saturating_sub(1);
                    vec![Command::PersistWorkspace]
                } else {
                    Vec::new()
                }
            }
            Action::NextTab => {
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                Vec::new()
            }
            Action::PreviousTab => {
                self.active_tab = self
                    .active_tab
                    .checked_sub(1)
                    .unwrap_or(self.tabs.len() - 1);
                Vec::new()
            }
            Action::ActivateTab(index) => {
                if index < self.tabs.len() {
                    self.active_tab = index;
                }
                Vec::new()
            }
            Action::FocusNext => {
                self.focus = self.focus.next();
                Vec::new()
            }
            Action::FocusPrevious => {
                self.focus = self.focus.previous();
                Vec::new()
            }
            Action::Focus(focus) => {
                self.focus = focus;
                Vec::new()
            }
            Action::ShowHelp => {
                self.overlay = Some(Overlay::Help(self.focus));
                Vec::new()
            }
            Action::DismissOverlay => {
                self.overlay = None;
                Vec::new()
            }
            Action::ReplaceEditor(text) => {
                self.active_console_mut().editor.set_text(text);
                vec![Command::PersistWorkspace]
            }
            Action::InsertCharacter(character) => {
                self.active_console_mut().editor.insert(character);
                Vec::new()
            }
            Action::InsertNewline => {
                self.active_console_mut().editor.newline();
                Vec::new()
            }
            Action::Backspace => {
                self.active_console_mut().editor.backspace();
                Vec::new()
            }
            Action::Delete => {
                self.active_console_mut().editor.delete();
                Vec::new()
            }
            Action::MoveLeft => {
                self.active_console_mut().editor.move_left();
                Vec::new()
            }
            Action::MoveRight => {
                self.active_console_mut().editor.move_right();
                Vec::new()
            }
            Action::MoveUp => {
                self.active_console_mut().editor.move_up();
                Vec::new()
            }
            Action::MoveDown => {
                self.active_console_mut().editor.move_down();
                Vec::new()
            }
            Action::MoveHome => {
                self.active_console_mut().editor.move_home();
                Vec::new()
            }
            Action::MoveEnd => {
                self.active_console_mut().editor.move_end();
                Vec::new()
            }
            Action::EnterNormalMode => {
                self.active_console_mut().editor.mode = EditorMode::Normal;
                Vec::new()
            }
            Action::EnterInsertMode => {
                self.active_console_mut().editor.mode = EditorMode::Insert;
                Vec::new()
            }
            Action::EnterAppendMode => {
                let editor = &mut self.active_console_mut().editor;
                editor.move_right();
                editor.mode = EditorMode::Insert;
                Vec::new()
            }
            Action::OpenLineBelow => {
                let editor = &mut self.active_console_mut().editor;
                editor.move_end();
                editor.newline();
                editor.mode = EditorMode::Insert;
                Vec::new()
            }
            Action::RunActiveSql => self.run_active_sql(),
            Action::CancelActiveQuery => {
                let tab = self.active_console_mut();
                if tab.query_status != QueryStatus::Running {
                    return Vec::new();
                }
                tab.query_status = QueryStatus::Cancelled;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Cancelled,
                    message: "Query cancellation requested".to_owned(),
                });
                vec![Command::CancelQuery {
                    tab_id: tab.id,
                    generation: tab.generation,
                }]
            }
            Action::RefreshCatalog => {
                let (Some(profile_id), ConnectionStatus::Connected) =
                    (self.connection.profile_id, self.connection.status)
                else {
                    return Vec::new();
                };
                vec![Command::LoadCatalog {
                    profile_id,
                    generation: self.connection.generation,
                }]
            }
            Action::PreviewSelected => self.preview_selected(),
            Action::DdlSelected => self.ddl_selected(),
            Action::RequestConnect(profile_id) => {
                if !self.profiles.iter().any(|profile| profile.id == profile_id) {
                    return Vec::new();
                }
                self.connection.generation += 1;
                self.connection.profile_id = Some(profile_id);
                self.connection.status = ConnectionStatus::Connecting;
                self.connection.server = None;
                self.connection.error = None;
                vec![Command::Connect {
                    profile_id,
                    generation: self.connection.generation,
                }]
            }
            Action::ConnectionSucceeded {
                profile_id,
                generation,
                server,
            } => {
                if !self.connection_matches(profile_id, generation) {
                    return Vec::new();
                }
                self.connection.status = ConnectionStatus::Connected;
                self.connection.server = Some(server);
                self.connection.error = None;
                vec![Command::LoadCatalog {
                    profile_id,
                    generation,
                }]
            }
            Action::ConnectionFailed {
                profile_id,
                generation,
                message,
            } => {
                if self.connection_matches(profile_id, generation) {
                    self.connection.status = ConnectionStatus::Failed;
                    self.connection.error = Some(message);
                }
                Vec::new()
            }
            Action::CatalogLoaded {
                profile_id,
                generation,
                nodes,
            } => {
                if self.connection_matches(profile_id, generation) {
                    self.explorer.set_nodes(nodes);
                }
                Vec::new()
            }
            Action::CatalogFailed {
                profile_id,
                generation,
                message,
            } => {
                if self.connection_matches(profile_id, generation) {
                    self.connection.error = Some(message);
                }
                Vec::new()
            }
            Action::QueryFinished {
                tab_id,
                generation,
                outcome,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
                    return Vec::new();
                }
                let total_ms = outcome.stats.total().as_millis();
                let rows = outcome.stats.row_count;
                tab.query_status = QueryStatus::Idle;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Success,
                    message: format!("{rows} row(s) retrieved in {total_ms} ms"),
                });
                tab.outcome = Some(outcome);
                tab.result_view = ResultView::Data;
                Vec::new()
            }
            Action::QueryFailed {
                tab_id,
                generation,
                message,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
                    return Vec::new();
                }
                tab.query_status = QueryStatus::Failed;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Error,
                    message,
                });
                tab.result_view = ResultView::Output;
                Vec::new()
            }
            Action::PreviewFinished {
                tab_id,
                generation,
                sql,
                outcome,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
                    return Vec::new();
                }
                tab.editor.set_text(sql);
                tab.editor.mode = EditorMode::Normal;
                let rows = outcome.stats.row_count;
                let total_ms = outcome.stats.total().as_millis();
                tab.outcome = Some(outcome);
                tab.query_status = QueryStatus::Idle;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Success,
                    message: format!("{rows} row(s) previewed in {total_ms} ms"),
                });
                tab.result_view = ResultView::Data;
                Vec::new()
            }
            Action::DdlLoaded {
                tab_id,
                generation,
                ddl,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
                    return Vec::new();
                }
                tab.editor.set_text(ddl);
                tab.editor.mode = EditorMode::Normal;
                tab.query_status = QueryStatus::Idle;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Success,
                    message: "Object DDL loaded".to_owned(),
                });
                self.focus = Focus::Editor;
                Vec::new()
            }
            Action::ExplorerMove(delta) => {
                self.explorer.move_selection(delta);
                Vec::new()
            }
            Action::ExplorerSelect(index) => {
                if index < self.explorer.visible().len() {
                    self.explorer.selected = index;
                }
                Vec::new()
            }
            Action::GridMove { rows, columns } => {
                let tab = self.active_console_mut();
                let (row_count, column_count) = tab
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.result_sets.last())
                    .map(|result| (result.rows.len(), result.columns.len()))
                    .unwrap_or((0, 0));
                tab.selected_row = move_bounded(tab.selected_row, rows, row_count);
                tab.selected_column = move_bounded(tab.selected_column, columns, column_count);
                Vec::new()
            }
            Action::GridSelect { row, column } => {
                let tab = self.active_console_mut();
                let (row_count, column_count) = tab
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.result_sets.last())
                    .map(|result| (result.rows.len(), result.columns.len()))
                    .unwrap_or((0, 0));
                tab.selected_row = row.min(row_count.saturating_sub(1));
                tab.selected_column = column.min(column_count.saturating_sub(1));
                Vec::new()
            }
            Action::ExplorerToggle => {
                self.explorer.toggle_selected();
                Vec::new()
            }
            Action::ToggleResultView => {
                let tab = self.active_console_mut();
                tab.result_view = match tab.result_view {
                    ResultView::Data => ResultView::Output,
                    ResultView::Output | ResultView::Plan => ResultView::Data,
                };
                Vec::new()
            }
            Action::Quit => {
                self.should_quit = true;
                vec![Command::Quit]
            }
        }
    }

    fn run_active_sql(&mut self) -> Vec<Command> {
        let tab = self.active_console_mut();
        let sql = tab.editor.text();
        if sql.trim().is_empty() || tab.query_status == QueryStatus::Running {
            return Vec::new();
        }
        tab.generation += 1;
        tab.query_status = QueryStatus::Running;
        tab.output.push(OutputEntry {
            kind: OutputKind::Info,
            message: "Executing SQL".to_owned(),
        });
        vec![Command::RunQuery {
            tab_id: tab.id,
            generation: tab.generation,
            sql,
        }]
    }

    fn preview_selected(&mut self) -> Vec<Command> {
        let Some(node) = self.explorer.selected_node().cloned() else {
            return Vec::new();
        };
        if !matches!(
            node.kind,
            crate::db::catalog::CatalogKind::Table | crate::db::catalog::CatalogKind::View
        ) {
            self.explorer.toggle_selected();
            return Vec::new();
        }
        let Some(schema) = node
            .id
            .native_path
            .get(node.id.native_path.len().saturating_sub(2))
            .cloned()
        else {
            return Vec::new();
        };
        let mut tab = ConsoleTab::new(format!("{} data", node.name));
        tab.generation = 1;
        tab.query_status = QueryStatus::Running;
        tab.editor
            .set_text(format!("-- Loading preview for {schema}.{}", node.name));
        tab.editor.mode = EditorMode::Normal;
        let tab_id = tab.id;
        let generation = tab.generation;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.focus = Focus::Results;
        vec![Command::PreviewTable {
            tab_id,
            generation,
            schema,
            name: node.name,
        }]
    }

    fn ddl_selected(&mut self) -> Vec<Command> {
        let Some(node) = self.explorer.selected_node().cloned() else {
            return Vec::new();
        };
        if !matches!(
            node.kind,
            crate::db::catalog::CatalogKind::Table
                | crate::db::catalog::CatalogKind::View
                | crate::db::catalog::CatalogKind::Index
                | crate::db::catalog::CatalogKind::Trigger
        ) {
            return Vec::new();
        }
        let Some(schema) = node
            .id
            .native_path
            .get(node.id.native_path.len().saturating_sub(2))
            .cloned()
        else {
            return Vec::new();
        };
        let mut tab = ConsoleTab::new(format!("{} DDL", node.name));
        tab.generation = 1;
        tab.query_status = QueryStatus::Running;
        let tab_id = tab.id;
        let generation = tab.generation;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.focus = Focus::Editor;
        vec![Command::LoadDdl {
            tab_id,
            generation,
            kind: node.kind,
            schema,
            name: node.name,
        }]
    }

    fn connection_matches(&self, profile_id: Uuid, generation: u64) -> bool {
        self.connection.profile_id == Some(profile_id) && self.connection.generation == generation
    }
}

fn move_bounded(current: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        current
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        action::{Action, Command},
        db::query::{QueryOutcome, QueryStats, ResultSet},
        model::workspace::{Focus, Overlay, QueryStatus},
        profile::import_connection_url,
    };

    use super::App;

    fn empty_outcome() -> QueryOutcome {
        QueryOutcome {
            result_sets: vec![ResultSet::default()],
            stats: QueryStats::new(Duration::from_millis(2), Duration::from_millis(3), 0),
        }
    }

    #[test]
    fn names_new_consoles_without_reusing_sequence_numbers() {
        let mut app = App::new(Vec::new());
        assert_eq!(app.active_console().name, "console");

        app.update(Action::NewConsole);
        app.update(Action::NewConsole);
        assert_eq!(
            app.tabs
                .iter()
                .map(|tab| tab.name.as_str())
                .collect::<Vec<_>>(),
            ["console", "console_2", "console_3"]
        );

        app.update(Action::CloseActiveTab);
        app.update(Action::NewConsole);
        assert_eq!(app.active_console().name, "console_4");
    }

    #[test]
    fn closing_active_tab_chooses_the_left_neighbor() {
        let mut app = App::new(Vec::new());
        app.update(Action::NewConsole);
        app.update(Action::NewConsole);
        assert_eq!(app.active_tab, 2);

        app.update(Action::CloseActiveTab);

        assert_eq!(app.active_tab, 1);
        assert_eq!(app.active_console().name, "console_2");
    }

    #[test]
    fn keeps_at_least_one_console_open() {
        let mut app = App::new(Vec::new());

        app.update(Action::CloseActiveTab);

        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_console().name, "console");
    }

    #[test]
    fn cycles_focus_in_both_directions() {
        let mut app = App::new(Vec::new());
        assert_eq!(app.focus, Focus::Editor);

        app.update(Action::FocusNext);
        assert_eq!(app.focus, Focus::Results);
        app.update(Action::FocusNext);
        assert_eq!(app.focus, Focus::Explorer);
        app.update(Action::FocusPrevious);
        assert_eq!(app.focus, Focus::Results);
    }

    #[test]
    fn help_is_scoped_and_escape_dismisses_it() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Explorer;

        app.update(Action::ShowHelp);
        assert_eq!(app.overlay, Some(Overlay::Help(Focus::Explorer)));
        app.update(Action::DismissOverlay);
        assert_eq!(app.overlay, None);
    }

    #[test]
    fn stale_query_results_cannot_replace_newer_runs() {
        let mut app = App::new(Vec::new());
        app.update(Action::ReplaceEditor("SELECT 1".into()));
        let commands = app.update(Action::RunActiveSql);
        let (tab_id, generation) = match &commands[0] {
            Command::RunQuery {
                tab_id, generation, ..
            } => (*tab_id, *generation),
            command => panic!("unexpected command: {command:?}"),
        };
        assert_eq!(app.active_console().query_status, QueryStatus::Running);

        app.update(Action::QueryFinished {
            tab_id,
            generation: generation.saturating_sub(1),
            outcome: empty_outcome(),
        });
        assert!(app.active_console().outcome.is_none());

        app.update(Action::QueryFinished {
            tab_id,
            generation,
            outcome: empty_outcome(),
        });
        assert!(app.active_console().outcome.is_some());
        assert_eq!(app.active_console().query_status, QueryStatus::Idle);
    }

    #[test]
    fn empty_sql_does_not_start_a_query() {
        let mut app = App::new(Vec::new());

        assert!(app.update(Action::RunActiveSql).is_empty());
        assert_eq!(app.active_console().query_status, QueryStatus::Idle);
    }

    #[test]
    fn stale_connection_events_are_ignored() {
        let profile = import_connection_url("sqlite::memory:", Some("demo"))
            .unwrap()
            .profile;
        let profile_id = profile.id;
        let mut app = App::new(vec![profile]);
        let commands = app.update(Action::RequestConnect(profile_id));
        let generation = match commands[0] {
            Command::Connect { generation, .. } => generation,
            ref command => panic!("unexpected command: {command:?}"),
        };

        app.update(Action::ConnectionFailed {
            profile_id,
            generation: generation + 1,
            message: "late".into(),
        });
        assert!(app.connection.error.is_none());
    }
}
