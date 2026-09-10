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

## Release + Auto-update

App tự động check cập nhật (tự tải + cài + khởi động lại, không hỏi công nhân) từ URL cấu
hình ở ô "URL server cập nhật" trong màn hình Cấu hình (`AppConfig.update_server_url`,
mặc định RỖNG = tắt tính năng). **Vì trạm sản xuất chỉ có LAN nội bộ (không ra Internet),
URL này PHẢI trỏ vào server nội bộ của khách hàng Youngmin — KHÔNG dùng GitHub Releases
làm endpoint runtime.**

### Quy trình phát hành bản mới

1. Bump version trong `src-tauri/tauri.conf.json` (`"version"`) + `package.json`.
2. Commit, tạo tag `vX.Y.Z`, push tag lên GitHub (`git push origin vX.Y.Z`) — trigger
   workflow `.github/workflows/release.yml` (build Windows + Linux, ký bằng signing key,
   tạo GitHub Release **draft**).
3. Vào tab Releases của repo (private), tải về: file installer (`.msi`/`.exe`/`.deb`/
   `.AppImage`), file `.sig` đi kèm mỗi installer, và `latest.json`.
4. **Upload thủ công** toàn bộ các file đó lên server nội bộ khách hàng Youngmin, đúng path
   khớp với URL sẽ điền vào `update_server_url` (vd `http://mes-server.youngmin.local/
   keyence-integration/latest.json` — domain/path thật do IT khách hàng cung cấp).
5. Trên từng trạm: mở app → Cấu hình → điền `update_server_url` trỏ đúng `latest.json` vừa
   upload → Lưu cấu hình. App tự check mỗi 6 giờ (và lúc khởi động), phát hiện bản mới sẽ
   tự tải/cài/khởi động lại — không cần thao tác gì thêm từ công nhân.

### Signing key (⚠ quan trọng)

Update được ký bằng Ed25519 keypair (minisign), sinh qua `npm run tauri signer generate`.
Private key + password lưu tại `.tauri-signing-key/` (đã `.gitignore`, KHÔNG commit) —
**PHẢI backup private key + password ra nơi an toàn ngoài máy dev** (password manager) vì
mất key = mọi máy đã cài (build với public key cũ) không nhận update mới được nữa. Set làm
GitHub Actions secret (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`)
để CI ký được khi build.

## Kiến trúc

Xem `.obsidian-vault/Index.md` (Architecture/Tech-Stack, DB-Write-Design,
CSV-Log-Format-Assumption, Data-Loss-Prevention) để hiểu các quyết định thiết kế + giả định
CHƯA verify với log thật cần đối chiếu trước khi go-live.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
