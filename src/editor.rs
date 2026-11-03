//! Which editor Enter starts for a new file view in the Herdr app, on the
//! machine the repo is on: the config's `editor`, else `$VISUAL`, else
//! `$EDITOR`, else `nvim`, `vim`, `vi`; here only, when none of those
//! exists, the system's opener. The op's contract is in
//! [`crate::appjson`]'s module doc.

use std::path::Path;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::remote::{Machine, Run};

/// How the editor takes a line: `+N` for the vi family, nothing otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Nvim,
    /// vim, vi (and their names: view, vimdiff…): `+N` works.
    Vim,
    Other,
    /// The system's opener (`open -t`, `xdg-open`): no line, no pane.
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Editor {
    /// The program (resolved to its path) and its own arguments; the file,
    /// and `+N` for nvim/vim, go after.
    pub argv: Vec<String>,
    pub kind: Kind,
    /// What to call it (`nvim`, `code`, `open`).
    pub name: String,
}

/// Tried last, in this order.
const FALLBACKS: [&str; 3] = ["nvim", "vim", "vi"];

/// What a machine says about editors: its `$VISUAL` and `$EDITOR`, and
/// where each asked program is (`None`: not there, or not executable).
struct Facts {
    visual: Option<String>,
    editor: Option<String>,
    found: Vec<(String, Option<String>)>,
}

impl Facts {
    fn path_of(&self, name: &str) -> Option<&str> {
        self.found
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, p)| p.as_deref())
    }
}

/// The editor on `machine` (a remote one is asked in `dir`, its login
/// shell's environment), as the config's `editor` (`None`: not set) and
/// that machine's environment say. `None`: there's none.
pub fn pick(
    machine: &Machine,
    dir: &Path,
    configured: Option<&[String]>,
) -> Result<Option<Editor>> {
    let configured = configured.filter(|argv| !argv.is_empty());
    let mut names: Vec<String> = configured.map(|a| vec![a[0].clone()]).unwrap_or_default();
    names.extend(FALLBACKS.map(String::from));
    let mut facts = facts(machine, dir, &names)?;
    // $VISUAL/$EDITOR name programs only known now; ask about those too.
    let from_env: Vec<Vec<String>> = [&facts.visual, &facts.editor]
        .into_iter()
        .flatten()
        .filter_map(|v| words(v))
        .filter(|argv| !argv.is_empty())
        .collect();
    let unknown: Vec<String> = from_env
        .iter()
        .map(|argv| argv[0].clone())
        .filter(|n| !facts.found.iter().any(|(k, _)| k == n))
        .collect();
    if !unknown.is_empty() {
        facts.found.extend(facts_about(machine, dir, &unknown)?);
    }

    let fallbacks = FALLBACKS.map(|n| vec![n.to_string()]);
    let candidates = configured
        .into_iter()
        .map(<[String]>::to_vec)
        .chain(from_env)
        .chain(fallbacks);
    for argv in candidates {
        if let Some(path) = facts.path_of(&argv[0]) {
            let name = Path::new(&argv[0])
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| argv[0].clone());
            let mut resolved = vec![path.to_string()];
            resolved.extend(argv[1..].iter().cloned());
            return Ok(Some(Editor {
                argv: resolved,
                kind: kind_of(&name),
                name,
            }));
        }
    }
    Ok(match machine {
        Machine::Local => system_opener(),
        Machine::On(_) => None,
    })
}

/// Whether `nvim` is there on `machine` (herdr-nvim is only asked then).
pub fn has_nvim(machine: &Machine, dir: &Path) -> Result<bool> {
    let found = facts_about(machine, dir, &["nvim".to_string()])?;
    Ok(found.first().is_some_and(|(_, p)| p.is_some()))
}

fn kind_of(name: &str) -> Kind {
    match name {
        "nvim" => Kind::Nvim,
        "vim" | "vi" | "view" | "vimdiff" | "gvim" | "mvim" => Kind::Vim,
        _ => Kind::Other,
    }
}

/// `open -t` on macOS (the default text editor), `xdg-open` elsewhere when
/// it's on `PATH`.
fn system_opener() -> Option<Editor> {
    let (argv, name) = if cfg!(target_os = "macos") {
        let open = Path::new("/usr/bin/open");
        executable(open).then(|| (vec![open.display().to_string(), "-t".to_string()], "open"))?
    } else {
        (vec![on_path("xdg-open")?], "xdg-open")
    };
    Some(Editor {
        argv,
        kind: Kind::System,
        name: name.to_string(),
    })
}

