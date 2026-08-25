use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use secrecy::SecretString;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use uuid::Uuid;

use crate::{
    action::{Action, Command},
    app::App,
    cli::{Cli, MouseMode},
    db::DatabaseConnection,
    input::{keymap::Keymap, mouse::map_mouse},
    model::workspace::QueryStatus,
    persistence::{paths::AppPaths, profiles::ProfileStore},
    profile::{ConnectionProfile, import_connection_url},
    terminal::TerminalSession,
    ui::{self, UiState},
};

#[derive(Clone, Debug)]
struct ActiveConnection {
    profile_id: Uuid,
    generation: u64,
    database: DatabaseConnection,
}

pub struct Runtime {
    profiles: HashMap<Uuid, ConnectionProfile>,
    secrets: HashMap<Uuid, SecretString>,
    event_sender: mpsc::UnboundedSender<Action>,
    connection: Arc<Mutex<Option<ActiveConnection>>>,
    latest_connection_generation: Arc<AtomicU64>,
    query_tasks: HashMap<(Uuid, u64), JoinHandle<()>>,
    background_tasks: Vec<JoinHandle<()>>,
}

impl Runtime {
    pub fn new(
        profiles: Vec<ConnectionProfile>,
        secrets: HashMap<Uuid, SecretString>,
        event_sender: mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            profiles: profiles
                .into_iter()
                .map(|profile| (profile.id, profile))
                .collect(),
            secrets,
            event_sender,
            connection: Arc::new(Mutex::new(None)),
            latest_connection_generation: Arc::new(AtomicU64::new(0)),
            query_tasks: HashMap::new(),
            background_tasks: Vec::new(),
        }
    }

    pub fn dispatch(&mut self, command: Command) {
        self.query_tasks.retain(|_, task| !task.is_finished());
        self.background_tasks.retain(|task| !task.is_finished());
        match command {
            Command::Connect {
                profile_id,
                generation,
            } => self.connect(profile_id, generation),
            Command::LoadCatalog {
                profile_id,
                generation,
            } => self.load_catalog(profile_id, generation),
            Command::RunQuery {
                tab_id,
                generation,
                sql,
            } => self.run_query(tab_id, generation, sql),
            Command::PreviewTable {
                tab_id,
                generation,
                schema,
                name,
            } => self.preview_table(tab_id, generation, schema, name),
            Command::LoadDdl {
                tab_id,
                generation,
                kind,
                schema,
                name,
            } => self.load_ddl(tab_id, generation, kind, schema, name),
            Command::CancelQuery { tab_id, generation } => {
                if let Some(task) = self.query_tasks.remove(&(tab_id, generation)) {
                    task.abort();
                }
            }
            Command::PersistWorkspace | Command::Quit => {}
        }
    }

    fn connect(&mut self, profile_id: Uuid, generation: u64) {
        let Some(profile) = self.profiles.get(&profile_id).cloned() else {
            let _ = self.event_sender.send(Action::ConnectionFailed {
                profile_id,
                generation,
                message: "Connection profile no longer exists".to_owned(),
            });
            return;
        };
        let password = self.secrets.get(&profile_id).cloned();
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let latest = Arc::clone(&self.latest_connection_generation);
        latest.store(generation, Ordering::SeqCst);
        self.background_tasks.push(tokio::spawn(async move {
            match DatabaseConnection::connect(&profile, password.as_ref()).await {
                Ok(database) => match database.probe().await {
                    Ok(server) => {
                        if latest.load(Ordering::SeqCst) != generation {
                            database.close().await;
                            return;
                        }
                        let previous = connection.lock().await.replace(ActiveConnection {
                            profile_id,
                            generation,
                            database,
                        });
                        if let Some(previous) = previous {
                            previous.database.close().await;
                        }
                        let _ = sender.send(Action::ConnectionSucceeded {
                            profile_id,
                            generation,
                            server,
                        });
                    }
                    Err(error) => {
                        database.close().await;
                        let _ = sender.send(Action::ConnectionFailed {
                            profile_id,
                            generation,
                            message: error.to_string(),
                        });
                    }
                },
                Err(error) => {
                    let _ = sender.send(Action::ConnectionFailed {
                        profile_id,
                        generation,
                        message: error.to_string(),
                    });
                }
            }
        }));
    }

    fn load_catalog(&mut self, profile_id: Uuid, generation: u64) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        self.background_tasks.push(tokio::spawn(async move {
            let database = {
                let guard = connection.lock().await;
                guard
                    .as_ref()
                    .filter(|active| {
                        active.profile_id == profile_id && active.generation == generation
                    })
                    .map(|active| active.database.clone())
            };
            let Some(database) = database else {
                return;
            };
            match database.load_catalog().await {
                Ok(nodes) => {
                    let _ = sender.send(Action::CatalogLoaded {
                        profile_id,
                        generation,
                        nodes,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogFailed {
                        profile_id,
                        generation,
                        message: error.to_string(),
                    });
                }
            }
        }));
    }

    fn run_query(&mut self, tab_id: Uuid, generation: u64, sql: String) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let database = active_database(connection).await;
            let Some(database) = database else {
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    message: "No active database connection".to_owned(),
                });
                return;
            };
            match database.execute(&sql).await {
                Ok(outcome) => {
                    let _ = sender.send(Action::QueryFinished {
                        tab_id,
                        generation,
                        outcome,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::QueryFailed {
                        tab_id,
                        generation,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.query_tasks.insert((tab_id, generation), task);
    }

    fn preview_table(&mut self, tab_id: Uuid, generation: u64, schema: String, name: String) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection).await else {
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    message: "No active database connection".to_owned(),
                });
                return;
            };
            let sql = format!(
                "SELECT * FROM {}.{} LIMIT 500",
                database.quote_identifier(&schema),
                database.quote_identifier(&name)
            );
            match database.execute(&sql).await {
                Ok(outcome) => {
                    let _ = sender.send(Action::PreviewFinished {
                        tab_id,
                        generation,
                        sql,
                        outcome,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::QueryFailed {
                        tab_id,
                        generation,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.query_tasks.insert((tab_id, generation), task);
    }

    fn load_ddl(
        &mut self,
        tab_id: Uuid,
        generation: u64,
        kind: crate::db::catalog::CatalogKind,
        schema: String,
        name: String,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection).await else {
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    message: "No active database connection".to_owned(),
                });
                return;
            };
            match database.object_ddl(kind, &schema, &name).await {
                Ok(Some(ddl)) => {
                    let _ = sender.send(Action::DdlLoaded {
                        tab_id,
                        generation,
                        ddl,
                    });
                }
                Ok(None) => {
                    let _ = sender.send(Action::QueryFailed {
                        tab_id,
                        generation,
                        message: "DDL is not available for this object type".to_owned(),
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::QueryFailed {
                        tab_id,
                        generation,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.query_tasks.insert((tab_id, generation), task);
    }

    pub async fn shutdown(mut self) {
        for (_, task) in self.query_tasks.drain() {
            task.abort();
        }
        for task in self.background_tasks.drain(..) {
            task.abort();
        }
        if let Some(connection) = self.connection.lock().await.take() {
            connection.database.close().await;
        }
    }
}

async fn active_database(
    connection: Arc<Mutex<Option<ActiveConnection>>>,
) -> Option<DatabaseConnection> {
    connection
        .lock()
        .await
        .as_ref()
        .map(|active| active.database.clone())
}

pub async fn run_tui(cli: Cli) -> Result<()> {
    let (profiles, secrets, selected_profile) = load_startup_profiles(&cli)?;
    let mut app = App::new(profiles.clone());
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(profiles, secrets, event_sender);
    let mut terminal = TerminalSession::enter(cli.mouse != MouseMode::Off)
        .context("failed to initialize terminal")?;
    let mut terminal_events = EventStream::new();
    let mut keymap = Keymap::default();
    let mut ui_state = UiState::default();
    let mut ticker = interval(Duration::from_millis(33));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    apply_action(
        &mut app,
        &mut runtime,
        Action::RequestConnect(selected_profile),
    );
    terminal.draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))?;

    while !app.should_quit {
        let mut redraw = false;
        tokio::select! {
            terminal_event = terminal_events.next() => {
                let Some(terminal_event) = terminal_event else { break; };
                match terminal_event.context("terminal input failed")? {
                    Event::Key(key) => {
                        if let Some(action) = keymap.map(key, &app) {
                            apply_action(&mut app, &mut runtime, action);
                            redraw = true;
                        }
                    }
                    Event::Mouse(mouse) => {
                        if let Some(action) = map_mouse(mouse, &ui_state, &app) {
                            apply_action(&mut app, &mut runtime, action);
                            redraw = true;
                        }
                    }
                    Event::Paste(value) => {
                        if app.focus == crate::model::workspace::Focus::Editor
                            && app.active_console().editor.mode
                                == crate::model::editor::EditorMode::Insert
                        {
                            for character in value.chars() {
                                let action = if character == '\n' {
                                    Action::InsertNewline
                                } else {
                                    Action::InsertCharacter(character)
                                };
                                apply_action(&mut app, &mut runtime, action);
                            }
                            redraw = true;
                        }
                    }
                    Event::Resize(_, _) | Event::FocusGained | Event::FocusLost => redraw = true,
                }
            }
            Some(action) = event_receiver.recv() => {
                apply_action(&mut app, &mut runtime, action);
                redraw = true;
            }
            _ = ticker.tick() => {
                redraw = ui_state.effects.is_active()
                    || app.tabs.iter().any(|tab| tab.query_status == QueryStatus::Running);
            }
        }

        if redraw && !app.should_quit {
            terminal.draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))?;
        }
    }

    runtime.shutdown().await;
    Ok(())
}

fn apply_action(app: &mut App, runtime: &mut Runtime, action: Action) {
    for command in app.update(action) {
        runtime.dispatch(command);
    }
}

type StartupProfiles = (Vec<ConnectionProfile>, HashMap<Uuid, SecretString>, Uuid);

fn load_startup_profiles(cli: &Cli) -> Result<StartupProfiles> {
    let profile_path = if let Some(path) = &cli.config {
        path.clone()
    } else {
        AppPaths::discover()?.profiles_file()
    };
    let store = ProfileStore::new(profile_path);
    let mut profiles = store.load().context("failed to load connection profiles")?;
    let mut secrets = HashMap::new();

    let direct_profile = if let Some(url) = &cli.url {
        let mut imported = import_connection_url(url, cli.profile.as_deref())?;
        if cli.read_only {
            imported.profile.read_only = true;
        }
        if let Some(password) = imported.transient_password {
            secrets.insert(imported.profile.id, password);
        }
        let profile_id = imported.profile.id;
        profiles.push(imported.profile);
        Some(profile_id)
    } else {
        None
    };

    let selected = direct_profile.or_else(|| {
        cli.profile.as_deref().and_then(|name| {
            profiles
                .iter()
                .find(|profile| profile.name == name)
                .map(|profile| profile.id)
        })
    });
    let selected = if let Some(selected) = selected.or_else(|| profiles.first().map(|p| p.id)) {
        selected
    } else {
        let mut imported = import_connection_url("sqlite::memory:", Some("local-memory"))?;
        imported.profile.read_only = cli.read_only;
        let selected = imported.profile.id;
        profiles.push(imported.profile);
        selected
    };

    if let Entry::Vacant(entry) = secrets.entry(selected)
        && let Ok(password) = std::env::var("LAZYDB_PASSWORD")
        && !password.is_empty()
    {
        entry.insert(SecretString::from(password));
    }

    Ok((profiles, secrets, selected))
}
