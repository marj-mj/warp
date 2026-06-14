# Warp Gateway Wrapper Architecture

## Overview
This crate implements a modular gateway wrapper for Warp that supports:
- Tool registration and execution
- Multiple adapters (stdio, HTTP, WebSocket planned)
- Async task management
- Protocol abstraction

## Architecture

### Directory Structure
```
src/
├── adapters/       # Protocol adapters (stdio, HTTP, WebSocket)
│   └── stdio.rs    # Line-delimited JSON via stdin/stdout
├── gateway/        # Core gateway engine
│   └── engine.rs   # Request routing and tool execution
├── launcher/       # Legacy Warp launcher (Windows-specific)
│   ├── config.rs
│   ├── endpoint_store.rs
│   ├── process.rs
│   └── platform/   # Platform-specific implementations
│       ├── windows_secure_storage.rs
│       └── windows_user_preferences.rs
├── protocol/       # Protocol messages and errors
│   ├── messages.rs # Request/response types
│   └── errors.rs   # Error types
├── streaming/      # SSE/WebSocket streaming support (planned)
├── tools/          # Tool system
│   ├── traits.rs   # Tool trait definition
│   ├── registry.rs # Tool registration
│   └── builtin/    # Built-in tools
│       └── echo.rs # Example echo tool
└── utils/          # Shared utilities
    └── task_id.rs  # Task identifier type
```

### Key Components

#### 1. Protocol Layer
- **GatewayRequest**: Execute, ListTools, CancelTask, Shutdown
- **GatewayResponse**: ToolResult, Error, ToolsList, Ack, Progress
- **ExecutionStatus**: Success, Error, Cancelled

#### 2. Tool System
- **Tool trait**: Async trait for tool implementations
- **ToolRegistry**: Manages tool registration and lookup
- **Builtin tools**: Echo (example)

#### 3. Gateway Engine
- Routes requests to appropriate tools
- Manages concurrent execution
- Handles errors and cancellation

#### 4. Adapters
- **StdioAdapter**: Line-delimited JSON via stdin/stdout
- **HTTPAdapter**: Planned
- **WebSocketAdapter**: Planned

### Request/Response Flow

```
Client (Warp)
    ↓
Adapter (stdio/HTTP)
    ↓
GatewayEngine
    ↓
ToolRegistry.get(tool_name)
    ↓
Tool.execute(task_id, parameters)
    ↓
GatewayResponse
    ↓
Adapter
    ↓
Client
```

## Usage

### Running with Stdio Adapter
```bash
echo '{"type":"execute","task_id":"123","tool_name":"echo","parameters":{"message":"test"}}' | cargo run
```

### Response Format
```json
{
  "type": "toolresult",
  "task_id": "123",
  "result": {"echoed": "test"},
  "status": "success"
}
```

## Adding New Tools

1. Implement the `Tool` trait:
```rust
use async_trait::async_trait;
use crate::tools::traits::{Tool, ToolResult};

pub struct MyTool;

#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "..." }
    fn parameters_schema(&self) -> Value { json!({...}) }
    async fn execute(&self, task_id: TaskId, parameters: Value) -> ToolResult<Value> {
        // Implementation
    }
}
```

2. Register in main.rs:
```rust
registry.register(Arc::new(MyTool));
```

## Testing

```bash
cargo test -p warp_gateway_wrapper
```

## Phase 1 Status
✅ Foundation complete:
- Protocol layer
- Tool system with registry
- Gateway engine
- Stdio adapter
- Integration tests

## Next Steps (Phase 2)
- [ ] HTTP adapter with axum
- [ ] WebSocket streaming
- [ ] Task cancellation
- [ ] Progress reporting
- [ ] Tool timeouts

## OZ Compatibility Roadmap (chốt 2026-06-14)

Mục tiêu dài hạn: đưa `warp_gateway_wrapper` khớp thiết kế OZ (Managed Gateway cho Warp).

