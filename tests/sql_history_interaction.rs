use lazydb::{
    action::{Action, Command},
    app::App,
    clipboard::ClipboardPayload,
    model::{history_tab::HistoryTab, sql_history::*, tab::WorkspaceTab},
};
use uuid::Uuid;

fn app_with_history(sql: &str) -> App {
    let mut app = App::new(Vec::new());
    let id = Uuid::new_v4();
    app.tabs.push(WorkspaceTab::History(HistoryTab {
        selected_execution: Some(id),
        items: vec![ExecutionHistory {
            execution_id: id,
            operation_id: Uuid::new_v4(),
            transaction_id: None,
            sql: sql.into(),
            status: HistoryExecutionStatus::Succeeded,
            certainty: HistoryResultCertainty::Confirmed,
            transaction_outcome: HistoryTransactionOutcome::AutoCommitted,
            affected_rows: Some(2),
            returned_rows: None,
            requested_at: 0,
            elapsed_millis: Some(4),
        }],
        ..HistoryTab::default()
    }));
    app.active_tab = app.tabs.len() - 1;
    app
}

#[test]
fn history_copy_action_uses_complete_sql_not_a_preview() {
    let sql = "SELECT * FROM users\nWHERE display_name = '中文'";
    let mut app = app_with_history(sql);

    let commands = app.update(Action::SqlHistoryCopy);

    assert!(matches!(
        commands.as_slice(),
        [Command::WriteClipboard(ClipboardPayload { text, .. })] if text == sql
    ));
}

#[test]
fn history_detail_action_opens_existing_complete_text_detail() {
    let sql = "SELECT * FROM users\nWHERE id = 1";
    let mut app = app_with_history(sql);

    app.update(Action::SqlHistoryOpenDetail);

    let lazydb::model::workspace::Overlay::TextDetail(detail) = app.overlay.unwrap() else {
        panic!("expected SQL detail overlay");
    };
    assert_eq!(detail.copy_text, sql);
}
