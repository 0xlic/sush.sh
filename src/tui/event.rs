use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// App 层消费的统一事件。
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
    /// 返回一个不启动后台任务的空 bus，仅用于 mem::take 占位。
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

    /// 停止后台读取任务，消费掉所有已积压的事件。
    pub fn shutdown(self) {
        self.cancel.cancel();
        // rx drop 后 sender 会收到错误，后台任务退出
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
