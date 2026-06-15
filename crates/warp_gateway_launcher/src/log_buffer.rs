//! In-memory log buffer fed by a `tracing` layer, with optional Tauri event emission.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Inner>,
}

struct Inner {
    lines: Mutex<VecDeque<String>>,
    capacity: usize,
    app_handle: Mutex<Option<AppHandle>>,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                lines: Mutex::new(VecDeque::with_capacity(capacity)),
                capacity,
                app_handle: Mutex::new(None),
            }),
        }
    }

    /// Wire the log buffer to a Tauri AppHandle so future log lines are emitted
    /// as `logs://line` events.
    pub fn attach(&self, handle: AppHandle) {
        *self.inner.app_handle.lock().unwrap() = Some(handle);
    }

    fn push(&self, line: String) {
        {
            let mut lines = self.inner.lines.lock().unwrap();
            if lines.len() == self.inner.capacity {
                lines.pop_front();
            }
            lines.push_back(line.clone());
        }
        if let Some(handle) = self.inner.app_handle.lock().unwrap().as_ref() {
            let _ = handle.emit("logs://line", line);
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.inner.lines.lock().unwrap().iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.inner.lines.lock().unwrap().clear();
    }
}

pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl LogBufferLayer {
    pub fn new(buffer: LogBuffer) -> Self { Self { buffer } }
}

impl<S: Subscriber> Layer<S> for LogBufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let mut line = format!("{:>5} {}", metadata.level(), metadata.target());
        if !visitor.message.is_empty() {
            line.push_str(": ");
            line.push_str(&visitor.message);
        }
        if !visitor.fields.is_empty() {
            line.push_str(" {");
            line.push_str(&visitor.fields.join(", "));
            line.push('}');
        }
        self.buffer.push(line);
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
            if self.message.starts_with('"') && self.message.ends_with('"') && self.message.len() >= 2 {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}
