//! Duplicate-request guard.
//!
//! Warp/Agent occasionally re-issues an identical request after a stream ends,
//! which can spam the upstream provider and burn tokens. The guard fingerprints
//! each request and, within a sliding time window, returns a local synthetic
//! response instead of calling upstream again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Tracks recently-seen request fingerprints with their first-seen instant.
pub struct DuplicateGuard {
    window: Duration,
    seen: Mutex<HashMap<u64, Instant>>,
}

impl DuplicateGuard {
    pub fn new(window_secs: u64) -> Self {
        Self {
            window: Duration::from_secs(window_secs),
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Whether the guard is active (window > 0).
    pub fn is_enabled(&self) -> bool {
        !self.window.is_zero()
    }

    /// Compute a stable fingerprint for a Chat Completions request body.
    ///
    /// Based on: model, stream flag, system instructions, the full conversation,
    /// and the tool count. Deliberately ignores volatile fields (ids, timestamps).
    pub fn fingerprint(body: &Value) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        body.get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .hash(&mut hasher);
        body.get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .hash(&mut hasher);

        // Conversation: hash each message's role + text content.
        if let Some(messages) = body.get("messages").and_then(Value::as_array) {
            messages.len().hash(&mut hasher);
            for message in messages {
                message
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .hash(&mut hasher);
                // content may be a string or array; hash its canonical JSON text.
                if let Some(content) = message.get("content") {
                    content.to_string().hash(&mut hasher);
                }
            }
        }

        // Tool count (not the full tool schemas, which can be large/volatile).
        let tool_count = body
            .get("tools")
            .and_then(Value::as_array)
            .map(|array| array.len())
            .unwrap_or(0);
        tool_count.hash(&mut hasher);

        hasher.finish()
    }

    /// Record a request and report whether it is a duplicate within the window.
    /// Also evicts expired entries to keep the map bounded.
    pub fn check_and_record(&self, fingerprint: u64) -> bool {
        if !self.is_enabled() {
            return false;
        }
        let now = Instant::now();
        let mut seen = self.seen.lock().unwrap();

        // Evict expired entries.
        seen.retain(|_, first_seen| now.duration_since(*first_seen) < self.window);

        match seen.get(&fingerprint) {
            Some(_) => true, // still within window -> duplicate
            None => {
                seen.insert(fingerprint, now);
                false
            }
        }
    }

    /// Build a local synthetic Chat Completion response for a suppressed duplicate.
    pub fn synthetic_response(model: &str) -> Value {
        json!({
            "id": "chatcmpl-mpg-duplicate",
            "object": "chat.completion",
            "created": 0,
            "model": model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": ""
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 },
            "x_managed_gateway": { "duplicate_suppressed": true }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_bodies_share_fingerprint() {
        let a = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let b = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        assert_eq!(
            DuplicateGuard::fingerprint(&a),
            DuplicateGuard::fingerprint(&b)
        );
    }

    #[test]
    fn different_content_differs() {
        let a = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let b = json!({"model": "m", "messages": [{"role": "user", "content": "bye"}]});
        assert_ne!(
            DuplicateGuard::fingerprint(&a),
            DuplicateGuard::fingerprint(&b)
        );
    }

    #[test]
    fn first_request_not_duplicate_second_is() {
        let guard = DuplicateGuard::new(60);
        let fp = 12345;
        assert!(!guard.check_and_record(fp));
        assert!(guard.check_and_record(fp));
    }

    #[test]
    fn disabled_guard_never_flags() {
        let guard = DuplicateGuard::new(0);
        assert!(!guard.is_enabled());
        let fp = 999;
        assert!(!guard.check_and_record(fp));
        assert!(!guard.check_and_record(fp));
    }

    #[test]
    fn synthetic_response_marks_suppressed() {
        let response = DuplicateGuard::synthetic_response("m");
        assert_eq!(response["x_managed_gateway"]["duplicate_suppressed"], true);
        assert_eq!(response["choices"][0]["finish_reason"], "stop");
    }
}
