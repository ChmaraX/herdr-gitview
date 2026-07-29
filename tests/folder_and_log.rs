//! App-level tests for the two folder/log affordances:
//!
//! * `s` / `u` / `x` on a *directory* row apply to every file under it in
//!   that section (staged vs changes), never crossing into the other one.
//! * `w` in the log view flips between "the commits this branch added" and
//!   the full history, and entering the log from branch scope starts filtered.
//!
//! These drive the real `App` against a real git repo, through `on_key`, so
//! the keymap → action → git path is exercised end to end.

mod common;

use std::collections::HashMap;

use crossterm::event::KeyEvent;

use common::{TempRepo, fixture, git, git_lenient, write};
use herdr_gitview::config::Config;
use herdr_gitview::git::{Repo, Scope, StageState};
use herdr_gitview::keymap::{Keymap, parse_key};
use herdr_gitview::list::App;
use herdr_gitview::list::app::{ListRow, Mode, Section};

fn app_for(repo: &TempRepo) -> App {
    let cfg = Config::default();
    let keys = Keymap::build(&HashMap::new()).unwrap();
    App::new(Repo::discover(&repo.dir).unwrap(), cfg, keys).unwrap()
}

/// Press a key by its default binding string ("s", "w", "l", …).
fn press(app: &mut App, spec: &str) {
    let (code, mods) = parse_key(spec).unwrap();
    app.on_key(KeyEvent::new(code, mods));
}

/// Move the cursor onto the directory row whose full path is `path`, in
/// `section`. Panics when that row isn't on screen.
fn select_dir(app: &mut App, path: &str, section: Section) {
    let idx = app
        .rows
        .iter()
        .position(|row| {
            matches!(row, ListRow::Dir { path: p, section: s, .. } if p == path && *s == section)
        })
        .unwrap_or_else(|| panic!("no dir row {path:?} in {section:?} in {:?}", app.rows));
    app.cursor = idx;
}

fn stage_of(app: &App, path: &str) -> StageState {
    app.entries
        .iter()
        .find(|e| e.path == std::path::Path::new(path))
        .unwrap_or_else(|| panic!("no entry for {path}"))
        .stage
}

/// A repo with `src/a.rs`, `src/deep/b.rs` and `other.rs` all untracked.
fn folder_repo(name: &str) -> TempRepo {
    let t = fixture(name);
    write(&t.dir, "src/a.rs", "a\n");
    write(&t.dir, "src/deep/b.rs", "b\n");
    write(&t.dir, "other.rs", "o\n");
    t
}

// ---- folder staging --------------------------------------------------------

#[test]
fn stage_on_a_folder_row_stages_everything_under_it() {
    let t = folder_repo("folder-stage");
    let mut app = app_for(&t);

    select_dir(&mut app, "src/", Section::Changes);
    press(&mut app, "s");

    assert_eq!(stage_of(&app, "src/a.rs"), StageState::Staged);
    assert_eq!(stage_of(&app, "src/deep/b.rs"), StageState::Staged);
    // the sibling outside the folder never moved
    assert_eq!(stage_of(&app, "other.rs"), StageState::Untracked);
}

#[test]
fn stage_on_a_nested_folder_row_stops_at_that_subtree() {
    let t = folder_repo("folder-stage-nested");
    let mut app = app_for(&t);

    select_dir(&mut app, "src/deep/", Section::Changes);
    press(&mut app, "s");

    assert_eq!(stage_of(&app, "src/deep/b.rs"), StageState::Staged);
    assert_eq!(stage_of(&app, "src/a.rs"), StageState::Untracked);
}

#[test]
fn stage_on_the_staged_section_folder_row_unstages_it() {
    let t = folder_repo("folder-unstage");
    git(&t.dir, &["add", "src"]);
    let mut app = app_for(&t);

    // `s` is section-aware: on the staged side of the tree it unstages.
    select_dir(&mut app, "src/", Section::Staged);
    press(&mut app, "s");

    assert_eq!(stage_of(&app, "src/a.rs"), StageState::Untracked);
    assert_eq!(stage_of(&app, "src/deep/b.rs"), StageState::Untracked);
}

