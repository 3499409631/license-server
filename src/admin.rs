use axum::{
    Form,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::SET_COOKIE},
    response::{Html, IntoResponse, Redirect, Response},
};
use uuid::Uuid;

use crate::{
    AppState,
    auth::{
        AuthUser, SESSION_COOKIE, can_manage_owner, create_session, delete_session,
        expired_session_cookie_value, get_cookie, hash_password, require_admin_or_agent,
        session_cookie_value, verify_password,
    },
    html::{escape, page},
    models::{
        BanLicenseForm, CreateLicenseForm, CreateLicenseTypeForm, CreateUserForm, License,
        LicenseType, LoginForm, SessionUser, User,
    },
};

pub async fn login_form() -> Html<String> {
    page(
        "登录",
        None,
        r#"
        <section>
            <h1>后台登录</h1>
            <form method="post" action="/login">
                <label>用户名</label>
                <input name="username" autocomplete="username" required>
                <label>密码</label>
                <input name="password" type="password" autocomplete="current-password" required>
                <p><button type="submit">登录</button></p>
            </form>
        </section>
        "#
        .to_string(),
    )
}

pub async fn login_submit(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT id, password_hash
        FROM users
        WHERE username = $1
        "#,
    )
    .bind(form.username.trim())
    .fetch_optional(&state.pool)
    .await;

    let Ok(Some(user)) = user else {
        return login_error("用户名或密码错误").into_response();
    };

    if !verify_password(&form.password, &user.password_hash) {
        return login_error("用户名或密码错误").into_response();
    }

    let Ok(session_id) = create_session(&state.pool, user.id).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "创建登录会话失败").into_response();
    };

    (
        [(SET_COOKIE, session_cookie_value(&session_id))],
        Redirect::to("/admin"),
    )
        .into_response()
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(session_id) = get_cookie(&headers, SESSION_COOKIE) {
        delete_session(&state.pool, &session_id).await;
    }

    (
        [(SET_COOKIE, expired_session_cookie_value())],
        Redirect::to("/login"),
    )
}

pub async fn admin_dashboard(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    let visible_owner_ids = visible_owner_ids(&state, &user).await?;
    let total_users = count_visible_users(&state, &user).await?;
    let total_licenses = count_licenses_by_owners(&state, &visible_owner_ids, None).await?;
    let active_licenses =
        count_licenses_by_owners(&state, &visible_owner_ids, Some("active")).await?;
    let banned_licenses =
        count_licenses_by_owners(&state, &visible_owner_ids, Some("banned")).await?;

    Ok(page(
        "后台首页",
        Some(&user),
        format!(
            r#"
            <section>
                <h1>后台首页</h1>
                <div class="grid">
                    <div class="stat"><span>可见账号</span><strong>{total_users}</strong></div>
                    <div class="stat"><span>卡密总数</span><strong>{total_licenses}</strong></div>
                    <div class="stat"><span>有效卡密</span><strong>{active_licenses}</strong></div>
                    <div class="stat"><span>封禁卡密</span><strong>{banned_licenses}</strong></div>
                </div>
            </section>
            <section>
                <h2>接口信息</h2>
                <p>客户端验证地址：<code>POST /api/verify</code></p>
                <p>请求头：<code>X-Api-Key: APP_API_KEY</code></p>
                <p class="muted">卡密首次验证成功后会自动绑定机器码，并从首次验证时间开始计算到期时间。</p>
            </section>
            "#
        ),
    ))
}

