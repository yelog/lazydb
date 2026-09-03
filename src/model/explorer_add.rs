use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerAddKind {
    Connection,
    ConnectionGroup,
    Database,
    User,
    Role,
}

impl ExplorerAddKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::ConnectionGroup => "Connection Group",
            Self::Database => "Database",
            Self::User => "User",
            Self::Role => "Role",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Connection => "New server profile",
            Self::ConnectionGroup => "Organize connections",
            Self::Database => "Create a database",
            Self::User => "Login-enabled role",
            Self::Role => "Permission role",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerAddAvailability {
    Available,
    Unavailable(&'static str),
}

impl ExplorerAddAvailability {
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExplorerAddOption {
    pub kind: ExplorerAddKind,
    pub availability: ExplorerAddAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerAddMenu {
    pub profile_id: Uuid,
    pub selected: usize,
    pub options: Vec<ExplorerAddOption>,
}

impl ExplorerAddMenu {
    pub fn new(profile_id: Uuid, options: Vec<ExplorerAddOption>) -> Self {
        let selected = options
            .iter()
            .position(|option| option.availability.is_available())
            .unwrap_or(0);
        Self {
            profile_id,
            selected,
            options,
        }
    }

    pub fn selected_option(&self) -> Option<&ExplorerAddOption> {
        self.options
            .get(self.selected)
            .filter(|option| option.availability.is_available())
    }

    pub fn selected_kind(&self) -> Option<ExplorerAddKind> {
        self.selected_option().map(|option| option.kind)
    }

    pub fn select(&mut self, index: usize) -> bool {
        if self
            .options
            .get(index)
            .is_none_or(|option| !option.availability.is_available())
        {
            return false;
        }
        let changed = self.selected != index;
        self.selected = index;
        changed
    }

    pub fn move_selection(&mut self, delta: isize) -> bool {
        if delta == 0 || self.options.is_empty() {
            return false;
        }
        let step = delta.signum();
        let mut index = self.selected as isize;
        loop {
            let next = index + step;
            if next < 0 || next >= self.options.len() as isize {
                return false;
            }
            index = next;
            if self.options[index as usize].availability.is_available() {
                self.selected = index as usize;
                return true;
            }
        }
    }
}
