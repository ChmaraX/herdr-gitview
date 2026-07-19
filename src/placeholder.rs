use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};

const ENV_KEYS: &[&str] = &[
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_PLUGIN_STATE_DIR",
    "GITVIEW_REPO",
    "GITVIEW_SOCKET",
    "GITVIEW_PREVIEW_PANE",
];

/// Phase-1 stand-in for both panes: dump the environment herdr gave us so
/// every orchestration assumption is visible on screen.
pub fn run(mode: &'static str) -> Result<()> {
    crate::logx::log(format!("{mode} placeholder starting"));
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, mode);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, mode: &str) -> Result<()> {
    loop {
        terminal.draw(|frame| {
            let mut lines = vec![Line::from(format!("mode: {mode}"))];
            for key in ENV_KEYS {
                let value = std::env::var(key).unwrap_or_else(|_| "<unset>".into());
                lines.push(Line::from(format!("{key}={value}")));
            }
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "<unknown>".into());
            lines.push(Line::from(format!("cwd={cwd}")));
            lines.push(Line::from(""));
            lines.push(Line::from("press q to close the whole view"));
            let block = Block::bordered().title(format!(" gitview {mode} (phase 1) "));
            frame.render_widget(Paragraph::new(lines).block(block), frame.area());
        })?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                    // Any pane's q tears down the whole view; outside herdr
                    // (standalone dev) there is nothing to tear down.
                    if std::env::var_os("HERDR_PANE_ID").is_some() {
                        crate::orchestrate::spawn_close();
                    }
                    return Ok(());
                }
            }
        }
    }
}
