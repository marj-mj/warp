# Phase 1: Foundation & Architecture ✅
- [x] Restructure project
- [x] Create protocol layer (messages, errors)
- [x] Create tool system (traits, registry)
- [x] Create gateway engine
- [x] Create stdio adapter
- [x] Add integration tests
- [x] Write architecture docs

# Phase 2: Transport Layer & Protocols ✅
- [x] Implement HTTP adapter with axum
  - [x] POST /api/execute endpoint
  - [x] GET /api/tools list endpoint
  - [x] GET /health check endpoint
  - [x] WebSocket support at /ws
- [x] WebSocket streaming support
  - [x] Bidirectional communication
  - [x] Request/response handling
  - [x] Error handling
- [x] CLI support for both stdio and HTTP modes
- [x] Integration test for HTTP adapter

## Implementation Notes
- HTTP server runs on configurable address (default: 127.0.0.1:3000)
- WebSocket provides full-duplex communication for real-time operations
- REST endpoints for simple request/response patterns
- Unified state management via Arc<GatewayEngine>
- Uses workspace dependencies (axum, tower, http, futures-util)

# Phase 3: Advanced Features
- [ ] Task cancellation mechanism
  - [ ] CancellationToken per task
  - [ ] Graceful shutdown
  - [ ] Timeout handling
- [ ] Progress reporting
  - [ ] ProgressSender/Receiver
  - [ ] Streaming progress updates
  - [ ] Percentage/status tracking
- [ ] Tool timeouts
  - [ ] Per-tool timeout configuration
  - [ ] Timeout error handling
- [ ] SSE (Server-Sent Events) for progress streaming

# Phase 4: Shell Tool (Core Feature)
- [ ] Design shell command execution protocol
  - [ ] Input: command, args, env, cwd
  - [ ] Output: stdout, stderr, exit_code
  - [ ] Streaming: real-time output
- [ ] Implement ShellTool
  - [ ] Cross-platform support
  - [ ] Environment isolation
  - [ ] Security constraints
- [ ] Add shell-specific features
  - [ ] Command cancellation
  - [ ] Output buffering
  - [ ] Error handling

# Phase 5: Provider Integration & Agent Loop (in progress)
- [x] LLM provider abstraction (`src/providers/`)
  - [x] `LlmProvider` trait + `ChatMessage`/`ToolCall`/`AssistantTurn` types
  - [x] `ProviderFamily` inference from model id
  - [x] `ProviderSettings::from_env` (managed gateway + OPENAI/ANTHROPIC/GEMINI vars)
  - [x] `resolve_provider` factory (live OpenAI vs offline mock)
- [x] MockProvider (deterministic offline default, drives full tool loop)
- [x] OpenAI-compatible provider (chat completions + tool calling over reqwest)
- [x] Real agent loop in `AgentSession::run`
  - [x] provider step -> tool execution -> feed results back, up to MAX_TURNS
  - [x] cancellation checks between turns
  - [x] SSE events for messages, tool calls, completion, errors
- [x] Unit tests (provider family, parsing, mock behavior)
- [x] End-to-end agent-loop integration test (mock provider + echo tool)
- [x] Anthropic native adapter (messages API + tool_use/tool_result)
- [x] Gemini native adapter (generateContent + functionDeclarations)
- [ ] Streaming token-level provider output (currently per-turn)

# Phase 5b: Additional Tools
- [x] ShellTool (run_shell_command: cross-platform, cwd/env, cancellation, timeout)
- [x] FilesystemTool (read/write/list/delete, root-confined, traversal rejection)
- [ ] NetworkTool (HTTP requests, DNS lookup)
- [ ] ProcessTool (list, kill, monitor)
- [ ] SystemInfoTool (OS info, resources)

# Phase 6: Production Readiness
- [ ] Comprehensive error handling
- [ ] Structured logging (tracing)
- [ ] Metrics collection
- [ ] Performance benchmarks
- [ ] Security audit
- [ ] Documentation
  - [ ] API reference
  - [ ] Tool development guide
  - [ ] Deployment guide

