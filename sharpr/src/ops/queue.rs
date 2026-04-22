use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::{Receiver, Sender};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub enum OpEvent {
    Added {
        id: u64,
        title: String,
    },
    Progress {
        id: u64,
        fraction: Option<f32>,
    },
    Completed(u64),
    Failed {
        id: u64,
        msg: String,
    },
    #[allow(dead_code)]
    Dismissed(u64),
}

/// A handle to a single in-progress operation. Callers use this to send
/// progress updates. Consuming it (complete/fail) sends a terminal event.
pub struct OpHandle {
    pub id: u64,
    tx: Sender<OpEvent>,
}

impl OpHandle {
    pub fn progress(&self, fraction: Option<f32>) {
        let _ = self.tx.send_blocking(OpEvent::Progress {
            id: self.id,
            fraction,
        });
    }

    pub fn complete(self) {
        let _ = self.tx.send_blocking(OpEvent::Completed(self.id));
    }

    pub fn fail(self, msg: impl Into<String>) {
        let _ = self.tx.send_blocking(OpEvent::Failed {
            id: self.id,
            msg: msg.into(),
        });
    }
}

/// Cloneable sender handle — callers hold one to register new operations.
#[derive(Clone)]
pub struct OpQueue(Sender<OpEvent>);

impl OpQueue {
    /// Register a new operation. Immediately fires `OpEvent::Added` so the UI
    /// indicator appears before any heavy work starts.
    pub fn add(&self, title: impl Into<String>) -> OpHandle {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let _ = self.0.send_blocking(OpEvent::Added {
            id,
            title: title.into(),
        });
        OpHandle {
            id,
            tx: self.0.clone(),
        }
    }
}

/// Create a new queue. Returns the sender (to store in AppState) and the
/// receiver (to drive the UI indicator).
pub fn new_queue() -> (OpQueue, Receiver<OpEvent>) {
    let (tx, rx) = async_channel::unbounded();
    (OpQueue(tx), rx)
}
