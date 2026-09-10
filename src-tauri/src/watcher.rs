//! Quét thư mục log Keyence + đọc phần MỚI của từng file.
//!
//! Module này CHỦ ĐÍCH không biết gì về parse/DB — chỉ lo I/O file, để test
//! được độc lập không cần DB thật. Ghép với parser/db nằm ở `lib.rs`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Liệt kê MỌI file thường (đệ quy) trong `root` — Keyence tạo folder mới
/// mỗi ngày (theo thiết kế) nên KHÔNG giả định chỉ có 1 cấp thư mục hay 1
/// đuôi file cụ thể.
pub fn list_log_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(list_log_files(&path));
        } else if path.is_file() {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Đọc các dòng MỚI đã hoàn chỉnh (kết thúc `\n`) kể từ `offset`, trả về
/// từng dòng kèm offset TÍCH LŨY ngay sau dòng đó.
///
/// Trả offset theo TỪNG DÒNG (không phải 1 offset cuối cùng) để caller có
/// thể commit offset theo từng dòng — dừng sớm & KHÔNG commit nếu 1 dòng
/// gửi DB thất bại, để lượt quét sau đọc lại đúng dòng đó (file log = bộ
/// đệm chống mất dữ liệu, theo đúng thiết kế ở slide 9).
///
/// Dòng CUỐI chưa kết thúc bằng `\n` (Keyence có thể đang ghi dở) được GIỮ
/// LẠI cho lượt quét sau, không trả về.
pub fn read_new_lines(path: &Path, offset: u64) -> std::io::Result<Vec<(String, u64)>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < offset {
        // File bị rotate/ghi đè (offset cũ vượt quá kích thước hiện tại) —
        // đọc lại từ đầu thay vì seek quá cuối file.
        return read_new_lines(path, 0);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, b) in buf.iter().enumerate() {
        if *b == b'\n' {
            let line_bytes = &buf[start..i];
            let line = String::from_utf8_lossy(line_bytes)
                .trim_end_matches('\r')
                .to_string();
            let cumulative_offset = offset + (i as u64 + 1);
            out.push((line, cumulative_offset));
            start = i + 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    const LINE_1: &str =
        "301,2026,8,26,13,34,40,OK,OK,OK,'400111062103002382600605,MMO-26052028-001-P005";
    const LINE_2: &str =
        "325,2026,8,26,13,52,37,OK,OK,OK,'400111062103002382600597,MMO-26052028-001-P005";

    #[test]
    fn reads_all_lines_from_zero_offset() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("vision.log");
        std::fs::write(&log_path, format!("{LINE_1}\n{LINE_2}\n")).unwrap();

        let lines = read_new_lines(&log_path, 0).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, LINE_1);
        assert_eq!(lines[1].0, LINE_2);
    }

    #[test]
    fn resumes_from_offset_without_reprocessing_old_lines() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("vision.log");
        std::fs::write(&log_path, format!("{LINE_1}\n")).unwrap();

        let first = read_new_lines(&log_path, 0).unwrap();
        assert_eq!(first.len(), 1);
        let committed_offset = first[0].1;

        // Không có gì mới -> không trả dòng nào.
        let second = read_new_lines(&log_path, committed_offset).unwrap();
        assert_eq!(second.len(), 0);

        // Ghi thêm 1 dòng -> chỉ dòng mới được lấy.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(f, "{LINE_2}").unwrap();
        let third = read_new_lines(&log_path, committed_offset).unwrap();
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].0, LINE_2);
    }

    #[test]
    fn keeps_incomplete_trailing_line_for_next_poll() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("vision.log");
        // Dòng KHÔNG kết thúc bằng \n -> Keyence có thể đang ghi dở.
        std::fs::write(&log_path, LINE_1).unwrap();

        let first = read_new_lines(&log_path, 0).unwrap();
        assert_eq!(first.len(), 0, "dòng dở dang chưa nên trả về");

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(f).unwrap();
        let second = read_new_lines(&log_path, 0).unwrap();
        assert_eq!(second.len(), 1);
    }

    #[test]
    fn per_line_offset_allows_partial_commit() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("vision.log");
        std::fs::write(&log_path, format!("{LINE_1}\n{LINE_2}\n")).unwrap();

        let lines = read_new_lines(&log_path, 0).unwrap();
        // Commit chỉ dòng đầu (giả lập DB insert dòng 2 thất bại) rồi đọc lại
        // từ offset đó -> phải thấy lại đúng dòng 2, không mất, không lặp dòng 1.
        let after_line_1 = lines[0].1;
        let retry = read_new_lines(&log_path, after_line_1).unwrap();
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].0, LINE_2);
    }

    #[test]
    fn truncated_file_shorter_than_offset_is_read_from_start_again() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("vision.log");
        std::fs::write(&log_path, format!("{LINE_1}\n{LINE_2}\n")).unwrap();
        let lines = read_new_lines(&log_path, 0).unwrap();
        let stale_offset = lines[1].1; // offset sau khi đã đọc cả 2 dòng

        // File bị rotate/ghi đè bằng nội dung NGẮN HƠN rõ rệt (kích thước
        // giảm) — đây là dấu hiệu phát hiện được: `len < offset`.
        //
        // ⚠ GIỚI HẠN ĐÃ BIẾT: nếu file bị ghi đè bằng nội dung CÙNG kích
        // thước hoặc LỚN HƠN (rotate giữ nguyên/tăng size), detection này
        // không phát hiện được (chỉ so sánh độ dài, không hash nội dung).
        // Chấp nhận được vì Keyence ghi log append-only theo TỪNG NGÀY một
        // file riêng (không rotate/reuse tên file trong ngày, theo thiết kế
        // ở slide 9) — nếu thực tế phát sinh rotate cùng tên, cần bổ sung
        // content-fingerprint (vd hash N byte đầu) thay vì chỉ so độ dài.
        std::fs::write(&log_path, "short\n").unwrap();
        let after_rotate = read_new_lines(&log_path, stale_offset).unwrap();
        assert_eq!(after_rotate.len(), 1);
        assert_eq!(after_rotate[0].0, "short");
    }

    #[test]
    fn scans_per_day_subfolders_recursively() {
        let dir = tempdir().unwrap();
        let day_folder = dir.path().join("2026-09-10");
        std::fs::create_dir_all(&day_folder).unwrap();
        std::fs::write(day_folder.join("vision.log"), format!("{LINE_1}\n")).unwrap();

        let files = list_log_files(dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], day_folder.join("vision.log"));
    }

    #[test]
    fn empty_root_returns_empty_list() {
        let dir = tempdir().unwrap();
        assert!(list_log_files(dir.path()).is_empty());
    }
}