pub async fn users_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    require_admin_or_agent(&user).await?;

    let users = visible_users(&state, &user).await?;
    let parent_options = parent_options_html(&users, &user);
    let mut rows = String::new();

    for item in users {
        rows.push_str(&format!(
            r#"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            item.id,
            escape(&item.username),
            escape(&item.role),
            item.parent_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
    }

    let allowed_roles = if user.role == "admin" {
        r#"<option value="agent">代理</option><option value="sub_agent">子代理</option>"#
    } else {
        r#"<option value="sub_agent">子代理</option>"#
    };

    Ok(page(
        "代理管理",
        Some(&user),
        format!(
            r#"
            <section>
                <h1>代理管理</h1>
                <table>
                    <thead><tr><th>ID</th><th>用户名</th><th>角色</th><th>上级 ID</th></tr></thead>
                    <tbody>{rows}</tbody>
                </table>
            </section>
            <section>
                <h2>新增账号</h2>
                <form method="post" action="/admin/users">
                    <label>用户名</label>
                    <input name="username" required>
                    <label>密码</label>
                    <input name="password" type="password" required>
                    <label>角色</label>
                    <select name="role">{allowed_roles}</select>
                    <label>上级账号</label>
                    <select name="parent_id">{parent_options}</select>
                    <p><button type="submit">创建</button></p>
                </form>
            </section>
            "#
        ),
    ))
}

pub async fn create_user(
    State(state): State<AppState>,
    AuthUser(current_user): AuthUser,
    Form(form): Form<CreateUserForm>,
) -> Result<Redirect, Response> {
    require_admin_or_agent(&current_user).await?;

    let role = form.role.trim();
    if role != "agent" && role != "sub_agent" {
        return Err((StatusCode::BAD_REQUEST, "角色只能是 agent 或 sub_agent").into_response());
    }

    if current_user.role == "agent" && role != "sub_agent" {
        return Err((StatusCode::FORBIDDEN, "代理只能创建子代理").into_response());
    }

    let parent_id = if current_user.role == "agent" {
        current_user.id
    } else {
        form.parent_id.unwrap_or(current_user.id)
    };

    let parent = sqlx::query_as::<_, SessionUser>(
        r#"
        SELECT id, username, role, parent_id
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(parent_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;

    let Some(parent) = parent else {
        return Err((StatusCode::BAD_REQUEST, "上级账号不存在").into_response());
    };

    if role == "agent" && parent.role != "admin" {
        return Err((StatusCode::BAD_REQUEST, "代理的上级必须是管理员").into_response());
    }

    if role == "sub_agent" && parent.role != "agent" {
        return Err((StatusCode::BAD_REQUEST, "子代理的上级必须是代理").into_response());
    }

    if !can_manage_owner(&current_user, parent.id, parent.parent_id) && current_user.id != parent.id
    {
        return Err((StatusCode::FORBIDDEN, "不能挂到不属于你的上级账号下").into_response());
    }

    let password_hash = hash_password(&form.password)
        .map_err(|_| (StatusCode::BAD_REQUEST, "密码哈希失败").into_response())?;

    sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, role, parent_id)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(form.username.trim())
    .bind(password_hash)
    .bind(role)
    .bind(parent_id)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("duplicate") {
            (StatusCode::BAD_REQUEST, "用户名已存在").into_response()
        } else {
            internal_error(err)
        }
    })?;

    Ok(Redirect::to("/admin/users"))
}

pub async fn types_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    let types = sqlx::query_as::<_, LicenseType>(
        r#"
        SELECT id, name, duration_days
        FROM license_types
        ORDER BY id DESC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;

    let mut rows = String::new();
    for item in types {
        let duration = if item.duration_days == 0 {
            "永久".to_string()
        } else {
            format!("{} 天", item.duration_days)
        };
        rows.push_str(&format!(
            r#"<tr><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            item.id,
            escape(&item.name),
            duration
        ));
    }

    Ok(page(
        "卡密类型",
        Some(&user),
        format!(
            r#"
            <section>
                <h1>卡密类型</h1>
                <table>
                    <thead><tr><th>ID</th><th>名称</th><th>默认时长</th></tr></thead>
                    <tbody>{rows}</tbody>
                </table>
            </section>
            <section>
                <h2>新增类型</h2>
                <form method="post" action="/admin/types">
                    <label>类型名称</label>
                    <input name="name" placeholder="例如：月卡" required>
                    <label>有效天数</label>
                    <input name="duration_days" type="number" min="0" value="30" required>
                    <p class="muted">填 0 表示永久卡。</p>
                    <p><button type="submit">创建</button></p>
                </form>
            </section>
            "#
        ),
    ))
}

pub async fn create_license_type(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Form(form): Form<CreateLicenseTypeForm>,
) -> Result<Redirect, Response> {
    if user.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "只有管理员可以创建卡密类型").into_response());
    }

    if form.duration_days < 0 {
        return Err((StatusCode::BAD_REQUEST, "有效天数不能小于 0").into_response());
    }

    sqlx::query(
        r#"
        INSERT INTO license_types (name, duration_days)
        VALUES ($1, $2)
        "#,
    )
    .bind(form.name.trim())
    .bind(form.duration_days)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/types"))
}

