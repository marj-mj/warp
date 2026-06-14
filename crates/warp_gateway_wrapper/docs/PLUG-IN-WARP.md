# Cắm Managed Provider Gateway vào Warp (hướng dẫn nhanh)

Không sửa source Warp. Chỉ cấu hình phía Warp + chạy gateway.

## 0. Binary đã build sẵn
- target\release\warp_gateway_wrapper.exe   (gateway CLI)
- target\release\warp-gateway-launcher.exe  (UI)

## 1. Chạy gateway (chọn 1 trong 2 cách)

### Cách A — UI (khuyên dùng)
1. Mở: target\release\warp-gateway-launcher.exe
2. Tab Provider:
   - Name: NROUTER (hoặc tên bạn muốn)
   - Base URL: https://api.trannhatcse.tokyo/v1
   - Model: gpt-5.5
   - Env key: NROUTER_KEY   (đặt biến môi trường này = API key trước khi mở UI)
   - Bấm Probe -> Apply recommendation -> Save
3. Tab Gateway: bấm Start gateway. Ghi lại endpoint hiện ra (vd http://127.0.0.1:8787/v1).

### Cách B — CLI
```powershell
$env:NROUTER_KEY = "<api-key>"
.\target\release\warp_gateway_wrapper.exe mpg `
  --port 8787 `
  --upstream-base-url https://api.trannhatcse.tokyo/v1 `
  --upstream-env-key NROUTER_KEY `
  --upstream-model gpt-5.5 `
  --upstream-wire-api responses `
  --upstream-adapter openai_responses
```

## 2. Verify gateway sống
```powershell
curl http://127.0.0.1:8787/healthz
curl http://127.0.0.1:8787/v1/models
```
healthz phải trả adapter/wire_api/default_model. models phải liệt kê model.

## 3. Cấu hình Warp (không sửa source)
Trong Warp: Settings -> AI -> Custom endpoints (Bring Your Own Key):
- Name: @gateway Managed Provider Gateway
- URL: http://127.0.0.1:8787/v1
- API key: để trống (gateway không bắt buộc) HOAC set WARP_MANAGED_PROVIDER_GATEWAY_AUTH_TOKEN rồi điền cùng giá trị
- Model: gpt-5.5  (config_key tùy ý)

Mẹo: tab "Warp setup" trong launcher có nút Copy JSON để dán nhanh.

## 4. Test trong Warp
- Chọn model gpt-5.5 (Managed Provider Gateway)
- Gõ prompt đơn giản: "say hello in one sentence"
- Phải thấy text trả về.

## 5. Khi có lỗi — bật log chi tiết
Chạy gateway với:
```powershell
$env:RUST_LOG = "info,warp_gateway_wrapper=debug"
```
Gửi tôi log + triệu chứng (vd: status 200 nhưng UI rỗng, hoặc duplicate guard chặn nhầm).

## Các triệu chứng thường gặp
- UI rỗng dù 200: provider trả Responses nhưng adapter để chat -> đổi --upstream-wire-api responses --upstream-adapter openai_responses (hoặc Probe lại).
- Bị spam request/đốt token: duplicate guard đang giúp; chỉnh duplicate_window_secs nếu cần.
- Tool lỗi: safe mode đang strip tools (disable_tools=true). Giữ vậy cho ổn định.

## QUAN TRONG: Warp yeu cau HTTPS public URL
Warp tu choi URL local/HTTP (vd http://127.0.0.1:8787/v1) — validate_url() bat buoc HTTPS + khong loopback.
=> Phai dung cloudflared tunnel:
1. Tab Warp setup -> Cloudflared: Install via winget (neu chua co) -> Re-detect -> Start tunnel.
2. Cho dong "Public URL" hien ra (vd https://abc-xyz.trycloudflare.com).
3. Bam Copy /v1 -> URL la https://abc-xyz.trycloudflare.com/v1.
4. Paste URL HTTPS nay vao Warp Custom endpoint (KHONG dung 127.0.0.1).
Phan endpoint config JSON trong UI tu dong dung public URL khi tunnel da chay.
