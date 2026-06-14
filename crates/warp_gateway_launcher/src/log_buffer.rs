//! In-memory log buffer fed by a `tracing` layer, so the UI can show a live
//! tail of gateway logs.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Shared, bounded ring buffer of recent log lines.
#[derive(Clone)]
pub struct LogBuffer {
    lines: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    fn push(&self, line: String) {
        let mut lines = self.lines.lock().unwrap();
        if lines.len() == self.capacity {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// Snapshot the current lines (oldest first).
    pub fn snapshot(&self) -> Vec<String> {
        self.lines.lock().unwrap().iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.lines.lock().unwrap().clear();
    }
}

/// A `tracing` layer that formats each event into a single line and appends it
/// to a [`LogBuffer`].
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl LogBufferLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: Subscriber> Layer<S> for LogBufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        // Compose: LEVEL target: message {fields}
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

/// Collects the `message` field plus any other key=value fields from an event.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
            // Strip the surrounding quotes Debug adds for plain strings.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest() {
        let buffer = LogBuffer::new(2);
        buffer.push("a".into());
        buffer.push("b".into());
        buffer.push("c".into());
        let snap = buffer.snapshot();
        assert_eq!(snap, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn clear_empties() {
        let buffer = LogBuffer::new(8);
        buffer.push("x".into());
        buffer.clear();
        assert!(buffer.snapshot().is_empty());
    }
}
