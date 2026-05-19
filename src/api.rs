use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::FromRow;
use std::net::SocketAddr;

use crate::{
    AppState,
    models::{VerifyRequest, VerifyResponse},
};

#[derive(FromRow)]
struct VerifyLicenseRow {
    id: i64,
    status: String,
    machine_code: Option<String>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    type_name: String,
    duration_days: i32,
}

pub async fn verify_license(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<VerifyRequest>,
) -> impl IntoResponse {
    let api_key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if api_key != state.config.app_api_key {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "status": "unauthorized",
                "message": "API Key 错误"
            })),
        )
            .into_response();
    }

    if payload.license_key.trim().is_empty() || payload.machine_code.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "bad_request",
                "message": "卡密和机器码不能为空"
            })),
        )
            .into_response();
    }

    let ip_address = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| addr.ip().to_string());

    let license = sqlx::query_as::<_, VerifyLicenseRow>(
        r#"
        SELECT
            licenses.id,
            licenses.status,
            licenses.machine_code,
            licenses.expires_at,
            license_types.name AS type_name,
            license_types.duration_days
        FROM licenses
        JOIN license_types ON license_types.id = licenses.type_id
        WHERE licenses.license_key = $1
        "#,
    )
    .bind(&payload.license_key)
    .fetch_optional(&state.pool)
    .await;

    let Ok(license) = license else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "status": "error",
                "message": "查询卡密失败"
            })),
        )
            .into_response();
    };

    let Some(license) = license else {
        log_verify(
            &state,
            None,
            &payload.license_key,
            &payload.machine_code,
            &ip_address,
            "invalid",
        )
        .await;

        return Json(VerifyResponse {
            status: "invalid".to_string(),
            message: "卡密不存在".to_string(),
            expires_at: None,
            license_type: None,
        })
        .into_response();
    };

    if license.status == "banned" {
        log_verify(
            &state,
            Some(license.id),
            &payload.license_key,
            &payload.machine_code,
            &ip_address,
            "banned",
        )
        .await;

        return Json(VerifyResponse {
            status: "banned".to_string(),
            message: "卡密已被封禁".to_string(),
            expires_at: license.expires_at.map(|time| time.to_rfc3339()),
            license_type: Some(license.type_name),
        })
        .into_response();
    }

    if let Some(bound_machine_code) = &license.machine_code {
        if bound_machine_code != &payload.machine_code {
            log_verify(
                &state,
                Some(license.id),
                &payload.license_key,
                &payload.machine_code,
                &ip_address,
                "machine_mismatch",
            )
            .await;

            return Json(VerifyResponse {
                status: "machine_mismatch".to_string(),
                message: "机器码不匹配".to_string(),
                expires_at: license.expires_at.map(|time| time.to_rfc3339()),
                license_type: Some(license.type_name),
            })
            .into_response();
        }
    }

    let mut expires_at = license.expires_at;

    if license.status == "unused" {
        let now = Utc::now();
        let new_expires_at = if license.duration_days == 0 {
            None
        } else {
            Some(now + Duration::days(i64::from(license.duration_days)))
        };

        // 首次验证成功时绑定机器码。之后同一卡密只能由相同机器码继续验证。
        let updated = sqlx::query(
            r#"
            UPDATE licenses
            SET status = 'active',
                machine_code = $1,
                activated_at = $2,
                expires_at = $3
            WHERE id = $4 AND status = 'unused' AND machine_code IS NULL
            "#,
        )
        .bind(&payload.machine_code)
        .bind(now)
        .bind(new_expires_at)
        .bind(license.id)
        .execute(&state.pool)
        .await;

        let Ok(updated) = updated else {
            return error_response("激活卡密失败");
        };

        if updated.rows_affected() == 0 {
            let current_machine_code: Option<String> =
                match sqlx::query_scalar("SELECT machine_code FROM licenses WHERE id = $1")
                    .bind(license.id)
                    .fetch_optional(&state.pool)
                    .await
                {
                    Ok(value) => value.flatten(),
                    Err(_) => return error_response("读取机器码失败"),
                };

            if current_machine_code.as_deref() != Some(payload.machine_code.as_str()) {
                log_verify(
                    &state,
                    Some(license.id),
                    &payload.license_key,
                    &payload.machine_code,
                    &ip_address,
                    "machine_mismatch",
                )
                .await;

                return Json(VerifyResponse {
                    status: "machine_mismatch".to_string(),
                    message: "机器码不匹配".to_string(),
                    expires_at: None,
                    license_type: Some(license.type_name),
                })
                .into_response();
            }
        }

        expires_at = new_expires_at;
    }

    if let Some(expire_time) = expires_at {
        if expire_time <= Utc::now() {
            log_verify(
                &state,
                Some(license.id),
                &payload.license_key,
                &payload.machine_code,
                &ip_address,
                "expired",
            )
            .await;

            return Json(VerifyResponse {
                status: "expired".to_string(),
                message: "卡密已到期".to_string(),
                expires_at: Some(expire_time.to_rfc3339()),
                license_type: Some(license.type_name),
            })
            .into_response();
        }
    }

    log_verify(
        &state,
        Some(license.id),
        &payload.license_key,
        &payload.machine_code,
        &ip_address,
        "valid",
    )
    .await;

    Json(VerifyResponse {
        status: "valid".to_string(),
        message: "卡密有效".to_string(),
        expires_at: expires_at.map(|time| time.to_rfc3339()),
        license_type: Some(license.type_name),
    })
    .into_response()
}

async fn log_verify(
    state: &AppState,
    license_id: Option<i64>,
    license_key: &str,
    machine_code: &str,
    ip_address: &str,
    result: &str,
) {
    // 日志写入失败不应该影响客户端验证结果，所以这里只忽略错误；生产环境可以接入日志系统报警。
    let _ = sqlx::query(
        r#"
        INSERT INTO verify_logs (license_id, license_key, machine_code, ip_address, result)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(license_id)
    .bind(license_key)
    .bind(machine_code)
    .bind(ip_address)
    .bind(result)
    .execute(&state.pool)
    .await;
}

fn error_response(message: &str) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "status": "error",
            "message": message
        })),
    )
        .into_response()
}
