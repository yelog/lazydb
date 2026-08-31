use crate::model::workspace::Focus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpShortcutId {
    Help,
    FocusExplorer,
    FocusResults,
    FocusEditorFromK,
    FocusEditorFromL,
    ResizeHeightIncrease,
    ResizeHeightDecrease,
    ResizeWidthIncrease,
    ResizeWidthDecrease,
    ResetPaneSizes,
    PreviousTab,
    NextTab,
    NewConsole,
    GotoSqlConsole,
    CloseTab,
    DeleteConsole,
    OpenSqlEditors,
    ExplorerMoveDown,
    ExplorerMoveUp,
    ExplorerFirst,
    ExplorerLast,
    ExplorerViewTop,
    ExplorerViewMiddle,
    ExplorerViewBottom,
    ExplorerHalfPageDown,
    ExplorerHalfPageUp,
    ExplorerPageDown,
    ExplorerPageUp,
    ExplorerAlignMiddle,
    ExplorerAlignTop,
    ExplorerAlignBottom,
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
    ExplorerAccess,
    EditorInsert,
    EditorNormal,
    EditorUndo,
    EditorRedo,
    EditorRun,
    EditorFormat,
    EditorCopyStatement,
    EditorCopyBuffer,
    ToggleTransaction,
    CommitTransaction,
    RollbackTransaction,
    OpenTargetSelector,
    ResultsMoveLeft,
    ResultsMoveDown,
    ResultsMoveUp,
    ResultsMoveRight,
    ResultsFirstRow,
    ResultsLastRow,
    ResultsViewTop,
    ResultsViewMiddle,
    ResultsViewBottom,
    ResultsHalfPageDown,
    ResultsHalfPageUp,
    ResultsPageDown,
    ResultsPageUp,
    ResultsAlignMiddle,
    ResultsAlignTop,
    ResultsAlignBottom,
    ResultsOpenRecordView,
    ResultsCopyCell,
    ResultsCopyRow,
    ResultsCopyRowWithHeaders,
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
            id: HelpShortcutId::Help,
            key: "? (also F1)",
            description: "open this help panel",
        },
        HelpShortcut {
            id: HelpShortcutId::FocusExplorer,
            key: "Ctrl-w h",
            description: "move focus left",
        },
        HelpShortcut {
            id: HelpShortcutId::FocusResults,
            key: "Ctrl-w j",
            description: "move focus down",
        },
        HelpShortcut {
            id: HelpShortcutId::FocusEditorFromK,
            key: "Ctrl-w k",
            description: "move focus up to Editor",
        },
        HelpShortcut {
            id: HelpShortcutId::FocusEditorFromL,
            key: "Ctrl-w l",
            description: "move focus right to Editor",
        },
        HelpShortcut {
            id: HelpShortcutId::PreviousTab,
            key: "gT (also [ then t)",
            description: "previous tab",
        },
        HelpShortcut {
            id: HelpShortcutId::NextTab,
            key: "gt (also ] then t)",
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
        HelpShortcut {
            id: HelpShortcutId::CloseTab,
            key: "Space q",
            description: "close current tab",
        },
        HelpShortcut {
            id: HelpShortcutId::DeleteConsole,
            key: "Space x",
            description: "permanently delete SQL editor",
        },
        HelpShortcut {
            id: HelpShortcutId::OpenSqlEditors,
            key: "Space e",
            description: "search SQL editors",
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
                id: HelpShortcutId::ExplorerFirst,
                key: "gg",
                description: "select first node",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerLast,
                key: "G",
                description: "select last node",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerViewTop,
                key: "H",
                description: "select top visible node",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerViewMiddle,
                key: "M",
                description: "select middle visible node",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerViewBottom,
                key: "L",
                description: "select bottom visible node",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerHalfPageDown,
                key: "Ctrl-d",
                description: "move down half page",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerHalfPageUp,
                key: "Ctrl-u",
                description: "move up half page",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerPageDown,
                key: "Ctrl-f",
                description: "move down one page",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerPageUp,
                key: "Ctrl-b",
                description: "move up one page",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerAlignMiddle,
                key: "zz",
                description: "align selection middle",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerAlignTop,
                key: "zt",
                description: "align selection top",
            },
            HelpShortcut {
                id: HelpShortcutId::ExplorerAlignBottom,
                key: "zb",
                description: "align selection bottom",
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
            HelpShortcut {
                id: HelpShortcutId::ExplorerAccess,
                key: "s",
                description: "connection access",
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
                id: HelpShortcutId::EditorFormat,
                key: "Space f",
                description: "format selected / current SQL",
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
            HelpShortcut {
                id: HelpShortcutId::EditorCopyStatement,
                key: "Space y",
                description: "copy current SQL statement",
            },
            HelpShortcut {
                id: HelpShortcutId::EditorCopyBuffer,
                key: "Space Y",
                description: "copy complete SQL buffer",
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
                    id: HelpShortcutId::ResultsFirstRow,
                    key: "gg",
                    description: "select first row",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsLastRow,
                    key: "G",
                    description: "select last row",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsViewTop,
                    key: "H",
                    description: "select top visible row",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsViewMiddle,
                    key: "M",
                    description: "select middle visible row",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsViewBottom,
                    key: "L",
                    description: "select bottom visible row",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsHalfPageDown,
                    key: "Ctrl-d",
                    description: "move down half a page",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsHalfPageUp,
                    key: "Ctrl-u",
                    description: "move up half a page",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsPageDown,
                    key: "Ctrl-f",
                    description: "move down one page",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsPageUp,
                    key: "Ctrl-b",
                    description: "move up one page",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsAlignMiddle,
                    key: "zz",
                    description: "align selected row to middle",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsAlignTop,
                    key: "zt",
                    description: "align selected row to top",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsAlignBottom,
                    key: "zb",
                    description: "align selected row to bottom",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsToggleView,
                    key: "o",
                    description: "switch Data / Output",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsOpenRecordView,
                    key: "v",
                    description: "open Record View",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsCopyCell,
                    key: "y",
                    description: "copy selected cell",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsCopyRow,
                    key: "Y",
                    description: "copy selected row as TSV",
                },
                HelpShortcut {
                    id: HelpShortcutId::ResultsCopyRowWithHeaders,
                    key: "Space Y",
                    description: "copy row with headers",
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
    entries.extend([
        HelpShortcut {
            id: HelpShortcutId::ResizeHeightIncrease,
            key: "Ctrl-w +",
            description: "increase focused pane height",
        },
        HelpShortcut {
            id: HelpShortcutId::ResizeHeightDecrease,
            key: "Ctrl-w -",
            description: "decrease focused pane height",
        },
        HelpShortcut {
            id: HelpShortcutId::ResizeWidthIncrease,
            key: "Ctrl-w >",
            description: "increase focused pane width",
        },
        HelpShortcut {
            id: HelpShortcutId::ResizeWidthDecrease,
            key: "Ctrl-w <",
            description: "decrease focused pane width",
        },
        HelpShortcut {
            id: HelpShortcutId::ResetPaneSizes,
            key: "Ctrl-w =",
            description: "restore default pane sizes",
        },
    ]);
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
    fn every_context_documents_the_help_shortcut() {
        for context in [Focus::Explorer, Focus::Editor, Focus::Results] {
            let help = shortcuts(context, false)
                .into_iter()
                .find(|shortcut| shortcut.id == HelpShortcutId::Help)
                .expect("help shortcut");
            assert_eq!(help.key, "? (also F1)");
            assert_eq!(help.description, "open this help panel");
        }
    }

    #[test]
    fn panel_focus_shortcuts_are_independent_rows() {
        let rows = shortcuts(Focus::Explorer, false);
        let rows = rows
            .iter()
            .filter(|row| row.key.starts_with("Ctrl-w"))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 9);
        assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>().len(), 9);
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

    #[test]
    fn editor_help_includes_sql_formatting() {
        let format = shortcuts(Focus::Editor, false)
            .into_iter()
            .find(|shortcut| shortcut.id == HelpShortcutId::EditorFormat)
            .expect("format shortcut");
        assert_eq!(format.key, "Space f");
        assert!(format.description.contains("format"));
    }

    #[test]
    fn results_help_includes_vim_viewport_navigation() {
        let rows = shortcuts(Focus::Results, false);
        let keys = rows.iter().map(|row| row.key).collect::<Vec<_>>();

        for key in [
            "gg", "G", "H", "M", "L", "Ctrl-d", "Ctrl-u", "Ctrl-f", "Ctrl-b", "zz", "zt", "zb",
        ] {
            assert!(keys.contains(&key), "missing Results help key {key}");
        }
    }
}