#[test]
fn unstage_on_a_folder_row_only_touches_the_staged_side() {
    let t = fixture("folder-unstage-mixed");
    write(&t.dir, "src/staged.rs", "s\n");
    write(&t.dir, "src/loose.rs", "l\n");
    git(&t.dir, &["add", "src/staged.rs"]);
    let mut app = app_for(&t);

    select_dir(&mut app, "src/", Section::Staged);
    press(&mut app, "u");

    assert_eq!(stage_of(&app, "src/staged.rs"), StageState::Untracked);
    assert_eq!(stage_of(&app, "src/loose.rs"), StageState::Untracked);
}

#[test]
fn discard_on_a_folder_row_confirms_then_removes_the_subtree() {
    let t = folder_repo("folder-discard");
    let mut app = app_for(&t);

    select_dir(&mut app, "src/", Section::Changes);
    press(&mut app, "x");
    assert!(app.modal.is_some(), "discard must ask first");

    press(&mut app, "y"); // confirm

    assert!(!t.dir.join("src/a.rs").exists());
    assert!(!t.dir.join("src/deep/b.rs").exists());
    assert!(t.dir.join("other.rs").exists(), "sibling survives");
}

#[test]
fn discard_on_a_folder_row_can_be_cancelled() {
    let t = folder_repo("folder-discard-cancel");
    let mut app = app_for(&t);

    select_dir(&mut app, "src/", Section::Changes);
    press(&mut app, "x");
    press(&mut app, "n"); // decline

    assert!(t.dir.join("src/a.rs").exists());
    assert!(t.dir.join("src/deep/b.rs").exists());
}

#[test]
fn folder_actions_are_refused_in_branch_scope() {
    let t = fixture("folder-branch-scope");
    // Commit the folder on a feature branch so branch scope actually has
    // something to show (diff vs the merge base with main).
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "src/a.rs", "a\n");
    write(&t.dir, "src/deep/b.rs", "b\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "files"]);
    let mut app = app_for(&t);

    press(&mut app, "w"); // -> branch scope
    assert_eq!(app.scope, Scope::Branch);
    // Branch scope is one unsectioned tree.
    select_dir(&mut app, "src/", Section::Flat);
    press(&mut app, "s");

    assert!(
        app.active_status().unwrap().contains("working-tree scope"),
        "got: {:?}",
        app.active_status()
    );
}

/// Regression: "merge conflicts" and "changes" are both unstaged sections, so
/// a `Dir { staged: bool }` row could not tell them apart — two identical
/// `("src/", false)` rows. Acting on the conflicts one reached into changes
/// and staged (or offered to discard) a file the user could not even see
/// under that row.
#[test]
fn a_folder_row_never_reaches_into_another_section() {
    let t = fixture("folder-section-isolation");
    write(&t.dir, "src/a.rs", "base\n");
    write(&t.dir, "src/b.rs", "base\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "base"]);
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "src/a.rs", "feature\n");
    git(&t.dir, &["commit", "-q", "-am", "f"]);
    git(&t.dir, &["checkout", "-q", "main"]);
    write(&t.dir, "src/a.rs", "main\n");
    git(&t.dir, &["commit", "-q", "-am", "m"]);
    git_lenient(&t.dir, &["merge", "feature"]); // leaves src/a.rs conflicted
    write(&t.dir, "src/b.rs", "modified\n"); // ...and src/b.rs merely modified

    let mut app = app_for(&t);
    // Both sections render a `src/` folder row; they must be distinguishable.
    let sections: Vec<Section> = app
        .rows
        .iter()
        .filter_map(|row| match row {
            ListRow::Dir { path, section, .. } if path == "src/" => Some(*section),
            _ => None,
        })
        .collect();
    assert_eq!(sections, vec![Section::Conflicts, Section::Changes]);

    // `s` on the conflicts folder is refused, and touches nothing.
    select_dir(&mut app, "src/", Section::Conflicts);
    press(&mut app, "s");
    assert!(
        app.active_status().unwrap().contains("conflict"),
        "got: {:?}",
        app.active_status()
    );
    assert_eq!(stage_of(&app, "src/b.rs"), StageState::Unstaged);

    // `x` on it is refused too, rather than offering to discard src/b.rs.
    press(&mut app, "x");
    assert!(app.modal.is_none(), "must not open a discard confirm");
    assert_eq!(stage_of(&app, "src/b.rs"), StageState::Unstaged);

    // The changes folder still works, and only on its own section.
    select_dir(&mut app, "src/", Section::Changes);
    press(&mut app, "s");
    assert_eq!(stage_of(&app, "src/b.rs"), StageState::Staged);
}

