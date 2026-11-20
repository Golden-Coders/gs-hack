# Jeden obraz ze wszystkimi binarkami backendu; compose wybiera binarkę komendą.
# cargo-chef cache'uje zależności (zwłaszcza rdkafka/librdkafka), więc zmiana
# kodu przebudowuje tylko nasze crate'y.

FROM rust:1-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends cmake && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY migrations migrations
RUN cargo build --release --bins

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/ingest /app/target/release/lifecycle /app/target/release/router \
     /app/target/release/sender /app/target/release/runbooks /app/target/release/api \
     /app/target/release/mock-targets /app/target/release/loadgen /usr/local/bin/
COPY config /app/config
ENV CONFIG_DIR=/app/config RUST_LOG=info
