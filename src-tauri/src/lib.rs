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
}

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
        let lines = match watcher::read_new_lines(&path, offset) {
            Ok(lines) => lines,
            Err(e) => {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
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
                            state_handle.status.lock().unwrap().last_db_error =
                                Some(e.to_string());
                            tokio::time::sleep(std::time::Duration::from_secs(poll_interval))
                                .await;
                            continue;
                        }
                    };
                    if let Err(e) = db::ensure_schema(&pool, &table).await {
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
                    let _ = rs_clone.save(&state_handle.state_path);
                    *state_handle.read_state.lock().unwrap() = rs_clone;

                    state_handle.status.lock().unwrap().last_poll_at =
                        Some(chrono::Local::now().to_rfc3339());

                    tokio::time::sleep(std::time::Duration::from_secs(poll_interval)).await;
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
