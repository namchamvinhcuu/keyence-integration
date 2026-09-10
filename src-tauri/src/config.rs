use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Thư mục gốc chứa log Keyence (có thể có thư mục con theo ngày —
    /// watcher quét đệ quy, xem `watcher.rs`).
    pub watch_folder: String,
    /// Chu kỳ quét thư mục (giây).
    pub poll_interval_secs: u64,
    /// Connection string PostgreSQL đích, dạng
    /// "host=... port=5432 user=... password=... dbname=...".
    pub postgres_conn_string: String,
    /// Tên bảng staging để ghi vào — KHÔNG ghi thẳng bảng nghiệp vụ Odoo,
    /// xem README §Thiết kế ghi DB.
    pub staging_table: String,
    /// URL manifest auto-update (`latest.json`) trên server nội bộ khách
    /// hàng — rỗng = TẮT tính năng auto-update (mặc định, cho tới khi IT
    /// cung cấp domain/path thật). `#[serde(default)]` để config.json cũ
    /// (chưa có field này) vẫn load được, không vỡ migration.
    #[serde(default)]
    pub update_server_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            watch_folder: default_watch_folder(),
            poll_interval_secs: 10,
            postgres_conn_string: String::new(),
            staging_table: "keyence_scan_log".to_string(),
            update_server_url: String::new(),
        }
    }
}

#[cfg(target_os = "windows")]
fn default_watch_folder() -> String {
    r"C:\LogsToWatch".to_string()
}

#[cfg(not(target_os = "windows"))]
fn default_watch_folder() -> String {
    "/var/log/keyence".to_string()
}

impl AppConfig {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Ghi ATOMIC (tmp file rồi rename) — tránh config hỏng nếu crash giữa
    /// chừng khi ghi.
    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, json)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");

        let mut cfg = AppConfig::default();
        cfg.watch_folder = "/custom/path".to_string();
        cfg.postgres_conn_string = "host=db user=u password=p dbname=mes".to_string();
        cfg.save(&path).unwrap();

        let loaded = AppConfig::load(&path);
        assert_eq!(loaded.watch_folder, "/custom/path");
        assert_eq!(loaded.postgres_conn_string, "host=db user=u password=p dbname=mes");
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let cfg = AppConfig::load(&path);
        assert_eq!(cfg.staging_table, "keyence_scan_log");
    }

    #[test]
    fn corrupted_config_falls_back_to_default_not_crash() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        let cfg = AppConfig::load(&path);
        assert_eq!(cfg.poll_interval_secs, 10);
    }

    /// Regression test cho migration `update_server_url`: config.json đã
    /// deploy ở trạm thật (viết TRƯỚC khi field này tồn tại) không có key
    /// `update_server_url`. `#[serde(default)]` phải cho phép load bình
    /// thường, KHÔNG lỗi/panic, và fallback về rỗng (= tắt auto-update).
    #[test]
    fn old_config_without_update_server_url_field_still_loads() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        // Giả lập config.json cũ — cố ý KHÔNG có key "update_server_url".
        std::fs::write(
            &path,
            r#"{
                "watch_folder": "/custom/path",
                "poll_interval_secs": 15,
                "postgres_conn_string": "host=db user=u password=p dbname=mes",
                "staging_table": "keyence_scan_log"
            }"#,
        )
        .unwrap();

        let cfg = AppConfig::load(&path);

        assert_eq!(cfg.watch_folder, "/custom/path");
        assert_eq!(cfg.poll_interval_secs, 15);
        assert_eq!(cfg.update_server_url, "");
    }
}
