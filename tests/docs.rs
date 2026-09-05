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
        "## Catalog Editor",
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
        "### Page Size Selector",
        "### Catalog Drop Confirmation",
        "## Mouse",
    ] {
        assert!(keys.contains(heading), "missing {heading}");
    }

    assert!(keys.contains("750 ms"));
    assert!(keys.contains("EditorLeader"));
    assert!(keys.contains("RelationDataBusy"));
    assert!(keys.contains("yy"));
    assert!(keys.contains("Output o"));
    assert!(keys.contains("0` / `^"));
    assert!(keys.contains("Printable `y` and `Y` are text input"));
    assert!(keys.contains("capability-aware"));
    assert!(keys.contains("Ctrl-Shift-s"));
    assert!(keys.contains("left-button drag selects text"));
    assert!(keys.contains("Copy all"));
    assert!(keys.contains("Ctrl-c`\nremains the quit key"));

    let configuration = include_str!("../docs/configuration.md");
    for term in [
        "backend = \"osc52\"",
        "max_bytes",
        "SSH and tmux",
        "drag only creates a text selection",
        "default `Ctrl-c` quit binding is unchanged",
    ] {
        assert!(
            configuration.contains(term),
            "missing configuration term {term}"
        );
    }

    assert!(readme.contains("Ctrl-Shift-s"));
    assert!(readme.contains("drag alone\n  never writes to the clipboard"));

    let architecture = include_str!("../docs/architecture.md");
    for term in [
        "cross-database",
        "stale checks",
        "Materialized View",
        "Role node",
        "targeted refresh",
    ] {
        assert!(
            architecture.contains(term),
            "missing architecture term {term}"
        );
    }
}
