//! The `ask` entrypoint: a tiny confirm dialog meant to run in a herdr
//! floating popup pane (≥0.7.4; overlay placement on older versions).
//!
//! Reads the question from `GITVIEW_ASK_TEXT`, waits for y / n / esc (or a
//! mouse click on a button), writes the answer ("y" | "n" | "cancel") to
//! `GITVIEW_ANSWER_FILE`, and exits. The list pane polls that file.

use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

pub fn run() -> Result<()> {
    let text = std::env::var("GITVIEW_ASK_TEXT").unwrap_or_else(|_| "Confirm? (y/n)".into());
    let answer_file =
        std::env::var("GITVIEW_ANSWER_FILE").context("GITVIEW_ANSWER_FILE not set")?;

    let mut terminal = ratatui::init();
    crate::term::enable_mouse();
    let answer = loop {
        let mut yes_span: Option<(u16, u16, u16)> = None; // (y, x0, x1)
        let mut no_span: Option<(u16, u16, u16)> = None;
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Min(1),    // question
                Constraint::Length(1), // buttons
                Constraint::Length(1), // hint
            ])
            .split(area);

            frame.render_widget(
                Paragraph::new(text.clone())
                    .wrap(Wrap { trim: true })
                    .alignment(Alignment::Center),
                chunks[0],
            );

            // Centered [ yes ]   [ no ] buttons; remember their x-ranges so
            // clicks can hit them.
            let yes = " yes (y) ";
            let no = " no (n) ";
            let total = yes.len() + 3 + no.len();
            let x0 = chunks[1].x + chunks[1].width.saturating_sub(total as u16) / 2;
            let row = chunks[1].y;
            yes_span = Some((row, x0, x0 + yes.len() as u16));
            no_span = Some((row, x0 + (yes.len() + 3) as u16, x0 + total as u16));
            let line = Line::from(vec![
                Span::styled(
                    yes,
                    Style::new()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled(
                    no,
                    Style::new()
                        .fg(Color::Black)
                        .bg(Color::Red)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            let mut btn = chunks[1];
            btn.x = x0;
            btn.width = btn.width.min(total as u16);
            frame.render_widget(Paragraph::new(line), btn);

            frame.render_widget(
                Paragraph::new("esc cancels")
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
                KeyCode::Char('y') | KeyCode::Enter => break "y",
                KeyCode::Char('n') => break "n",
                KeyCode::Esc | KeyCode::Char('q') => break "cancel",
                _ => {}
            },
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                let hit = |span: Option<(u16, u16, u16)>| {
                    span.map(|(y, x0, x1)| m.row == y && m.column >= x0 && m.column < x1)
                        .unwrap_or(false)
                };
                if hit(yes_span) {
                    break "y";
                }
                if hit(no_span) {
                    break "n";
                }
            }
            _ => {}
        }
    };
    crate::term::disable_mouse();
    ratatui::restore();

    // Write atomically-ish: temp file + rename, so the poller never sees a
    // half-written answer.
    let tmp = format!("{answer_file}.tmp");
    std::fs::write(&tmp, answer)?;
    std::fs::rename(&tmp, &answer_file)?;
    Ok(())
}
