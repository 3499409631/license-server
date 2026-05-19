use sqlx::{Executor, PgPool};

pub async fn init_database(pool: &PgPool) -> Result<(), sqlx::Error> {
    // users 保存后台账号。parent_id 用来表示代理层级：代理的 parent_id 是管理员，子代理的 parent_id 是代理。
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id BIGSERIAL PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL CHECK (role IN ('admin', 'agent', 'sub_agent')),
            parent_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    // license_types 定义卡密类型和默认有效天数，例如“月卡 30 天”。
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS license_types (
            id BIGSERIAL PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            duration_days INTEGER NOT NULL CHECK (duration_days >= 0),
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    // licenses 是卡密主体。status 只保存人工状态；是否过期由 expires_at 动态判断。
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS licenses (
            id BIGSERIAL PRIMARY KEY,
            license_key TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL CHECK (status IN ('unused', 'active', 'banned')) DEFAULT 'unused',
            machine_code TEXT,
            activated_at TIMESTAMPTZ,
            expires_at TIMESTAMPTZ,
            remark TEXT NOT NULL DEFAULT '',
            owner_id BIGINT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
            type_id BIGINT NOT NULL REFERENCES license_types(id) ON DELETE RESTRICT,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    pool.execute("ALTER TABLE licenses ADD COLUMN IF NOT EXISTS remark TEXT NOT NULL DEFAULT ''")
        .await?;

    // verify_logs 保存每次客户端验证记录，方便排查卡密滥用、IP 变化、机器码冲突等问题。
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS verify_logs (
            id BIGSERIAL PRIMARY KEY,
            license_id BIGINT REFERENCES licenses(id) ON DELETE SET NULL,
            license_key TEXT NOT NULL,
            machine_code TEXT NOT NULL,
            ip_address TEXT NOT NULL,
            result TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    // machine_rebind_logs 保存自动换绑记录，用来限制每张卡密 24 小时内的自动换绑次数。
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS machine_rebind_logs (
            id BIGSERIAL PRIMARY KEY,
            license_id BIGINT NOT NULL REFERENCES licenses(id) ON DELETE CASCADE,
            old_machine_code TEXT NOT NULL,
            new_machine_code TEXT NOT NULL,
            ip_address TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    // app_settings 保存后台可配置的全局参数。
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    // sessions 保存后台登录态。浏览器只保存随机 session_id，真实用户信息放在数据库里。
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            expires_at TIMESTAMPTZ NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    // 默认卡密类型，避免第一次进入后台时没有类型可选。
    sqlx::query(
        r#"
        INSERT INTO license_types (name, duration_days)
        VALUES ('月卡', 30)
        ON CONFLICT (name) DO NOTHING
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO app_settings (key, value)
        VALUES ('auto_rebind_limit_per_24h', '1')
        ON CONFLICT (key) DO NOTHING
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
