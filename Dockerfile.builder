FROM docker.m.daocloud.io/library/rust:1.95-bookworm

WORKDIR /workspace

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        pkg-config \
        build-essential \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_HOME=/usr/local/cargo

CMD ["cargo", "build", "--release"]
