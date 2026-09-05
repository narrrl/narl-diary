# syntax=docker/dockerfile:1

# 1. Frontend. Built first so a Rust-only change does not re-run bun.
FROM oven/bun:1-alpine AS web
WORKDIR /web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ ./
RUN bun run build

# 2. Backend. rust-embed bakes web/dist into the binary in release mode,
#    so the result is a single self-contained executable.
FROM rust:1-slim-trixie AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY migrations/ ./migrations/
COPY --from=web /web/dist/ ./web/dist/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
 && cp target/release/narl-diary /usr/local/bin/narl-diary

# 3. Runtime. SQLite is compiled into the binary, so nothing else is needed.
FROM debian:trixie-slim
RUN useradd --system --uid 10001 --create-home --home-dir /home/diary diary \
 && mkdir -p /data \
 && chown diary:diary /data
COPY --from=build /usr/local/bin/narl-diary /usr/local/bin/narl-diary
USER diary
WORKDIR /home/diary
VOLUME ["/data"]
ENV DIARY_BIND=0.0.0.0:4242 \
    DIARY_DATA_DIR=/data
EXPOSE 4242
ENTRYPOINT ["/usr/local/bin/narl-diary"]
