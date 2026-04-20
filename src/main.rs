mod app;
mod config;
mod sftp;
mod ssh;
mod tui;
mod utils;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = app::App::new()?;
    app.run().await
}
