# syntax=docker/dockerfile:1

# --- Build stage ---
FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

# --- Runtime stage ---
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        curl \
        nodejs \
        npm \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/flare /usr/local/bin/flare

ENV FLARE_DATA_DIR=/data
ENV PORT=8080
VOLUME ["/data"]
EXPOSE 8080

CMD ["flare"]
