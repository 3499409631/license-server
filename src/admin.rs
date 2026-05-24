use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::SET_COOKIE},
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::FixedOffset;
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
        AdjustLicenseTimeForm, AdjustLicensesTimeForm, BanLicenseForm, CreateLicenseForm,
        CreateLicenseTypeForm, CreateUserForm, License, LicenseFilters, LicenseType, LoginForm,
        OnlineLicense, SessionUser, SettingsForm, UpdateUserPasswordForm, User,
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
    let today_login_users = count_today_login_users(&state, &visible_owner_ids).await?;

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
                    <div class="stat"><span>今日登录用户</span><strong>{today_login_users}</strong></div>
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

pub async fn online_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    let owner_ids = visible_owner_ids(&state, &user).await?;
    let online_licenses = online_licenses_for_owners(&state, &owner_ids).await?;
    let mut rows = String::new();

    for item in online_licenses {
        let expires_at = item
            .expires_at
            .map(format_beijing_time)
            .unwrap_or_else(|| "永久或未激活".to_string());
        let last_seen_at = format_beijing_time(item.last_seen_at);

        rows.push_str(&format!(
            r#"
            <tr>
                <td><code>{}</code></td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
            </tr>
            "#,
            escape(&item.license_key),
            escape(&item.type_name),
            escape(&item.owner_name),
            escape(&item.machine_code),
            escape(&item.ip_address),
            escape(&last_seen_at),
            escape(&expires_at)
        ));
    }

    Ok(page(
        "在线用户",
        Some(&user),
        format!(
            r#"
            <section>
                <h1>在线用户</h1>
                <p class="muted">最近 1 小时内成功验证过的卡密视为在线。</p>
                <table>
                    <thead>
                        <tr><th>卡密</th><th>类型</th><th>归属</th><th>机器码</th><th>IP</th><th>最后验证时间</th><th>到期时间</th></tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </section>
            "#
        ),
    ))
}

pub async fn settings_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    if user.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "只有管理员可以修改系统设置").into_response());
    }

    let limit = get_auto_rebind_limit(&state).await?;

    Ok(page(
        "系统设置",
        Some(&user),
        format!(
            r#"
            <section>
                <h1>系统设置</h1>
                <form method="post" action="/admin/settings">
                    <label>每张卡密 24 小时自动换绑最大次数</label>
                    <input name="auto_rebind_limit_per_24h" type="number" min="0" max="100" value="{limit}" required>
                    <p class="muted">填 0 表示关闭自动换绑。达到限制后，新机器码验证会返回机器码不匹配。</p>
                    <p><button type="submit">保存设置</button></p>
                </form>
            </section>
            "#
        ),
    ))
}

pub async fn update_settings(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Form(form): Form<SettingsForm>,
) -> Result<Redirect, Response> {
    if user.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "只有管理员可以修改系统设置").into_response());
    }

    if form.auto_rebind_limit_per_24h < 0 || form.auto_rebind_limit_per_24h > 100 {
        return Err((StatusCode::BAD_REQUEST, "自动换绑次数必须在 0 到 100 之间").into_response());
    }

    sqlx::query(
        r#"
        INSERT INTO app_settings (key, value, updated_at)
        VALUES ('auto_rebind_limit_per_24h', $1, NOW())
        ON CONFLICT (key)
        DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()
        "#,
    )
    .bind(form.auto_rebind_limit_per_24h.to_string())
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/settings"))
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
        let actions = if item.id == user.id || item.role == "admin" {
            String::new()
        } else {
            format!(
                r#"
                <div class="actions">
                    <form method="post" action="/admin/users/{}/password">
                        <input name="password" type="password" placeholder="新密码" required>
                        <button type="submit">改密码</button>
                    </form>
                    <form method="post" action="/admin/users/{}/delete">
                        <button class="danger" type="submit">删除</button>
                    </form>
                </div>
                "#,
                item.id, item.id
            )
        };
        rows.push_str(&format!(
            r#"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
            item.id,
            escape(&item.username),
            escape(&item.role),
            item.parent_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            actions
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
                    <thead><tr><th>ID</th><th>用户名</th><th>角色</th><th>上级 ID</th><th>操作</th></tr></thead>
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

pub async fn update_user_password(
    State(state): State<AppState>,
    AuthUser(current_user): AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<UpdateUserPasswordForm>,
) -> Result<Redirect, Response> {
    require_admin_or_agent(&current_user).await?;

    if form.password.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "密码不能为空").into_response());
    }

    let target = get_user(&state, id).await?;
    if target.role == "admin" {
        return Err((StatusCode::FORBIDDEN, "不能在代理管理里修改管理员密码").into_response());
    }

    if !can_manage_owner(&current_user, target.id, target.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能修改这个账号").into_response());
    }

    let password_hash = hash_password(&form.password)
        .map_err(|_| (StatusCode::BAD_REQUEST, "密码哈希失败").into_response())?;

    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(password_hash)
        .bind(target.id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;

    Ok(Redirect::to("/admin/users"))
}

