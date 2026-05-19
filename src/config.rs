use crate::crypto::validate_aes_key;
use std::env;

#[derive(Debug)]
pub struct Config {
    pub database_url: String,
    pub admin_user: String,
    pub admin_password: String,
    pub server_addr: String,
    pub app_api_key: String,
    pub client_aes_key: String,
    pub server_aes_key: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        // 这些配置都有默认值，方便本地学习；真正部署时建议全部显式设置环境变量。
        let config = Self {
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://postgres:manong666.@localhost:5432/license_server".to_string()
            }),
            admin_user: env::var("ADMIN_USER").unwrap_or_else(|_| "admin".to_string()),
            admin_password: env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "Manong666.".to_string()),
            server_addr: env::var("SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string()),
            app_api_key: env::var("APP_API_KEY")
                .unwrap_or_else(|_| "change-me-api-key".to_string()),
            client_aes_key: env::var("CLIENT_AES_KEY")
                .unwrap_or_else(|_| "client-aes-key-32-bytes-demo!!!!".to_string()),
            server_aes_key: env::var("SERVER_AES_KEY")
                .unwrap_or_else(|_| "server-aes-key-32-bytes-demo!!!!".to_string()),
        };

        validate_aes_key(&config.client_aes_key)
            .map_err(|err| format!("CLIENT_AES_KEY 配置错误: {err}"))?;
        validate_aes_key(&config.server_aes_key)
            .map_err(|err| format!("SERVER_AES_KEY 配置错误: {err}"))?;

        Ok(config)
    }
}
