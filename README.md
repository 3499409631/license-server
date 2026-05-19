# license-server

一个用 Rust + Axum + PostgreSQL 实现的卡密验证系统示例，包含客户端验证 API 和后台管理网页。

## 功能

- 客户端通过 `POST /api/verify` 验证卡密。
- 卡密首次验证成功时绑定机器码。
- 验证结果包括：有效、到期、封禁、不存在、机器码不匹配。
- 每次验证都会记录 IP、卡密、机器码和结果。
- 后台支持管理员、代理、子代理登录。
- 后台支持创建代理/子代理、卡密类型、生成卡密、封禁/解封卡密。

## 环境变量

```bash
export DATABASE_URL='postgres://postgres:manong666.@localhost:5432/license_server'
export ADMIN_USER='admin'
export ADMIN_PASSWORD='admin123'
export SERVER_ADDR='0.0.0.0:3000'
export APP_API_KEY='901F0CF02D2B48D19966A4947E884398'
export CLIENT_AES_KEY='client-aes-key-32-bytes-demo!!!!'
export SERVER_AES_KEY='server-aes-key-32-bytes-demo!!!!'
```

第一次启动时，如果数据库里没有管理员，会自动使用 `ADMIN_USER` 和 `ADMIN_PASSWORD` 创建初始管理员。

`CLIENT_AES_KEY` 和 `SERVER_AES_KEY` 必须都是 32 字节字符串。`CLIENT_AES_KEY` 用来加密客户端请求，`SERVER_AES_KEY` 用来加密服务端响应。

## 运行

先创建数据库：

```bash
createdb license_server
```

如果你用 Docker 跑 PostgreSQL，可以先启动一个本地数据库：

```bash
docker run --name license-postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=license_server -p 5432:5432 -d postgres:16
```

```bash
cargo run
```

启动后访问：

```text
http://127.0.0.1:3000/login
```

## 客户端验证 API

验证接口只接受 AES-256-GCM 加密后的 JSON：

```json
{
  "data": "base64(nonce+ciphertext)"
}
```

运行 Rust 客户端示例：

```bash
export VERIFY_API_URL='http://127.0.0.1:3000/api/verify'
export APP_API_KEY='901F0CF02D2B48D19966A4947E884398'
export CLIENT_AES_KEY='client-aes-key-32-bytes-demo!!!!'
export SERVER_AES_KEY='server-aes-key-32-bytes-demo!!!!'
export LICENSE_KEY='LIC-03a5f94d3e3a4f239b76f36d829d4115'
export MACHINE_CODE='机器码-001'

cargo run --features client-example --example encrypted_client
```

解密后的返回示例：

```json
{
  "status": "valid",
  "message": "卡密有效",
  "expires_at": "2026-06-19T08:00:00+08:00",
  "license_type": "月卡"
}
```

`duration_days = 0` 表示永久卡，返回的 `expires_at` 会是 `null`。