pub async fn delete_user(
    State(state): State<AppState>,
    AuthUser(current_user): AuthUser,
    Path(id): Path<i64>,
) -> Result<Redirect, Response> {
    require_admin_or_agent(&current_user).await?;

    if id == current_user.id {
        return Err((StatusCode::BAD_REQUEST, "不能删除当前登录账号").into_response());
    }

    let target = get_user(&state, id).await?;
    if target.role == "admin" {
        return Err((StatusCode::FORBIDDEN, "不能删除管理员账号").into_response());
    }

    if !can_manage_owner(&current_user, target.id, target.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能删除这个账号").into_response());
    }

    let child_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE parent_id = $1")
        .bind(target.id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    if child_count > 0 {
        return Err((StatusCode::BAD_REQUEST, "账号下还有子代理，不能删除").into_response());
    }

    let license_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM licenses WHERE owner_id = $1")
            .bind(target.id)
            .fetch_one(&state.pool)
            .await
            .map_err(internal_error)?;
    if license_count > 0 {
        return Err((StatusCode::BAD_REQUEST, "账号下还有卡密，不能删除").into_response());
    }

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(target.id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;

    Ok(Redirect::to("/admin/users"))
}

pub async fn types_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    let types = sqlx::query_as::<_, LicenseType>(
        r#"
        SELECT id, name, prefix, duration_days
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
            r#"<tr><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>"#,
            item.id,
            escape(&item.name),
            escape(&item.prefix),
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
                    <thead><tr><th>ID</th><th>名称</th><th>前缀</th><th>默认时长</th></tr></thead>
                    <tbody>{rows}</tbody>
                </table>
            </section>
            <section>
                <h2>新增类型</h2>
                <form method="post" action="/admin/types">
                    <label>类型名称</label>
                    <input name="name" placeholder="例如：月卡" required>
                    <label>卡密前缀</label>
                    <input name="prefix" maxlength="32" value="LIC-" required>
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

    let prefix = form.prefix.trim();
    if prefix.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "卡密前缀不能为空").into_response());
    }

    if prefix.chars().count() > 32 {
        return Err((StatusCode::BAD_REQUEST, "卡密前缀最多 32 个字符").into_response());
    }

    sqlx::query(
        r#"
        INSERT INTO license_types (name, prefix, duration_days)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(form.name.trim())
    .bind(prefix)
    .bind(form.duration_days)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/types"))
}