### Trạng thái hiện tại (Phase 1-5 hoàn thành)
- HTTP Server (axum): `/agent/run`, `/agent/stream/{task_id}` (SSE), `/agent/task/{task_id}`, `/agent/cancel`, `/health`
- Gateway Core (`GatewayEngine`): `spawn_agent` (flow 7 bước), `determine_provider` qua `ProviderFamily::from_model`
- Agent Session loop: provider step -> tool call -> submit result -> complete
- LLM Provider Adapter: OpenAI, Anthropic, Gemini, Mock (offline default)
- Tools: ShellTool (run_shell_command), FilesystemTool (root-confined), cancellation + timeout

### Lệch tên gọi so với spec (không ảnh hưởng chức năng)
| Spec | Code hiện tại |
|------|---------------|
| `Gateway` (struct) | `GatewayEngine` |
| `sessions`/`adapters` fields | `registry`/`executions` |
| LLM trong `src/adapters/` | LLM trong `src/providers/`; `src/adapters/` là transport |
| trait `LLMAdapter`+`Conversation` (stateful) | trait `LlmProvider` (stateless) |

### Còn thiếu so với spec
- `SpawnAgentRequest`: `attachments`, `initial_snapshot_token`; `mode` chưa là enum; `config` chưa phải `AgentConfigSnapshot`
- Harness Types (oz/claude/opencode/gemini/codex delegate CLI)
- Orchestration Remote (Docker, `environment_id`, `worker_host`), MAA, child agent
- Auth layer (`agent_identity_uid` chưa verify), `at_capacity` luôn false

### Các Phase tiếp theo (chi tiết trong TODO.md)
- Phase 6: Protocol Compatibility (SpawnAgentRequest/Response khớp OZ) <- ĐANG LÀM
- Phase 7: Harness Abstraction (oz vs delegate-CLI)
- Phase 8: Conversation Trait Alignment (tùy chọn)
- Phase 9: Authentication / Authorization
- Phase 10: Capacity & Lifecycle
- Phase 11: Remote Orchestration (Docker/Worker)
- Phase 12: MAA & Child Agents
- Phase 13: SSE Format Warp-Compat & Streaming token-level
- Phase 14: Tools mở rộng & Production

Thứ tự ưu tiên: tương thích OZ protocol sớm (6 -> 9 -> 13 -> 7); tái tạo đầy đủ backend (6 -> 7 -> 10 -> 11 -> 12).

### Phase 13 chi tiết (SSE Warp-compat + streaming)
- SSE wire format: mỗi event phát kèm `id:` (sequence id tăng dần per task) + `event:` (tên loại) + `data:` (JSON). EventSource client dùng được named events.
- Token-level streaming: assistant content được chunk theo whitespace thành các event `message_delta`, kết thúc bằng một `message` đầy đủ. Nối các delta tái tạo đúng nội dung.
- Replay: `StreamManager` giữ history buffer per task (ring buffer). Handler `/agent/stream/{task_id}` đọc `Last-Event-ID` header hoặc query `?last_event_id=`, replay các event id > last rồi nối live stream.

### Phase 7 chi tiết (Harness Abstraction)
- `HarnessType { Oz, Claude, Opencode, Gemini, Codex }` (khớp tên harness trong crate `ai`).
- trait `Harness { harness_type(); async run(HarnessContext) }`. `HarnessContext` gói task_id/run_id/request/identity/engine/cancellation_token.
- `OzHarness` (harness/oz.rs): agent loop native (LLM provider + gateway tools + streaming token-level + auth per-tool). Mặc định.
- `CliHarness` (harness/cli.rs): delegate sang CLI ngoài (claude/opencode/gemini/codex). Spawn subprocess, stream stdout từng dòng thành Message, map exit code -> Complete/Error. Override binary/args qua env `WARP_GATEWAY_<HARNESS>_CMD` / `_ARGS`.
- `resolve_harness(request)` đọc `config.harness`; session chỉ resolve + delegate, không còn chứa loop.
- Field mới: `AgentConfigSnapshot.harness: Option<String>`.

