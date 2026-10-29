FROM node:22-bookworm-slim AS web-build

WORKDIR /workspace
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build:web

FROM rust:1.97-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS server-build

WORKDIR /workspace
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./src-tauri/
COPY src-tauri/build.rs ./src-tauri/build.rs
COPY src-tauri/src ./src-tauri/src
RUN cargo build --release --manifest-path src-tauri/Cargo.toml \
    --locked --no-default-features --features server --bin soundrobe-server

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /app/dist /config /libraries /tmp \
    && chown -R 65532:65532 /app /config /libraries \
    && chmod 1777 /tmp

COPY --from=server-build /workspace/src-tauri/target/release/soundrobe-server /usr/local/bin/soundrobe-server
COPY --from=web-build /workspace/dist /app/dist

ENV SOUNDROBE_LISTEN_ADDR=0.0.0.0:8080 \
    SOUNDROBE_DATA_DIR=/config \
    SOUNDROBE_LIBRARY_ROOT_DIR=/libraries \
    SOUNDROBE_WEB_ROOT=/app/dist \
    HOME=/tmp

EXPOSE 8080
USER 65532:65532
STOPSIGNAL SIGTERM

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/soundrobe-server"]
