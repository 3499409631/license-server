use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use chrono::{Duration, FixedOffset, Utc};
use serde_json::json;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::net::SocketAddr;

use crate::{
    AppState,
    crypto::{CryptoError, decrypt_json, encrypt_json},
    models::{EncryptedPayload, VerifyRequest, VerifyResponse},
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
    Json(encrypted_payload): Json<EncryptedPayload>,
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

    let payload =
        match decrypt_json::<VerifyRequest>(&encrypted_payload, &state.config.client_aes_key) {
            Ok(payload) => payload,
            Err(_) => return bad_request_response("加密数据格式错误或解密失败"),
        };

    let response = verify_license_payload(&state, addr, headers, payload).await;
    encrypted_response(response, &state.config.server_aes_key)
}

async fn verify_license_payload(
    state: &AppState,
    addr: SocketAddr,
    headers: HeaderMap,
    payload: VerifyRequest,
) -> VerifyResponse {
    if payload.license_key.trim().is_empty() || payload.machine_code.trim().is_empty() {
        return VerifyResponse {
            status: "bad_request".to_string(),
            message: "卡密和机器码不能为空".to_string(),
            expires_at: None,
            license_type: None,
        };
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
        return VerifyResponse {
            status: "error".to_string(),
            message: "查询卡密失败".to_string(),
            expires_at: None,
            license_type: None,
        };
    };

    let Some(license) = license else {
        log_verify(
            &state.pool,
            None,
            &payload.license_key,
            &payload.machine_code,
            &ip_address,
            "invalid",
        )
        .await;

        return VerifyResponse {
            status: "invalid".to_string(),
            message: "卡密不存在".to_string(),
            expires_at: None,
            license_type: None,
        };
    };

    if license.status == "banned" {
        log_verify(
            &state.pool,
            Some(license.id),
            &payload.license_key,
            &payload.machine_code,
            &ip_address,
            "banned",
        )
        .await;

        return VerifyResponse {
            status: "banned".to_string(),
            message: "卡密已被封禁".to_string(),
            expires_at: license.expires_at.map(format_beijing_time),
            license_type: Some(license.type_name),
        };
    }

    let mut expires_at = license.expires_at;

    if let Some(expire_time) = expires_at {
        if expire_time <= Utc::now() {
            log_verify(
                &state.pool,
                Some(license.id),
                &payload.license_key,
                &payload.machine_code,
                &ip_address,
                "expired",
            )
            .await;

            return VerifyResponse {
                status: "expired".to_string(),
                message: "卡密已到期".to_string(),
                expires_at: Some(format_beijing_time(expire_time)),
                license_type: Some(license.type_name),
            };
        }
    }

    if let Some(bound_machine_code) = &license.machine_code {
        if bound_machine_code != &payload.machine_code {
            let rebind_result = try_auto_rebind(
                &state.pool,
                license.id,
                bound_machine_code,
                &payload.machine_code,
                &ip_address,
            )
            .await;

            match rebind_result {
                Ok(true) => {
                    log_verify(
                        &state.pool,
                        Some(license.id),
                        &payload.license_key,
                        &payload.machine_code,
                        &ip_address,
                        "valid",
                    )
                    .await;

                    return VerifyResponse {
                        status: "valid".to_string(),
                        message: "卡密有效，已自动换绑机器码".to_string(),
                        expires_at: expires_at.map(format_beijing_time),
                        license_type: Some(license.type_name),
                    };
                }
                Ok(false) => {
                    log_verify(
                        &state.pool,
                        Some(license.id),
                        &payload.license_key,
                        &payload.machine_code,
                        &ip_address,
                        "machine_mismatch",
                    )
                    .await;

                    return VerifyResponse {
                        status: "machine_mismatch".to_string(),
                        message: "机器码不匹配，自动换绑次数已达到限制".to_string(),
                        expires_at: expires_at.map(format_beijing_time),
                        license_type: Some(license.type_name),
                    };
                }
                Err(_) => {
                    return VerifyResponse {
                        status: "error".to_string(),
                        message: "自动换绑失败".to_string(),
                        expires_at: None,
                        license_type: None,
                    };
                }
            }
        }
    }

    if license.status == "unused" {
        let now = Utc::now();
        let new_expires_at = if expires_at.is_some() {
            expires_at
        } else if license.duration_days == 0 {
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
            return VerifyResponse {
                status: "error".to_string(),
                message: "激活卡密失败".to_string(),
                expires_at: None,
                license_type: None,
            };
        };

        if updated.rows_affected() == 0 {
            let current_machine_code: Option<String> =
                match sqlx::query_scalar("SELECT machine_code FROM licenses WHERE id = $1")
                    .bind(license.id)
                    .fetch_optional(&state.pool)
                    .await
                {
                    Ok(value) => value.flatten(),
                    Err(_) => {
                        return VerifyResponse {
                            status: "error".to_string(),
                            message: "读取机器码失败".to_string(),
                            expires_at: None,
                            license_type: None,
                        };
                    }
                };

            if current_machine_code.as_deref() != Some(payload.machine_code.as_str()) {
                log_verify(
                    &state.pool,
                    Some(license.id),
                    &payload.license_key,
                    &payload.machine_code,
                    &ip_address,
                    "machine_mismatch",
                )
                .await;

                return VerifyResponse {
                    status: "machine_mismatch".to_string(),
                    message: "机器码不匹配".to_string(),
                    expires_at: None,
                    license_type: Some(license.type_name),
                };
            }
        }

        expires_at = new_expires_at;
    }

    log_verify(
        &state.pool,
        Some(license.id),
        &payload.license_key,
        &payload.machine_code,
        &ip_address,
        "valid",
    )
    .await;

    VerifyResponse {
        status: "valid".to_string(),
        message: "卡密有效".to_string(),
        expires_at: expires_at.map(format_beijing_time),
        license_type: Some(license.type_name),
    }
}

