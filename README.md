# Keyence Integration

App Tauri v2 (Rust + webview UI) chạy tại trạm sản xuất Youngmin — theo dõi thư mục log
CSV do camera Keyence ghi ra, ghép mỗi lần quét QR sản phẩm (FG serial no) với đúng lệnh
sản xuất (MO), rồi đẩy dữ liệu về PostgreSQL của MES (Odoo, project `youngmin-odoo`).

Nguồn thiết kế: `Youngmin_MES_Keyence_Integration_VI.pptx`.

**Phạm vi:** CHỈ app chạy tại trạm (đọc log → ghi DB). Các thay đổi phía Odoo (nút in tem
MO trên module MMS, quy luật tạo tem túi bóng) thuộc repo `youngmin-odoo`, ngoài phạm vi
project này.

## Yêu cầu hệ thống

- **Windows 10/11 (64-bit)** — hỗ trợ đầy đủ, WebView2 chính thức.
- **Windows 10 (32-bit)** — hỗ trợ, build qua target `i686-pc-windows-msvc` + WebView2
  Runtime bản x86.
- **Windows 7/8/8.1** — ⚠ KHÔNG được hỗ trợ chính thức (WebView2/Chromium đã bỏ hỗ trợ các
  bản này từ ~2023, Rust std Tier-1 yêu cầu tối thiểu Windows 10 từ 1.78). Best-effort,
  chưa test — xem `.obsidian-vault/Architecture/Windows-Version-Support-Matrix.md`.
- **Linux (x64)** — cần `webkit2gtk-4.1` + các gói dev liệt kê ở `CLAUDE.md`.

## Chạy dev

```bash
npm install
npm run tauri dev
```

## Test

```bash
cd src-tauri && cargo test
```

## Build

```bash
npm run tauri build                              # theo OS hiện tại
npm run tauri build -- --target i686-pc-windows-msvc   # Windows 32-bit (build trên Windows)
```

**Lưu ý:** build Windows (MSI/NSIS) nên chạy TRÊN Windows hoặc CI runner `windows-latest` —
không cross-compile tốt từ Linux như `dotnet publish`.

## Kiến trúc

Xem `.obsidian-vault/Index.md` (Architecture/Tech-Stack, DB-Write-Design,
CSV-Log-Format-Assumption, Data-Loss-Prevention) để hiểu các quyết định thiết kế + giả định
CHƯA verify với log thật cần đối chiếu trước khi go-live.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
