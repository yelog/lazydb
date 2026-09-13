use lazydb::commands::{
    CommandAvailability, CommandContext, CommandId, UserIntent, availability, command_for_help,
    intent_for_command, matching_commands,
};
use lazydb::help::HelpShortcutId;
use uuid::Uuid;

#[test]
fn help_aliases_share_one_semantic_id() {
    assert_eq!(
        command_for_help(HelpShortcutId::RunSql),
        command_for_help(HelpShortcutId::EditorRun)
    );
    assert_eq!(
        matching_commands("execute statement")
            .into_iter()
            .map(|spec| spec.id)
            .collect::<Vec<_>>(),
        vec![CommandId::RunStatement]
    );
}

#[test]
fn semantic_intent_targets_the_origin_tab() {
    let tab_id = Uuid::from_u128(42);
    let context = CommandContext {
        tab_id: Some(tab_id),
        ..CommandContext::default()
    };

    assert_eq!(
        intent_for_command(CommandId::CloseTab, &context),
        Some(UserIntent::CloseTab { tab_id })
    );
}

#[test]
fn editor_command_without_a_console_has_an_explanation() {
    assert_eq!(
        availability(CommandId::RunStatement, &CommandContext::default()),
        CommandAvailability::Disabled("No SQL console is available")
    );
}
