//! Semantic user commands shared by Help, key bindings, and Omni.

use uuid::Uuid;

use crate::{db::catalog::CatalogId, model::execution_target::ExecutionTarget};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandId {
    OpenDashboard,
    RunStatement,
    RunBuffer,
    CloseTab,
    OpenConsoleManager,
    NewConsole,
    OpenRelation,
    ShowRelationDdl,
    FormatSql,
    TransactionControl,
    ReturnToPreviousLocation,
    OpenNotificationHistory,
    OpenUpdateCenter,
    FocusExplorer,
    FocusResults,
    FocusEditor,
    CyclePaneFocus,
    TogglePaneMaximized,
    ResetPaneSizes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandAvailability {
    Ready,
    NeedsArguments,
    Disabled(&'static str),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandContext {
    pub profile_id: Option<Uuid>,
    pub target: Option<ExecutionTarget>,
    pub tab_id: Option<Uuid>,
    pub catalog_id: Option<CatalogId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserIntent {
    OpenDashboard,
    OpenConsole {
        profile_id: Option<Uuid>,
        console_id: Uuid,
    },
    OpenTab {
        tab_id: Uuid,
    },
    RunStatement {
        context: CommandContext,
    },
    RunBuffer {
        context: CommandContext,
    },
    CloseTab {
        tab_id: Uuid,
    },
    OpenConsoleManager,
    NewConsole {
        profile_id: Option<Uuid>,
        target: Option<ExecutionTarget>,
    },
    OpenRelation {
        catalog_id: CatalogId,
        view: crate::model::relation::RelationView,
    },
    FormatSql {
        context: CommandContext,
    },
    TransactionControl {
        context: CommandContext,
    },
    ReturnToPreviousLocation,
    OpenNotificationHistory,
    OpenUpdateCenter,
    FocusExplorer,
    FocusResults,
    FocusEditor,
    CyclePaneFocus,
    TogglePaneMaximized,
    ResetPaneSizes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub id: CommandId,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
    pub category: &'static str,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: CommandId::OpenDashboard,
        title: "Open Dashboard",
        aliases: &["dashboard", "monitor"],
        category: "Workspace",
    },
    CommandSpec {
        id: CommandId::RunStatement,
        title: "Run Current Statement",
        aliases: &["run statement", "execute statement"],
        category: "SQL",
    },
    CommandSpec {
        id: CommandId::RunBuffer,
        title: "Run SQL Buffer",
        aliases: &["run all", "execute buffer"],
        category: "SQL",
    },
    CommandSpec {
        id: CommandId::CloseTab,
        title: "Close Current Tab",
        aliases: &["close tab"],
        category: "Workspace",
    },
    CommandSpec {
        id: CommandId::OpenConsoleManager,
        title: "Open Console Manager",
        aliases: &["consoles", "sql editors"],
        category: "Console",
    },
    CommandSpec {
        id: CommandId::NewConsole,
        title: "New Console",
        aliases: &["create console", "new sql editor"],
        category: "Console",
    },
    CommandSpec {
        id: CommandId::OpenRelation,
        title: "Open Table Data",
        aliases: &["open table", "preview table", "relation data"],
        category: "Catalog",
    },
    CommandSpec {
        id: CommandId::ShowRelationDdl,
        title: "Show Table DDL",
        aliases: &["ddl", "definition", "create statement"],
        category: "Catalog",
    },
    CommandSpec {
        id: CommandId::FormatSql,
        title: "Format SQL",
        aliases: &["format query", "pretty sql"],
        category: "SQL",
    },
    CommandSpec {
        id: CommandId::TransactionControl,
        title: "Open Transaction Controls",
        aliases: &[
            "commit",
            "rollback",
            "commit transaction",
            "roll back transaction",
        ],
        category: "Transaction",
    },
    CommandSpec {
        id: CommandId::ReturnToPreviousLocation,
        title: "Return to Previous Location",
        aliases: &["back", "previous location"],
        category: "Navigation",
    },
    CommandSpec {
        id: CommandId::OpenNotificationHistory,
        title: "Open Notification History",
        aliases: &["notifications", "notification history"],
        category: "Workspace",
    },
    CommandSpec {
        id: CommandId::OpenUpdateCenter,
        title: "Open Update Center",
        aliases: &["updates", "check for updates"],
        category: "Workspace",
    },
    CommandSpec {
        id: CommandId::FocusExplorer,
        title: "Focus Explorer",
        aliases: &["explorer", "database tree"],
        category: "Focus",
    },
    CommandSpec {
        id: CommandId::FocusResults,
        title: "Focus Results",
        aliases: &["results", "data grid"],
        category: "Focus",
    },
    CommandSpec {
        id: CommandId::FocusEditor,
        title: "Focus SQL Editor",
        aliases: &["editor", "sql"],
        category: "Focus",
    },
    CommandSpec {
        id: CommandId::CyclePaneFocus,
        title: "Cycle Pane Focus",
        aliases: &["next pane", "cycle panes"],
        category: "Focus",
    },
    CommandSpec {
        id: CommandId::TogglePaneMaximized,
        title: "Toggle Pane Maximized",
        aliases: &["maximize pane", "restore pane"],
        category: "Layout",
    },
    CommandSpec {
        id: CommandId::ResetPaneSizes,
        title: "Reset Pane Sizes",
        aliases: &["reset layout"],
        category: "Layout",
    },
];

pub fn command_for_help(id: crate::help::HelpShortcutId) -> Option<CommandId> {
    use crate::help::HelpShortcutId as Help;

    Some(match id {
        Help::OpenDashboard => CommandId::OpenDashboard,
        Help::RunSql | Help::EditorRun => CommandId::RunStatement,
        Help::RunAllSql => CommandId::RunBuffer,
        Help::CloseTab => CommandId::CloseTab,
        Help::OpenSqlEditors => CommandId::OpenConsoleManager,
        Help::ExplorerPreview => CommandId::OpenRelation,
        Help::ExplorerDdl => CommandId::ShowRelationDdl,
        Help::EditorFormat => CommandId::FormatSql,
        Help::OpenNotificationHistory | Help::OpenNotificationHistoryLeader => {
            CommandId::OpenNotificationHistory
        }
        Help::OpenUpdateCenter => CommandId::OpenUpdateCenter,
        Help::FocusExplorer | Help::FocusExplorerLeader => CommandId::FocusExplorer,
        Help::FocusResults | Help::FocusResultsFromL => CommandId::FocusResults,
        Help::FocusEditorFromK | Help::FocusEditorFromL => CommandId::FocusEditor,
        Help::CyclePaneFocus => CommandId::CyclePaneFocus,
        Help::TogglePaneMaximized => CommandId::TogglePaneMaximized,
        Help::ResetPaneSizes => CommandId::ResetPaneSizes,
        Help::TransactionControl | Help::RelationCommit | Help::RelationRollback => {
            CommandId::TransactionControl
        }
        _ => return None,
    })
}

pub fn command(id: CommandId) -> &'static CommandSpec {
    COMMANDS
        .iter()
        .find(|spec| spec.id == id)
        .expect("every semantic command has a specification")
}

pub fn intent_for_command(id: CommandId, context: &CommandContext) -> Option<UserIntent> {
    Some(match id {
        CommandId::OpenDashboard => UserIntent::OpenDashboard,
        CommandId::RunStatement => UserIntent::RunStatement {
            context: context.clone(),
        },
        CommandId::RunBuffer => UserIntent::RunBuffer {
            context: context.clone(),
        },
        CommandId::CloseTab => UserIntent::CloseTab {
            tab_id: context.tab_id?,
        },
        CommandId::OpenConsoleManager => UserIntent::OpenConsoleManager,
        CommandId::NewConsole => UserIntent::NewConsole {
            profile_id: context.profile_id,
            target: context.target.clone(),
        },
        CommandId::OpenRelation | CommandId::ShowRelationDdl => {
            let view = if id == CommandId::OpenRelation {
                crate::model::relation::RelationView::Data
            } else {
                crate::model::relation::RelationView::Ddl
            };
            UserIntent::OpenRelation {
                catalog_id: context.catalog_id.clone()?,
                view,
            }
        }
        CommandId::FormatSql => UserIntent::FormatSql {
            context: context.clone(),
        },
        CommandId::TransactionControl => UserIntent::TransactionControl {
            context: context.clone(),
        },
        CommandId::OpenNotificationHistory => UserIntent::OpenNotificationHistory,
        CommandId::OpenUpdateCenter => UserIntent::OpenUpdateCenter,
        CommandId::FocusExplorer => UserIntent::FocusExplorer,
        CommandId::FocusResults => UserIntent::FocusResults,
        CommandId::FocusEditor => UserIntent::FocusEditor,
        CommandId::CyclePaneFocus => UserIntent::CyclePaneFocus,
        CommandId::TogglePaneMaximized => UserIntent::TogglePaneMaximized,
        CommandId::ResetPaneSizes => UserIntent::ResetPaneSizes,
        CommandId::ReturnToPreviousLocation => UserIntent::ReturnToPreviousLocation,
    })
}

pub fn availability(id: CommandId, context: &CommandContext) -> CommandAvailability {
    match id {
        CommandId::RunStatement
        | CommandId::RunBuffer
        | CommandId::FormatSql
        | CommandId::TransactionControl
            if context.tab_id.is_none() =>
        {
            CommandAvailability::Disabled("No SQL console is available")
        }
        CommandId::OpenRelation | CommandId::ShowRelationDdl if context.catalog_id.is_none() => {
            CommandAvailability::NeedsArguments
        }
        CommandId::NewConsole | CommandId::ReturnToPreviousLocation => {
            CommandAvailability::NeedsArguments
        }
        _ => CommandAvailability::Ready,
    }
}

pub fn matching_commands(query: &str) -> Vec<&'static CommandSpec> {
    let tokens = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    COMMANDS
        .iter()
        .filter(|spec| {
            let fields = std::iter::once(spec.title)
                .chain(std::iter::once(spec.category))
                .chain(spec.aliases.iter().copied())
                .map(str::to_lowercase)
                .collect::<Vec<_>>();
            tokens
                .iter()
                .all(|token| fields.iter().any(|field| field.contains(token)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn aliases_resolve_to_one_semantic_command() {
        let specs = matching_commands("execute statement");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].id, CommandId::RunStatement);
        assert_eq!(
            COMMANDS
                .iter()
                .map(|spec| spec.id)
                .collect::<HashSet<_>>()
                .len(),
            COMMANDS.len()
        );
    }

    #[test]
    fn help_aliases_map_to_the_same_semantic_command() {
        use crate::help::HelpShortcutId as Help;

        assert_eq!(
            command_for_help(Help::RunSql),
            command_for_help(Help::EditorRun)
        );
        assert_eq!(
            command_for_help(Help::ExplorerPreview),
            Some(CommandId::OpenRelation)
        );
    }

    #[test]
    fn command_context_retains_the_origin_target_and_tab() {
        let tab_id = Uuid::from_u128(7);
        let profile_id = Uuid::from_u128(9);
        let context = CommandContext {
            profile_id: Some(profile_id),
            tab_id: Some(tab_id),
            ..CommandContext::default()
        };
        let intent = UserIntent::RunStatement {
            context: context.clone(),
        };

        assert!(matches!(
            intent,
            UserIntent::RunStatement { context: captured }
                if captured.tab_id == Some(tab_id)
                    && captured.profile_id == Some(profile_id)
        ));
    }

    #[test]
    fn intent_uses_the_captured_console_instead_of_a_later_active_tab() {
        let tab_id = Uuid::from_u128(7);
        let context = CommandContext {
            tab_id: Some(tab_id),
            ..CommandContext::default()
        };

        assert_eq!(
            intent_for_command(CommandId::CloseTab, &context).unwrap(),
            UserIntent::CloseTab { tab_id }
        );
    }

    #[test]
    fn contextless_editor_command_explains_why_it_is_disabled() {
        assert_eq!(
            availability(CommandId::RunStatement, &CommandContext::default()),
            CommandAvailability::Disabled("No SQL console is available")
        );
    }

    #[test]
    fn command_search_matches_tokens_across_aliases() {
        assert_eq!(
            matching_commands("table ddl")[0].id,
            CommandId::ShowRelationDdl
        );
    }
}
