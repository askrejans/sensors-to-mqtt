# ── Build stage ────────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder

# cmake + perl for aws-lc-sys (rustls crypto provider), libudev for serialport
RUN apt-get update && apt-get install -y --no-install-recommends \
        perl make cmake pkg-config libudev-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies: copy manifests first, build a dummy main, then replace
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main(){}' > src/main.rs \
    && cargo build --release --locked 2>/dev/null || true \
    && rm -rf src

# Build the real binary. The copied sources keep their host mtimes, which can be
# older than the dummy build above – without touching them cargo considers the
# dummy artifact fresh and the image ships a binary that does nothing.
COPY . .
RUN find src web -type f -exec touch {} + \
    && cargo build --release --locked

# ── Runtime stage ──────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libudev1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r sensors-to-mqtt \
    && useradd -r -g sensors-to-mqtt --groups dialout --no-create-home \
        --shell /usr/sbin/nologin sensors-to-mqtt \
    && mkdir -p /etc/sensors-to-mqtt

COPY --from=builder /app/target/release/sensors-to-mqtt /usr/local/bin/sensors-to-mqtt
COPY example.settings.toml /etc/sensors-to-mqtt/settings.toml
COPY LICENSE /usr/share/doc/sensors-to-mqtt/copyright

USER sensors-to-mqtt

# Loopback only unless SENSORS_TO_MQTT_HTTP_BIND=0.0.0.0 (then also set
# SENSORS_TO_MQTT_HTTP_TOKEN).
ENV SENSORS_TO_MQTT_HTTP_ENABLED=true \
    SENSORS_TO_MQTT_HTTP_BIND=127.0.0.1

# HTTP API, web dashboard and Prometheus metrics
EXPOSE 9092

# Healthy = every enabled sensor delivers (HTTP 200 from /health).
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["sensors-to-mqtt", "--healthcheck"]

# I2C/GPIO devices: pass them in (devices: /dev/i2c-1) and add the host's i2c
# group (group_add). Configure with SENSORS_TO_MQTT_* or mount a settings.toml.
ENTRYPOINT ["sensors-to-mqtt", "--headless"]
CMD ["--config", "/etc/sensors-to-mqtt/settings.toml"]
