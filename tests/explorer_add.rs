use lazydb::model::explorer_add::{
    ExplorerAddAvailability, ExplorerAddKind, ExplorerAddMenu, ExplorerAddOption,
};
use uuid::Uuid;

fn option(kind: ExplorerAddKind, enabled: bool) -> ExplorerAddOption {
    ExplorerAddOption {
        kind,
        availability: if enabled {
            ExplorerAddAvailability::Available
        } else {
            ExplorerAddAvailability::Unavailable("connect first")
        },
    }
}

#[test]
fn add_menu_starts_on_the_first_enabled_option() {
    let menu = ExplorerAddMenu::new(
        Uuid::from_u128(1),
        vec![
            option(ExplorerAddKind::Connection, true),
            option(ExplorerAddKind::Database, false),
        ],
    );
    assert_eq!(menu.selected, 0);
    assert_eq!(menu.selected_kind(), Some(ExplorerAddKind::Connection));
}

#[test]
fn movement_skips_disabled_options_and_clamps() {
    let mut menu = ExplorerAddMenu::new(
        Uuid::from_u128(1),
        vec![
            option(ExplorerAddKind::Connection, true),
            option(ExplorerAddKind::Database, false),
            option(ExplorerAddKind::Role, true),
        ],
    );
    assert!(menu.move_selection(1));
    assert_eq!(menu.selected, 2);
    assert!(!menu.move_selection(1));
    assert_eq!(menu.selected, 2);
    assert!(menu.move_selection(-1));
    assert_eq!(menu.selected, 0);
}

#[test]
fn direct_selection_rejects_disabled_options() {
    let mut menu = ExplorerAddMenu::new(
        Uuid::from_u128(1),
        vec![
            option(ExplorerAddKind::Connection, true),
            option(ExplorerAddKind::Database, false),
        ],
    );
    assert!(!menu.select(1));
    assert_eq!(menu.selected, 0);
}
