# syntax=docker/dockerfile:1
FROM rust:1-bookworm@sha256:59037199c44290f2befcdd58dcc540164763fc296950255aaefeef096a1866b0 AS builder
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /source/target/release/durable-streams-server /usr/local/bin/durable-streams-server
VOLUME ["/data"]
EXPOSE 4437 4438
CMD ["durable-streams-server", "--host", "0.0.0.0", "--port", "4437", "--h2-port", "4438", "--data-dir", "/data/durable-streams", "--durability", "wal"]
