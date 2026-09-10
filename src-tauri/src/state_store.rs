use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Theo dõi vị trí (byte offset) đã xử lý XONG (đã gửi DB thành công) của
/// từng file log, để restart không gửi trùng và không bỏ sót dòng (yêu cầu
/// ở slide 9 của thiết kế: "ghi nhớ vị trí đã đọc trong file").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadState {
    /// key = đường dẫn tuyệt đối file log, value = byte offset đã xử lý xong.
    pub file_offsets: HashMap<String, u64>,
}

impl ReadState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Ghi ATOMIC (tmp file rồi rename) — tránh state file hỏng nếu crash
    /// giữa chừng khi ghi.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }

    pub fn offset_for(&self, file_path: &str) -> u64 {
        self.file_offsets.get(file_path).copied().unwrap_or(0)
    }

    pub fn set_offset(&mut self, file_path: &str, offset: u64) {
        self.file_offsets.insert(file_path.to_string(), offset);
    }

    /// Dùng cho thao tác "force resync" từ UI — quên vị trí đã đọc của 1
    /// file để đọc lại từ đầu.
    pub fn forget(&mut self, file_path: &str) {
        self.file_offsets.remove(file_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("state.json");

        let mut state = ReadState::default();
        state.set_offset("/logs/2026-09-10/vision.log", 1234);
        state.save(&state_path).unwrap();

        let loaded = ReadState::load(&state_path);
        assert_eq!(loaded.offset_for("/logs/2026-09-10/vision.log"), 1234);
    }

    #[test]
    fn missing_file_returns_default_zero_offset() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("does-not-exist.json");
        let state = ReadState::load(&state_path);
        assert_eq!(state.offset_for("/anything"), 0);
    }

    #[test]
    fn corrupted_state_file_falls_back_to_default_not_crash() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        std::fs::write(&state_path, "{ not valid json").unwrap();
        let state = ReadState::load(&state_path);
        assert_eq!(state.offset_for("/anything"), 0);
    }

    #[test]
    fn save_does_not_leave_tmp_file_behind() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let state = ReadState::default();
        state.save(&state_path).unwrap();
        assert!(state_path.exists());
        assert!(!state_path.with_extension("json.tmp").exists());
    }

    #[test]
    fn forget_resets_offset_to_zero() {
        let mut state = ReadState::default();
        state.set_offset("/a", 500);
        state.forget("/a");
        assert_eq!(state.offset_for("/a"), 0);
    }
}
