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