fn format_beijing_time(time: chrono::DateTime<chrono::Utc>) -> String {
    time.with_timezone(&FixedOffset::east_opt(8 * 3600).expect("valid Beijing timezone offset"))
        .to_rfc3339()
}

fn encrypted_response(response: VerifyResponse, key: &str) -> axum::response::Response {
    match encrypt_json(&response, key) {
        Ok(payload) => Json(payload).into_response(),
        Err(CryptoError::InvalidKeyLength { .. }) => internal_error_response("服务器加密配置错误"),
        Err(_) => internal_error_response("加密响应失败"),
    }
}

async fn log_verify(
    pool: &PgPool,
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
    .execute(pool)
    .await;
}

async fn try_auto_rebind(
    pool: &PgPool,
    license_id: i64,
    old_machine_code: &str,
    new_machine_code: &str,
    ip_address: &str,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let current_machine_code: Option<String> = sqlx::query_scalar(
        r#"
        SELECT machine_code
        FROM licenses
        WHERE id = $1 AND status = 'active'
        FOR UPDATE
        "#,
    )
    .bind(license_id)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();

    if current_machine_code.as_deref() == Some(new_machine_code) {
        tx.commit().await?;
        return Ok(true);
    }

    if current_machine_code.as_deref() != Some(old_machine_code) {
        tx.commit().await?;
        return Ok(false);
    }

    let limit = auto_rebind_limit(&mut tx).await?;
    if limit <= 0 {
        tx.commit().await?;
        return Ok(false);
    }

    let recent_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM machine_rebind_logs
        WHERE license_id = $1
          AND created_at >= NOW() - INTERVAL '24 hours'
        "#,
    )
    .bind(license_id)
    .fetch_one(&mut *tx)
    .await?;

    if recent_count >= i64::from(limit) {
        tx.commit().await?;
        return Ok(false);
    }

    sqlx::query("UPDATE licenses SET machine_code = $1 WHERE id = $2")
        .bind(new_machine_code)
        .bind(license_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        r#"
        INSERT INTO machine_rebind_logs (license_id, old_machine_code, new_machine_code, ip_address)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(license_id)
    .bind(old_machine_code)
    .bind(new_machine_code)
    .bind(ip_address)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

async fn auto_rebind_limit(tx: &mut Transaction<'_, Postgres>) -> Result<i32, sqlx::Error> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM app_settings WHERE key = 'auto_rebind_limit_per_24h'",
    )
    .fetch_optional(&mut **tx)
    .await?;

    Ok(value
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(1))
}

fn bad_request_response(message: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "status": "bad_request",
            "message": message
        })),
    )
        .into_response()
}

fn internal_error_response(message: &str) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "status": "error",
            "message": message
        })),
    )
        .into_response()
}