/// A folder path is prefix-matched, so a sibling sharing its prefix must not
/// be swept in: `src/` and `src-old/` are different folders.
#[test]
fn a_folder_row_does_not_match_a_prefix_sibling() {
    let t = fixture("folder-prefix-sibling");
    write(&t.dir, "src/a.rs", "a\n");
    write(&t.dir, "src-old/b.rs", "b\n");
    let mut app = app_for(&t);

    select_dir(&mut app, "src/", Section::Changes);
    press(&mut app, "s");
    assert_eq!(stage_of(&app, "src/a.rs"), StageState::Staged);
    assert_eq!(stage_of(&app, "src-old/b.rs"), StageState::Untracked);
}

// ---- branch-scoped log -----------------------------------------------------

/// `main` gets an extra commit, then `feature` adds two of its own.
fn branch_repo(name: &str) -> TempRepo {
    let t = fixture(name);
    write(&t.dir, "base.txt", "main2\n");
    git(&t.dir, &["commit", "-q", "-am", "main second"]);
    git(&t.dir, &["checkout", "-q", "-b", "feature"]);
    write(&t.dir, "f1.txt", "a\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature one"]);
    write(&t.dir, "f2.txt", "b\n");
    git(&t.dir, &["add", "."]);
    git(&t.dir, &["commit", "-q", "-m", "feature two"]);
    t
}

fn subjects(app: &App) -> Vec<String> {
    app.commits.iter().map(|c| c.subject.clone()).collect()
}

#[test]
fn w_in_the_log_view_filters_to_this_branchs_commits() {
    let t = branch_repo("log-toggle");
    let mut app = app_for(&t);

    press(&mut app, "l");
    assert_eq!(app.mode, Mode::Log);
    assert!(!app.log_branch_only);
    assert_eq!(subjects(&app).len(), 4);

    press(&mut app, "w");
    assert!(app.log_branch_only);
    assert_eq!(subjects(&app), vec!["feature two", "feature one"]);

    press(&mut app, "w"); // back to everything
    assert!(!app.log_branch_only);
    assert_eq!(subjects(&app).len(), 4);
}

#[test]
fn entering_the_log_from_branch_scope_starts_filtered() {
    let t = branch_repo("log-from-branch-scope");
    let mut app = app_for(&t);

    press(&mut app, "w"); // worktree -> branch scope
    assert_eq!(app.scope, Scope::Branch);
    press(&mut app, "l");

    assert_eq!(app.mode, Mode::Log);
    assert!(
        app.log_branch_only,
        "branch scope should carry into the log"
    );
    assert_eq!(subjects(&app), vec!["feature two", "feature one"]);
}

#[test]
fn filtered_log_still_opens_a_commits_files() {
    let t = branch_repo("log-filtered-open");
    let mut app = app_for(&t);

    press(&mut app, "l");
    press(&mut app, "w");
    app.open_commit();

    assert_eq!(app.mode, Mode::CommitFiles);
    assert_eq!(
        app.entries
            .iter()
            .map(|e| e.path.display().to_string())
            .collect::<Vec<_>>(),
        vec!["f2.txt"]
    );
}

#[test]
fn filtering_the_log_on_the_base_branch_yields_nothing() {
    let t = fixture("log-filter-on-base");
    let mut app = app_for(&t);

    press(&mut app, "l");
    press(&mut app, "w");

    assert!(app.log_branch_only);
    assert!(app.commits.is_empty());

    // and it is reversible — no dead end
    press(&mut app, "w");
    assert_eq!(subjects(&app), vec!["base"]);
}

#[test]
fn leaving_the_log_returns_to_the_file_list() {
    let t = branch_repo("log-leave");
    let mut app = app_for(&t);

    press(&mut app, "l");
    press(&mut app, "w");
    press(&mut app, "q");

    assert_eq!(app.mode, Mode::Files);
    assert_ne!(app.scope, Scope::Branch, "the log filter is not the scope");
}
