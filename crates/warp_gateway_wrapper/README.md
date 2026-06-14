# Warp Gateway Wrapper

A flexible gateway service that provides multiple transport options (stdio, HTTP, WebSocket) for tool execution. Built to support MCP (Model Context Protocol) and other integrations.

## Features

- **Multiple Transport Layers**: stdio, HTTP REST, and WebSocket
- **Extensible Tool System**: Easy plugin architecture for custom tools
- **Async-first Design**: Built on tokio for high performance
- **Type-safe Protocol**: Strongly-typed request/response messages
- **Streaming Support**: Real-time progress updates via WebSocket

## Quick Start

### Stdio Mode (MCP)

```bash
cargo run --bin warp-gateway-wrapper -- stdio
```

Reads JSON requests from stdin, writes JSON responses to stdout. Perfect for MCP integration.

### HTTP Mode

```bash
# Default port 3000
cargo run --bin warp-gateway-wrapper -- http

# Custom address
cargo run --bin warp-gateway-wrapper -- http --addr 127.0.0.1:8080
```

Provides REST endpoints and WebSocket at `/ws` for browser/API integration.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  Transport Layer                     │
│  ┌──────────┐  ┌──────────┐  ┌────────────────┐   │
│  │  Stdio   │  │   HTTP   │  │   WebSocket    │   │
│  └────┬─────┘  └────┬─────┘  └────────┬───────┘   │
└───────┼─────────────┼─────────────────┼────────────┘
        │             │                 │
        └─────────────┴─────────────────┘
                      │
          ┌───────────▼──────────────┐
          │   Gateway Engine         │
          │  - Request routing       │
          │  - Task management       │
          │  - Error handling        │
          └───────────┬──────────────┘
                      │
          ┌───────────▼──────────────┐
          │   Tool Registry          │
          │  - Tool registration     │
          │  - Tool discovery        │
          └───────────┬──────────────┘
                      │
        ┌─────────────┴─────────────┐
        │                           │
   ┌────▼─────┐              ┌─────▼──────┐
   │  Echo    │   ...        │  Shell     │
   │  Tool    │              │  Tool      │
   └──────────┘              └────────────┘
```

## Project Structure

```
crates/warp_gateway_wrapper/
├── src/
│   ├── adapters/         # Transport layer implementations
│   │   ├── stdio.rs      # MCP stdio adapter
│   │   └── http.rs       # HTTP/WebSocket adapter
│   ├── gateway/          # Core gateway engine
│   │   └── engine.rs     # Request routing and execution
│   ├── protocol/         # Protocol definitions
│   │   ├── messages.rs   # Request/response types
│   │   └── errors.rs     # Error types
│   ├── tools/            # Tool system
│   │   ├── mod.rs        # Tool trait and registry
│   │   └── builtin/      # Built-in tools
│   ├── streaming/        # Progress streaming
│   └── utils/            # Utilities
├── tests/                # Integration tests
├── docs/                 # Documentation
│   ├── architecture.md
│   └── http_adapter.md
└── TODO.md              # Development roadmap
```

## Protocol

### Request Format

```json
{
  "type": "execute",
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "tool_name": "echo",
  "parameters": {
    "message": "Hello, World!"
  }
}
```

### Response Format

```json
{
  "type": "toolresult",
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "result": {
    "message": "Hello, World!"
  },
  "status": "success"
}
```

See [docs/http_adapter.md](docs/http_adapter.md) for complete API reference.

## Development Status

- ✅ Phase 1: Foundation & Architecture
- ✅ Phase 2: Transport Layer (HTTP/WebSocket)
- 🚧 Phase 3: Advanced Features (cancellation, progress, timeouts)
- 📋 Phase 4: Shell Tool
- 📋 Phase 5: Additional Tools
- 📋 Phase 6: Production Readiness

See [TODO.md](TODO.md) for detailed roadmap.

## Testing

```bash
# Run all tests
cargo test -p warp_gateway_wrapper

# Run specific test module
cargo test -p warp_gateway_wrapper http_adapter
```

## Contributing

See [docs/architecture.md](docs/architecture.md) for design principles and contribution guidelines.
