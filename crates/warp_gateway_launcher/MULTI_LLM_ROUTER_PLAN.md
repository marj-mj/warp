# Multi-LLM Gateway Router — Kế hoạch triển khai

> Mục tiêu: biến Warp Gateway Launcher từ single-provider thành một **router đa
> nhà cung cấp tự host (self-hosted, kiểu OpenRouter/9router)**. Nhiều API key từ
> nhiều bên → 1 gateway local → phát hành **1 gateway key duy nhất** → Warp chỉ
> thấy một custom endpoint OpenAI với nhiều model.
>
> Ràng buộc tuyệt đối: **KHÔNG sửa source code của Warp.** Warp là bản cập nhật hệ
> thống. Mọi logic nằm trong `crates/warp_gateway_wrapper` (core) và
> `crates/warp_gateway_launcher` (UI/quản lý). Tương tác duy nhất với Warp là ghi
> 1 custom endpoint vào file `AiApiKeys` (DPAPI) — y như cơ chế hiện tại.

## 1. Bối cảnh hiện trạng (đã xác minh trong source)

- `crates/warp_gateway_wrapper/src/mpg/config.rs`
  - `GatewayConfig { provider: ProviderConfig, auth_token, disable_tools, ... }`
    — **một** provider duy nhất.
  - `ProviderConfig { name, base_url, model: Option<String>, wire_api, adapter,
    api_key, env_key }`.
  - `ProviderConfig::resolved_api_key()` đã ưu tiên `env_key` trước `api_key`.
- `crates/warp_gateway_wrapper/src/mpg/server.rs`
  - `MpgState { config, client (reqwest tái sử dụng), duplicate_guard }`.
  - Routes: `GET /healthz`, `GET /v1/healthz`, `GET /v1/models`,
    `POST /v1/chat/completions`.
  - `auth_middleware`: nếu `auth_token` rỗng → mở; nếu set → bắt buộc
    `Authorization: Bearer <token>`. **Đây là chỗ phát hành/verify gateway key.**
  - `chat_handler`: dispatch theo `state.config.provider.adapter`.
  - `models_handler`: chỉ advertise 1 model của provider duy nhất.
- `crates/warp_gateway_wrapper/src/mpg/openai_chat.rs`
  - `forward_chat_completions` đọc `state.config.provider.base_url` + key, **stream
    pass-through từng chunk** (giữ TTFT). Đây là tính chất phải bảo toàn.
- Adapter khác: `native.rs` (`forward_anthropic`, `forward_gemini`),
  `openai_responses.rs` (`forward_responses`).
- Launcher:
  - `src/store.rs`: `ProviderStore` lưu `providers.json` ở
    `<config_dir>/WarpGatewayLauncher/providers.json`. **Không bao giờ ghi API key
    thật** — chỉ `env_key`.
  - `src/commands.rs`: `start_gateway` / `start_launch_flow` build 1 `GatewayConfig`
    từ **một** `ProviderInput`.
  - `src/warp_setup.rs`: `upsert_warp_gateway_endpoint` / `remove_warp_gateway_endpoint`
    ghi/xóa custom endpoint trong DPAPI `AiApiKeys`. `is_warp_running`,
    `restart_warp` (kill + spawn theo yêu cầu thủ công).
- Hành vi hiện tại khi Warp đang chạy: ghi vào `AiApiKeys` luôn, hiện phase
  `PendingWarpRestart` + nút "Restart Warp". (Đã xác nhận Warp chỉ đọc `AiApiKeys`
  lúc khởi động — `crates/ai/src/api_keys.rs::ApiKeyManager::new`.)

## 2. Kiến trúc mục tiêu

```
                 (1 gateway key: wgk_xxx)
Warp ───────────────────────────────▶ Gateway 127.0.0.1:8787
  model = "openai/gpt-4o"                │
                                         │ 1. auth_middleware: verify gateway key
                                         │ 2. router.select(model) → ProviderConfig
                                         │ 3. rewrite model thật (bỏ prefix name/)
                                         │ 4. adapter convert (nếu cần)
                                         │ 5. forward + stream pass-through
                                         │ 6. fallback nếu provider lỗi
            ┌───────────┬────────────┬───┴────────┬───────────────┐
            ▼           ▼            ▼            ▼               ▼
        OpenAI     Anthropic      Gemini      Ollama         OpenRouter ...
       (key #1)    (key #2)      (key #3)   (no key)         (key #n)
```

- Warp giữ **1 key** (gateway key). 10 key upstream nằm kín trong gateway.
- Warp thấy **1 endpoint, nhiều model** (union qua `/v1/models`).

## 3. Quyết định thiết kế đã chốt

1. **Trùng tên model** giữa các provider → hiển thị `name/model` trong `/v1/models`
   (vd `openai/gpt-4o`, `azure/gpt-4o`). Router tách prefix để chọn provider rồi
   rewrite lại `model` thật trước khi forward.