# Phase 7: Integration
- [ ] Integrate with existing launcher code
- [ ] Migrate from subprocess to gateway
- [ ] Update Warp client to use gateway
- [ ] Migration guide

# Phase 8: CI/CD
- [ ] Unit tests for all modules
- [ ] Integration tests for adapters
- [ ] End-to-end tests
- [ ] Performance tests
- [ ] Security tests


# ============================================================
# ROADMAP: OZ-Compatible Managed Gateway (chốt 2026-06-14)
# ============================================================
# Mục tiêu: đưa warp_gateway_wrapper khớp thiết kế OZ gốc.
# Hiện trạng: Phase 1-5 xong (HTTP server, gateway core, agent
# session loop, multi-provider adapter OpenAI/Anthropic/Gemini/Mock,
# ShellTool + FilesystemTool, SSE stream, cancellation).

## Phase 6: Protocol Compatibility (SpawnAgentRequest/Response khớp OZ) [DONE 2026-06-14]
- [x] 6.1 enum UserQueryMode { Normal, Plan, Orchestrate } thay mode: Option<String>
- [x] 6.2 AgentConfig -> AgentConfigSnapshot (environment_id, model, temperature, max_tokens, system_prompt)
- [x] 6.3 attachments: Vec<FileAttachment> (path/name/mime/content) + wiring vào conversation
- [x] 6.4 initial_snapshot_token: Option<String> + parent_run_id: Option<String> (parse)
- [x] 6.5 backward-compat #[serde(default)]; cập nhật tests + ví dụ JSON
# Done khi: deserialize payload OZ mẫu; unit test mode enum + attachments.

## Phase 7: Harness Abstraction (oz vs delegate-CLI) [DONE 2026-06-14]
- [x] 7.1 enum HarnessType { Oz, Claude, Opencode, Gemini, Codex } + trait Harness + HarnessContext
- [x] 7.2 OzHarness: agent loop (provider + tools + streaming) trong harness/oz.rs
- [x] 7.3 CliHarness: delegate subprocess, stdout -> SSE, env override cmd/args, map exit code
- [x] 7.4 resolve_harness(req) theo config.harness (field mới trong AgentConfigSnapshot)
- [x] 7.5 Session delegate sang harness; tests (OzHarness e2e, CliHarness mock + launch failure)
# Done khi: OzHarness chạy như cũ + 1 CliHarness mock test.

## Phase 8: Conversation Trait Alignment (tùy chọn, khớp interface spec)
- [ ] 8.1 trait Conversation { id; step; submit_tool_result } bọc provider stateless
- [ ] 8.2 trait LLMAdapter::create_conversation
- [ ] 8.3 Refactor session dùng StepResult { Message, ToolCall, Complete }
# Lưu ý: đổi kiến trúc nội bộ, không đổi hành vi. Có thể bỏ qua.