### Phase 10 chi tiết (Capacity & Lifecycle)
- `GatewayConfig { max_concurrent_tasks (16), completed_ttl_secs (3600) }`; `GatewayEngine::with_config`.
- `at_capacity()` = running_count >= max. `spawn_agent*` trả `SpawnOutcome::{Spawned{task_id,run_id}, AtCapacity}`; handler trả `at_capacity=true` khi đầy (HTTP 200, khớp OZ).
- `list_executions()`, `running_count()`; endpoint `GET /agent/tasks` (auth) trả running/max_concurrent/at_capacity/tasks.
- `ExecutionRecord.completed_at` (epoch secs) set khi terminal. `cleanup_expired()` reap record terminal quá TTL; `start_cleanup_reaper()` chạy nền (wired trong `serve`).
- Lưu ý: at_capacity dùng giới hạn cứng (chưa có hàng đợi). Hàng đợi/backpressure để dành cho phase production.

### Phase 14a chi tiết (Logging + Tools)
- Structured logging: thay toàn bộ println!/eprintln! bằng `tracing`. `init_tracing()` trong main dùng `RUST_LOG` (mặc định info). Instrumentation: spawn agent, at_capacity reject, cleanup reaper.
- NetworkTool (`http_request`): GET/POST/PUT/PATCH/DELETE/HEAD, headers/body/timeout, body cap 256KB. SSRF guard chặn localhost + loopback/private/link-local/metadata IP; chỉ http/https. Cần quyền `Network`.
- SystemInfoTool (`system_info`): OS/kernel/host/CPU count/memory qua sysinfo (spawn_blocking). Tool basic.
- Permission: thêm `ToolPermission::Network` (map `http_request`); `system_info` là basic.

### Phase P chi tiết (Transparent Warp Proxy)
- Mục tiêu: client Warp dùng `WARP_SERVER_ROOT_URL` / `WARP_WS_SERVER_URL` để trỏ vào proxy local; proxy forward sang `https://app.warp.dev` (hoặc channel khác), gắn Bearer token cấu hình. Không sửa source Warp.
- HTTP (`proxy/http.rs`): pass-through method/path/query/body; strip hop-by-hop headers; override `Authorization` (giữ nguyên nếu không có token cấu hình). Response stream để SSE relay đúng.
- WebSocket (`proxy/ws.rs`): tokio-tungstenite client; bidirectional relay frames text/binary/ping/pong/close. Route theo path: chứa `session` -> sessions WS, còn lại -> RTC.
- Server (`proxy/server.rs`): axum router fallback HTTP forward; middleware kiểm `Upgrade: websocket` để dispatch sang WS handler.
- CLI: `proxy --channel production|staging|dev` hoặc override từng URL `--upstream-http/--upstream-ws-rtc/--upstream-ws-sessions`. Token đọc env `WARP_GATEWAY_OZ_TOKEN` / `OZ_TOKEN` / `WARP_TOKEN`.
- Vẫn còn: TLS termination cho đường hardcoded HTTPS, IAP staging, per-identity token map. Liệt kê trong TODO.

### Phase MPG-A chi tiết (Managed Provider Gateway)
- Tái hiện thiết kế Managed Provider Gateway từ `warp-oss`. Là một local OpenAI-compatible HTTP server (mặc định `127.0.0.1:8787`) mà Warp Agent có thể dùng qua cơ chế custom-endpoint sẵn có.
- Module `mpg/`:
  - `config.rs`: `ProviderConfig` (name/base_url/model/wire_api/adapter/api_key/env_key); `GatewayConfig` thêm `auth_token`, `disable_tools`, `disable_mcp`, `duplicate_window_secs`, `force_model_config_key`; `ProvidersFile` đọc JSON preset (giống `script/provider_gateway.providers.json`); `apply_env_overrides` đọc các biến `WARP_MANAGED_PROVIDER_*`.
  - `openai_chat.rs`: forward `POST /v1/chat/completions` sang `<base_url>/chat/completions`. Override `Authorization: Bearer <upstream_api_key>`. `apply_safe_mode` strip `tools`/`tool_choice`/`parallel_tool_calls` khi `disable_tools=true`. Response stream để SSE relay đúng.
  - `server.rs`: axum router với routes `/healthz` (public), `/v1/healthz` + `/v1/models` + `/v1/chat/completions` (sau Bearer auth nếu cấu hình). Healthz trả debug summary (adapter, compatibility_group, wire_api, default_model, base_url, tools_state, mcp_state, duplicate_window, force_model).