2. **Fallback** chỉ trong **cùng `compatibility_group`** (tránh phải convert chéo
   định dạng). Cap bằng `max_fallback`.
3. **Gateway key**: 1 key chung cho cả gateway (v1). Random `wgk_<rand>`, đặt vào
   `auth_token`. Mở rộng multi-key (nhãn/quota) để sau.
4. **Key upstream**: hỗ trợ **cả 3 nguồn**, ưu tiên:
   `env_key` → **DPAPI (lưu mã hóa)** → in-memory (phiên). Không có → forward
   không kèm Authorization.
5. **Streaming pass-through** + **reqwest client tái sử dụng** phải giữ nguyên để
   không ảnh hưởng tốc độ (overhead ~1–6ms/request, <1% so với thời gian LLM).
6. Thứ tự build: **backend (Phase 0→3) + test trước**, rồi UI (Phase 4→5).
7. Giữ **backward-compat**: `GatewayConfig::new(single_provider)` vẫn hoạt động
   (bọc thành router 1 phần tử); JSON cũ vẫn parse được nhờ `#[serde(default)]`.

## 4. Bảo mật key

- **DPAPI** (Windows Data Protection API, theo user account) — đúng cơ chế Warp dùng
  cho `AiApiKeys`. Key chỉ giải mã được trên cùng máy + tài khoản.
- Key upstream mã hóa lưu **tách riêng** trong `secrets.dat` (DPAPI), KHÔNG trộn vào
  `providers.json` plaintext.
- Gateway key cũng lưu DPAPI để bền qua các phiên.
- Không log giá trị key. Khi hiển thị chỉ show nguồn: `env: NAME` / `saved` /
  `in-memory` / `none`.
- Non-Windows: DPAPI là no-op → fallback env/in-memory (giữ chạy được cross-platform).

## 5. Các Phase chi tiết

### Phase 0 — Nền dữ liệu (config + virtual key + secret store)
Files: `mpg/config.rs`, `mpg/mod.rs`, launcher `src/store.rs`, secret store mới.
- [ ] Mở rộng `ProviderConfig`: thêm `models: Vec<String>`, `tags: Vec<String>`,
      `priority: u32`, `enabled: bool` (tất cả `#[serde(default)]`). Giữ `model`
      đơn để tương thích (coi như phần tử đầu của `models`).
- [ ] Thêm `RouterConfig { providers: Vec<ProviderConfig>, default_provider:
      Option<String>, fallback: bool, max_fallback: u8 }`.
- [ ] `GatewayConfig.provider` → `GatewayConfig.router`. Thêm
      `GatewayConfig::new(single)` bọc router 1 phần tử (giữ chữ ký cũ).
- [ ] Secret store DPAPI (`launcher/src/secrets.rs`): `save_key(provider_name,
      key)`, `load_key(provider_name)`, `remove_key(...)`, `load_gateway_key/save`.
      Lưu `secrets.dat` mã hóa cạnh `providers.json`.
- [ ] `ProviderStore` lưu thêm `models/tags/priority/enabled` (vẫn không lưu key
      plaintext).
- Test: parse JSON cũ → router 1 provider; round-trip config mới; DPAPI no-op
  trên non-windows.

### Phase 1 — Router (chọn provider)
File mới: `mpg/router.rs`.
- [ ] `fn select<'a>(model: &str, providers: &'a [ProviderConfig]) ->
      Option<(&'a ProviderConfig, String /*model thật*/)>` theo precedence:
      1. prefix `name/model` hoặc `tag:model` → chọn provider, trả model đã bỏ prefix.
      2. exact match: provider có `models` chứa `model` (trùng → `priority` cao
         thắng, rồi tới thứ tự khai báo).
      3. `default_provider`.
- [ ] `fn fallback_candidates(primary, providers) -> Vec<&ProviderConfig>`: cùng
      `compatibility_group`, sắp theo `priority`, loại primary.
- Test bảng: nhiều case model → provider + model rewrite mong đợi; trùng tên;
  default; không match.

### Phase 2 — Dispatch nhiều provider
Files: `mpg/server.rs`, `mpg/openai_chat.rs`, `mpg/native.rs`,
`mpg/openai_responses.rs`.
- [ ] `MpgState` giữ `router` (đổi từ `config.provider`).
- [ ] Đổi chữ ký forward: nhận `&ProviderConfig` (provider đã chọn) thay vì đọc
      `state.config.provider`.
- [ ] `chat_handler`: đọc `model` → `router.select` → rewrite body `model` →
      dispatch theo `adapter` của provider được chọn.
- [ ] `models_handler`: union model tất cả provider `enabled`, id dạng `name/model`.
- [ ] `healthz`: liệt kê providers + group + enabled.
- Test: model A → provider A, model B → provider B (dùng `providers/mock.rs`).

