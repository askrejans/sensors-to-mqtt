# Sensors-to-MQTT

Rust application for reading sensor data from I2C devices and publishing to MQTT brokers. Supports interactive terminal UI mode and daemon mode for systemd service deployment.

## Features

- I2C sensor support (MPU6500 IMU with extensible architecture)
- Real-time MQTT publishing with automatic reconnection
- Kalman filtering with configurable parameters
- Interactive terminal UI with live charts and G-force visualization
- Daemon mode for background service operation
- Structured logging with configurable levels
- YAML configuration with CLI overrides

## Quick Start

1. Install [Rust and Cargo](https://www.rust-lang.org)
2. Clone this repository
3. Configure `config.yaml` for your sensors and MQTT broker
4. Build and run:
   ```bash
   cargo build --release
   cargo run
   ```

## Configuration

Create or edit `config.yaml`:

```yaml
# Service configuration
service:
  run_mode: interactive
  update_interval_ms: 10
  auto_reconnect: true
  reconnect_delay_ms: 1000

# Logging
logging:
  level: info
  colored: true

# MQTT broker
mqtt:
  host: "localhost"
  port: 1883
  base_topic: "/SENSORS"
  client_id: "sensors-to-mqtt"
  qos: 1
  # Optional authentication
  # username: "user"
  # password: "pass"

# Sensors
sensors:
  - type: i2c
    bus: /dev/i2c-1
    devices:
      - name: mpu6500_1
        address: 0x68
        driver: mpu6500
        enabled: true
        settings:
          accel_range: 16
          gyro_range: 2000
          sample_rate: 100
          # Kalman filter tuning
          accel_filter:
            process_noise: 0.00001
            measurement_noise: 0.05
            dead_zone: 0.005
          accel_z_filter:
            process_noise: 0.000005
            measurement_noise: 0.08
            dead_zone: 0.008
          gyro_filter:
            process_noise: 0.00005
            measurement_noise: 0.025
            dead_zone: 0.1
```

## Usage

```bash
# Interactive mode with TUI (default)
./sensors-to-mqtt

# Daemon mode (no UI)
./sensors-to-mqtt --mode daemon

# With options
./sensors-to-mqtt --config /path/to/config.yaml --log-level debug
```

### Keyboard Shortcuts (Interactive Mode)

- `q` / `Esc` - Quit
- `Space` - Toggle measuring
- `↑`/`↓` - Select sensor
- `d` - Toggle sensor on/off
- `c` - Clear charts
- `r` - Reload config
- `?` - Show help

### Running as systemd service

```bash
sudo cp target/release/sensors-to-mqtt /usr/local/bin/
sudo mkdir -p /etc/sensors-to-mqtt
sudo cp config.yaml /etc/sensors-to-mqtt/
sudo cp sensors-to-mqtt.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now sensors-to-mqtt
```

## MQTT Topics

Data is published to three topics per sensor:

### `/SENSORS/IMU/{sensor_name}/INFO`
Sensor identification and configuration
```json
{
  "sensor": "mpu6500_1",
  "timestamp": "2025-12-19T10:30:45.123Z"
}
```

### `/SENSORS/IMU/{sensor_name}/FILTERED`
Filtered sensor readings
```json
{
  "timestamp": "2025-12-19T10:30:45.123Z",
  "accel_x": 0.042,
  "accel_y": -0.015,
  "accel_z": 1.002,
  "gyro_x": -0.5,
  "gyro_y": 0.2,
  "gyro_z": 0.1
}
```

### `/SENSORS/IMU/{sensor_name}/DERIVED`
Calculated values (angles, rates, G-forces)
```json
{
  "timestamp": "2025-12-19T10:30:45.123Z",
  "g_force_x": 0.042,
  "g_force_y": -0.015,
  "g_force_z": 0.002,
  "roll_rate": -0.5,
  "pitch_rate": 0.2,
  "yaw_rate": 0.1,
  "lean_angle": -0.86,
  "bank_angle": 2.41
}
```

## Development

```bash
# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run

# Cross-compile for Raspberry Pi
cargo install cross
cross build --release --target armv7-unknown-linux-gnueabihf
```

## License

MIT
