use std::{
    io::{self, Stdout},
    panic,
};

use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalSession {
    terminal: Tui,
    mouse_enabled: bool,
}

impl TerminalSession {
    pub fn enter(mouse_enabled: bool) -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange
        ) {
            restore_terminal();
            return Err(error);
        }
        if mouse_enabled && let Err(error) = execute!(stdout, EnableMouseCapture) {
            restore_terminal();
            return Err(error);
        }
        let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_terminal();
                return Err(error);
            }
        };
        if let Err(error) = terminal.clear().and_then(|_| terminal.hide_cursor()) {
            restore_terminal();
            return Err(error);
        }
        Ok(Self {
            terminal,
            mouse_enabled,
        })
    }

    pub fn draw<F>(&mut self, render: F) -> io::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal.draw(render).map(|_| ())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let backend = self.terminal.backend_mut();
        if self.mouse_enabled {
            let _ = execute!(backend, DisableMouseCapture);
        }
        let _ = execute!(
            backend,
            DisableFocusChange,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
        let _ = self.terminal.show_cursor();
    }
}

pub fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

pub fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        DisableMouseCapture,
        DisableFocusChange,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}
