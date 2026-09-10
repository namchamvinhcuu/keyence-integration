//! Ghi dữ liệu quét vào bảng STAGING của PostgreSQL (Odoo MES DB).
//!
//! ⚠ QUYẾT ĐỊNH THIẾT KẾ: ghi vào bảng TRUNG GIAN (`staging_table`, mặc định
//! `keyence_scan_log`) do app này tự tạo/quản lý — KHÔNG ghi thẳng vào bảng
//! nghiệp vụ của Odoo (vd bảng đứng sau module `autonsi_mms_youngmin`).
//! Lý do: (1) app này không biết schema/constraint thật của model Odoo và
//! insert sai sẽ vi phạm business rule của ORM mà không ai validate; (2) Odoo
//! cache dữ liệu trong process — ghi thẳng bảng nghiệp vụ có thể "vô hình"
//! với các Odoo worker đang chạy tới khi cache invalidate. Phía MES (module
//! Odoo, ngoài phạm vi app này) tự đọc bảng staging bằng ir.cron hoặc SQL
//! view riêng để tạo/threo record nghiệp vụ thật. Xem README §Thiết kế ghi DB.

use crate::models::ScanRecord;
use deadpool_postgres::{Config as PoolConfig, Pool, Runtime};
use tokio_postgres::NoTls;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("kết nối PostgreSQL thất bại: {0}")]
    Connect(String),
    #[error("query thất bại: {0}")]
    Query(String),
}

/// Tạo connection pool từ chuỗi kết nối dạng
/// "host=... port=... user=... password=... dbname=...".
pub fn build_pool(conn_string: &str) -> Result<Pool, DbError> {
    let mut cfg = PoolConfig::new();
    cfg.url = Some(conn_string.to_string());
    cfg.create_pool(Some(Runtime::Tokio1), NoTls)
        .map_err(|e| DbError::Connect(e.to_string()))
}

/// Tạo bảng staging nếu chưa có (idempotent — chạy lại nhiều lần vô hại).
pub async fn ensure_schema(pool: &Pool, table: &str) -> Result<(), DbError> {
    let client = pool.get().await.map_err(|e| DbError::Connect(e.to_string()))?;
    let stmt = format!(
        "CREATE TABLE IF NOT EXISTS {table} (
            id BIGSERIAL PRIMARY KEY,
            record_key TEXT UNIQUE NOT NULL,
            sequence BIGINT NOT NULL,
            scan_ts TIMESTAMP NOT NULL,
            judgements JSONB NOT NULL,
            qr_serial TEXT NOT NULL,
            mo TEXT NOT NULL,
            overall_ok BOOLEAN NOT NULL,
            received_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"
    );
    client
        .execute(stmt.as_str(), &[])
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
    Ok(())
}

/// Insert 1 record, IDEMPOTENT qua `record_key` — gửi lại (resend) không tạo
/// dữ liệu trùng, đúng yêu cầu ở slide 9 thiết kế.
pub async fn insert_record(pool: &Pool, table: &str, record: &ScanRecord) -> Result<(), DbError> {
    let client = pool.get().await.map_err(|e| DbError::Connect(e.to_string()))?;
    let stmt = format!(
        "INSERT INTO {table}
            (record_key, sequence, scan_ts, judgements, qr_serial, mo, overall_ok)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (record_key) DO NOTHING"
    );
    let judgements_json =
        serde_json::to_value(&record.judgements).map_err(|e| DbError::Query(e.to_string()))?;
    client
        .execute(
            stmt.as_str(),
            &[
                &record.record_key,
                &record.sequence,
                &record.timestamp,
                &judgements_json,
                &record.qr_serial,
                &record.mo,
                &record.overall_ok(),
            ],
        )
        .await
        .map_err(|e| DbError::Query(e.to_string()))?;
    Ok(())
}

// ⚠ insert_record/ensure_schema KHÔNG có unit test tự chứa — cần một
// PostgreSQL THẬT để verify (không mock được ý nghĩa của ON CONFLICT
// idempotent bằng test thuần Rust). Xem README §Test còn thiếu (residual
// risk) — cần integration test chạy với DB dev thật trước khi go-live.
//
// `build_pool` THÌ test được không cần Postgres thật (chỉ parse connection
// string cục bộ) — xem test bên dưới, regression-guard cho việc lỗi kết nối
// không được lộ password (rust-reviewer nêu khi review hệ thống logger, vì
// lỗi này giờ được ghi vào log persistent).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_pool_error_does_not_leak_password() {
        // Port sai định dạng (không phải số) khiến deadpool-postgres parse
        // URL lỗi ngay lúc build_pool, không cần kết nối mạng/PostgreSQL
        // thật -- đủ để regression-test rằng message lỗi trả về (giờ bị ghi
        // vào log file persistent qua `log::error!`) KHÔNG in kèm password.
        let conn_string = "host=localhost port=notanumber user=u password=SUPERSECRET dbname=d";
        let err = build_pool(conn_string).unwrap_err();
        let msg = err.to_string();
        assert!(
            !msg.contains("SUPERSECRET"),
            "Lỗi build_pool không được lộ password, nhưng message là: {msg}"
        );
    }
}
