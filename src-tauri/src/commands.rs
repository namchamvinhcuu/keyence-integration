use crate::config::AppConfig;
use crate::AppState;

#[tauri::command]
pub fn get_status(state: tauri::State<AppState>) -> crate::StatusSnapshot {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_config(state: tauri::State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_config(state: tauri::State<AppState>, config: AppConfig) -> Result<(), String> {
    config.save(&state.config_path).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

/// Thao tác thủ công từ UI: quên vị trí đã đọc của 1 file cụ thể để lượt
/// quét kế tiếp đọc lại TOÀN BỘ file đó từ đầu (dùng khi nghi ngờ có dòng
/// bị bỏ sót, hoặc test lại sau khi sửa cấu hình DB).
#[tauri::command]
pub fn force_resync(state: tauri::State<AppState>, file_path: String) -> Result<(), String> {
    let mut read_state = state.read_state.lock().unwrap();
    read_state.forget(&file_path);
    read_state.save(&state.state_path).map_err(|e| e.to_string())
}
