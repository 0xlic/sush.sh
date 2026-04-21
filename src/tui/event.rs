use std::time::Duration;

use crossterm::event::{Event as CtEvent, EventStream, KeyEvent};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// App 层消费的统一事件。
#[derive(Debug)]
#[allow(dead_code)]
pub enum AppEvent {
    Key(KeyEvent),
    Resize(u16, u16),
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
        let mut reader = EventStream::new();
        let mut ticker = time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {
                    if tx.send(AppEvent::Tick).await.is_err() { break; }
                }
                Some(Ok(ev)) = reader.next() => {
                    let mapped = match ev {
                        CtEvent::Key(k) => Some(AppEvent::Key(k)),
                        CtEvent::Resize(w, h) => Some(AppEvent::Resize(w, h)),
                        _ => None,
                    };
                    if let Some(e) = mapped
                        && tx.send(e).await.is_err() {
                            break;
                        }
                }
            }
        }
    });
}
