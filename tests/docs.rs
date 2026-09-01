#[test]
fn keyboard_reference_is_dedicated_and_complete() {
    let readme = include_str!("../README.md");
    let keys = include_str!("../docs/keybindings.md");

    assert!(!readme.contains("## Essential Keys"));
    assert!(!readme.contains("| Action | Key |"));
    assert!(readme.contains("docs/keybindings.md"));
    assert!(readme.contains("contextual controls"));

    for heading in [
        "## Conventions",
        "## Global",
        "## Pane Navigation and Resize",
        "## Prefixes",
        "## Explorer",
        "## SQL Editor",
        "### Normal",
        "### Insert and Replace",
        "### Visual",
        "## SQL Results Data",
        "## SQL Output and Plan",
        "## Relation Data",
        "### Browse",
        "### EditCell",
        "### Visual Line",
        "### Busy",
        "## Relation DDL",
        "## Record View",
        "## Data Query Inputs and Completion",
        "## Profile Manager",
        "### Form",
        "### Scope",
        "### Delete and Loading",
        "## SQL Editor List",
        "## Help Search",
        "## Profile Access",
        "## Message",
        "## Confirmations and Selectors",
        "### Substitute Confirmation",
        "### Execution Confirmation",
        "### Manual Cancellation",
        "### Transaction Exit",
        "### Clear Transaction Outcome",
        "### Target Selector",
        "### Delete SQL Editor",
        "## Mouse",
    ] {
        assert!(keys.contains(heading), "missing {heading}");
    }

    assert!(keys.contains("750 ms"));
    assert!(keys.contains("EditorLeader"));
    assert!(keys.contains("RelationDataBusy"));
    assert!(keys.contains("yy"));
    assert!(keys.contains("Output o"));
}
