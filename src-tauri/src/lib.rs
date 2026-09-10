mod commands;
mod config;
mod db;
mod models;
mod parser;
mod state_store;
mod watcher;

use config::AppConfig;
use state_store::ReadState;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, WindowEvent};

/// Ảnh chụp trạng thái hiện tại, hiển thị lên dashboard UI.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct StatusSnapshot {
    pub last_poll_at: Option<String>,
    pub last_mo: Option<String>,
    pub records_sent_total: u64,
    pub last_parse_error: Option<String>,
    pub last_db_error: Option<String>,
    pub db_connected: bool,
    pub last_update_check_at: Option<String>,
    pub last_update_error: Option<String>,
}

/// Chu kỳ check auto-update — dài hơn hẳn poll log vì đây là việc hạ tầng,
/// không cần gấp như dữ liệu sản xuất.
const UPDATE_CHECK_INTERVAL_SECS: u64 = 6 * 60 * 60;

pub struct AppState {
    pub config_path: PathBuf,
    pub state_path: PathBuf,
    pub config: Mutex<AppConfig>,
    pub read_state: Mutex<ReadState>,
    pub status: Mutex<StatusSnapshot>,
}

fn app_data_dir(app: &tauri::App) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Một lượt quét: đọc dòng mới của mọi file trong `root`, parse, ghi DB.
///
/// ⚠ Cơ chế chống mất dữ liệu: offset của 1 file CHỈ được commit (lưu vào
/// `state`) sau khi dòng đó đã parse hợp lệ VÀ ghi DB thành công (hoặc dòng
/// lỗi vĩnh viễn — parse fail thì bỏ qua luôn, không kẹt mãi). Nếu ghi DB
/// thất bại, dừng xử lý file đó ngay — dòng thất bại và mọi dòng sau nó
/// KHÔNG bị mất, sẽ được đọc lại đúng nguyên vẹn ở lượt quét kế tiếp, vì bản
/// thân file log (Keyence không xoá) chính là bộ đệm (xem slide 9 thiết kế).
async fn poll_cycle(
    root: &std::path::Path,
    state: &mut ReadState,
    pool: &deadpool_postgres::Pool,
    table: &str,
    status: &Mutex<StatusSnapshot>,
) {
    for path in watcher::list_log_files(root) {
        let path_key = path.to_string_lossy().to_string();
        let offset = state.offset_for(&path_key);

        // Fast-path: file cũ đã đọc hết, kích thước không đổi từ lần trước
        // -> bỏ qua ngay, KHÔNG mở file. Tích luỹ file theo ngày càng lâu
        // càng nhiều file cũ tĩnh, tối ưu này giữ chi phí mỗi lượt poll
        // không phình theo tổng số file cũ (xem watcher::has_new_content).
        if !watcher::has_new_content(&path, offset) {
            continue;
        }

        let lines = match watcher::read_new_lines(&path, offset) {
            Ok(lines) => lines,
            Err(e) => {
                log::error!("Đọc file log {} thất bại: {e}", path.display());
                status.lock().unwrap().last_parse_error =
                    Some(format!("{}: {e}", path.display()));
                continue;
            }
        };

        for (line, line_offset) in lines {
            if line.trim().is_empty() {
                state.set_offset(&path_key, line_offset);
                continue;
            }
            match parser::parse_line(&line) {
                Err(e) => {
                    // Dòng hỏng vĩnh viễn (sai format) — log lỗi nhưng VẪN
                    // commit offset, tránh kẹt mãi ở 1 dòng không bao giờ
                    // parse được.
                    log::warn!("Dòng log sai format, bỏ qua: {e} — raw: {line}");
                    status.lock().unwrap().last_parse_error = Some(e.to_string());
                    state.set_offset(&path_key, line_offset);
                }
                Ok(record) => match db::insert_record(pool, table, &record).await {
                    Ok(()) => {
                        state.set_offset(&path_key, line_offset);
                        let mut s = status.lock().unwrap();
                        s.records_sent_total += 1;
                        s.last_mo = Some(record.mo.clone());
                        s.last_db_error = None;
                        s.db_connected = true;
                    }
                    Err(e) => {
                        log::error!("Ghi DB thất bại (MO {}): {e}", record.mo);
                        let mut s = status.lock().unwrap();
                        s.last_db_error = Some(e.to_string());
                        s.db_connected = false;
                        drop(s);
                        // KHÔNG commit offset — dừng file này, thử lại lượt sau.
                        return;
                    }
                },
            }
        }
    }
}

