mod app;
mod config;
mod putty_args;
mod putty_shim;
mod sftp;
mod ssh;
mod tui;
mod tunnel;
mod utils;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--daemon") {
        #[cfg(unix)]
        return tunnel::daemon::run_daemon().await;
        #[cfg(not(unix))]
        anyhow::bail!("daemon mode is not supported on this platform yet");
    }
    if args.get(1).map(String::as_str) == Some("--putty-compatible") {
        let launch = putty_args::parse_putty_args(args.into_iter().skip(2))?;
        let mut app = app::App::new_putty_direct(launch)?;
        return app.run().await;
    }
    if putty_shim::is_current_putty_shim() {
        putty_shim::launch_terminal_from_current_shim(&args[1..])?;
        return Ok(());
    }
    let mut app = app::App::new()?;
    app.run().await
}
