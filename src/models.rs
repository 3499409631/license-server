use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: i64,
    pub password_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionUser {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct LicenseType {
    pub id: i64,
    pub name: String,
    pub duration_days: i32,
}

#[derive(Debug, Clone, FromRow)]
pub struct License {
    pub id: i64,
    pub license_key: String,
    pub status: String,
    pub machine_code: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub owner_id: i64,
    pub owner_name: String,
    pub type_name: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub license_key: String,
    pub machine_code: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub status: String,
    pub message: String,
    pub expires_at: Option<String>,
    pub license_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserForm {
    pub username: String,
    pub password: String,
    pub role: String,
    pub parent_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLicenseTypeForm {
    pub name: String,
    pub duration_days: i32,
}

#[derive(Debug, Deserialize)]
pub struct CreateLicenseForm {
    pub type_id: i64,
    pub owner_id: i64,
    pub count: i32,
}

#[derive(Debug, Deserialize)]
pub struct BanLicenseForm {
    pub action: String,
}
