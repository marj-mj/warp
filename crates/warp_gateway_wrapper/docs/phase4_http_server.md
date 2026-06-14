# Phase 4: HTTP Server & SSE Streaming

## Overview

Phase 4 implements a production-ready HTTP/REST API server with Server-Sent Events (SSE) for real-time agent execution streaming. The server provides OZ-compatible endpoints for spawning agents, monitoring progress, and canceling tasks.

## Architecture

### Components

```
http/
├── server.rs           # Main HTTP server with axum
├── handlers.rs         # Request handlers
├── types.rs           # Request/Response types
├── sse.rs             # Server-Sent Events types
├── stream_manager.rs  # Event broadcasting
└── mod.rs             # Module exports
```

### Key Features

1. **RESTful API**: Standard HTTP endpoints with JSON payloads
2. **SSE Streaming**: Real-time event delivery to clients
3. **Task Management**: Track and control agent executions
4. **CORS Support**: Cross-origin requests enabled
5. **Health Checks**: Service availability monitoring

## API Endpoints

### POST /agent/run
Spawn a new agent execution.

**Request:**
```json
{
  "prompt": "Write a hello world program",
  "config": {
    "model": "gpt-4",
    "temperature": 0.7,
    "environment_id": "env-123"
  },
  "mode": "normal",
  "agent_identity_uid": "user-456",
  "parent_run_id": "run-789"
}
```

**Response:**
```json
{
  "task_id": "task-abc123",
  "run_id": "run-def456",
  "at_capacity": false
}
```

### GET /agent/stream/:task_id
Subscribe to task execution events via SSE.

**Response:** SSE stream with events:
```
data: {"type":"stream_init","run_id":"run-123","task_id":"task-456"}

data: {"type":"progress","task_id":"task-456","progress":0.5,"message":"Processing..."}

data: {"type":"tool_call_started","task_id":"task-456","tool_name":"execute_code","parameters":{}}

data: {"type":"complete","task_id":"task-456","result":{"output":"Done"}}
```

### POST /agent/cancel
Cancel a running task.

**Request:**
```json
{
  "task_id": "task-abc123"
}
```

**Response:**
```json
{
  "success": true,
  "message": "Task cancelled successfully"
}
```

### GET /agent/task/:task_id
Get task status and result.

**Response:**
```json
{
  "task_id": "task-abc123",
  "status": "completed",
  "progress": 1.0,
  "result": {"output": "Hello, World!"},
  "error": null
}
```

### GET /health
Health check endpoint.

**Response:**
```json
{
  "status": "healthy",
  "version": "0.1.0"
}
```

## SSE Event Types

### StreamInit
Initial connection setup.
```json
{
  "type": "stream_init",
  "run_id": "run-123",
  "task_id": "task-456"
}
```

### Progress
Execution progress update.
```json
{
  "type": "progress",
  "task_id": "task-456",
  "progress": 0.75,
  "message": "Processing step 3 of 4",
  "metadata": {"step": 3, "total": 4}
}
```

### ToolCallStarted
Tool execution began.
```json
{
  "type": "tool_call_started",
  "task_id": "task-456",
  "tool_name": "execute_code",
  "parameters": {"language": "rust", "code": "..."}
}
```

### ToolCallCompleted
Tool execution finished.
```json
{
  "type": "tool_call_completed",
  "task_id": "task-456",
  "tool_name": "execute_code",
  "result": {"stdout": "Hello, World!"}
}
```

### ToolCallFailed
Tool execution error.
```json
{
  "type": "tool_call_failed",
  "task_id": "task-456",
  "tool_name": "execute_code",
  "error": "Compilation failed: syntax error"
}
```

### Message
Agent message or output.
```json
{
  "type": "message",
  "task_id": "task-456",
  "content": "The code compiled successfully",
  "role": "assistant"
}
```

### Complete
Task completed successfully.
```json
{
  "type": "complete",
  "task_id": "task-456",
  "result": {"output": "Task finished"}
}
```

### Error
Task execution error.
```json
{
  "type": "error",
  "task_id": "task-456",
  "code": "execution_failed",
  "message": "Tool execution timeout"
}
```

### Cancelled
Task was cancelled.
```json
{
  "type": "cancelled",
  "task_id": "task-456",
  "reason": "User requested cancellation"
}
```

### Ping
Keep-alive heartbeat (sent every 15s).
```json
{
  "type": "ping"
}
```

## Usage Examples

### Starting the Server

```rust
use std::sync::Arc;
use warp_gateway_wrapper::{
    GatewayEngine, ToolRegistry, Server, ServerConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = ToolRegistry::new();
    let gateway = Arc::new(GatewayEngine::new(registry));
    
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 8080,
    };
    
    let server = Server::new(config, gateway);
    server.run().await?;
    Ok(())
}
```

### Client Example (Rust)

```rust
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    
    // Spawn agent
    let response = client
        .post("http://127.0.0.1:8080/agent/run")
        .json(&json!({
            "prompt": "Write hello world",
            "config": {"model": "gpt-4"}
        }))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    
    let task_id = response["task_id"].as_str().unwrap();
    
    // Stream events
    let stream_url = format!("http://127.0.0.1:8080/agent/stream/{}", task_id);
    // ... handle SSE stream
    
    Ok(())
}
```