fn facts(machine: &Machine, dir: &Path, names: &[String]) -> Result<Facts> {
    match machine {
        Machine::Local => {
            let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
            Ok(Facts {
                visual: var("VISUAL"),
                editor: var("EDITOR"),
                found: local_paths(names),
            })
        }
        Machine::On(_) => {
            let mut fields = remote_answers(machine, dir, true, names)?.into_iter();
            let mut var = || fields.next().filter(|v| !v.trim().is_empty());
            let (visual, editor) = (var(), var());
            let found = names
                .iter()
                .cloned()
                .zip(fields.map(|p| (!p.is_empty()).then_some(p)));
            Ok(Facts {
                visual,
                editor,
                found: found.collect(),
            })
        }
    }
}

fn facts_about(
    machine: &Machine,
    dir: &Path,
    names: &[String],
) -> Result<Vec<(String, Option<String>)>> {
    match machine {
        Machine::Local => Ok(local_paths(names)),
        Machine::On(_) => {
            let fields = remote_answers(machine, dir, false, names)?;
            Ok(names
                .iter()
                .cloned()
                .zip(fields.into_iter().map(|p| (!p.is_empty()).then_some(p)))
                .collect())
        }
    }
}

/// One login shell there (so `$VISUAL`, `$EDITOR` and `PATH` are the
/// person's): NUL-separated fields, `$VISUAL` and `$EDITOR` first when
/// `vars`, then each name's path (empty: not there).
const LOOKUP: &str = r#"[ "$1" = vars ] && printf '%s\0%s\0' "$VISUAL" "$EDITOR"; shift
for n; do p=$(command -v -- "$n" 2>/dev/null); case "$p" in /*) [ -f "$p" ] && [ -x "$p" ] || p= ;; *) p= ;; esac; printf '%s\0' "$p"; done"#;

fn remote_answers(
    machine: &Machine,
    dir: &Path,
    vars: bool,
    names: &[String],
) -> Result<Vec<String>> {
    let Machine::On(m) = machine else {
        unreachable!("remote_answers on this machine")
    };
    let mut argv = vec!["sh", "-lc", LOOKUP, "sh", if vars { "vars" } else { "-" }];
    argv.extend(names.iter().map(String::as_str));
    let out = machine.output(
        machine.command(dir, &argv, Run::default()),
        "the editor lookup",
    )?;
    if !out.status.success() {
        bail!(
            "looking for an editor on {m}: {}",
            crate::remote::remote_error(&String::from_utf8_lossy(&out.stderr))
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut fields: Vec<String> = text.split('\0').map(str::to_string).collect();
    fields.pop(); // after the last NUL
    let want = names.len() + if vars { 2 } else { 0 };
    if fields.len() != want {
        bail!(
            "looking for an editor on {m}: an answer of {} fields, not {want}",
            fields.len()
        );
    }
    Ok(fields)
}

fn local_paths(names: &[String]) -> Vec<(String, Option<String>)> {
    names
        .iter()
        .map(|n| {
            let path = if n.contains('/') {
                executable(Path::new(n)).then(|| n.clone())
            } else {
                on_path(n)
            };
            (n.clone(), path)
        })
        .collect()
}

fn on_path(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_absolute() && executable(p))
        .map(|p| p.display().to_string())
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `s` split as a shell splits a word list: blanks separate words, quotes
/// (`'…'`, `"…"`) and `\` keep them together. `None` when a quote isn't
/// closed. No expansion of any kind.
pub fn words(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => out.extend(word.take()),
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => w.push(c),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            c @ ('"' | '\\' | '$' | '`') => w.push(c),
                            '\n' => {}
                            c => {
                                w.push('\\');
                                w.push(c);
                            }
                        },
                        c => w.push(c),
                    }
                }
            }
            '\\' => {
                let w = word.get_or_insert_with(String::new);
                match chars.next() {
                    Some('\n') | None => {}
                    Some(c) => w.push(c),
                }
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    out.extend(word);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::words;

    #[test]
    fn an_editor_variable_splits_like_a_shell_word_list() {
        let w = |s: &str| words(s).map(|v| v.join("|"));
        assert_eq!(w("code --wait").as_deref(), Some("code|--wait"));
        assert_eq!(
            w(r#""/Applications/My Editor/bin/ed" -w"#).as_deref(),
            Some("/Applications/My Editor/bin/ed|-w")
        );
        assert_eq!(w(r"my\ ed 'a b' x").as_deref(), Some("my ed|a b|x"));
        assert_eq!(w("  nvim  ").as_deref(), Some("nvim"));
        // No expansion, and a quote left open is no word list at all.
        assert_eq!(w("$HOME/ed").as_deref(), Some("$HOME/ed"));
        assert_eq!(w("'open"), None);
        assert_eq!(w("").as_deref(), Some(""));
    }
}
