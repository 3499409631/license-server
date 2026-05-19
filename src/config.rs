use std::env;

#[derive(Debug)]
pub struct Config {
    pub database_url: String,
    pub admin_user: String,
    pub admin_password: String,
    pub server_addr: String,
    pub app_api_key: String,
}

impl Config {
    pub fn from_env() -> Self {
        // 这些配置都有默认值，方便本地学习；真正部署时建议全部显式设置环境变量。
        Self {
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://postgres:manong666.@localhost:5432/license_server".to_string()
            }),
            admin_user: env::var("ADMIN_USER").unwrap_or_else(|_| "admin".to_string()),
            admin_password: env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "Manong666.".to_string()),
            server_addr: env::var("SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string()),
            app_api_key: env::var("APP_API_KEY")
                .unwrap_or_else(|_| "change-me-api-key".to_string()),
        }
    }
}
