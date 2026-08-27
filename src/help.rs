use crate::model::workspace::Focus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpShortcutId {
    FocusExplorer,
    FocusResults,
    FocusEditorFromK,
    FocusEditorFromL,
    PreviousTab,
    NextTab,
    NewConsole,
    GotoSqlConsole,
    ExplorerMoveDown,
    ExplorerMoveUp,
    ExplorerExpand,
    ExplorerCollapse,
    ExplorerToggle,
    ExplorerActivate,
    ExplorerNewProfile,
    ExplorerEditProfile,
    ExplorerDeleteProfile,
    ExplorerConnect,
    ExplorerDisconnect,
    ExplorerRefresh,
    ExplorerPreview,
    ExplorerDdl,
    EditorInsert,
    EditorNormal,
    EditorUndo,
    EditorRedo,
    EditorRun,
    ToggleTransaction,
    CommitTransaction,
    RollbackTransaction,
    OpenTargetSelector,
    ResultsMoveLeft,
    ResultsMoveDown,
    ResultsMoveUp,
    ResultsMoveRight,
    ResultsToggleView,
    RelationWhere,
    RelationOrderBy,
    RelationApplyInputs,
    RelationResizeLeft,
    RelationResizeRight,
    RelationResetWidth,
    RelationRefresh,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HelpShortcut {
    pub id: HelpShortcutId,
    pub key: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpState {
    pub context: Focus,
    pub query: String,
    pub selected: usize,
}

impl HelpState {
    pub fn new(context: Focus) -> Self {
        Self {
            context,
            query: String::new(),
            selected: 0,
        }
    }

    pub fn insert(&mut self, character: char) {
        self.query.push(character);
        self.selected = 0;
    }

    pub fn paste(&mut self, value: &str) {
        let value = value
            .chars()
            .map(|character| match character {
                '\r' | '\n' | '\t' => ' ',
                character => character,
            })
            .collect::<String>();
        self.query.push_str(&value);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize, result_count: usize) {
        if result_count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = if delta.is_negative() {
            self.selected
                .checked_sub(delta.unsigned_abs())
                .unwrap_or(result_count - 1)
        } else {
            (self.selected + delta as usize) % result_count
        };
    }

    pub fn selected_id(&self, relation_data: bool) -> Option<HelpShortcutId> {
        filtered_shortcuts(self.context, relation_data, &self.query)
            .get(self.selected)
            .map(|shortcut| shortcut.id)
    }
}

pub fn shortcuts(context: Focus, relation_data: bool) -> Vec<HelpShortcut> {
    let mut entries = vec![
        HelpShortcut {
            id: HelpShortcutId::FocusExplorer,
            key: "Ctrl-w h",
            description: "focus Explorer",
        },
        HelpShortcut {
            id: HelpShortcutId::FocusResults,
            key: "Ctrl-w j",
            description: "focus Results",
        },
        HelpShortcut {
            id: HelpShortcutId::FocusEditorFromK,
            key: "Ctrl-w k",
            description: "focus Editor",
        },
        HelpShortcut {
            id: HelpShortcutId::FocusEditorFromL,
            key: "Ctrl-w l",
            description: "focus Editor",
        },
        HelpShortcut {
            id: HelpShortcutId::PreviousTab,
            key: "[ then t",
            description: "previous tab",
        },
        HelpShortcut {
            id: HelpShortcutId::NextTab,
            key: "] then t",
            description: "next tab",
        },
        HelpShortcut {
            id: HelpShortcutId::NewConsole,
            key: "Space n",
            description: "new SQL console",
        },
        HelpShortcut {
            id: HelpShortcutId::GotoSqlConsole,
            key: "Space s",
            description: "go to first SQL console",
        },
    ];
    entries.extend(match context {
        Focus::Explorer => vec![
            HelpShortcut {
                id: HelpShortcutId::ExplorerMoveDown,
                key: "j",
                description: "move selection down",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerMoveUp,
                key: "k",
                description: "move selection up",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerExpand,
                key: "l",
                description: "expand selection",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerCollapse,
                key: "h",
                description: "collapse selection",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerToggle,
                key: "o",
                description: "toggle expand / collapse",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerActivate,
                key: "Enter",
                description: "open table preview / activate",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerNewProfile,
                key: "n",
                description: "new connection",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerEditProfile,
                key: "e",
                description: "edit connection",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerDeleteProfile,
                key: "d",
                description: "delete connection",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerConnect,
                key: "c",
                description: "connect",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerDisconnect,
                key: "x",
                description: "disconnect",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerRefresh,
                key: "r",
                description: "refresh connection or catalog",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerPreview,
                key: "p",
                description: "open table preview",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerDdl,
                key: "D",
                description: "open object DDL",
            },
        ],
        Focus::Editor => vec![
            HelpShortcut {
                id: HelpShortcutId::EditorInsert,
                key: "i",
                description: "enter Insert mode",
            },
            HelpShortcut {
                id: HelpShortcutId::EditorNormal,
                key: "Esc",
                description: "return to Normal mode",
            },
            HelpShortcut {
                id: HelpShortcutId::EditorUndo,
                key: "u",
                description: "undo",
            },
            HelpShortcut {
                id: HelpShortcutId::EditorRedo,
                key: "Ctrl-r",
                description: "redo",
            },
            HelpShortcut {
                id: HelpShortcutId::EditorRun,
                key: "F5",
                description: "execute SQL buffer",
            },
            HelpShortcut {
                id: HelpShortcutId::ToggleTransaction,
                key: "Space tt",
                description: "toggle AUTO / MANUAL transaction",
            },
            HelpShortcut {
                id: HelpShortcutId::CommitTransaction,
                key: "Space tc",
                description: "commit transaction",
            },
            HelpShortcut {
                id: HelpShortcutId::RollbackTransaction,
                key: "Space tr",
                description: "roll back transaction",
            },
            HelpShortcut {
                id: HelpShortcutId::OpenTargetSelector,
                key: "Space d",
                description: "choose editor connection target",
            },
        ],
        Focus::Results => {
            let mut results = vec![
                HelpShortcut {
                    id: HelpShortcutId::ResultsMoveLeft,
                    key: "h",
                    description: "move through cells left",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsMoveDown,
                    key: "j",
                    description: "move through cells down",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsMoveUp,
                    key: "k",
                    description: "move through cells up",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsMoveRight,
                    key: "l",
                    description: "move through cells right",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsToggleView,
                    key: "o",
                    description: "switch Data / Output",
                },
            ];
            if relation_data {
                results.extend([
                    HelpShortcut {
                        id: HelpShortcutId::RelationWhere,
                        key: "/",
                        description: "focus WHERE filter",
                    },
                    HelpShortcut {
                        id: HelpShortcutId::RelationOrderBy,
                        key: "s",
                        description: "focus ORDER BY",
                    },
                    HelpShortcut {
                        id: HelpShortcutId::RelationApplyInputs,
                        key: "Enter",
                        description: "apply preview inputs",
                    },
                    HelpShortcut {
                        id: HelpShortcutId::RelationResizeLeft,
                        key: "[",
                        description: "resize selected column narrower",
                    },
                    HelpShortcut {
                        id: HelpShortcutId::RelationResizeRight,
                        key: "]",
                        description: "resize selected column wider",
                    },
                    HelpShortcut {
                        id: HelpShortcutId::RelationResetWidth,
                        key: "=",
                        description: "reset selected column width",
                    },
                    HelpShortcut {
                        id: HelpShortcutId::RelationRefresh,
                        key: "r",
                        description: "refresh relation preview",
                    },
                ]);
            }
            results
        }
    });
    entries
}

pub fn filtered_shortcuts(context: Focus, relation_data: bool, query: &str) -> Vec<HelpShortcut> {
    let tokens = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    shortcuts(context, relation_data)
        .into_iter()
        .filter(|shortcut| {
            let haystack = format!("{} {}", shortcut.key, shortcut.description).to_lowercase();
            tokens.iter().all(|token| haystack.contains(token))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_state_starts_empty() {
        let state = HelpState::new(Focus::Explorer);
        assert_eq!(state.query, "");
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn panel_focus_shortcuts_are_independent_rows() {
        let rows = shortcuts(Focus::Explorer, false);
        let rows = rows
            .iter()
            .filter(|row| row.key.starts_with("Ctrl-w"))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>().len(), 4);
    }

    #[test]
    fn search_matches_all_case_insensitive_tokens() {
        let rows = filtered_shortcuts(Focus::Explorer, false, "CTRL editor");
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![
                HelpShortcutId::FocusEditorFromK,
                HelpShortcutId::FocusEditorFromL,
            ]
        );
    }

    #[test]
    fn editing_resets_selection_and_normalizes_paste() {
        let mut state = HelpState::new(Focus::Editor);
        state.selected = 4;
        state.paste("ctrl\neditor");
        assert_eq!(state.query, "ctrl editor");
        assert_eq!(state.selected, 0);
        state.backspace();
        assert_eq!(state.query, "ctrl edito");
    }
}
