# HTTP Adapter Usage

The gateway wrapper now supports both stdio (for MCP protocol) and HTTP modes for broader integration.

## Running the HTTP Server

```bash
# Start HTTP server on default port (3000)
cargo run --bin warp-gateway-wrapper -- http

# Start on custom address
cargo run --bin warp-gateway-wrapper -- http --addr 127.0.0.1:8080
```

## Available Endpoints

### Health Check
```bash
curl http://localhost:3000/health
# Response: {"status":"ok","service":"warp-gateway-wrapper"}
```

### List Tools
```bash
curl http://localhost:3000/api/tools
# Response: {"type":"toolslist","tools":[...]}
```

### Execute Tool (REST)
```bash
curl -X POST http://localhost:3000/api/execute \
  -H "Content-Type: application/json" \
  -d '{
    "type": "execute",
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "tool_name": "echo",
    "parameters": {"message": "Hello, World!"}
  }'
# Response: {"type":"toolresult","task_id":"...","result":{...},"status":"success"}
```

### WebSocket Connection

The WebSocket endpoint at `ws://localhost:3000/ws` provides bidirectional communication:

```javascript
const ws = new WebSocket('ws://localhost:3000/ws');

ws.onopen = () => {
  // Send a request
  ws.send(JSON.stringify({
    type: 'execute',
    task_id: '550e8400-e29b-41d4-a716-446655440000',
    tool_name: 'echo',
    parameters: { message: 'Hello via WebSocket!' }
  }));
};

ws.onmessage = (event) => {
  const response = JSON.parse(event.data);
  console.log('Response:', response);
};
```

## Protocol Compatibility

Both HTTP and WebSocket use the same JSON protocol:

### Request Types
- `execute`: Run a tool
- `cancel_task`: Cancel a running task
- `list_tools`: List available tools
- `shutdown`: Gracefully shutdown gateway

### Response Types
- `toolresult`: Tool execution result
- `error`: Error response
- `toolslist`: List of tools
- `ack`: Acknowledgment
- `progress`: Progress update (streaming)

## Stdio Mode (MCP)

The stdio mode remains unchanged for MCP integration:

```bash
cargo run --bin warp-gateway-wrapper -- stdio
```

This mode reads JSON requests from stdin and writes JSON responses to stdout.
