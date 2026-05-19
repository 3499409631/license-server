use axum::response::Html;

use crate::models::SessionUser;

pub fn page(title: &str, user: Option<&SessionUser>, body: String) -> Html<String> {
    let nav = if let Some(user) = user {
        format!(
            r#"
            <nav>
                <strong>卡密验证系统</strong>
                <a href="/admin">首页</a>
                <a href="/admin/users">代理</a>
                <a href="/admin/types">类型</a>
                <a href="/admin/licenses">卡密</a>
                <a href="/admin/online">在线</a>
                <a href="/admin/settings">设置</a>
                <span>{} ({})</span>
                <form method="post" action="/logout"><button type="submit">退出</button></form>
            </nav>
            "#,
            escape(&user.username),
            escape(&user.role)
        )
    } else {
        "<nav><strong>卡密验证系统</strong></nav>".to_string()
    };

    Html(format!(
        r#"<!doctype html>
        <html lang="zh-CN">
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1">
            <title>{}</title>
            <style>
                body {{ margin: 0; font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #f6f7f9; color: #1f2933; }}
                nav {{ display: flex; align-items: center; gap: 16px; padding: 12px 24px; background: #ffffff; border-bottom: 1px solid #d9dee7; flex-wrap: wrap; }}
                nav a {{ color: #155c9e; text-decoration: none; }}
                nav form {{ margin-left: auto; }}
                main {{ max-width: 1120px; margin: 24px auto; padding: 0 20px; }}
                section {{ background: #ffffff; border: 1px solid #d9dee7; border-radius: 8px; padding: 18px; margin-bottom: 18px; }}
                h1, h2 {{ margin: 0 0 14px; }}
                table {{ width: 100%; border-collapse: collapse; background: #ffffff; }}
                th, td {{ border-bottom: 1px solid #e6eaf0; padding: 10px; text-align: left; vertical-align: top; }}
                th {{ background: #f0f3f7; }}
                label {{ display: block; font-weight: 600; margin: 10px 0 4px; }}
                input, select {{ width: 100%; max-width: 360px; box-sizing: border-box; padding: 8px 10px; border: 1px solid #c8d0dc; border-radius: 6px; }}
                button {{ padding: 8px 12px; border: 1px solid #1f6fb2; background: #1f6fb2; color: white; border-radius: 6px; cursor: pointer; }}
                .danger {{ border-color: #b42318; background: #b42318; }}
                .muted {{ color: #697586; }}
                .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; }}
                .stat {{ border: 1px solid #d9dee7; border-radius: 8px; padding: 14px; }}
                .stat strong {{ display: block; font-size: 26px; }}
                .error {{ color: #b42318; }}
                .actions {{ display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }}
                .actions form {{ display: flex; gap: 8px; align-items: center; margin: 0; }}
                .actions input {{ width: 160px; }}
            </style>
        </head>
        <body>
            {}
            <main>{}</main>
        </body>
        </html>"#,
        escape(title),
        nav,
        body
    ))
}

pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
