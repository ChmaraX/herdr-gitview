fn main() -> anyhow::Result<()> {
    herdr_gitview::logx::init_panic_hook();
    let mode = std::env::args().nth(1);
    match mode.as_deref() {
        Some("list") => herdr_gitview::list::run(),
        Some("preview") => herdr_gitview::preview::run(),
        Some("toggle") | None => herdr_gitview::orchestrate::toggle(),
        Some("open") => herdr_gitview::orchestrate::open(),
        Some("close") => herdr_gitview::orchestrate::close(),
        Some(other) => anyhow::bail!("unknown mode: {other} (expected list|preview|toggle|open|close)"),
    }
}
