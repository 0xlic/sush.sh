use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// Unified events consumed by the app layer.
#[derive(Debug)]
#[allow(dead_code)]
pub enum AppEvent {
    Input(Vec<u8>),
    Tick,
}

pub struct EventBus {
    rx: mpsc::Receiver<AppEvent>,
    cancel: CancellationToken,
}

impl Default for EventBus {
    /// Return an empty bus without background tasks, only for mem::take placeholders.
    fn default() -> Self {
        let (_tx, rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        Self { rx, cancel }
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(128);
        let cancel = CancellationToken::new();
        spawn_terminal_reader(tx, cancel.clone());
        Self { rx, cancel }
    }

    /// Stop the background reader task and drop any queued events.
    pub fn shutdown(self) {
        self.cancel.cancel();
        // After rx is dropped, sender errors out and the background task exits.
    }

    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}

fn spawn_terminal_reader(tx: mpsc::Sender<AppEvent>, cancel: CancellationToken) {
    tokio::spawn(async move {
        let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(32);
        tokio::task::spawn_blocking(move || {
            use std::io::Read;

            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 4096];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if input_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut ticker = time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {
                    if tx.send(AppEvent::Tick).await.is_err() { break; }
                }
                Some(data) = input_rx.recv() => {
                    if tx.send(AppEvent::Input(data)).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
}
