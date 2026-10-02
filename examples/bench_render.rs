//! Diff-render benchmark: `cargo run --release --example bench_render -- [file…]`.
//!
//! For each file, diffs it against a copy with three scattered one-line
//! edits (a typical "agent touched a big file" change) and prints the median
//! time of the render stages. Defaults to `tests/fixtures/large.ts`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use herdr_gitview::config::{Config, Theme};
use herdr_gitview::git::{ChangeKind, Repo, Scope};
use herdr_gitview::keymap::Keymap;
use herdr_gitview::preview::app::{PreviewApp, ShowReq};
use herdr_gitview::preview::highlight::Highlighter;
use herdr_gitview::preview::render;
use herdr_gitview::preview::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> Duration {
    median(
        (0..runs)
            .map(|_| {
                let t = Instant::now();
                std::hint::black_box(f());
                t.elapsed()
            })
            .collect(),
    )
}

/// The same content with three lines edited at 1/4, 1/2 and 3/4 of the file.
fn edited(old: &str) -> String {
    let mut lines: Vec<String> = old.lines().map(str::to_string).collect();
    let n = lines.len();
    for at in [n / 4, n / 2, 3 * n / 4] {
        lines[at].push_str(" // edited");
    }
    lines.join("\n") + "\n"
}

fn main() {
    let mut files: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if files.is_empty() {
        files.push(PathBuf::from("tests/fixtures/large.ts"));
    }
    let hl = Highlighter::new(Theme::Dark);
    for path in files {
        let old = std::fs::read_to_string(&path).expect("read file");
        let new = edited(&old);
        let ext = path.extension().and_then(|e| e.to_str());
        let lines = old.lines().count();
        let runs = 5;
        let whole = time(runs, || hl.highlight(&new, ext));
        let plain = time(runs, || {
            render::build_plain(&path, &old, &new, &hl, Theme::Dark, 3, 4)
        });
        let build = time(runs, || {
            render::build(&path, &old, &new, &hl, Theme::Dark, 3, 4)
        });
        println!(
            "{} ({lines} lines): highlight whole file {whole:>7.1?}  plain build {plain:>7.1?}  build {build:>7.1?}",
            path.display(),
        );

        // The whole file as one added side: no folds, every line on screen.
        let mut app = PreviewApp::new(
            Config::default(),
            Repo { root: ".".into() },
            Keymap::build(&Default::default()).unwrap(),
        );
        let req = ShowReq {
            file: path.clone(),
            orig_path: None,
            scope: Scope::Worktree,
            cached: false,
            kind: ChangeKind::Untracked,
            commit: None,
        };
        app.begin_show(req.clone());
        app.apply_diff(
            &req,
            Ok(render::build(&path, "", &new, &hl, Theme::Dark, 3)),
        );
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let frame = time(runs * 4, || {
            term.draw(|f| ui::render(f, &mut app)).unwrap();
        });
        let j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let step = time(runs * 4, || app.on_key(j));
        println!("  all-added view: draw a frame {frame:>7.1?}  cursor step {step:>7.1?}");
    }
}
