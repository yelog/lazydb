use std::collections::HashSet;

use uuid::Uuid;

use crate::db::{
    ServerInfo,
    catalog::{CatalogId, CatalogKind, CatalogNode},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Focus {
    Explorer,
    #[default]
    Editor,
    Results,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Self::Explorer => Self::Editor,
            Self::Editor => Self::Results,
            Self::Results => Self::Explorer,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Explorer => Self::Results,
            Self::Editor => Self::Explorer,
            Self::Results => Self::Editor,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Overlay {
    Help(Focus),
    ProfilePicker,
    Message { title: String, body: String },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QueryStatus {
    #[default]
    Idle,
    Running,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionState {
    pub profile_id: Option<Uuid>,
    pub generation: u64,
    pub status: ConnectionStatus,
    pub server: Option<ServerInfo>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleCatalogNode {
    pub node_index: usize,
    pub depth: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ExplorerState {
    pub nodes: Vec<CatalogNode>,
    pub expanded: HashSet<CatalogId>,
    pub selected: usize,
    pub scroll: usize,
}

impl ExplorerState {
    pub fn set_nodes(&mut self, nodes: Vec<CatalogNode>) {
        self.nodes = nodes;
        self.expanded.clear();
        for node in &self.nodes {
            if matches!(node.kind, CatalogKind::Database | CatalogKind::Schema) {
                self.expanded.insert(node.id.clone());
            }
        }
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn visible(&self) -> Vec<VisibleCatalogNode> {
        let mut visible = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if node.parent_id.is_none() {
                self.append_visible(index, 0, &mut visible);
            }
        }
        visible
    }

    pub fn selected_node(&self) -> Option<&CatalogNode> {
        let visible = self.visible();
        visible
            .get(self.selected)
            .and_then(|visible| self.nodes.get(visible.node_index))
    }

    pub fn move_selection(&mut self, delta: isize) {
        let count = self.visible().len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
    }

    pub fn toggle_selected(&mut self) {
        let Some(id) = self.selected_node().map(|node| node.id.clone()) else {
            return;
        };
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
    }

    fn append_visible(
        &self,
        node_index: usize,
        depth: usize,
        visible: &mut Vec<VisibleCatalogNode>,
    ) {
        visible.push(VisibleCatalogNode { node_index, depth });
        let node = &self.nodes[node_index];
        if !self.expanded.contains(&node.id) {
            return;
        }
        for (child_index, child) in self.nodes.iter().enumerate() {
            if child.parent_id.as_ref() == Some(&node.id) {
                self.append_visible(child_index, depth + 1, visible);
            }
        }
    }
}