pub async fn licenses_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Query(filters): Query<LicenseFilters>,
) -> Result<Html<String>, Response> {
    let owners = visible_users(&state, &user).await?;
    let owner_ids: Vec<i64> = owners.iter().map(|owner| owner.id).collect();
    let licenses = licenses_for_owners(&state, &owner_ids, &filters).await?;
    let query = filters.q.as_deref().unwrap_or_default();
    let activation_status = normalized_filter(filters.activation_status.as_deref());
    let expiry_status = normalized_filter(filters.expiry_status.as_deref());

    let mut rows = String::new();
    for item in licenses {
        let status = render_license_status(&item);
        let expires_at = item
            .expires_at
            .map(format_beijing_time)
            .unwrap_or_else(|| "永久或未激活".to_string());
        let can_unbind = item.status != "banned" && item.machine_code.is_some();
        let machine_code = item.machine_code.unwrap_or_else(|| "-".to_string());
        let remark = if item.remark.trim().is_empty() {
            "-".to_string()
        } else {
            item.remark.clone()
        };
        let ban_action = if item.status == "banned" {
            r#"<button type="submit">解封</button><input type="hidden" name="action" value="unban">"#
        } else {
            r#"<button class="danger" type="submit">封禁</button><input type="hidden" name="action" value="ban">"#
        };
        let unbind_action = if can_unbind {
            format!(
                r#"<form method="post" action="/admin/licenses/{}/unbind"><button type="submit">解绑</button></form>"#,
                item.id
            )
        } else {
            String::new()
        };

        rows.push_str(&format!(
            r#"
            <tr>
                <td>{}</td>
                <td><code>{}</code></td>
                <td>{}</td>
                <td class="remark-cell">{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>
                    <div class="actions">
                        <form method="post" action="/admin/licenses/{}/time">
                            <input name="hours" type="number" min="-87600" max="87600" placeholder="小时" required>
                            <button class="secondary" type="submit">调整</button>
                        </form>
                        <form method="post" action="/admin/licenses/{}/ban">{}</form>
                        {}
                    </div>
                </td>
            </tr>
            "#,
            item.id,
            escape(&item.license_key),
            escape(&item.type_name),
            escape(&remark),
            escape(&item.owner_name),
            status,
            escape(&machine_code),
            escape(&expires_at),
            item.id,
            item.id,
            ban_action,
            unbind_action
        ));
    }

    let global_time_form = if user.role == "admin" {
        r#"
            <section>
                <h2>全局调整时间</h2>
                <form method="post" action="/admin/licenses/time" class="filters">
                    <div>
                        <label>调整小时数</label>
                        <input name="hours" type="number" min="-87600" max="87600" placeholder="例如 24 或 -24" required>
                    </div>
                    <div>
                        <label>调整范围</label>
                        <select name="scope">
                            <option value="active_only">仅未到期卡密</option>
                            <option value="all_with_expired">包含已到期卡密</option>
                        </select>
                    </div>
                    <p><button type="submit">全局调整</button></p>
                </form>
                <p class="muted">只会调整已有到期时间的卡密，永久卡和未激活无到期卡会跳过。</p>
            </section>
        "#
    } else {
        ""
    };

    Ok(page(
        "卡密管理",
        Some(&user),
        format!(
            r#"
            <section>
                <h1>卡密管理</h1>
                <form method="get" action="/admin/licenses" class="filters">
                    <div>
                        <label>搜索</label>
                        <input name="q" value="{}" placeholder="卡密 / 备注 / 归属 / 机器码">
                    </div>
                    <div>
                        <label>激活状态</label>
                        <select name="activation_status">
                            {}
                        </select>
                    </div>
                    <div>
                        <label>到期状态</label>
                        <select name="expiry_status">
                            {}
                        </select>
                    </div>
                    <p>
                        <button type="submit">筛选</button>
                        <a href="/admin/licenses">清空</a>
                    </p>
                </form>
            </section>
            <section>
                <table>
                    <thead>
                        <tr><th>ID</th><th>卡密</th><th>类型</th><th>备注</th><th>归属</th><th>状态</th><th>机器码</th><th>到期时间</th><th>操作</th></tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
                <p class="muted">最多显示 300 条结果。</p>
            </section>
            {global_time_form}
            "#,
            escape(query),
            activation_filter_options(activation_status),
            expiry_filter_options(expiry_status)
        ),
    ))
}

pub async fn new_license_page(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Html<String>, Response> {
    render_new_license_page(&state, &user, &[]).await
}

pub async fn create_license(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Form(form): Form<CreateLicenseForm>,
) -> Result<Html<String>, Response> {
    if form.count < 1 || form.count > 100 {
        return Err((StatusCode::BAD_REQUEST, "单次生成数量必须在 1 到 100 之间").into_response());
    }

    let owner = get_user(&state, form.owner_id).await?;
    if !can_manage_owner(&user, owner.id, owner.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能给这个账号生成卡密").into_response());
    }

    let prefix: Option<String> =
        sqlx::query_scalar("SELECT prefix FROM license_types WHERE id = $1")
            .bind(form.type_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal_error)?;

    let Some(prefix) = prefix else {
        return Err((StatusCode::BAD_REQUEST, "卡密类型不存在").into_response());
    };

    let remark = form.remark.unwrap_or_default().trim().to_string();
    if remark.chars().count() > 500 {
        return Err((StatusCode::BAD_REQUEST, "备注最多 500 个字符").into_response());
    }

    let mut generated_keys = Vec::new();
    for _ in 0..form.count {
        let license_key = format!("{}{}", prefix, Uuid::new_v4().simple());

        sqlx::query(
            r#"
            INSERT INTO licenses (license_key, owner_id, type_id, remark)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(&license_key)
        .bind(owner.id)
        .bind(form.type_id)
        .bind(&remark)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;

        generated_keys.push(license_key);
    }

    render_new_license_page(&state, &user, &generated_keys).await
}

async fn render_new_license_page(
    state: &AppState,
    user: &SessionUser,
    generated_keys: &[String],
) -> Result<Html<String>, Response> {
    let owners = visible_users(state, user).await?;
    let types = sqlx::query_as::<_, LicenseType>(
        "SELECT id, name, prefix, duration_days FROM license_types ORDER BY id DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;

    let mut type_options = String::new();
    for item in types {
        type_options.push_str(&format!(
            r#"<option value="{}">{} ({} 天)</option>"#,
            item.id,
            escape(&item.name),
            item.duration_days
        ));
    }

    let generated_result = if generated_keys.is_empty() {
        String::new()
    } else {
        format!(
            r#"
            <section>
                <h2>本次生成结果</h2>
                <textarea class="generated-licenses" readonly>{}</textarea>
                <p class="muted">一行一张卡密，可直接全选复制。</p>
            </section>
            "#,
            escape(&generated_keys.join("\n"))
        )
    };

    Ok(page(
        "生成卡密",
        Some(user),
        format!(
            r#"
            <section>
                <h1>生成卡密</h1>
                <form method="post" action="/admin/licenses">
                    <label>卡密类型</label>
                    <select name="type_id">{type_options}</select>
                    <label>归属账号</label>
                    <select name="owner_id">{}</select>
                    <label>生成数量</label>
                    <input name="count" type="number" min="1" max="100" value="1" required>
                    <label>备注</label>
                    <textarea name="remark" maxlength="500" placeholder="可选，生成的每张卡密都会使用这条备注"></textarea>
                    <p><button type="submit">生成</button></p>
                </form>
            </section>
            {generated_result}
            "#,
            owner_options_html(&owners)
        ),
    ))
}

pub async fn ban_license(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BanLicenseForm>,
) -> Result<Redirect, Response> {
    let license = license_by_id(&state, id).await?;
    let owner = get_user(&state, license.owner_id).await?;
    if !can_manage_owner(&user, owner.id, owner.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能管理这张卡密").into_response());
    }

    let new_status = match form.action.as_str() {
        "ban" => "banned",
        "unban" => {
            if license.activated_at.is_some() || license.machine_code.is_some() {
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

pub async fn unbind_license(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
) -> Result<Redirect, Response> {
    let license = license_by_id(&state, id).await?;
    let owner = get_user(&state, license.owner_id).await?;

    if !can_manage_owner(&user, owner.id, owner.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能管理这张卡密").into_response());
    }

    if license.status == "banned" {
        return Err((StatusCode::BAD_REQUEST, "已封禁卡密不能解绑").into_response());
    }

    sqlx::query("UPDATE licenses SET machine_code = NULL WHERE id = $1")
        .bind(license.id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;

    Ok(Redirect::to("/admin/licenses"))
}

pub async fn adjust_license_time(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<AdjustLicenseTimeForm>,
) -> Result<Redirect, Response> {
    validate_adjust_hours(form.hours)?;

    let license = license_by_id(&state, id).await?;
    let owner = get_user(&state, license.owner_id).await?;
    if !can_manage_owner(&user, owner.id, owner.parent_id) {
        return Err((StatusCode::FORBIDDEN, "不能管理这张卡密").into_response());
    }

    sqlx::query(
        r#"
        UPDATE licenses
        SET expires_at = expires_at + ($1 * INTERVAL '1 hour')
        WHERE id = $2 AND expires_at IS NOT NULL
        "#,
    )
    .bind(form.hours)
    .bind(license.id)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;

    Ok(Redirect::to("/admin/licenses"))
}

pub async fn adjust_licenses_time(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Form(form): Form<AdjustLicensesTimeForm>,
) -> Result<Redirect, Response> {
    if user.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "只有管理员可以全局调整卡密时间").into_response());
    }

    validate_adjust_hours(form.hours)?;

    match form.scope.as_str() {
        "active_only" => {
            sqlx::query(
                r#"
                UPDATE licenses
                SET expires_at = expires_at + ($1 * INTERVAL '1 hour')
                WHERE expires_at IS NOT NULL
                  AND expires_at > NOW()
                  AND status != 'banned'
                "#,
            )
            .bind(form.hours)
            .execute(&state.pool)
            .await
            .map_err(internal_error)?;
        }
        "all_with_expired" => {
            sqlx::query(
                r#"
                UPDATE licenses
                SET expires_at = expires_at + ($1 * INTERVAL '1 hour')
                WHERE expires_at IS NOT NULL
                "#,
            )
            .bind(form.hours)
            .execute(&state.pool)
            .await
            .map_err(internal_error)?;
        }
        _ => return Err((StatusCode::BAD_REQUEST, "未知调整范围").into_response()),
    }

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

async fn license_by_id(state: &AppState, id: i64) -> Result<License, Response> {
    let license = sqlx::query_as::<_, License>(
        r#"
        SELECT
            licenses.id,
            licenses.license_key,
            licenses.status,
            licenses.machine_code,
            licenses.activated_at,
            licenses.expires_at,
            licenses.owner_id,
            users.username AS owner_name,
            license_types.name AS type_name,
            licenses.remark
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

    license.ok_or_else(|| (StatusCode::NOT_FOUND, "卡密不存在").into_response())
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

async fn count_today_login_users(state: &AppState, owner_ids: &[i64]) -> Result<i64, Response> {
    if owner_ids.is_empty() {
        return Ok(0);
    }

    sqlx::query_scalar(
        r#"
        SELECT COUNT(DISTINCT verify_logs.license_id)
        FROM verify_logs
        JOIN licenses ON licenses.id = verify_logs.license_id
        WHERE licenses.owner_id = ANY($1)
          AND verify_logs.result = 'valid'
          AND verify_logs.created_at >= date_trunc('day', NOW())
          AND verify_logs.created_at < date_trunc('day', NOW()) + INTERVAL '1 day'
        "#,
    )
    .bind(owner_ids)
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error)
}

async fn online_licenses_for_owners(
    state: &AppState,
    owner_ids: &[i64],
) -> Result<Vec<OnlineLicense>, Response> {
    if owner_ids.is_empty() {
        return Ok(Vec::new());
    }

    sqlx::query_as::<_, OnlineLicense>(
        r#"
        SELECT DISTINCT ON (licenses.id)
            licenses.license_key,
            license_types.name AS type_name,
            users.username AS owner_name,
            verify_logs.machine_code,
            verify_logs.ip_address,
            verify_logs.created_at AS last_seen_at,
            licenses.expires_at
        FROM verify_logs
        JOIN licenses ON licenses.id = verify_logs.license_id
        JOIN users ON users.id = licenses.owner_id
        JOIN license_types ON license_types.id = licenses.type_id
        WHERE licenses.owner_id = ANY($1)
          AND verify_logs.result = 'valid'
          AND verify_logs.created_at >= NOW() - INTERVAL '1 hour'
        ORDER BY licenses.id, verify_logs.created_at DESC
        LIMIT 300
        "#,
    )
    .bind(owner_ids)
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)
}

async fn get_auto_rebind_limit(state: &AppState) -> Result<i32, Response> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM app_settings WHERE key = 'auto_rebind_limit_per_24h'",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;

    Ok(value
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(1))
}

async fn licenses_for_owners(
    state: &AppState,
    owner_ids: &[i64],
    filters: &LicenseFilters,
) -> Result<Vec<License>, Response> {
    if owner_ids.is_empty() {
        return Ok(Vec::new());
    }

    let query = filters.q.as_deref().map(str::trim).unwrap_or_default();
    let query_pattern = format!("%{}%", query);
    let activation_status = normalized_filter(filters.activation_status.as_deref()).to_string();
    let expiry_status = normalized_filter(filters.expiry_status.as_deref()).to_string();

    sqlx::query_as::<_, License>(
        r#"
        SELECT
            licenses.id,
            licenses.license_key,
            licenses.status,
            licenses.machine_code,
            licenses.activated_at,
            licenses.expires_at,
            licenses.owner_id,
            users.username AS owner_name,
            license_types.name AS type_name,
            licenses.remark
        FROM licenses
        JOIN users ON users.id = licenses.owner_id
        JOIN license_types ON license_types.id = licenses.type_id
        WHERE licenses.owner_id = ANY($1)
          AND (
              $2 = ''
              OR licenses.license_key ILIKE $3
              OR licenses.remark ILIKE $3
              OR users.username ILIKE $3
              OR COALESCE(licenses.machine_code, '') ILIKE $3
          )
          AND (
              $4 = ''
              OR ($4 = 'unused' AND licenses.status = 'unused')
              OR ($4 = 'active' AND licenses.status = 'active')
              OR ($4 = 'active_unbound' AND licenses.status = 'active' AND licenses.machine_code IS NULL)
              OR ($4 = 'banned' AND licenses.status = 'banned')
          )
          AND (
              $5 = ''
              OR ($5 = 'not_expired' AND licenses.expires_at IS NOT NULL AND licenses.expires_at > NOW())
              OR ($5 = 'expired' AND licenses.expires_at IS NOT NULL AND licenses.expires_at <= NOW())
              OR ($5 = 'no_expiry' AND licenses.expires_at IS NULL)
          )
        ORDER BY licenses.id DESC
        LIMIT 300
        "#,
    )
    .bind(owner_ids)
    .bind(query)
    .bind(query_pattern)
    .bind(activation_status)
    .bind(expiry_status)
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

fn normalized_filter(value: Option<&str>) -> &str {
    value.map(str::trim).unwrap_or_default()
}

fn activation_filter_options(selected: &str) -> String {
    select_options(
        &[
            ("", "全部"),
            ("unused", "未激活"),
            ("active", "已激活"),
            ("active_unbound", "已激活未绑定"),
            ("banned", "已封禁"),
        ],
        selected,
    )
}

fn expiry_filter_options(selected: &str) -> String {
    select_options(
        &[
            ("", "全部"),
            ("not_expired", "未到期"),
            ("expired", "已到期"),
            ("no_expiry", "永久或未设置到期"),
        ],
        selected,
    )
}

fn select_options(options: &[(&str, &str)], selected: &str) -> String {
    let mut html = String::new();

    for (value, label) in options {
        let selected_attr = if *value == selected { " selected" } else { "" };
        html.push_str(&format!(
            r#"<option value="{}"{}>{}</option>"#,
            escape(value),
            selected_attr,
            escape(label)
        ));
    }

    html
}

fn validate_adjust_hours(hours: i32) -> Result<(), Response> {
    if hours == 0 {
        return Err((StatusCode::BAD_REQUEST, "调整小时数不能为 0").into_response());
    }

    if !(-87_600..=87_600).contains(&hours) {
        return Err((
            StatusCode::BAD_REQUEST,
            "调整小时数必须在 -87600 到 87600 之间",
        )
            .into_response());
    }

    Ok(())
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
        "active" if item.machine_code.is_none() => "已激活未绑定".to_string(),
        "active" => "有效".to_string(),
        other => escape(other),
    }
}

fn format_beijing_time(time: chrono::DateTime<chrono::Utc>) -> String {
    time.with_timezone(&FixedOffset::east_opt(8 * 3600).expect("valid Beijing timezone offset"))
        .to_rfc3339()
}

fn internal_error(err: sqlx::Error) -> Response {
    eprintln!("database error: {err}");
    (StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误").into_response()
}
