use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    extract::FromRequestParts,
    http::{
        StatusCode,
        header::{COOKIE, SET_COOKIE},
        request::Parts,
    },
    response::{IntoResponse, Redirect, Response},
};
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{AppState, config::Config, models::SessionUser};

pub const SESSION_COOKIE: &str = "license_session";

pub async fn bootstrap_admin(pool: &PgPool, config: &Config) -> Result<(), sqlx::Error> {
    let admin_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(pool)
        .await?;

    if admin_count == 0 {
        let password_hash = hash_password(&config.admin_password)
            .map_err(|err| sqlx::Error::Protocol(format!("hash admin password failed: {err}")))?;

        sqlx::query(
            r#"
            INSERT INTO users (username, password_hash, role)
            VALUES ($1, $2, 'admin')
            "#,
        )
        .bind(&config.admin_user)
        .bind(password_hash)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, password_hash::Error> {
    // 不能把明文密码存进数据库。Argon2 会把密码变成不可逆哈希，即使数据库泄露也更难直接拿到密码。
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(password_hash) else {
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

pub async fn create_session(pool: &PgPool, user_id: i64) -> Result<String, sqlx::Error> {
    let session_id = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO sessions (id, user_id, expires_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(&session_id)
    .bind(user_id)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(session_id)
}

pub async fn delete_session(pool: &PgPool, session_id: &str) {
    let _ = sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await;
}

pub fn session_cookie_value(session_id: &str) -> String {
    format!(
        "{SESSION_COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        60 * 60 * 24 * 7
    )
}

pub fn expired_session_cookie_value() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}

pub fn get_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get(COOKIE)?.to_str().ok()?;

    cookie_header.split(';').find_map(|part| {
        let mut item = part.trim().splitn(2, '=');
        let key = item.next()?;
        let value = item.next()?;

        if key == name {
            Some(value.to_string())
        } else {
            None
        }
    })
}

pub struct AuthUser(pub SessionUser);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(session_id) = get_cookie(&parts.headers, SESSION_COOKIE) else {
            return Err(Redirect::to("/login").into_response());
        };

        // 每次访问后台时都通过 session_id 查询用户，确保封号/删除用户后会话立刻失效。
        let user = sqlx::query_as::<_, SessionUser>(
            r#"
            SELECT users.id, users.username, users.role, users.parent_id
            FROM sessions
            JOIN users ON users.id = sessions.user_id
            WHERE sessions.id = $1 AND sessions.expires_at > NOW()
            "#,
        )
        .bind(&session_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "读取登录会话失败，请稍后再试",
            )
                .into_response()
        })?;

        match user {
            Some(user) => Ok(AuthUser(user)),
            None => Err((
                [(SET_COOKIE, expired_session_cookie_value())],
                Redirect::to("/login"),
            )
                .into_response()),
        }
    }
}

pub async fn require_admin_or_agent(user: &SessionUser) -> Result<(), Response> {
    if user.role == "admin" || user.role == "agent" {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "当前账号没有这个操作权限").into_response())
    }
}

pub fn can_manage_owner(
    current_user: &SessionUser,
    owner_id: i64,
    owner_parent_id: Option<i64>,
) -> bool {
    // 权限规则保持简单：管理员看全部；代理看自己和直属子代理；子代理只看自己。
    match current_user.role.as_str() {
        "admin" => true,
        "agent" => owner_id == current_user.id || owner_parent_id == Some(current_user.id),
        "sub_agent" => owner_id == current_user.id,
        _ => false,
    }
}
