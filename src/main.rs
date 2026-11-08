fn main() -> anyhow::Result<()> {
    herdr_gitview::logx::init_panic_hook();
    let mode = std::env::args().nth(1);
    let op = std::env::args().nth(2);
    herdr_gitview::logx::init(&match (mode.as_deref(), op.as_deref()) {
        (Some("json"), Some(op)) => format!("json {op}"),
        (mode, _) => mode.unwrap_or("?").to_string(),
    });
    match mode.as_deref() {
        Some("list") => herdr_gitview::list::run(),
        Some("preview") => herdr_gitview::preview::run(),
        Some("toggle") | None => herdr_gitview::orchestrate::toggle(),
        Some("toggle-tab") => herdr_gitview::orchestrate::toggle_tab(),
        Some("open") => herdr_gitview::orchestrate::open(),
        Some("close") => herdr_gitview::orchestrate::close(),
        Some("ask") => herdr_gitview::ask::run(),
        Some("pick-agent") => herdr_gitview::annotate::run_pick_agent(),
        Some("json") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            herdr_gitview::appjson::run(&args)
        }
        // `env K=V... program args`: `/usr/bin/env` for the mods where
        // there is none (Windows): the program's exit code and output.
        Some("env") => {
            let mut args = std::env::args_os().skip(2);
            let mut vars = Vec::new();
            let program = loop {
                let Some(a) = args.next() else {
                    anyhow::bail!("env: no program")
                };
                match a.to_str().and_then(|s| s.split_once('=')) {
                    Some((k, v)) if !k.is_empty() && !k.starts_with('-') => {
                        vars.push((k.to_string(), v.to_string()))
                    }
                    _ => break a,
                }
            };
            let status = std::process::Command::new(program)
                .args(args)
                .envs(vars)
                .status()?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Some("app-setup") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            herdr_gitview::appsetup::run(&args)
        }
        Some(other) => {
            anyhow::bail!(
                "unknown mode: {other} (expected list|preview|toggle|toggle-tab|open|close|ask|pick-agent|json|app-setup)"
            )
        }
    }
}