- CLI: subcommand `mpg --host --port --config <providers.json> --provider <name>` cùng các override (`--upstream-base-url`, `--upstream-env-key`, `--upstream-wire-api`, `--upstream-adapter`, `--upstream-model`, `--disable-tools`, `--disable-mcp`, `--duplicate-window-secs`, `--force-model-config-key`).
- Không sửa Warp source: Warp dùng custom-endpoint mechanism sẵn có trong `crates/ai/src/api_keys.rs` (CustomEndpoint với `@gateway` marker) để trỏ vào gateway.
- Còn lại: adapter `openai_responses` (Phase B), duplicate-request guard (Phase C), provider probe (Phase D), Anthropic/Gemini native (Phase E).

### Phase MPG B-E chi tiết
- `openai_responses` (mpg/openai_responses.rs): chat -> /responses request (messages -> input, max_tokens -> max_output_tokens). SSE convert sang chat.completion.chunk với fallback chain (output_text.delta -> .done -> content_part.done -> completed.response.output). Non-stream JSON convert qua extract_text_from_responses_json.
- Duplicate guard (mpg/duplicate_guard.rs): fingerprint (model, stream, conversation, tool_count) + sliding window; trùng trong cửa sổ trả synthetic response (x_managed_gateway.duplicate_suppressed=true), không gọi upstream. Cấu hình qua duplicate_window_secs (0 = tắt).
- Probe (mpg/probe.rs + subcommand mpg-probe): thử chat/responses x {non_stream, streaming, tools}, chỉ 2xx là pass; phân loại diagnostic auth/rate_limit/server/network; recommend adapter + compatibility_group + wire_api + confidence + safe_mode_defaults.
- Native (mpg/native.rs): anthropic_messages (chat -> /v1/messages, system hoisted, x-api-key + anthropic-version) và gemini_generate_content (chat -> models/{model}:generateContent?key=, assistant->model role). Convert response về Chat Completion non-stream.
- Adapter enum mở rộng: OpenaiChat, OpenaiResponses, BridgeOpenai, AnthropicMessages, GeminiGenerateContent.

### Phase L (Managed Gateway Launcher) — kế hoạch
Mục tiêu: vỏ bọc UI cho `mpg` để máy mới khỏi config thủ công.
- Stack: egui + eframe (single binary, không Node/webview).
- Server: in-process (embed `MpgServer` qua tokio task), oneshot shutdown.
- Phạm vi UI: 3 tab Provider / Gateway / Warp setup + system tray + log tail.
- Warp setup: chỉ guide + copy + spawn-with-env. KHÔNG ghi vào secure storage Warp.
- Cloudflared: tích hợp ở Phase L-D (detect/install/run tunnel, hiện public URL).
- Crate mới: `warp_gateway_launcher` (cùng workspace).

### Phase Cleanup — tàn dư launcher đời đầu
Phạm vi: chỉ trong `crates/warp_gateway_wrapper/` và `target/`. KHÔNG đụng source Warp.
- Xóa: `src/main.rs.old` (16KB, không build), `src/launcher/process.rs` + `endpoint_store.rs` (rỗng), file `.exe` tên gạch ngang trong `target/`.
- Cân nhắc giữ: `src/launcher/config.rs` (logic discovery Warp binary) — Phase L-D có thể tái dùng. Đánh dấu deprecated trước khi xóa.
- Xem xét: `warp-gateway-wrapper.toml` (root) và `example-config.toml` (crate) — di chuyển sang `docs/` hoặc xóa.