pub async fn licenses_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    let owners = visible_users(&state, &user).await?;
    let owner_ids: Vec<i64> = owners.iter().map(|owner| owner.id).collect();
    let licenses = licenses_for_owners(&state, &owner_ids).await?;
    let types = sqlx::query_as::<_, LicenseType>(
        "SELECT id, name, duration_days FROM license_types ORDER BY id DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;

    let mut rows = String::new();
    for item in licenses {
        let status = render_license_status(&item);
        let expires_at = item
            .expires_at
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| "永久或未激活".to_string());
        let machine_code = item.machine_code.unwrap_or_else(|| "-".to_string());
        let action = if item.status == "banned" {
            r#"<button type="submit">解封</button><input type="hidden" name="action" value="unban">"#
        } else {
            r#"<button class="danger" type="submit">封禁</button><input type="hidden" name="action" value="ban">"#
        };

        rows.push_str(&format!(
            r#"
            <tr>
                <td>{}</td>
                <td><code>{}</code></td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td><form method="post" action="/admin/licenses/{}/ban">{}</form></td>
            </tr>
            "#,
            item.id,
            escape(&item.license_key),
            escape(&item.type_name),
            escape(&item.owner_name),
            status,
            escape(&machine_code),
            escape(&expires_at),
            item.id,
            action
        ));
    }

    let mut type_options = String::new();
    for item in types {
        type_options.push_str(&format!(
            r#"<option value="{}">{} ({} 天)</option>"#,
            item.id,
            escape(&item.name),
            item.duration_days
        ));
    }

    let owner_options = owner_options_html(&owners);

    Ok(page(
        "卡密管理",
        Some(&user),
        format!(
            r#"
            <section>
                <h1>卡密管理</h1>
                <table>
                    <thead>
                        <tr><th>ID</th><th>卡密</th><th>类型</th><th>归属</th><th>状态</th><th>机器码</th><th>到期时间</th><th>操作</th></tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </section>
            <section>
                <h2>生成卡密</h2>
                <form method="post" action="/admin/licenses">
                    <label>卡密类型</label>
                    <select name="type_id">{type_options}</select>
                    <label>归属账号</label>
                    <select name="owner_id">{owner_options}</select>
                    <label>生成数量</label>
                    <input name="count" type="number" min="1" max="100" value="1" required>
                    <p><button type="submit">生成</button></p>
                </form>
            </section>
            "#
        ),
    ))
}

