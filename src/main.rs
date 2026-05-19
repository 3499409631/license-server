mod admin;
mod api;
mod auth;
mod config;
mod db;
mod html;
mod models;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
use sqlx::PgPool;

use crate::{
    admin::{
        admin_dashboard, ban_license, create_license, create_license_type, create_user,
        delete_user, licenses_page, login_form, login_submit, logout, online_page, settings_page,
        types_page, unbind_license, update_settings, update_user_password, users_page,
    },
    api::verify_license,
    auth::bootstrap_admin,
    config::Config,
    db::init_database,
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 从环境变量读取配置，方便部署到 Linux 服务器时通过 systemd/docker 注入配置。
    let config = Arc::new(Config::from_env());

    // 建立 PostgreSQL 连接池。连接池会复用数据库连接，避免每个请求都重新连接数据库。
    let pool = PgPool::connect(&config.database_url).await?;

    // 启动时自动建表，学习项目可以省去单独迁移工具；生产环境后续建议改成 sqlx migrate。
    init_database(&pool).await?;

    // 如果数据库里没有管理员，则用 ADMIN_USER/ADMIN_PASSWORD 创建初始管理员。
    bootstrap_admin(&pool, &config).await?;

    let state = AppState {
        pool,
        config: config.clone(),
    };

    let app = Router::new()
        .route(
            "/",
            get(|| async { axum::response::Redirect::to("/admin") }),
        )
        .route("/api/verify", post(verify_license))
        .route("/login", get(login_form).post(login_submit))
        .route("/logout", post(logout))
        .route("/admin", get(admin_dashboard))
        .route("/admin/online", get(online_page))
        .route("/admin/settings", get(settings_page).post(update_settings))
        .route("/admin/users", get(users_page).post(create_user))
        .route("/admin/users/{id}/password", post(update_user_password))
        .route("/admin/users/{id}/delete", post(delete_user))
        .route("/admin/types", get(types_page).post(create_license_type))
        .route("/admin/licenses", get(licenses_page).post(create_license))
        .route("/admin/licenses/{id}/ban", post(ban_license))
        .route("/admin/licenses/{id}/unbind", post(unbind_license))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.server_addr).await?;
    println!("license-server listening on http://{}", config.server_addr);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
