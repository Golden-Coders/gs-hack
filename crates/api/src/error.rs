//! Common HTTP error responses, parameter parsers, and database helpers for the API.

use axum::{http::StatusCode, Json};
use common::db::sqlx::{
    self,
    postgres::PgRow,
    types::{Json as SqlJson, Uuid},
    Row,
};
use serde_json::{json, Value};

pub type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

pub fn err(status: StatusCode, error: &str, detail: impl ToString) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({ "error": error, "detail": detail.to_string() })),
    )
}

pub fn err_with_line(
    status: StatusCode,
    error: &str,
    detail: impl ToString,
    line: Option<usize>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "error": error,
            "detail": detail.to_string(),
            "line": line,
        })),
    )
}

pub fn db_err(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %e, "db error");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        "database error",
        "database error",
    )
}

pub fn parse_uuid(s: &str, field: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(s).map_err(|e| err(StatusCode::BAD_REQUEST, &format!("invalid {field}"), e))
}

pub fn opt_json(r: &PgRow, col: &str) -> Value {
    r.get::<Option<SqlJson<Value>>, _>(col)
        .map_or(Value::Null, |j| j.0)
}
