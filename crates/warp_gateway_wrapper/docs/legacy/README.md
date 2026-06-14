# Legacy launcher config (DEPRECATED)

Các file TOML ở đây thuộc về thiết kế launcher đời đầu (`WrapperConfig` +
`managed-gateway.exe` subprocess), trước khi gateway có các subcommand
`mpg` / `proxy` / `serve`.

Chúng KHÔNG còn được đọc bởi binary hiện tại. Giữ lại làm tham chiếu lịch sử
cho Phase L (Managed Gateway Launcher). Cấu hình hiện hành dùng:

- `mpg` : flags `--upstream-*` hoặc `--config providers.json`, env `WARP_MANAGED_PROVIDER_*`.
- `serve` : env `WARP_GATEWAY_TOKENS`, cờ `--no-auth`.
- `proxy` : `--channel` / `--upstream-*`, env `WARP_GATEWAY_OZ_TOKEN`.

Xem `../../README.md` và `../../TODO.md` để biết chi tiết.