### Phase 3 — Fallback
File: `mpg/server.rs` (+ helper trong `router.rs`).
- [ ] Bọc dispatch trong vòng lặp: lỗi (5xx / timeout / connection) → thử ứng viên
      fallback kế tiếp, tối đa `max_fallback`.
- [ ] Chỉ fallback khi **non-stream** hoặc **chưa gửi byte đầu của stream**.
- [ ] Log mỗi lần fallback (provider from→to, lý do).
- Test: primary trả 502 → fallback provider 2 thành công; vượt cap → trả lỗi cuối.

### Phase 4 — Launcher backend
Files: `launcher/src/commands.rs`, `state.rs`, `store.rs`, `secrets.rs`,
`warp_setup.rs`.
- [ ] `start_gateway`/`start_launch_flow` gom **các provider enabled** → build
      `RouterConfig`; resolve key theo env→DPAPI→in-memory; đính kèm gateway key.
- [ ] Command mới: `generate_gateway_key`, `get_gateway_key`,
      `save_provider_secret`, `delete_provider_secret`.
- [ ] `endpoint_config_json`: 1 URL + gateway key + danh sách model union.
- [ ] `upsert_warp_gateway_endpoint`: ghi gateway key vào field `api_key` của custom
      endpoint Warp (thay vì để trống), kèm danh sách models.
- Test: build router từ nhiều provider; resolve key đúng thứ tự ưu tiên.

### Phase 5 — UI (React)
Files: `ui/src/components/ProviderEditor.tsx`, `App.tsx`, `StatusSidebar.tsx`,
`HeroCard.tsx`, `lib/types.ts`, `lib/tauri.ts`, hooks.
- [ ] ProviderEditor: ô `models` (multi), `tags`, `priority`, toggle `enabled`,
      checkbox "Lưu key (DPAPI)", badge nguồn key.
- [ ] Sidebar/HeroCard: checkbox "đưa provider vào gateway"; nút "Generate gateway
      key" + hiển thị + copy; JSON config nhiều model.
- [ ] Cập nhật types khớp serde Rust (RouterConfig, ProviderConfig mở rộng).
- Build: `tsc -b && vite build`.

### Phase 6 — Build + đóng gói
- [ ] `cargo test -p warp_gateway_wrapper` + `-p warp_gateway_launcher`.
- [ ] `cargo tauri build` → `.exe` (NSIS) + `.msi`.
- [ ] Smoke test: 2–3 provider thật (vd OpenAI + Anthropic + Ollama), kiểm tra
      route đúng, fallback, model list trong Warp.

## 6. Tính tốc độ (đã phân tích)
- Overhead gateway ~1–6ms/request (hop localhost + parse + route + convert).
- LLM call thật 300ms–vài giây → overhead <1%, không cảm nhận được.
- Bắt buộc giữ: stream pass-through chunk-by-chunk; reqwest client tái sử dụng;
  không buffer toàn bộ response; fallback chỉ kích hoạt khi có lỗi.

## 7. Rủi ro & lưu ý
- Trùng tên model → bắt buộc namespace `name/model` để tránh route nhầm.
- Convert chéo định dạng (chat→Anthropic/Gemini) đã có ở `native.rs`; fallback
  giữ trong cùng group để khỏi convert chéo.
- Nếu giữa phiên user sửa key trong Settings của Warp, Warp ghi đè `AiApiKeys` từ
  RAM của nó → có thể đè endpoint launcher vừa ghi. Không tránh được nếu không sửa
  Warp (ngoài ràng buộc). Hiếm gặp; sẽ ghi rõ trong thông báo.
- DPAPI chỉ trên Windows; non-Windows fallback env/in-memory.

## 8. Trạng thái hiện tại của codebase (điểm bắt đầu phiên sau)
- Đã xong trước phiên này (chưa commit):
  - Provider delete giờ stop gateway + tunnel + gỡ endpoint khỏi Warp + cảnh báo.
  - Add khi Warp đang chạy: ghi `AiApiKeys` + phase `PendingWarpRestart` + nút
    "Restart Warp" (không tự kill session).
  - Fix nút "Start gateway flow" hiện lại khi chọn/tạo provider khác.
  - `remove_warp_gateway_endpoint`, `restart_warp` trong `warp_setup.rs`.
- Tests: `cargo test -p warp_gateway_launcher --lib` = 10/10 pass.
- Build installer OK (NSIS + MSI).
- **Việc tiếp theo: bắt đầu Phase 0.**

## 9. Checklist nhanh cho phiên sau
- [ ] Phase 0: config + RouterConfig + secret store DPAPI + tests
- [ ] Phase 1: router.rs select/fallback + tests
- [ ] Phase 2: dispatch nhiều provider + union models + tests
- [ ] Phase 3: fallback loop + tests
- [ ] Phase 4: launcher commands + gateway key + secrets
- [ ] Phase 5: UI
- [ ] Phase 6: build + đóng gói + smoke test
