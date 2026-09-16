FROM oven/bun:1.3.6 AS web
WORKDIR /app/web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ ./
RUN bun run build

FROM rust:1.95-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/
COPY install.sh ./
RUN cargo build --release --locked --bin camofy-cloud

FROM build AS verify
COPY examples/ examples/
RUN cargo test --locked --lib --bin camofy-cloud \
    && cargo build --locked --no-default-features --features agent --bin camofy-agent --example mock-core \
    && CAMOFY_TEST_CORE=/app/target/debug/examples/mock-core cargo test --locked --no-default-features --features agent --bin camofy-agent -- --ignored

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && useradd -r -u 10001 camofy
WORKDIR /app
COPY --from=build /app/target/release/camofy-cloud /usr/local/bin/
COPY --from=web /app/web/dist ./web/dist
USER camofy
ENV CAMOFY_LISTEN=0.0.0.0:3000 RUST_LOG=info
EXPOSE 3000
CMD ["camofy-cloud"]
