use std::collections::HashSet;

use uuid::Uuid;

use crate::{
    commands::{CommandContext, CommandId, UserIntent},
    db::catalog::CatalogId,
    model::{execution_target::ExecutionTarget, text_input::TextInput},
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum OmniItemId {
    Command(CommandId),
    UnboundConsole,
    Profile(Uuid),
    Console {
        profile_id: Option<Uuid>,
        console_id: Uuid,
    },
    Tab(Uuid),
    Catalog(CatalogId),
    SuspendedSession(Uuid),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OmniItemKind {
    Command,
    Connection(crate::profile::DatabaseKind),
    Console,
    Catalog(crate::db::catalog::CatalogKind),
    Recent,
    Resume,
    Action,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OmniItemAction {
    Command(CommandId),
    OpenProfile(Uuid),
    CreateUnboundConsole,
    OpenConsole {
        profile_id: Option<Uuid>,
        console_id: Uuid,
    },
    OpenTab(Uuid),
    OpenRelation {
        id: CatalogId,
        view: crate::model::relation::RelationView,
    },
    ShowRelationActions(CatalogId),
    ResumeSession(Uuid),
    ResumeInteraction(Uuid),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OmniLocation {
    pub profile_id: Uuid,
    pub database: Option<String>,
    pub schema: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OmniItem {
    pub id: OmniItemId,
    pub kind: OmniItemKind,
    pub title: String,
    pub subtitle: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub action: OmniItemAction,
    pub context: CommandContext,
    pub availability: crate::commands::CommandAvailability,
    pub opened: bool,
    pub location: Option<OmniLocation>,
}

impl OmniItem {
    pub fn new(
        id: OmniItemId,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        category: impl Into<String>,
        action: OmniItemAction,
    ) -> Self {
        let kind = OmniItemKind::from_id(&id);
        Self {
            id,
            kind,
            title: title.into(),
            subtitle: subtitle.into(),
            category: category.into(),
            keywords: Vec::new(),
            action,
            context: CommandContext::default(),
            availability: crate::commands::CommandAvailability::Ready,
            opened: false,
            location: None,
        }
    }

    fn searchable_fields(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str())
            .chain(std::iter::once(self.subtitle.as_str()))
            .chain(std::iter::once(self.category.as_str()))
            .chain(self.keywords.iter().map(String::as_str))
    }
}

impl OmniItemKind {
    fn from_id(id: &OmniItemId) -> Self {
        match id {
            OmniItemId::Command(_) => Self::Command,
            OmniItemId::UnboundConsole => Self::Action,
            OmniItemId::Profile(_) => Self::Connection(crate::profile::DatabaseKind::Sqlite),
            OmniItemId::Console { .. } => Self::Console,
            OmniItemId::Catalog(_) => Self::Catalog(crate::db::catalog::CatalogKind::Table),
            OmniItemId::Tab(_) => Self::Recent,
            OmniItemId::SuspendedSession(_) => Self::Resume,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OmniFilter {
    #[default]
    All,
    Commands,
    Profiles,
    Consoles,
    Catalog,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OmniStep {
    Root,
    ObjectActions(OmniItemId),
    PickConnection,
    PickTarget {
        profile_id: Uuid,
    },
    NameConsole {
        profile_id: Option<Uuid>,
        target: Option<ExecutionTarget>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OmniStepState {
    step: OmniStep,
    query: TextInput,
    selected: Option<OmniItemId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OmniState {
    pub session_id: u64,
    pub query_generation: u64,
    pub query: TextInput,
    pub filter: OmniFilter,
    pub profile_scope: Option<Uuid>,
    pub step: OmniStep,
    pub selected: Option<OmniItemId>,
    pub origin: CommandContext,
    pub data_origin: CommandContext,
    pub origin_overlay: Option<crate::model::workspace::Overlay>,
    pub origin_tab_id: Option<Uuid>,
    pub origin_profile_id: Option<Uuid>,
    pub suspended_sessions: Vec<Uuid>,
    pub items: Vec<OmniItem>,
    pub scroll: usize,
    pub viewport_rows: usize,
    pub status: Option<String>,
    history: Vec<OmniStepState>,
}

impl OmniState {
    pub fn new(session_id: u64, origin: CommandContext) -> Self {
        Self {
            session_id,
            query_generation: 0,
            query: TextInput::default(),
            filter: OmniFilter::All,
            profile_scope: None,
            step: OmniStep::Root,
            selected: None,
            data_origin: origin.clone(),
            origin,
            origin_overlay: None,
            origin_tab_id: None,
            origin_profile_id: None,
            suspended_sessions: Vec::new(),
            items: Vec::new(),
            scroll: 0,
            viewport_rows: 0,
            status: None,
            history: Vec::new(),
        }
    }

    pub fn query(&self) -> &str {
        self.query.value()
    }

    pub fn edit(&mut self, edit: crate::model::text_input::TextInputEdit) {
        if self.query.apply(edit) {
            self.query_changed();
        }
    }

    pub fn paste(&mut self, value: &str) {
        let value = value
            .chars()
            .map(|character| match character {
                '\r' | '\n' | '\t' => ' ',
                other => other,
            })
            .collect::<String>();
        self.query.paste(value);
        self.query_changed();
    }

    pub fn query_changed(&mut self) {
        self.query_generation = self.query_generation.saturating_add(1);
        self.scroll = 0;
        self.status = None;
        let (filter, scope, _) = parse_query(self.query.value());
        self.filter = filter;
        if let Some(profile_id) = scope {
            self.profile_scope = Some(profile_id);
        }
        self.reconcile_selection(false);
    }

    pub fn parsed_query(&self) -> String {
        parse_query(self.query.value()).2
    }

    pub fn set_items(&mut self, items: Vec<OmniItem>) {
        let mut seen = HashSet::with_capacity(items.len());
        self.items = items
            .into_iter()
            .filter(|item| seen.insert(item.id.clone()))
            .collect();
        self.reconcile_selection(true);
    }

    pub fn visible_items(&self) -> Vec<&OmniItem> {
        let query = self.parsed_query();
        let tokens = query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let mut matches = self
            .items
            .iter()
            .filter(|item| self.filter.includes(item))
            .filter(|item| {
                self.profile_scope.is_none_or(|profile_id| {
                    item.context.profile_id == Some(profile_id)
                        || matches!(item.id, OmniItemId::Profile(id) if id == profile_id)
                })
            })
            .filter_map(|item| {
                let fields = item
                    .searchable_fields()
                    .map(str::to_lowercase)
                    .collect::<Vec<_>>();
                let matched = tokens
                    .iter()
                    .all(|token| fields.iter().any(|field| field.contains(token)));
                matched.then(|| {
                    let title = item.title.to_lowercase();
                    let score = if query.is_empty() {
                        0
                    } else if title == query.to_lowercase() {
                        4
                    } else if title.starts_with(&query.to_lowercase()) {
                        3
                    } else if title.contains(&query.to_lowercase()) {
                        2
                    } else {
                        1
                    };
                    (score, item)
                })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
                .then_with(|| left.subtitle.cmp(&right.subtitle))
                .then_with(|| format!("{:?}", left.id).cmp(&format!("{:?}", right.id)))
        });
        matches.into_iter().map(|(_, item)| item).collect()
    }

    pub fn move_selection(&mut self, delta: isize) {
        let (next_id, count) = {
            let visible = self.visible_items();
            if visible.is_empty() {
                self.selected = None;
                return;
            }
            let current = visible
                .iter()
                .position(|item| Some(&item.id) == self.selected.as_ref())
                .unwrap_or(0);
            let next = (current as isize + delta).rem_euclid(visible.len() as isize) as usize;
            (visible[next].id.clone(), visible.len())
        };
        self.selected = Some(next_id);
        self.ensure_selected_visible(count);
        self.query.finish_edit_group();
    }

    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows;
        self.normalize_scroll();
        if let Some(index) = self.selected_index() {
            self.ensure_selected_visible(index + 1);
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta);
        self.normalize_scroll();
        if let Some(index) = self.selected_index()
            && self.viewport_rows > 0
        {
            self.selected = Some(
                self.visible_items()[index.clamp(
                    self.scroll,
                    (self.scroll + self.viewport_rows - 1).min(self.visible_items().len() - 1),
                )]
                .id
                .clone(),
            );
        }
    }

    pub fn set_scroll(&mut self, offset: usize) {
        self.scroll = offset;
        self.normalize_scroll();
        if let Some(index) = self.selected_index()
            && self.viewport_rows > 0
        {
            let visible = self.visible_items();
            let clamped = index.clamp(
                self.scroll,
                (self.scroll + self.viewport_rows - 1).min(visible.len() - 1),
            );
            self.selected = Some(visible[clamped].id.clone());
        }
    }

    fn selected_index(&self) -> Option<usize> {
        let visible = self.visible_items();
        self.selected
            .as_ref()
            .and_then(|id| visible.iter().position(|item| &item.id == id))
    }

    fn ensure_selected_visible(&mut self, _count: usize) {
        let Some(index) = self.selected_index() else {
            return;
        };
        if self.viewport_rows == 0 {
            return;
        }
        if index < self.scroll {
            self.scroll = index;
        }
        if index >= self.scroll + self.viewport_rows {
            self.scroll = index - self.viewport_rows + 1;
        }
        self.normalize_scroll();
    }

    fn normalize_scroll(&mut self) {
        let count = self.visible_items().len();
        if self.viewport_rows > 0 {
            self.scroll = self.scroll.min(count.saturating_sub(self.viewport_rows));
        }
    }

    pub fn selected_item(&self) -> Option<&OmniItem> {
        self.visible_items()
            .into_iter()
            .find(|item| Some(&item.id) == self.selected.as_ref())
    }

    pub fn push_step(&mut self, step: OmniStep) {
        self.history.push(OmniStepState {
            step: self.step.clone(),
            query: self.query.clone(),
            selected: self.selected.clone(),
        });
        self.step = step;
        self.query = TextInput::default();
        self.items.clear();
        self.selected = None;
        self.scroll = 0;
        self.query_changed();
    }

    pub fn pop_step(&mut self) -> bool {
        let Some(previous) = self.history.pop() else {
            return false;
        };
        self.step = previous.step;
        self.query = previous.query;
        self.selected = previous.selected;
        self.items.clear();
        self.scroll = 0;
        self.query_generation = self.query_generation.saturating_add(1);
        true
    }

    pub fn root_intent(&self) -> Option<UserIntent> {
        let item = self.selected_item()?;
        match &item.action {
            OmniItemAction::Command(id) => crate::commands::intent_for_command(*id, &item.context),
            OmniItemAction::OpenProfile(_) => None,
            OmniItemAction::CreateUnboundConsole => None,
            OmniItemAction::OpenConsole {
                profile_id,
                console_id,
            } => Some(UserIntent::OpenConsole {
                profile_id: *profile_id,
                console_id: *console_id,
            }),
            OmniItemAction::OpenTab(id) => Some(UserIntent::OpenTab { tab_id: *id }),
            OmniItemAction::OpenRelation { id, view } => Some(UserIntent::OpenRelation {
                catalog_id: id.clone(),
                view: *view,
            }),
            OmniItemAction::ShowRelationActions(_)
            | OmniItemAction::ResumeSession(_)
            | OmniItemAction::ResumeInteraction(_) => None,
        }
    }

    pub fn command_intent(&self) -> Option<(CommandId, CommandContext)> {
        let item = self.selected_item()?;
        let OmniItemAction::Command(id) = item.action else {
            return None;
        };
        Some((id, item.context.clone()))
    }

    fn reconcile_selection(&mut self, preserve_missing: bool) {
        let visible = self.visible_items();
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| visible.iter().any(|item| &item.id == selected))
        {
            return;
        }
        if preserve_missing && self.selected.is_some() {
            self.selected = None;
        } else {
            self.selected = visible.first().map(|item| item.id.clone());
        }
    }
}

impl OmniFilter {
    fn includes(self, item: &OmniItem) -> bool {
        match self {
            Self::All => true,
            Self::Commands => matches!(item.id, OmniItemId::Command(_)),
            Self::Profiles => matches!(item.id, OmniItemId::Profile(_)),
            Self::Consoles => matches!(item.id, OmniItemId::Console { .. }),
            Self::Catalog => matches!(item.id, OmniItemId::Catalog(_)),
        }
    }
}

fn parse_query(query: &str) -> (OmniFilter, Option<Uuid>, String) {
    let trimmed = query.trim_start();
    if let Some(rest) = trimmed.strip_prefix('>') {
        return (OmniFilter::Commands, None, rest.trim_start().to_owned());
    }
    if let Some(rest) = trimmed.strip_prefix('@') {
        return (OmniFilter::Profiles, None, rest.trim_start().to_owned());
    }
    (OmniFilter::All, None, query.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::catalog::CatalogKind;
    use crate::model::relation::RelationView;

    fn command_item(id: CommandId, title: &str) -> OmniItem {
        let mut item = OmniItem::new(
            OmniItemId::Command(id),
            title,
            "",
            "Command",
            OmniItemAction::Command(id),
        );
        item.context = CommandContext::default();
        item
    }

    #[test]
    fn command_prefix_filters_and_removes_the_prefix_from_search_text() {
        let mut state = OmniState::new(1, CommandContext::default());
        state.query.set("> format");
        state.query_changed();
        state.set_items(vec![
            command_item(CommandId::FormatSql, "Format SQL"),
            command_item(CommandId::NewConsole, "New Console"),
        ]);

        assert_eq!(state.filter, OmniFilter::Commands);
        assert_eq!(state.parsed_query(), "format");
        assert_eq!(state.query(), "> format");
        assert_eq!(state.visible_items().len(), 1);
        assert_eq!(
            state.selected,
            Some(OmniItemId::Command(CommandId::FormatSql))
        );
    }

    #[test]
    fn token_search_matches_across_fields_and_unicode_safely() {
        let mut state = OmniState::new(2, CommandContext::default());
        let mut item = OmniItem::new(
            OmniItemId::Command(CommandId::OpenRelation),
            "用户表",
            "prod / app / public",
            "Table",
            OmniItemAction::Command(CommandId::OpenRelation),
        );
        item.keywords.push("users".into());
        state.query.set("public 用户");
        state.set_items(vec![item]);

        assert_eq!(state.visible_items().len(), 1);
    }

    #[test]
    fn result_refresh_keeps_a_stable_selection_and_drops_missing_selection() {
        let mut state = OmniState::new(3, CommandContext::default());
        let alpha = command_item(CommandId::OpenDashboard, "Alpha");
        let beta = command_item(CommandId::NewConsole, "Beta");
        state.set_items(vec![alpha.clone(), beta.clone()]);
        state.selected = Some(beta.id.clone());
        state.set_items(vec![beta.clone(), alpha]);
        assert_eq!(state.selected, Some(beta.id.clone()));

        state.set_items(vec![command_item(CommandId::OpenDashboard, "Other")]);
        assert_eq!(state.selected, None);
    }

    #[test]
    fn set_items_keeps_catalog_ids_unique() {
        let mut state = OmniState::new(5, CommandContext::default());
        let id = OmniItemId::Catalog(CatalogId::new(
            Uuid::nil(),
            CatalogKind::Table,
            ["db", "public", "users"],
        ));
        let first = OmniItem::new(
            id.clone(),
            "users",
            "first",
            "Table",
            OmniItemAction::OpenRelation {
                id: match &id {
                    OmniItemId::Catalog(id) => id.clone(),
                    _ => unreachable!(),
                },
                view: RelationView::Data,
            },
        );
        let second = OmniItem::new(
            id,
            "users",
            "second",
            "Table",
            OmniItemAction::Command(CommandId::OpenRelation),
        );

        state.set_items(vec![first, second]);

        assert_eq!(state.items.len(), 1);
        assert_eq!(state.visible_items().len(), 1);
    }

    #[test]
    fn step_back_restores_query_and_selection() {
        let mut state = OmniState::new(4, CommandContext::default());
        state.query.set("users");
        state.push_step(OmniStep::PickConnection);
        state.query.set("prod");
        assert!(state.pop_step());

        assert_eq!(state.step, OmniStep::Root);
        assert_eq!(state.query(), "users");
    }
}
