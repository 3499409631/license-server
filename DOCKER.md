# 编译 Linux 版本并用 Docker 运行

这个项目是 Rust + Axum + PostgreSQL 服务。推荐直接用 Docker 多阶段构建：不需要在 macOS 上手动配置 Linux 交叉编译工具链，Docker 会在 Linux 环境里编译出 Linux 可执行文件。

## 方式一：直接构建 Docker 镜像

在项目根目录新建 `Dockerfile`：

```dockerfile
FROM docker.m.daocloud.io/library/rust:1.95-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM docker.m.daocloud.io/library/debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/license-server /usr/local/bin/license-server

ENV SERVER_ADDR=0.0.0.0:3000
EXPOSE 3000

CMD ["license-server"]
```

建议同时新建 `.dockerignore`，避免把本地编译产物复制进镜像构建上下文：

```dockerignore
target
.git
.DS_Store
```

构建镜像：

```bash
docker build -t license-server:latest .
```

如果你能稳定访问 Docker Hub，也可以把 `Dockerfile` 里的镜像改回官方地址：

```dockerfile
FROM rust:1.95-bookworm AS builder
FROM debian:bookworm-slim
```

如果你在 Apple Silicon Mac 上构建，但最终要部署到普通 Linux x86_64 服务器，可以指定平台：

```bash
docker build --platform linux/amd64 -t license-server:latest .
```

如果目标服务器也是 ARM64 Linux，则用：

```bash
docker build --platform linux/arm64 -t license-server:latest .
```

## 可复用的编译容器

如果你想要一个专门用来编译 Linux 版本的 Docker 容器，使用项目里的 `Dockerfile.builder` 和 `docker-compose.build.yml`。

先构建 builder 镜像：

```bash
docker compose -f docker-compose.build.yml build
```

以后每次编译都运行：

```bash
docker compose -f docker-compose.build.yml run --rm builder
```

编译成功后的 Linux 可执行文件在：

```text
target/release/license-server
```

这个 builder 会复用 Cargo registry、Cargo git 和 `target` 缓存，第二次以后会快很多。

也可以直接在 builder 容器里执行其他 Cargo 命令：

```bash
docker compose -f docker-compose.build.yml run --rm builder cargo test
docker compose -f docker-compose.build.yml run --rm builder cargo check
docker compose -f docker-compose.build.yml run --rm builder cargo clean
```

如果你在 Apple Silicon Mac 上，但要编译给普通 Linux x86_64 服务器用，运行时指定平台：

```bash
docker compose -f docker-compose.build.yml run --rm --platform linux/amd64 builder
```

这个命令编出来的是 Linux x86_64 版本：

```text
target/release/license-server
```

## 启动 PostgreSQL

先创建一个 Docker 网络，让服务容器可以通过容器名访问数据库：

```bash
docker network create license-net
```

启动 PostgreSQL：

```bash
docker run -d \
  --name license-postgres \
  --network license-net \
  -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_DB=license_server \
  -v license-postgres-data:/var/lib/postgresql/data \
  docker.m.daocloud.io/library/postgres:16
```

## 启动 license-server

```bash
docker run -d \
  --name license-server \
  --network license-net \
  -p 3000:3000 \
  -e DATABASE_URL='postgres://postgres:postgres@license-postgres:5432/license_server' \
  -e ADMIN_USER='admin' \
  -e ADMIN_PASSWORD='admin123' \
  -e SERVER_ADDR='0.0.0.0:3000' \
  -e APP_API_KEY='change-me-api-key' \
  license-server:latest
```

启动后访问：

```text
http://127.0.0.1:3000/login
```

第一次启动时，如果数据库里没有管理员，程序会自动使用 `ADMIN_USER` 和 `ADMIN_PASSWORD` 创建初始管理员。

查看日志：

```bash
docker logs -f license-server
```

停止并删除服务容器：

```bash
docker rm -f license-server
```

停止并删除数据库容器：

```bash
docker rm -f license-postgres
```

如果要连数据库数据一起删除：

```bash
docker volume rm license-postgres-data
```

## 方式二：先编译 Linux 可执行文件，再放进 Docker

如果你只是想得到一个 Linux 二进制文件，可以用 `cross`：

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-gnu
```

生成文件位置：

```text
target/x86_64-unknown-linux-gnu/release/license-server
```

然后可以写一个只打包二进制的 `Dockerfile`：

```dockerfile
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY target/x86_64-unknown-linux-gnu/release/license-server /usr/local/bin/license-server

ENV SERVER_ADDR=0.0.0.0:3000
EXPOSE 3000

CMD ["license-server"]
```

构建并运行：

```bash
docker build -t license-server:latest .
docker run -d \
  --name license-server \
  --network license-net \
  -p 3000:3000 \
  -e DATABASE_URL='postgres://postgres:postgres@license-postgres:5432/license_server' \
  -e ADMIN_USER='admin' \
  -e ADMIN_PASSWORD='admin123' \
  -e SERVER_ADDR='0.0.0.0:3000' \
  -e APP_API_KEY='change-me-api-key' \
  license-server:latest
```

## 验证 API

```bash
curl -X POST 'http://127.0.0.1:3000/api/verify' \
  -H 'Content-Type: application/json' \
  -H 'X-Api-Key: change-me-api-key' \
  -d '{"license_key":"LIC-xxxx","machine_code":"machine-001"}'
```

## 常见问题

### 拉取基础镜像超时

如果看到类似下面的错误：

```text
failed to fetch anonymous token
i/o timeout
```

说明 Docker 当前网络拉取 Docker Hub 镜像超时。当前项目里的 `Dockerfile` 和 `docker-compose.yml` 已经使用 Docker Hub 代理地址：

```text
docker.m.daocloud.io/library/rust:1.95-bookworm
docker.m.daocloud.io/library/debian:bookworm-slim
docker.m.daocloud.io/library/postgres:16
```

如果你的网络可以正常访问 Docker Hub，可以改回：

```text
rust:1.95-bookworm
debian:bookworm-slim
postgres:16
```

### 容器启动后连不上数据库

如果服务容器和数据库容器在同一个 Docker 网络里，`DATABASE_URL` 里的主机名要写数据库容器名：

```text
postgres://postgres:postgres@license-postgres:5432/license_server
```

不要写 `localhost`。在容器里，`localhost` 指的是服务容器自己，不是 PostgreSQL 容器。

### 端口被占用

如果本机 `3000` 端口已经被占用，可以改左边的宿主机端口：

```bash
docker run -d \
  --name license-server \
  --network license-net \
  -p 8080:3000 \
  -e DATABASE_URL='postgres://postgres:postgres@license-postgres:5432/license_server' \
  -e ADMIN_USER='admin' \
  -e ADMIN_PASSWORD='admin123' \
  -e SERVER_ADDR='0.0.0.0:3000' \
  -e APP_API_KEY='change-me-api-key' \
  license-server:latest
```

然后访问：

```text
http://127.0.0.1:8080/login
```

### 生产环境注意

生产环境不要继续使用文档里的默认密码和默认 API Key，至少要修改：

```bash
ADMIN_PASSWORD
APP_API_KEY
DATABASE_URL
```