/// Check + tự động tải-cài bản cập nhật mới từ server nội bộ khách hàng.
///
/// ⚠ `endpoint` đến từ `AppConfig.update_server_url` (cấu hình qua UI, giống
/// `postgres_conn_string`) — KHÔNG phải endpoint tĩnh trong `tauri.conf.json`,
/// vì trạm sản xuất chỉ có LAN nội bộ, mỗi khách hàng/site có thể có domain
/// server nội bộ khác nhau. `pubkey` để verify chữ ký VẪN lấy từ
/// `tauri.conf.json` (baked lúc build, giống nhau cho mọi trạm cùng 1 khoá ký).
///
/// Không tìm thấy bản mới hoặc lỗi mạng → chỉ ghi vào `status`, KHÔNG panic —
/// vòng lặp nền phải sống sót qua lỗi tạm thời (server nội bộ down, v.v.).
async fn check_and_install_update(app: &tauri::AppHandle, endpoint: &str, status: &Mutex<StatusSnapshot>) {
    use tauri_plugin_updater::UpdaterExt;

    status.lock().unwrap().last_update_check_at = Some(chrono::Local::now().to_rfc3339());

    let url = match url::Url::parse(endpoint) {
        Ok(u) => u,
        Err(e) => {
            log::error!("URL update không hợp lệ ({endpoint}): {e}");
            status.lock().unwrap().last_update_error = Some(format!("URL update không hợp lệ: {e}"));
            return;
        }
    };

    let updater = match app.updater_builder().endpoints(vec![url]) {
        Ok(builder) => builder,
        Err(e) => {
            log::error!("Cấu hình updater lỗi: {e}");
            status.lock().unwrap().last_update_error = Some(e.to_string());
            return;
        }
    };
    let updater = match updater.build() {
        Ok(u) => u,
        Err(e) => {
            log::error!("Khởi tạo updater lỗi: {e}");
            status.lock().unwrap().last_update_error = Some(e.to_string());
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            log::info!(
                "Phát hiện bản cập nhật mới: {} -> {}, đang tải + cài...",
                update.current_version,
                update.version
            );
            if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
                log::error!("Tải/cài bản cập nhật thất bại: {e}");
                status.lock().unwrap().last_update_error =
                    Some(format!("Tải/cài bản cập nhật thất bại: {e}"));
                return;
            }
            log::info!("Cài đặt bản cập nhật thành công, chuẩn bị khởi động lại.");
            status.lock().unwrap().last_update_error = None;
            // Windows: download_and_install() đã tự thoát app để installer
            // (NSIS/MSI) ghi đè file đang chạy rồi tự khởi động lại (/R).
            // macOS/Linux: phải tự relaunch để chạy đúng bản vừa cài.
            #[cfg(not(windows))]
            app.request_restart();
        }
        Ok(None) => {
            status.lock().unwrap().last_update_error = None;
        }
        Err(e) => {
            log::error!("Check update thất bại: {e}");
            status.lock().unwrap().last_update_error = Some(e.to_string());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            // Ghi log ra rotating file (thư mục log app, xem `tauri-plugin-log`
            // docs) — app chạy packaged/nền tại trạm sản xuất, KHÔNG ai xem
            // stdout, nên bắt buộc phải có kênh persistent để debug sau này
            // (đặc biệt quan trọng với auto-update: lỗi ở trạm xa không SSH
            // vào được). Giữ 5 file x 5MB (KeepSome(5)) — đủ lịch sử vài
            // ngày cho 1 app poll log liên tục, không phình vô hạn.
            tauri_plugin_log::Builder::new()
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            log::info!("keyence-integration khởi động (version {})", env!("CARGO_PKG_VERSION"));

            let data_dir = app_data_dir(app);
            let config_path = data_dir.join("config.json");
            let state_path = data_dir.join("read-state.json");

            let config = AppConfig::load(&config_path);
            let read_state = ReadState::load(&state_path);

            app.manage(AppState {
                config_path: config_path.clone(),
                state_path: state_path.clone(),
                config: Mutex::new(config),
                read_state: Mutex::new(read_state),
                status: Mutex::new(StatusSnapshot::default()),
            });

            // Tray icon — đóng cửa sổ chỉ ẩn (minimize-to-tray), app vẫn
            // chạy nền theo dõi log; "Thoát" mới thật sự dừng process.
            let show_item =
                tauri::menu::MenuItem::with_id(app, "show", "Hiện cửa sổ", true, None::<&str>)?;
            let quit_item =
                tauri::menu::MenuItem::with_id(app, "quit", "Thoát", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show_item, &quit_item])?;
            tauri::tray::TrayIconBuilder::new()
                .menu(&menu)
                .icon(app.default_window_icon().unwrap().clone())
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            // Vòng lặp nền: poll folder log -> parse -> ghi PostgreSQL.
            // Chạy độc lập với UI — đóng cửa sổ (ẩn xuống tray) KHÔNG dừng
            // vòng lặp này.
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let state_handle = app_handle.state::<AppState>();
                    let (watch_folder, poll_interval, conn_string, table) = {
                        let cfg = state_handle.config.lock().unwrap();
                        (
                            cfg.watch_folder.clone(),
                            cfg.poll_interval_secs.max(1),
                            cfg.postgres_conn_string.clone(),
                            cfg.staging_table.clone(),
                        )
                    };

                    if conn_string.trim().is_empty() {
                        // Chưa cấu hình DB qua UI — chờ rồi thử lại, không
                        // báo lỗi (đây là trạng thái ban đầu bình thường).
                        tokio::time::sleep(std::time::Duration::from_secs(poll_interval)).await;
                        continue;
                    }

                    let pool = match db::build_pool(&conn_string) {
                        Ok(p) => p,
                        Err(e) => {
                            log::error!("Kết nối PostgreSQL thất bại: {e}");
                            state_handle.status.lock().unwrap().last_db_error =
                                Some(e.to_string());
                            tokio::time::sleep(std::time::Duration::from_secs(poll_interval))
                                .await;
                            continue;
                        }
                    };
                    if let Err(e) = db::ensure_schema(&pool, &table).await {
                        log::error!("Tạo bảng staging thất bại: {e}");
                        state_handle.status.lock().unwrap().last_db_error = Some(e.to_string());
                        tokio::time::sleep(std::time::Duration::from_secs(poll_interval)).await;
                        continue;
                    }

                    let root = PathBuf::from(&watch_folder);
                    // Clone state ra ngoài Mutex trước khi await (tránh giữ
                    // std::sync::MutexGuard xuyên điểm await), ghi lại sau
                    // khi xử lý xong. An toàn vì chỉ 1 task nền duy nhất
                    // truy cập read_state — không có tranh chấp thật.
                    let mut rs_clone = { state_handle.read_state.lock().unwrap().clone() };
                    poll_cycle(&root, &mut rs_clone, &pool, &table, &state_handle.status).await;
                    if let Err(e) = rs_clone.save(&state_handle.state_path) {
                        log::error!("Ghi read-state.json thất bại: {e}");
                    }
                    *state_handle.read_state.lock().unwrap() = rs_clone;

                    state_handle.status.lock().unwrap().last_poll_at =
                        Some(chrono::Local::now().to_rfc3339());

                    tokio::time::sleep(std::time::Duration::from_secs(poll_interval)).await;
                }
            });

            // Vòng lặp nền riêng cho auto-update — chu kỳ dài hơn hẳn poll
            // log, chạy độc lập, KHÔNG chặn vòng poll log nếu server update
            // chậm/down.
            let app_handle_updater = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let state_handle = app_handle_updater.state::<AppState>();
                    let update_server_url = {
                        let cfg = state_handle.config.lock().unwrap();
                        cfg.update_server_url.clone()
                    };

                    if !update_server_url.trim().is_empty() {
                        check_and_install_update(
                            &app_handle_updater,
                            update_server_url.trim(),
                            &state_handle.status,
                        )
                        .await;
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(UPDATE_CHECK_INTERVAL_SECS))
                        .await;
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_config,
            commands::save_config,
            commands::force_resync,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const LINE_1: &str =
        "301,2026,8,26,13,34,40,OK,OK,OK,'400111062103002382600605,MMO-26052028-001-P005";

    /// `deadpool_postgres::Pool::builder(...).build()` KHÔNG kết nối ngay —
    /// connection thật chỉ xảy ra khi `.get().await` được gọi (bên trong
    /// `db::insert_record`). Vì vậy có thể build 1 Pool trỏ tới địa chỉ
    /// không ai lắng nghe để test `poll_cycle` mà KHÔNG cần PostgreSQL thật —
    /// miễn code không đi vào nhánh gọi `pool.get()`.
    fn unreachable_pool() -> deadpool_postgres::Pool {
        db::build_pool("host=127.0.0.1 port=1 dbname=nope user=nope connect_timeout=1")
            .expect("build_pool chỉ parse config, không kết nối -> luôn Ok")
    }

    #[tokio::test]
    async fn poll_cycle_skips_db_entirely_when_no_new_content() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("vision.log");
        std::fs::write(&log_path, format!("{LINE_1}\n")).unwrap();
        let path_key = log_path.to_string_lossy().to_string();
        let size = std::fs::metadata(&log_path).unwrap().len();

        let mut state = ReadState::default();
        state.set_offset(&path_key, size); // offset == size -> has_new_content == false

        let pool = unreachable_pool();
        let status = Mutex::new(StatusSnapshot::default());

        poll_cycle(dir.path(), &mut state, &pool, "keyence_scan_log", &status).await;

        // Fast-path phải chặn TRƯỚC khi chạm DB: offset không đổi, không ghi
        // nhận lỗi DB nào, không có record nào được tính là đã gửi — nếu
        // `has_new_content` bị bỏ qua (mutation "luôn continue" ngược lại,
        // hoặc bug ngược "không bao giờ skip"), test này phân biệt được nhờ
        // test contrast bên dưới (proves đường đi tới DB thực sự khác nhau).
        assert_eq!(state.offset_for(&path_key), size);
        let s = status.lock().unwrap();
        assert!(s.last_db_error.is_none());
        assert_eq!(s.records_sent_total, 0);
    }

    #[tokio::test]
    async fn poll_cycle_attempts_db_when_content_is_new() {
        // Đối chứng cho test trên: file CÓ dữ liệu mới (offset mặc định 0,
        // size > 0) -> poll_cycle PHẢI đi tới bước gọi DB (dù DB không tồn
        // tại nên insert lỗi), KHÔNG được skip. Chứng minh test trước không
        // "luôn pass bất kể điều kiện" (tautology) mà thật sự phân nhánh
        // theo has_new_content.
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("vision.log");
        std::fs::write(&log_path, format!("{LINE_1}\n")).unwrap();
        let path_key = log_path.to_string_lossy().to_string();

        let mut state = ReadState::default(); // offset mặc định 0 < size

        let pool = unreachable_pool();
        let status = Mutex::new(StatusSnapshot::default());

        poll_cycle(dir.path(), &mut state, &pool, "keyence_scan_log", &status).await;

        // DB không kết nối được -> insert lỗi -> offset KHÔNG được commit,
        // và last_db_error phải được ghi nhận (chứng tỏ code đã thật sự đi
        // vào nhánh gọi `db::insert_record`, không bị skip nhầm).
        assert_eq!(state.offset_for(&path_key), 0);
        assert!(status.lock().unwrap().last_db_error.is_some());
    }
}
