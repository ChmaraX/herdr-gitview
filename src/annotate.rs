//! Popup entrypoints for the review-notes flow:
//! - `annotate`: a one-line note input (title from `GITVIEW_ASK_TEXT`);
//!   writes the note text (empty = cancelled) to the answer file.
//! - `pick-agent`: choose an agent pane from `GITVIEW_AGENTS` (JSON
//!   `[[pane_id, agent, status], …]`); writes `pane\tplace`, `pane\tsubmit`,
//!   or `cancel`. Enter places the notes in the agent's prompt; shift+enter
//!   or ctrl+enter also submits them.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::popup::write_answer;

pub fn run_annotate() -> Result<()> {
    let title = std::env::var("GITVIEW_ASK_TEXT").unwrap_or_else(|_| "note".into());
    // Editing an existing note pre-fills the input.
    let mut input = std::env::var("GITVIEW_PREFILL").unwrap_or_default();

    let mut terminal = ratatui::init();
    let answer = loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(1), // title
                Constraint::Min(1),    // input
                Constraint::Length(1), // hint
            ])
            .split(area);
            frame.render_widget(
                Paragraph::new(format!(" {title}"))
                    .style(Style::new().add_modifier(Modifier::BOLD)),
                chunks[0],
            );
            frame.render_widget(
                Paragraph::new(format!(" {input}█"))
                    .wrap(Wrap { trim: false })
                    .block(Block::bordered()),
                chunks[1],
            );
            frame.render_widget(
                Paragraph::new("enter save · esc cancel")
                    .alignment(Alignment::Center)
                    .style(Style::new().add_modifier(Modifier::DIM)),
                chunks[2],
            );
        })?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Enter => break input.clone(),
                KeyCode::Esc => break String::new(),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            }
        }
    };
    ratatui::restore();
    write_answer(&answer)
}

pub fn run_pick_agent() -> Result<()> {
    let title = std::env::var("GITVIEW_ASK_TEXT").unwrap_or_else(|_| "send notes to…".into());
    let agents: Vec<(String, String, String, String, String)> =
        serde_json::from_str(&std::env::var("GITVIEW_AGENTS").unwrap_or_default())
            .unwrap_or_default();
    if agents.is_empty() {
        write_answer("cancel")?;
        return Ok(());
    }
    let mut cursor = 0usize;

    let mut terminal = ratatui::init();
    crate::term::enable_mouse();
    let answer = loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(1), // title
                Constraint::Min(1),    // agents
                Constraint::Length(2), // hint
            ])
            .split(area);
            frame.render_widget(
                Paragraph::new(format!(" {title}"))
                    .style(Style::new().add_modifier(Modifier::BOLD)),
                chunks[0],
            );
            let items: Vec<ListItem> = agents
                .iter()
                .map(|(pane, agent, status, tab, cwd)| {
                    let mut spans = vec![
                        Span::styled(
                            format!("  {agent}"),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" · {status}"),
                            Style::new().fg(color_for_status(status)),
                        ),
                    ];
                    if !tab.is_empty() {
                        spans.push(Span::raw(format!("  tab: {tab}")));
                    }
                    if !cwd.is_empty() {
                        spans.push(Span::styled(
                            format!("  {cwd}"),
                            Style::new().add_modifier(Modifier::DIM),
                        ));
                    }
                    spans.push(Span::styled(
                        format!("  {pane}"),
                        Style::new().add_modifier(Modifier::DIM),
                    ));
                    ListItem::new(Line::from(spans))
                })
                .collect();
            let list =
                List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            let mut state = ListState::default();
            state.select(Some(cursor));
            frame.render_stateful_widget(list, chunks[1], &mut state);
            frame.render_widget(
                Paragraph::new("enter place in prompt · shift/ctrl+enter send now\nesc cancel")
                    .alignment(Alignment::Center)
                    .style(Style::new().add_modifier(Modifier::DIM)),
                chunks[2],
            );
        })?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Enter
                    if key
                        .modifiers
                        .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
                {
                    break format!("{}\tsubmit", agents[cursor].0);
                }
                KeyCode::Enter => break format!("{}\tplace", agents[cursor].0),
                KeyCode::Down | KeyCode::Char('j') => {
                    cursor = (cursor + 1).min(agents.len() - 1);
                }
                KeyCode::Up | KeyCode::Char('k') => cursor = cursor.saturating_sub(1),
                KeyCode::Esc | KeyCode::Char('q') => break "cancel".to_string(),
                _ => {}
            },
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                // Rows start under the title line.
                let idx = m.row.saturating_sub(1) as usize;
                if idx < agents.len() {
                    if cursor == idx {
                        break format!("{}\tplace", agents[idx].0); // click-click = choose
                    }
                    cursor = idx;
                }
            }
            _ => {}
        }
    };
    crate::term::disable_mouse();
    ratatui::restore();
    write_answer(&answer)
}

fn color_for_status(status: &str) -> Color {
    match status {
        "idle" => Color::Green,
        "working" => Color::Yellow,
        _ => Color::DarkGray,
    }
}