### Client Example (JavaScript/TypeScript)

```typescript
// Spawn agent
const response = await fetch('http://127.0.0.1:8080/agent/run', {
  method: 'POST',
  headers: {'Content-Type': 'application/json'},
  body: JSON.stringify({
    prompt: 'Write hello world',
    config: {model: 'gpt-4'}
  })
});

const {task_id, run_id} = await response.json();

// Subscribe to events
const eventSource = new EventSource(
  `http://127.0.0.1:8080/agent/stream/${task_id}`
);

eventSource.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('Event:', data);
  
  if (data.type === 'complete' || data.type === 'error') {
    eventSource.close();
  }
};
```

### Client Example (Python)

```python
import requests
import json
from sseclient import SSEClient

# Spawn agent
response = requests.post(
    'http://127.0.0.1:8080/agent/run',
    json={
        'prompt': 'Write hello world',
        'config': {'model': 'gpt-4'}
    }
)
data = response.json()
task_id = data['task_id']

# Stream events
messages = SSEClient(f'http://127.0.0.1:8080/agent/stream/{task_id}')
for msg in messages:
    if msg.data:
        event = json.loads(msg.data)
        print(f"Event: {event}")
        
        if event['type'] in ['complete', 'error']:
            break
```

## Implementation Details

### Stream Manager

Manages broadcast channels for active tasks:

```rust
pub struct StreamManager {
    channels: Arc<RwLock<HashMap<TaskId, broadcast::Sender<SSEEvent>>>>,
    channel_capacity: usize,
}
```

- Creates broadcast channel per task
- Supports multiple subscribers
- Automatic cleanup on task completion
- Configurable buffer capacity

### Handler Flow

1. **Spawn Agent**:
   - Validate request
   - Create task_id and run_id
   - Initialize broadcast channel
   - Send StreamInit event
   - Return IDs to client

2. **Subscribe to Stream**:
   - Validate task exists
   - Subscribe to broadcast channel
   - Convert to SSE stream
   - Add keep-alive pings

3. **Cancel Task**:
   - Signal task manager
   - Send Cancelled event
   - Clean up resources

### Error Handling

All errors return structured JSON:

```json
{
  "error": "Task not found: task-123"
}
```

HTTP status codes:
- `200 OK`: Successful request
- `400 Bad Request`: Invalid parameters
- `404 Not Found`: Task not found
- `500 Internal Server Error`: Server error

## Testing

Run the test suite:

```bash
cargo test -p warp_gateway_wrapper http
```

Run integration tests:

```bash
cargo test -p warp_gateway_wrapper --test http_integration
```

Run the example server:

```bash
cargo run -p warp_gateway_wrapper --example http_server
```

Test with the example client:

```bash
cargo run -p warp_gateway_wrapper --example http_client
```

## Configuration

### ServerConfig

```rust
pub struct ServerConfig {
    pub host: String,  // Bind address (e.g., "127.0.0.1")
    pub port: u16,     // Port number (e.g., 8080)
}
```

### Environment Variables

- `GATEWAY_HOST`: Override bind address (default: `127.0.0.1`)
- `GATEWAY_PORT`: Override port (default: `8080`)

## Performance Considerations

1. **Channel Capacity**: Default 100 events per task
   - Increase for high-throughput tasks
   - Decrease to save memory

2. **Keep-Alive Interval**: 15 seconds
   - Prevents proxy timeouts
   - Detects disconnected clients

3. **CORS**: Enabled for all origins
   - Restrict in production environments
   - Configure via tower_http::cors

4. **Concurrent Tasks**: No hard limit
   - Bounded by system resources
   - Consider task queuing for production

## Security Considerations

1. **Authentication**: Not implemented
   - Add auth middleware in production
   - Consider JWT or API keys

2. **Rate Limiting**: Not implemented
   - Add tower_governor or similar
   - Protect against DoS

3. **Input Validation**: Basic validation
   - Sanitize prompts in production
   - Validate model names

4. **CORS**: Permissive by default
   - Restrict origins in production
   - Configure allowed methods/headers

## Integration with OZ

The API is compatible with OZ's agent spawning protocol:

1. **SpawnAgentRequest**: Matches OZ format
2. **SSE Events**: Compatible with OZ event types
3. **Task Management**: Similar lifecycle

Differences:
- Additional status endpoint (`/agent/task/:id`)
- Simplified configuration options
- No built-in authentication

## Next Steps (Phase 5)

1. **Provider Integration**: Connect to actual LLM providers
2. **Tool Execution**: Wire up real tool implementations
3. **Agent Logic**: Implement the agent execution loop
4. **Error Recovery**: Add retry and fallback logic
5. **Persistence**: Store task history and results

## Dependencies

```toml
axum = { workspace = true, features = ["ws"] }
tokio = { workspace = true, features = ["full"] }
tokio-stream = "0.1"
tower = { workspace = true }
tower-http = { version = "0.5", features = ["cors"] }
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true, features = ["v4"] }
```

## References

- [Axum Documentation](https://docs.rs/axum)
- [Server-Sent Events Spec](https://html.spec.whatwg.org/multipage/server-sent-events.html)
- [OZ Agent Protocol](internal documentation)
- [Tower HTTP Middleware](https://docs.rs/tower-http)
