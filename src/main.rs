mod app;
mod config;
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
    let mut app = app::App::new()?;
    app.run().await
}
