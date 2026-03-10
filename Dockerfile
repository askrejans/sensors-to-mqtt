# Build stage
FROM rust:1-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake perl make pkg-config libudev-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system sensors \
    && useradd --system --no-create-home --shell /usr/sbin/nologin \
       --gid sensors --groups dialout sensors

COPY --from=builder /build/target/release/sensors-to-mqtt /usr/bin/sensors-to-mqtt
COPY example.settings.toml /etc/sensors-to-mqtt/settings.toml.example

RUN mkdir -p /etc/sensors-to-mqtt && chown sensors:sensors /etc/sensors-to-mqtt

USER sensors

ENTRYPOINT ["/usr/bin/sensors-to-mqtt"]
CMD ["--config", "/etc/sensors-to-mqtt/settings.toml"]
