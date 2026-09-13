use std::collections::VecDeque;

use uuid::Uuid;

use crate::{model::execution_target::ExecutionTarget, model::tab::WorkspaceTab};

const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceLocation {
    pub profile_id: Option<Uuid>,
    pub tab_id: Uuid,
    pub target: Option<ExecutionTarget>,
    pub title: String,
}

impl WorkspaceLocation {
    pub fn from_tab(profile_id: Option<Uuid>, tab: &WorkspaceTab) -> Self {
        Self {
            profile_id,
            tab_id: tab.id(),
            target: tab
                .as_console()
                .and_then(|console| console.execution_target.clone()),
            title: tab.title().to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NavigationHistory {
    back: VecDeque<WorkspaceLocation>,
}

impl NavigationHistory {
    pub fn push(&mut self, location: WorkspaceLocation) {
        if self.back.back().is_some_and(|previous| {
            previous.profile_id == location.profile_id && previous.tab_id == location.tab_id
        }) {
            return;
        }
        if self.back.len() == HISTORY_LIMIT {
            self.back.pop_front();
        }
        self.back.push_back(location);
    }

    pub fn pop(&mut self) -> Option<WorkspaceLocation> {
        self.back.pop_back()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &WorkspaceLocation> {
        self.back.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_deduplicates_current_location_and_is_bounded() {
        let mut history = NavigationHistory::default();
        let tab = crate::model::tab::WorkspaceTab::Sql(crate::model::tab::ConsoleTab::new("one"));
        let location = WorkspaceLocation::from_tab(None, &tab);
        history.push(location.clone());
        history.push(location.clone());
        assert_eq!(history.iter().count(), 1);
        for index in 0..HISTORY_LIMIT + 5 {
            let tab = crate::model::tab::WorkspaceTab::Sql(crate::model::tab::ConsoleTab::new(
                format!("console-{index}"),
            ));
            history.push(WorkspaceLocation::from_tab(None, &tab));
        }
        assert_eq!(history.iter().count(), HISTORY_LIMIT);
    }
}
