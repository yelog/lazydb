use clap::ValueEnum;
use nerd_font_symbols::{dev, md};

use crate::{
    db::catalog::{CatalogKind, ObjectGroup},
    profile::DatabaseKind,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum IconMode {
    #[default]
    NerdFont,
    Unicode,
    Ascii,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IconSet {
    mode: IconMode,
}

impl IconSet {
    pub const fn new(mode: IconMode) -> Self {
        Self { mode }
    }

    pub const fn database(self, kind: DatabaseKind) -> &'static str {
        match self.mode {
            IconMode::NerdFont => match kind {
                DatabaseKind::Postgres => dev::DEV_POSTGRESQL,
                DatabaseKind::MySql => dev::DEV_MYSQL,
                DatabaseKind::Sqlite => dev::DEV_SQLITE,
            },
            IconMode::Unicode => match kind {
                DatabaseKind::Postgres => "PG",
                DatabaseKind::MySql => "MY",
                DatabaseKind::Sqlite => "SQ",
            },
            IconMode::Ascii => match kind {
                DatabaseKind::Postgres => "PG",
                DatabaseKind::MySql => "MY",
                DatabaseKind::Sqlite => "SQ",
            },
        }
    }

    pub const fn catalog(self, kind: CatalogKind) -> &'static str {
        match self.mode {
            IconMode::NerdFont => match kind {
                CatalogKind::Database => md::MD_DATABASE,
                CatalogKind::Schema => md::MD_DATABASE_OUTLINE,
                CatalogKind::Table => md::MD_TABLE,
                CatalogKind::View => md::MD_TABLE_EYE,
                CatalogKind::MaterializedView => md::MD_TABLE_SYNC,
                CatalogKind::Column => md::MD_TABLE_COLUMN,
                CatalogKind::Index => md::MD_FORMAT_LIST_NUMBERED,
                CatalogKind::PrimaryKey => md::MD_KEY,
                CatalogKind::UniqueConstraint => md::MD_KEY_STAR,
                CatalogKind::ForeignKey => md::MD_KEY_LINK,
                CatalogKind::CheckConstraint => md::MD_CHECK_DECAGRAM,
                CatalogKind::Function => md::MD_FUNCTION,
                CatalogKind::Procedure => md::MD_CODE_BRACES,
                CatalogKind::Trigger => md::MD_LIGHTNING_BOLT,
                CatalogKind::Sequence => md::MD_ORDER_NUMERIC_ASCENDING,
                CatalogKind::Type => md::MD_SHAPE_OUTLINE,
            },
            IconMode::Unicode => match kind {
                CatalogKind::Database => "◆",
                CatalogKind::Schema => "◇",
                CatalogKind::Table => "▦",
                CatalogKind::View => "◈",
                CatalogKind::MaterializedView => "◉",
                CatalogKind::Column => "│",
                CatalogKind::Index => "#",
                CatalogKind::PrimaryKey => "●",
                CatalogKind::UniqueConstraint => "○",
                CatalogKind::ForeignKey => "↗",
                CatalogKind::CheckConstraint => "✓",
                CatalogKind::Function => "ƒ",
                CatalogKind::Procedure => "λ",
                CatalogKind::Trigger => "!",
                CatalogKind::Sequence => "≡",
                CatalogKind::Type => "τ",
            },
            IconMode::Ascii => match kind {
                CatalogKind::Database => "DB",
                CatalogKind::Schema => "SC",
                CatalogKind::Table => "TB",
                CatalogKind::View => "VW",
                CatalogKind::MaterializedView => "MV",
                CatalogKind::Column => "CL",
                CatalogKind::Index => "IX",
                CatalogKind::PrimaryKey => "PK",
                CatalogKind::UniqueConstraint => "UQ",
                CatalogKind::ForeignKey => "FK",
                CatalogKind::CheckConstraint => "CK",
                CatalogKind::Function => "FN",
                CatalogKind::Procedure => "PR",
                CatalogKind::Trigger => "TG",
                CatalogKind::Sequence => "SQ",
                CatalogKind::Type => "TY",
            },
        }
    }

    pub const fn group(self, _group: ObjectGroup, expanded: bool) -> &'static str {
        match self.mode {
            IconMode::NerdFont => {
                if expanded {
                    md::MD_FOLDER_OPEN
                } else {
                    md::MD_FOLDER
                }
            }
            IconMode::Unicode => {
                if expanded {
                    "▾"
                } else {
                    "▸"
                }
            }
            IconMode::Ascii => {
                if expanded {
                    "[D]"
                } else {
                    "[d]"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATABASE_KINDS: [DatabaseKind; 3] = [
        DatabaseKind::Postgres,
        DatabaseKind::MySql,
        DatabaseKind::Sqlite,
    ];

    const CATALOG_KINDS: [CatalogKind; 16] = [
        CatalogKind::Database,
        CatalogKind::Schema,
        CatalogKind::Table,
        CatalogKind::View,
        CatalogKind::MaterializedView,
        CatalogKind::Column,
        CatalogKind::Index,
        CatalogKind::PrimaryKey,
        CatalogKind::UniqueConstraint,
        CatalogKind::ForeignKey,
        CatalogKind::CheckConstraint,
        CatalogKind::Function,
        CatalogKind::Procedure,
        CatalogKind::Trigger,
        CatalogKind::Sequence,
        CatalogKind::Type,
    ];

    fn is_private_use(character: char) -> bool {
        matches!(
            character as u32,
            0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD
        )
    }

    #[test]
    fn every_mode_has_safe_mappings() {
        for mode in [IconMode::NerdFont, IconMode::Unicode, IconMode::Ascii] {
            let icons = IconSet::new(mode);
            for kind in DATABASE_KINDS {
                let icon = icons.database(kind);
                assert!(!icon.is_empty());
                assert!(icon.chars().all(|character| !character.is_control()));
                if mode == IconMode::Ascii {
                    assert!(icon.is_ascii());
                }
            }
            for kind in CATALOG_KINDS {
                let icon = icons.catalog(kind);
                assert!(!icon.is_empty());
                assert!(icon.chars().all(|character| !character.is_control()));
                if mode == IconMode::Ascii {
                    assert!(icon.is_ascii());
                }
                if mode == IconMode::Unicode {
                    assert!(!icon.chars().any(is_private_use));
                }
            }
        }
    }

    #[test]
    fn nerd_font_uses_database_brands_and_object_icons() {
        let icons = IconSet::default();
        assert_eq!(icons.database(DatabaseKind::Postgres), dev::DEV_POSTGRESQL);
        assert_eq!(icons.database(DatabaseKind::MySql), dev::DEV_MYSQL);
        assert_eq!(icons.database(DatabaseKind::Sqlite), dev::DEV_SQLITE);
        assert_eq!(icons.catalog(CatalogKind::Database), md::MD_DATABASE);
        assert_eq!(icons.catalog(CatalogKind::Schema), md::MD_DATABASE_OUTLINE);
        assert_eq!(icons.catalog(CatalogKind::Table), md::MD_TABLE);
        assert_eq!(icons.catalog(CatalogKind::Column), md::MD_TABLE_COLUMN);
        assert_eq!(icons.catalog(CatalogKind::PrimaryKey), md::MD_KEY);
        assert_eq!(icons.catalog(CatalogKind::Function), md::MD_FUNCTION);
        assert_eq!(icons.group(ObjectGroup::Tables, false), md::MD_FOLDER);
        assert_eq!(icons.group(ObjectGroup::Tables, true), md::MD_FOLDER_OPEN);
    }
}