## Phase 9: Authentication / Authorization [DONE 2026-06-14]
- [x] 9.1 Module http/auth.rs: AuthConfig, Identity, Authenticator, ToolPermission policy
- [x] 9.2 Middleware auth (Bearer / x-agent-identity-uid) cho /agent/* (health public); inject Identity
- [x] 9.3 Gắn identity vào session + enforce per-tool (Shell/Filesystem privileged; basic luôn cho phép)
- [x] 9.4 401/403 đúng chuẩn; subcommand serve + WARP_GATEWAY_TOKENS / --no-auth; tests
# Done khi: request không token bị từ chối; tool nhạy cảm gắn policy.

## Phase 10: Capacity & Lifecycle [DONE 2026-06-14]
- [x] 10.1 GatewayConfig (max_concurrent_tasks, completed_ttl_secs); at_capacity() + SpawnOutcome reject khi đầy
- [x] 10.2 list_executions() + running_count() + GET /agent/tasks (running/max/at_capacity/tasks)
- [x] 10.3 completed_at timestamp; cleanup_expired() + start_cleanup_reaper() (wired vào serve)
# Done khi: vượt ngưỡng -> at_capacity=true; reclaim tài nguyên.

## Phase 11: Remote Orchestration (Docker/Worker)
- [ ] 11.1 enum OrchestrationExecutionMode { Local, Remote { environment_id, worker_host } }
- [ ] 11.2 Local: chạy tại gateway (đã có)
- [ ] 11.3 Remote: spawn Docker theo environment_id, route worker_host, proxy SSE
- [ ] 11.4 Snapshot handoff (initial_snapshot_token) local -> cloud
- [ ] 11.5 Health/heartbeat worker, retry, cleanup container
# Done khi: spawn container chạy harness, stream về client; integration test container giả.

## Phase 12: MAA & Child Agents
- [ ] 12.1 parent_run_id spawn child agent, liên kết task cha-con
- [ ] 12.2 Định tuyến SSE / aggregate kết quả con về cha
- [ ] 12.3 Mode Orchestrate điều phối nhiều agent
# Done khi: 1 agent cha spawn >=1 con, tổng hợp kết quả.

## Phase 13: SSE Format Warp-Compat & Streaming token-level [DONE 2026-06-14]
- [x] 13.1 SSE wire format chuẩn EventSource: id/event/data lines, event_name(), MessageDelta event, StreamEnvelope
- [x] 13.2 Streaming token-level: session chunk assistant content -> MessageDelta + Message cuối
- [x] 13.3 Reconnect/replay (Last-Event-ID header + ?last_event_id): StreamManager history buffer + replay_since
# Done khi: Warp client subscribe và render đúng.

## Phase 14: Tools mở rộng & Production
- [x] 14.1a NetworkTool (http_request, SSRF guard) + SystemInfoTool (sysinfo); ProcessTool còn lại
- [x] 14.2a Structured logging (tracing) thay println!/eprintln!; init qua RUST_LOG
- [ ] 14.1b ProcessTool (list, kill, monitor)
- [ ] 14.2b Metrics, benchmarks
- [ ] 14.3 Security audit (Shell/Filesystem sandboxing, allowlist)
- [ ] 14.4 API reference + deployment guide

# Thứ tự ưu tiên:
# - Tương thích OZ protocol sớm cho Warp: 6 -> 9 -> 13 -> 7
# - Tái tạo đầy đủ OZ backend: 6 -> 7 -> 10 -> 11 -> 12
# - Phase 8 là tùy chọn thẩm mỹ kiến trúc.


## Phase P (Transparent Warp Proxy) [DONE 2026-06-14]
- [x] P.A.1 ProxyConfig + WarpChannel presets (production/staging/dev)
- [x] P.A.2 HTTP forward (REST/SSE byte pass-through, override Authorization, strip hop-by-hop, stream body)
- [x] P.A.3 Axum fallback router; tracing log per request
- [x] P.B.1 tokio-tungstenite WS client; bidirectional relay (rtc + sessions)
- [x] P.B.2 WS dispatch middleware (Upgrade: websocket -> ws_handler, else -> forward)
- [x] P.C.1 Subcommand proxy --channel|--upstream-* --port; OZ token tu env (WARP_GATEWAY_OZ_TOKEN / OZ_TOKEN / WARP_TOKEN)
- [x] Tests: header override, query/path passthrough, SSE streaming, no-token passthrough
- [ ] TLS termination (chi can khi client khong dung WARP_SERVER_ROOT_URL)
- [ ] IAP cho staging
- [ ] Per-identity token map

## Phase MPG (Managed Provider Gateway) [A DONE 2026-06-14]
Local OpenAI-compatible server cho Warp Agent (port mac dinh 8787, khop voi warp-oss).
- [x] MPG-A.1 ProviderConfig + GatewayConfig + WireApi/Adapter + ProvidersFile JSON loader
- [x] MPG-A.2 openai_chat adapter: forward /chat/completions, safe-mode strip tools/tool_choice/parallel_tool_calls
- [x] MPG-A.3 Server: /healthz, /v1/healthz, /v1/models, /v1/chat/completions; Bearer auth gating
- [x] MPG-A.4 Subcommand mpg --config <providers.json> --provider <name> voi env overrides
- [x] MPG-A.5 Tests: healthz, models, chat forward (auth rewrite), safe-mode strip, SSE relay, gateway auth gate
- [x] MPG-B Adapter openai_responses (chat <-> responses + SSE fallback chain output_text.delta/.done/content_part.done/completed)
- [x] MPG-C Duplicate-request guard (fingerprint model/stream/conversation/tool_count + sliding window + synthetic response)
- [x] MPG-D Provider probe (subcommand mpg-probe): capability matrix chat/responses x non_stream/streaming/tools + recommendation + diagnostics
- [x] MPG-E Native adapters anthropic_messages (/v1/messages) + gemini_generate_content (generateContent), convert ve Chat shape (non-stream)

Nguyen tac (giu nguyen tu warp-oss):
- Khong sua source Warp; Warp dung custom-endpoint mechanism san co tro vao gateway.
- Khong hardcode logic theo ten provider; chi qua adapter + compatibility group.
- Safe mode mac dinh bat (disable_tools=true, disable_mcp=true) de uu tien on dinh.
### MPG B-E ghi chu (2026-06-14)
- Native adapters (anthropic/gemini) hien convert non-stream de dam bao text luon hien; streaming native co the them sau.
- Probe la subcommand mpg-probe --base-url --env-key; chi 2xx tinh pass.
- Test: responses/anthropic/gemini convert, duplicate guard suppress, probe recommend, SSE relay.

# ============================================================
# PHASE L: Managed Gateway Launcher (UI vo boc) [PLAN - chot 2026-06-14]
# ============================================================
# Quyet dinh da chot:
# 1. UI framework: egui + eframe (single static binary, khong Node/webview).
# 2. Server: in-process (embed MpgServer trong tokio task), xu ly panic can than.
# 3. Warp setup: chi GUIDE + COPY config (KHONG ghi vao secure storage Warp).
# 4. Cloudflared: co tich hop trong launcher (Phase L-D mo rong).
#
# Crate moi: warp_gateway_launcher (cung workspace). Deps: egui, eframe, tokio,
# keyring (DPAPI/Keychain cho API key), tray-icon, arboard (clipboard).

## Phase L-A: Khung launcher [DONE 2026-06-14]
- [x] L-A.1 Crate warp_gateway_launcher (eframe/egui 0.29 + tokio); bin warp-gateway-launcher
- [x] L-A.2 Cua so chinh + tab navigation (Provider/Gateway/Warp setup)
- [ ] L-A.3 System tray (de Phase sau; chua bat buoc)
- [x] L-A.4 Embed MpgServer in-process: GatewayController (oneshot shutdown + JoinHandle + status enum); MpgServer::run_with_shutdown

## Phase L-B: Provider tab [DONE 2026-06-14]
- [x] L-B.1 Form CRUD provider + list (new/save/delete/select)
- [x] L-B.2 store.rs: doc/ghi config_dir/WarpGatewayLauncher/providers.json
- [x] L-B.3 API key KHONG plaintext: chi luu env_key ref tren dia; key value nhap runtime giu in-memory (keyring co the them sau)
- [x] L-B.4 Nut Probe -> mpg::probe (async tren runtime, khong block UI); hien capability matrix; nut Apply recommendation auto-fill adapter/wire_api/safe-mode

## Phase L-C: Gateway tab [DONE 2026-06-14]
- [x] L-C.1 Start/Stop + status mau + endpoint/health URL + nut Copy (arboard)
- [x] L-C.2 Toggles disable_tools/disable_mcp/duplicate_window_secs + field force_model_config_key
- [x] L-C.3 Log tail: LogBufferLayer (tracing) ring-buffer 500 dong + panel auto-scroll + Clear

## Phase L-D: Warp setup tab [DONE 2026-06-14]
- [x] L-D.1 detect_warp() quet path mac dinh (LocalAppData/Programs, ProgramFiles, mac/linux)
- [x] L-D.2 Install Warp qua winget (Warp.Warp)
- [x] L-D.3 endpoint_config_json (@gateway marker) + Copy JSON/URL clipboard + huong dan paste
- [x] L-D.4 Spawn Warp voi WARP_SERVER_ROOT_URL env (cho proxy mode)
- [x] L-D.5 Cloudflared: detect (override/PATH/winget paths) + install winget + start tunnel

## Phase L-E: Dong goi
- [ ] L-E.1 cargo bundle / MSI Windows; shortcut
- [ ] L-E.2 First-run: tao providers.json mac dinh, khong can env nao
- [ ] L-E.3 (Tuy chon) auto-update

# Nguyen tac bao mat / khong dung Warp:
# - Launcher chi paste/spawn/copy; KHONG ghi de secure storage hay config cua Warp.
# - Detect Warp = read-only (registry/path). Khong sua file Warp.
# - API key cua user luu qua OS keyring, khong plaintext.

# ============================================================
# PHASE CLEANUP: Don managed gateway cu [PLAN - chot 2026-06-14]
# ============================================================
# MUC TIEU: go bo tan du tu thiet ke launcher doi dau (truoc khi co
# mpg/proxy/serve subcommands) de tranh nham lan va loi build.
#
# === RANH GIOI AN TOAN (DOC KY TRUOC KHI XOA) ===
# - CHI duoc dong cham trong: crates/warp_gateway_wrapper/ va target/.
# - TUYET DOI KHONG xoa/sua: app/, crates/ai, crates/warp_core, crates/graphql,
#   crates/managed_secrets*, va moi crate khac cua Warp.
# - Khong xoa file ngoai workspace. Khong xoa .git, .codegraph.
#
# === HANG MUC CLEANUP (da dieu tra) ===
# C1. src/main.rs.old (16KB): launcher doi dau, goi process::spawn_gateway_if_configured
#     (khong con ton tai). KHONG build (duoi .old). -> XOA an toan.
# C2. src/launcher/process.rs (0 byte) + src/launcher/endpoint_store.rs (0 byte):
#     file rong, code da go do. -> XOA.
# C3. src/launcher/ (config.rs/mod.rs/platform/*): logic discovery Warp binary +
#     secure storage + spawn gateway process cua thiet ke cu. KHONG duoc dung
#     boi lib.rs (chi co pub mod launcher dead) hay main.rs.
#     -> Phuong an: GIU LAI config.rs (discovery Warp binary) vi Phase L-D.1 se
#        tai dung; nhung tach ra cho ro. Tam thoi: danh dau deprecated, KHONG xoa
#        ca thu muc cho toi khi Phase L-D dung lai. Go pub mod launcher khoi
#        lib.rs neu khong reference, hoac giu lai co chu dich.
# C4. Hai ten binary trong target/debug: warp-gateway-wrapper.exe (gach ngang, CU,
#     chi stdio/http) vs warp_gateway_wrapper.exe (gach duoi, MOI). Ten gach ngang
#     la tan du build cu. -> XOA file .exe cu trong target/ (an toan, chi la artifact).
# C5. warp-gateway-wrapper.toml (root repo): config cua launcher cu. Kiem tra con
#     ai dung khong; neu khong -> di chuyen vao docs/ hoac xoa.
# C6. example-config.toml trong crate: config launcher cu. Cap nhat hoac xoa.
#
# === THU TU AN TOAN ===
# 1. Build + test PASS lam moc (dam bao khong hong san).
# 2. Xoa C1, C2 (chac chan dead/empty). Build lai.
# 3. Quyet dinh C3: giu config.rs cho Phase L hay go han. Build lai.
# 4. Don C4 (exe cu). C5/C6 (toml cu) -> di chuyen docs.
# 5. Test full suite sau moi buoc.
#
# Tieu chi done: cargo build + test PASS; chi con 1 ten binary; khong con .old/empty;
# khong dong cham bat ky file Warp nao.

### CLEANUP DONE 2026-06-14 (phuong an 1)
- [x] C1 Xoa src/main.rs.old
- [x] C2 Xoa src/launcher/process.rs + endpoint_store.rs (rong); go khoi launcher/mod.rs
- [x] C3 launcher/mod.rs danh dau DEPRECATED + #![allow(dead_code)]; giu config.rs cho Phase L-D
- [x] C4 Khong con file .exe ten gach ngang trong target/ (da het); chi con warp_gateway_wrapper.exe
- [x] C5/C6 Di chuyen warp-gateway-wrapper.toml + example-config.toml -> docs/legacy/ (kem README)
- Khong dong cham bat ky file Warp nao. cargo test PASS 108 unit + integration, EXIT=0, 0 warning.
### Phase L-A ghi chu (2026-06-14)
- MpgServer them addr() + run_with_shutdown(future) cho graceful stop.
- GatewayController giu tokio runtime rieng; start/stop khong block UI; Drop tu stop.
- Gateway tab da chuc nang (Start/Stop, status, hien endpoint URL). Provider tab co form co ban (CRUD/probe -> L-B). Warp setup tab placeholder (-> L-D).
- System tray hoan sang sau (khong bat buoc cho L-A).
- Tests: controller start_then_stop + double_start_noop PASS (bind port 0).
### Phase L-B ghi chu (2026-06-14)
- Quyet dinh: KHONG dung keyring o L-B de tranh deps nen tang phuc tap. providers.json chi luu env_key (reference), khong luu api_key value. An toan hon plaintext. Keyring co the them sau neu can luu key tren dia.
- PendingProbe chay probe tren controller runtime, poll non-blocking moi frame.
- Tests: store upsert/to_config/json-omits-api_key + controller = 5 PASS.
### Phase L-C ghi chu (2026-06-14)
- LogBufferLayer la tracing Layer custom (MessageVisitor) day vao ring-buffer chia se voi UI.
- main.rs subscriber gom: EnvFilter + fmt layer (console) + LogBufferLayer (UI tail).
- Copy clipboard qua arboard, fail-silent.
- Tests: 7 PASS (controller 2 + store 3 + log_buffer 2).
### Phase L-D ghi chu (2026-06-14)
- warp_setup.rs: detect_warp/detect_cloudflared read-only (path-based, khong registry/winreg); install qua winget; spawn detached.
- endpoint_config_json dung @gateway marker (khop warp-oss); KHONG ghi vao secure storage Warp - chi copy clipboard de user paste.
- Cloudflared public URL hien o cua so cloudflared (capture tu dong la enhancement sau).
- Tests: 11 PASS (controller 2 + store 3 + log_buffer 2 + warp_setup 4).
### Fix 2026-06-14: Warp tu choi URL local
- Phat hien: Warp custom_inference_modal.rs validate_url() yeu cau HTTPS + chan loopback/private host. Vi vay http://127.0.0.1:8787/v1 bi tu choi (co y, khong phai bug).
- Giai phap (3 diem):
  1. warp_setup: start_cloudflared_tunnel tra CloudflaredTunnel (capture stdout/stderr, parse https://*.trycloudflare.com qua channel). extract_trycloudflare_url + tests.
  2. UI Cloudflared group: giu tunnel handle, poll public URL moi frame, hien Public URL + Copy /v1; Stop tunnel.
  3. UI Custom endpoint: dung public_url (HTTPS tunnel) khi co; canh bao mau vang khi chua co tunnel.
- Tests: 13 PASS. Release binary rebuild OK.
### Phương án 1 - "Launch all" [DONE 2026-06-14]
- LaunchPhase state machine: Idle / StartingGateway / StartingTunnel / WaitingPublicUrl / SpawningWarp / Done / Failed.
- Bar Launch all hien o moi tab; mot click chay tuan tu: start gateway -> start tunnel -> doi public URL -> copy endpoint JSON vao clipboard -> spawn Warp.
- spawn_warp(path) launch Warp khong env (MPG dung custom endpoint, khong can env override).
- Cancel/Reset button. Mau trang thai: vang khi busy, xanh khi Done, do khi Failed.
- Khong dung source Warp.
- Tests: 13 PASS.
### UI redesign + hide console [DONE 2026-06-14]
- Console window: #![windows_subsystem = "windows"] cho release; child process (cloudflared/winget) spawn voi CREATE_NO_WINDOW (no_window helper).
- UI Huong A (one-screen dashboard): bo tabs.
  - Status row 3 den: Gateway / Tunnel / Warp.
  - Provider: dropdown chon + Edit/New (form gap khi khong sua).
  - Launch all: nut lon + progress bar + label trang thai mau.
  - Warp endpoint: URL + Copy config JSON / Copy URL + canh bao khi chua co HTTPS.
  - Advanced (gap mac dinh): host/port/adapter/wire_api/force_model/safe-mode + Start/Stop gateway, Start/Stop tunnel, Install winget, Re-detect.
  - Logs (gap mac dinh).
- Theme dark; window 520x600.
- Tests: 13 PASS, 0 warning.
### UI restyle Huong 1 [DONE 2026-06-14]
- theme.rs: palette dark hien dai (BG #14171c, surface #1d222b, accent #4d8dff), rounding, spacing rong.
- Font: load Segoe UI (Windows) + Consolas mono, fallback default; text size to hon (body 14.5, heading 20).
- Layout card: moi section trong card_frame (surface + border + rounding 10 + padding 14).
- Nut Launch all accent xanh noi bat (170x38, text trang); progress bar accent.
- Header 2 mau; status dot mau theme.
- egui 0.29 API: Rounding/.rounding/Margin::same(f32).
- Tests: 13 PASS, 0 warning. Release rebuilt.
### Cross-platform (Windows + macOS) [DONE 2026-06-14]
- Xoa hoan toan src/launcher/ deprecated khoi warp_gateway_wrapper.
- Cargo.toml: bo deps Windows-only (windows, windows-registry, winreg cfg block) va deps thua (ai, warp_core, toml).
- Gateway crate gio doc lap voi Warp ecosystem; build trên macOS không bị warp_core/ai keo.
- Launcher: Windows-specific code (no_window/CommandExt/creation_flags) bao trong #[cfg(windows)]; #![windows_subsystem] cung chi tac dong Windows.
- detect_warp da co nhanh macOS (/Applications/Warp.app/...) va Linux.
- detect_cloudflared chi co nhanh Windows; can them macOS/Linux khi build mac.
- Tests: 103 wrapper + 13 launcher PASS, 0 warning. Release rebuilt.

### Text color fix [DONE 2026-06-14]
- Bo override_text_color = Some(TEXT) — cho per-widget/RichText color hoat dong.
- TEXT pure white #ffffff, TEXT_WEAK #c8ced8 (do tuong phan cao hon).
- pixels_per_point 1.2 cho text crisper tren high-DPI.
### Tauri (Phuong an 2) ABORTED -> quay ve egui (Phuong an B) [2026-06-14]
- Tauri v2 trong workspace wrap gap conflict semver khong giai duoc: tauri 2.x keo tauri-runtime 2.11.2 nhung tauri-runtime-wry 2.9.3 -> trait mismatch (eval_script_with_callback). Pin xuong gap webkit2gtk conflict o workspace level.
- Ket luan: khong kha thi trong workspace nay (deps Warp rang buoc). Da quay ve egui.
- egui launcher khoi phuc + polish: theme dark (BG #0b0d12, accent #4d8dff), font he thong, pixels_per_point 1.25, card layout, Launch all accent, Advanced + Logs collapsing.
- Xoa frontend/ Tauri thua, Cargo.lock sach (0 tauri entries).
- Tests: 13 PASS. Release rebuilt (12.1 MB).
- Neu sau nay van muon Tauri: tach launcher ra workspace rieng (ngoai wrap).