# 用 Docker 编译 x86_64 Linux 版本

这个项目是 Rust + Axum + PostgreSQL 服务。当前 Docker 编译流程只保留一种方式：先构建一个专门用于 `x86_64` Linux 的编译容器，然后你自己启动容器、连接进去，并手动运行 `cargo build` 命令。

这样做的好处是编译环境固定在 Linux `x86_64`，同时源码、`target`、Cargo registry 和 Cargo git 缓存都会保留在本地或 Docker volume 里，后续重复编译会快很多。

## 构建 x86_64 编译镜像

在项目根目录执行：

```bash
docker compose -f docker-compose.build.yml build
```

这个命令会构建镜像：

```text
license-server-builder:x86_64-rust-1.95
```

`docker-compose.build.yml` 已经固定：

```yaml
platform: linux/amd64
```

所以即使你在 Apple Silicon Mac 上执行，builder 容器也会按 Linux `x86_64` 环境运行。

## 启动编译容器

```bash
docker compose -f docker-compose.build.yml up -d builder
```

容器名是：

```text
license-server-builder
```

这个容器不会自动编译，它会常驻运行，等待你连接进去手动执行命令。

## 连接到编译容器

```bash
docker exec -it license-server-builder bash
```

进入容器后，当前目录是：

```text
/workspace
```

这个目录挂载的是项目根目录。

## 手动编译

在容器里执行：

```bash
cargo build --release
```

编译成功后的 x86_64 Linux 可执行文件在宿主机项目目录：

```text
target/release/license-server
```

你也可以在容器里手动运行其他 Cargo 命令：

```bash
cargo check
cargo test
cargo clean
```

退出容器 shell：

```bash
exit
```

## 停止编译容器

```bash
docker compose -f docker-compose.build.yml stop builder
```

再次需要编译时重新启动：

```bash
docker compose -f docker-compose.build.yml up -d builder
docker exec -it license-server-builder bash
```

如果要删除这个常驻容器：

```bash
docker compose -f docker-compose.build.yml down
```

Cargo 缓存 volume 默认会保留。删除容器不会删除这些缓存。

## 清理编译缓存

只清理 Rust 编译产物：

```bash
rm -rf target
```

同时删除 Cargo registry 和 git 缓存：

```bash
docker compose -f docker-compose.build.yml down -v
```

下次编译会重新下载依赖。

## 运行 PostgreSQL

如果你只是在本机用 Docker 跑数据库，可以先创建网络：

```bash
docker network create license-net
```

启动 PostgreSQL：

```bash
docker run -d \
  --name license-postgres \
  --network license-net \
  -p 5432:5432 \
  -e POSTGRES_PASSWORD=manong666. \
  -e POSTGRES_DB=license_server \
  -v license-postgres-data:/var/lib/postgresql/data \
  docker.m.daocloud.io/library/postgres:16
```

## 运行编译出来的程序

编译出来的是 Linux `x86_64` 可执行文件。把它放到 Linux `x86_64` 机器上后，准备数据库连接和环境变量：

```bash
export DATABASE_URL='postgres://postgres:manong666.@127.0.0.1:5432/license_server'
export ADMIN_USER='admin'
export ADMIN_PASSWORD='Manong1314520.'
export SERVER_ADDR='0.0.0.0:3000'
export APP_API_KEY='901F0CF02D2B48D19966A4947E884398'
export CLIENT_AES_KEY='client-aes-key-32-bytes-demo!!!!'
export SERVER_AES_KEY='server-aes-key-32-bytes-demo!!!!'

./target/release/license-server
```

启动后访问：

```text
http://127.0.0.1:3000/login
```

第一次启动时，如果数据库里没有管理员，程序会自动使用 `ADMIN_USER` 和 `ADMIN_PASSWORD` 创建初始管理员。

## 验证 API

验证接口只接受 AES-256-GCM 加密后的 JSON。可以使用项目里的 Rust 示例客户端测试：

```bash
export VERIFY_API_URL='http://1.15.171.108:3000/api/verify'
export APP_API_KEY='901F0CF02D2B48D19966A4947E884398'
export CLIENT_AES_KEY='client-aes-key-32-bytes-demo!!!!'
export SERVER_AES_KEY='server-aes-key-32-bytes-demo!!!!'
export LICENSE_KEY='LIC-f55f3d691ef24774b60d6c12eb4576ac'
export MACHINE_CODE='machine-001'

cargo run --features client-example --example encrypted_client
```

## 生产部署

把编译出来的文件复制到目标 Linux `x86_64` 服务器：

```bash
scp target/release/license-server user@server:/opt/license-server/
```

在服务器上设置环境变量后运行即可。目标服务器必须是 Linux `x86_64`，并且能够连接 PostgreSQL。

生产环境不要继续使用文档里的默认密码和默认 API Key，至少要修改：

```text
ADMIN_PASSWORD
APP_API_KEY
DATABASE_URL
CLIENT_AES_KEY
SERVER_AES_KEY
```

## 常见问题

### 拉取基础镜像超时

项目里的 Docker 文件使用了 Docker Hub 代理地址：

```text
docker.m.daocloud.io/library/rust:1.95-bookworm
docker.m.daocloud.io/library/postgres:16
```

如果你的网络可以正常访问 Docker Hub，可以改回：

```text
rust:1.95-bookworm
postgres:16
```

### 容器里执行 cargo 很慢

第一次编译会下载依赖并完整构建，耗时较长。后续会复用这些缓存：

```text
cargo-registry
cargo-git
target
```

### 容器启动后连不上数据库

如果服务和数据库都在 Docker 容器里，并且位于同一个 Docker 网络，`DATABASE_URL` 里的主机名要写数据库容器名：

```text
postgres://postgres:postgres@license-postgres:5432/license_server
```

不要写 `localhost`。在容器里，`localhost` 指的是当前容器自己。
