use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Database,
    Schema,
    Table,
    View,
    Column,
    Index,
    PrimaryKey,
    UniqueConstraint,
    ForeignKey,
    CheckConstraint,
    Function,
    Procedure,
    Trigger,
    Sequence,
    Type,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct CatalogId {
    pub connection_id: Uuid,
    pub kind: CatalogKind,
    pub native_path: Vec<String>,
}

impl CatalogId {
    pub fn new<I, S>(connection_id: Uuid, kind: CatalogKind, native_path: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            connection_id,
            kind,
            native_path: native_path.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogNode {
    pub id: CatalogId,
    pub parent_id: Option<CatalogId>,
    pub kind: CatalogKind,
    pub name: String,
    pub native_kind: String,
    pub detail: Option<String>,
    pub expandable: bool,
}

impl CatalogNode {
    pub fn new(
        id: CatalogId,
        parent_id: Option<CatalogId>,
        name: impl Into<String>,
        native_kind: impl Into<String>,
        detail: Option<String>,
        expandable: bool,
    ) -> Self {
        Self {
            kind: id.kind,
            id,
            parent_id,
            name: name.into(),
            native_kind: native_kind.into(),
            detail,
            expandable,
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{CatalogId, CatalogKind};

    #[test]
    fn object_identity_includes_connection_kind_and_native_path() {
        let connection_id = Uuid::new_v4();
        let id = CatalogId::new(
            connection_id,
            CatalogKind::Table,
            ["app", "public", "users"],
        );

        assert_eq!(id.connection_id, connection_id);
        assert_eq!(id.kind, CatalogKind::Table);
        assert_eq!(id.native_path, vec!["app", "public", "users"]);
    }
}
