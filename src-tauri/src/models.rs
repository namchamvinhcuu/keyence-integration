use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// Một dòng log camera Keyence đã parse xong.
///
/// ⚠ Field `judgements` là danh sách kết quả kiểm tra theo TỪNG công đoạn
/// (số lượng phụ thuộc cấu hình máy — xem `parser.rs`), KHÔNG cố định 3 phần tử.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScanRecord {
    pub sequence: i64,
    pub timestamp: NaiveDateTime,
    pub judgements: Vec<String>,
    pub qr_serial: String,
    pub mo: String,
    /// sha256(raw line) — khoá chống trùng khi gửi lại (resend không tạo record trùng).
    pub record_key: String,
}

impl ScanRecord {
    /// PASS tổng khi TẤT CẢ judgement đều "OK" (không phân biệt hoa/thường).
    ///
    /// ⚠ ASSUMPTION nghiệp vụ — slide thiết kế không nói rõ quy tắc tổng hợp
    /// khi có NHIỀU công đoạn kiểm tra; xác nhận lại với khách trước khi dùng
    /// giá trị này để quyết định PASS/NG hiển thị trên MES.
    pub fn overall_ok(&self) -> bool {
        !self.judgements.is_empty() && self.judgements.iter().all(|j| j.eq_ignore_ascii_case("OK"))
    }
}
