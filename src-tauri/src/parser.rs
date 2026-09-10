//! Parser cho log CSV camera Keyence.
//!
//! ⚠ ASSUMPTION — CHƯA verify với sample log thật từ máy Keyence, chỉ suy ra
//! từ 3 dòng mẫu trong slide thiết kế (Youngmin_MES_Keyence_Integration_VI.pptx,
//! slide 9):
//!   301,2026,8,26,13,34,40,OK,OK,OK,'400111062103002382600605,MMO-26052028-001-P005
//!
//! Format suy ra: `seq,YYYY,M,D,H,Mi,S,<judgement...>,'<qr_serial>,<mo>`
//! - `seq`: số thứ tự dòng trong phiên/ngày (không đảm bảo unique giữa các ngày).
//! - `judgement`: SỐ LƯỢNG có thể thay đổi theo số công đoạn kiểm tra cấu hình
//!   trên máy Keyence — parser KHÔNG hard-code đúng 3, mà lấy mọi field nằm
//!   giữa phần timestamp cố định (7 field đầu) và 2 field cuối (qr_serial, mo).
//! - `qr_serial`: có dấu `'` ở đầu (Excel force-text marker) — bị strip.
//!
//! PHẢI đối chiếu lại với file log thật (từ máy Keyence tại trạm) trước khi
//! go-live — sai giả định này làm record bị parse sai mà KHÔNG crash (silent
//! wrong data), nguy hiểm hơn crash rõ ràng.

use crate::models::ScanRecord;
use chrono::NaiveDate;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    #[error("dòng rỗng")]
    Empty,
    #[error("thiếu field (cần tối thiểu {min}, có {actual})")]
    TooFewFields { min: usize, actual: usize },
    #[error("sequence không phải số nguyên: {0}")]
    InvalidSequence(String),
    #[error("timestamp không hợp lệ: {0}")]
    InvalidTimestamp(String),
    #[error("QR serial rỗng sau khi bỏ dấu nháy")]
    EmptyQrSerial,
    #[error("MO code rỗng")]
    EmptyMo,
}

/// seq + 6 field datetime (year,month,day,hour,minute,second) + tối thiểu 1
/// judgement + qr_serial + mo.
const MIN_FIELDS: usize = 9;

pub fn parse_line(raw: &str) -> Result<ScanRecord, ParseError> {
    let line = raw.trim();
    if line.is_empty() {
        return Err(ParseError::Empty);
    }
    let fields: Vec<&str> = line.split(',').collect();
    if fields.len() < MIN_FIELDS {
        return Err(ParseError::TooFewFields {
            min: MIN_FIELDS,
            actual: fields.len(),
        });
    }

    let sequence: i64 = fields[0]
        .trim()
        .parse()
        .map_err(|_| ParseError::InvalidSequence(fields[0].to_string()))?;

    let as_i32 = |s: &str| s.trim().parse::<i32>().ok();
    let as_u32 = |s: &str| s.trim().parse::<u32>().ok();
    let timestamp = (|| {
        let year = as_i32(fields[1])?;
        let month = as_u32(fields[2])?;
        let day = as_u32(fields[3])?;
        let hour = as_u32(fields[4])?;
        let minute = as_u32(fields[5])?;
        let second = as_u32(fields[6])?;
        NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second)
    })()
    .ok_or_else(|| {
        ParseError::InvalidTimestamp(format!(
            "{},{},{},{},{},{}",
            fields[1], fields[2], fields[3], fields[4], fields[5], fields[6]
        ))
    })?;

    let qr_raw = fields[fields.len() - 2].trim();
    let qr_serial = qr_raw.trim_start_matches('\'').to_string();
    if qr_serial.is_empty() {
        return Err(ParseError::EmptyQrSerial);
    }

    let mo = fields[fields.len() - 1].trim().to_string();
    if mo.is_empty() {
        return Err(ParseError::EmptyMo);
    }

    let judgements: Vec<String> = fields[7..fields.len() - 2]
        .iter()
        .map(|s| s.trim().to_string())
        .collect();

    let record_key = {
        let mut hasher = Sha256::new();
        hasher.update(line.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    Ok(ScanRecord {
        sequence,
        timestamp,
        judgements,
        qr_serial,
        mo,
        record_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_1: &str =
        "301,2026,8,26,13,34,40,OK,OK,OK,'400111062103002382600605,MMO-26052028-001-P005";
    const SAMPLE_2: &str =
        "325,2026,8,26,13,52,37,OK,OK,OK,'400111062103002382600597,MMO-26052028-001-P005";
    const SAMPLE_3: &str =
        "327,2026,8,26,13,53,16,OK,OK,OK,'400111062103002382600598,MMO-26052028-001-P005";

    #[test]
    fn parses_sample_lines_from_design_slide() {
        for raw in [SAMPLE_1, SAMPLE_2, SAMPLE_3] {
            let rec = parse_line(raw).expect("sample line phải parse được");
            assert_eq!(rec.mo, "MMO-26052028-001-P005");
            assert!(rec.qr_serial.starts_with("400111062103"));
            assert!(!rec.qr_serial.starts_with('\''));
            assert_eq!(rec.judgements, vec!["OK", "OK", "OK"]);
            assert!(rec.overall_ok());
        }
    }

    #[test]
    fn extracts_sequence_and_timestamp() {
        let rec = parse_line(SAMPLE_1).unwrap();
        assert_eq!(rec.sequence, 301);
        assert_eq!(rec.timestamp.to_string(), "2026-08-26 13:34:40");
    }

    #[test]
    fn same_raw_line_produces_same_record_key_different_lines_differ() {
        let a = parse_line(SAMPLE_1).unwrap();
        let b = parse_line(SAMPLE_1).unwrap();
        assert_eq!(a.record_key, b.record_key);
        let c = parse_line(SAMPLE_2).unwrap();
        assert_ne!(a.record_key, c.record_key);
    }

    #[test]
    fn handles_variable_judgement_count() {
        // Giả sử trạm chỉ cấu hình 1 công đoạn kiểm tra (không phải 3 như mẫu).
        let raw = "10,2026,1,1,0,0,0,NG,'ABC123,MO-001";
        let rec = parse_line(raw).unwrap();
        assert_eq!(rec.judgements, vec!["NG"]);
        assert!(!rec.overall_ok());
    }

    #[test]
    fn rejects_empty_line() {
        assert_eq!(parse_line(""), Err(ParseError::Empty));
        assert_eq!(parse_line("   "), Err(ParseError::Empty));
    }

    #[test]
    fn rejects_too_few_fields() {
        let err = parse_line("1,2026,1,1,0,0,0").unwrap_err();
        assert!(matches!(err, ParseError::TooFewFields { .. }));
    }

    #[test]
    fn rejects_invalid_timestamp() {
        let raw = "1,2026,13,40,0,0,0,OK,'ABC,MO-001"; // tháng 13 không tồn tại
        let err = parse_line(raw).unwrap_err();
        assert!(matches!(err, ParseError::InvalidTimestamp(_)));
    }

    #[test]
    fn rejects_empty_qr_serial() {
        let raw = "1,2026,1,1,0,0,0,OK,',MO-001"; // chỉ có dấu nháy, không có serial
        let err = parse_line(raw).unwrap_err();
        assert_eq!(err, ParseError::EmptyQrSerial);
    }

    #[test]
    fn rejects_invalid_sequence() {
        let raw = "abc,2026,1,1,0,0,0,OK,'X,MO-001";
        let err = parse_line(raw).unwrap_err();
        assert!(matches!(err, ParseError::InvalidSequence(_)));
    }
}