pub async fn create_license(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Form(form): Form<CreateLicenseForm>,
) -> Result<Redirect, Response> {
    if form.count < 1 || form.count > 100 {
        return Err((StatusCode::BAD_REQUEST, "单次生成数量必须在 1 到 100 之间").into_response());
    }

    let owner = get_user(&state, form.owner_id).await?;
    if !can_manage_owner(&user, owner.id, owner.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能给这个账号生成卡密").into_response());
    }

    let type_exists: Option<i64> = sqlx::query_scalar("SELECT id FROM license_types WHERE id = $1")
        .bind(form.type_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal_error)?;

    if type_exists.is_none() {
        return Err((StatusCode::BAD_REQUEST, "卡密类型不存在").into_response());
    }

    for _ in 0..form.count {
        let license_key = format!("LIC-{}", Uuid::new_v4().simple());

        sqlx::query(
            r#"
            INSERT INTO licenses (license_key, owner_id, type_id)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(license_key)
        .bind(owner.id)
        .bind(form.type_id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    }

    Ok(Redirect::to("/admin/licenses"))
}

pub async fn ban_license(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BanLicenseForm>,
) -> Result<Redirect, Response> {
    let license = sqlx::query_as::<_, License>(
        r#"
        SELECT
            licenses.id,
            licenses.license_key,
            licenses.status,
            licenses.machine_code,
            licenses.expires_at,
            licenses.owner_id,
            users.username AS owner_name,
            license_types.name AS type_name
        FROM licenses
        JOIN users ON users.id = licenses.owner_id
        JOIN license_types ON license_types.id = licenses.type_id
        WHERE licenses.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;

    let Some(license) = license else {
        return Err((StatusCode::NOT_FOUND, "卡密不存在").into_response());
    };

    let owner = get_user(&state, license.owner_id).await?;
    if !can_manage_owner(&user, owner.id, owner.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能管理这张卡密").into_response());
    }

    let new_status = match form.action.as_str() {
        "ban" => "banned",
        "unban" => {
            if license.machine_code.is_some() {
                "active"
            } else {
                "unused"
            }
        }
        _ => return Err((StatusCode::BAD_REQUEST, "未知操作").into_response()),
    };

    sqlx::query("UPDATE licenses SET status = $1 WHERE id = $2")
        .bind(new_status)
        .bind(license.id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;

    Ok(Redirect::to("/admin/licenses"))
}

fn login_error(message: &str) -> Html<String> {
    page(
        "登录失败",
        None,
        format!(
            r#"
            <section>
                <h1>后台登录</h1>
                <p class="error">{}</p>
                <form method="post" action="/login">
                    <label>用户名</label>
                    <input name="username" autocomplete="username" required>
                    <label>密码</label>
                    <input name="password" type="password" autocomplete="current-password" required>
                    <p><button type="submit">登录</button></p>
                </form>
            </section>
            "#,
            escape(message)
        ),
    )
}

async fn get_user(state: &AppState, user_id: i64) -> Result<SessionUser, Response> {
    let user = sqlx::query_as::<_, SessionUser>(
        "SELECT id, username, role, parent_id FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;

    user.ok_or_else(|| (StatusCode::BAD_REQUEST, "账号不存在").into_response())
}

async fn visible_users(state: &AppState, user: &SessionUser) -> Result<Vec<SessionUser>, Response> {
    // 后台列表只返回当前用户权限范围内的数据，避免代理看到不属于自己的子代理和卡密。
    let users = match user.role.as_str() {
        "admin" => {
            sqlx::query_as::<_, SessionUser>(
                "SELECT id, username, role, parent_id FROM users ORDER BY id DESC",
            )
            .fetch_all(&state.pool)
            .await
        }
        "agent" => {
            sqlx::query_as::<_, SessionUser>(
                r#"
                SELECT id, username, role, parent_id
                FROM users
                WHERE id = $1 OR parent_id = $1
                ORDER BY id DESC
                "#,
            )
            .bind(user.id)
            .fetch_all(&state.pool)
            .await
        }
        _ => {
            sqlx::query_as::<_, SessionUser>(
                "SELECT id, username, role, parent_id FROM users WHERE id = $1",
            )
            .bind(user.id)
            .fetch_all(&state.pool)
            .await
        }
    }
    .map_err(internal_error)?;

    Ok(users)
}

async fn visible_owner_ids(state: &AppState, user: &SessionUser) -> Result<Vec<i64>, Response> {
    Ok(visible_users(state, user)
        .await?
        .into_iter()
        .map(|item| item.id)
        .collect())
}

async fn count_visible_users(state: &AppState, user: &SessionUser) -> Result<i64, Response> {
    match user.role.as_str() {
        "admin" => sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&state.pool)
            .await
            .map_err(internal_error),
        "agent" => sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = $1 OR parent_id = $1")
            .bind(user.id)
            .fetch_one(&state.pool)
            .await
            .map_err(internal_error),
        _ => Ok(1),
    }
}

async fn count_licenses_by_owners(
    state: &AppState,
    owner_ids: &[i64],
    status: Option<&str>,
) -> Result<i64, Response> {
    if owner_ids.is_empty() {
        return Ok(0);
    }

    if let Some(status) = status {
        sqlx::query_scalar("SELECT COUNT(*) FROM licenses WHERE owner_id = ANY($1) AND status = $2")
            .bind(owner_ids)
            .bind(status)
            .fetch_one(&state.pool)
            .await
            .map_err(internal_error)
    } else {
        sqlx::query_scalar("SELECT COUNT(*) FROM licenses WHERE owner_id = ANY($1)")
            .bind(owner_ids)
            .fetch_one(&state.pool)
            .await
            .map_err(internal_error)
    }
}

async fn licenses_for_owners(
    state: &AppState,
    owner_ids: &[i64],
) -> Result<Vec<License>, Response> {
    if owner_ids.is_empty() {
        return Ok(Vec::new());
    }

    sqlx::query_as::<_, License>(
        r#"
        SELECT
            licenses.id,
            licenses.license_key,
            licenses.status,
            licenses.machine_code,
            licenses.expires_at,
            licenses.owner_id,
            users.username AS owner_name,
            license_types.name AS type_name
        FROM licenses
        JOIN users ON users.id = licenses.owner_id
        JOIN license_types ON license_types.id = licenses.type_id
        WHERE licenses.owner_id = ANY($1)
        ORDER BY licenses.id DESC
        LIMIT 300
        "#,
    )
    .bind(owner_ids)
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)
}

fn parent_options_html(users: &[SessionUser], current_user: &SessionUser) -> String {
    let mut html = String::new();

    for item in users {
        if current_user.role == "agent" && item.id != current_user.id {
            continue;
        }

        if item.role == "admin" || item.role == "agent" {
            html.push_str(&format!(
                r#"<option value="{}">{} ({})</option>"#,
                item.id,
                escape(&item.username),
                escape(&item.role)
            ));
        }
    }

    html
}

fn owner_options_html(users: &[SessionUser]) -> String {
    let mut html = String::new();

    for item in users {
        html.push_str(&format!(
            r#"<option value="{}">{} ({})</option>"#,
            item.id,
            escape(&item.username),
            escape(&item.role)
        ));
    }

    html
}

fn render_license_status(item: &License) -> String {
    if item.status == "banned" {
        return "已封禁".to_string();
    }

    if let Some(expires_at) = item.expires_at {
        if expires_at <= chrono::Utc::now() {
            return "已到期".to_string();
        }
    }

    match item.status.as_str() {
        "unused" => "未激活".to_string(),
        "active" => "有效".to_string(),
        other => escape(other),
    }
}

fn internal_error(err: sqlx::Error) -> Response {
    eprintln!("database error: {err}");
    (StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误").into_response()
}
