# sensors-to-mqtt

Rust application that reads data from a wide range of sensors (I2C, GPIO, serial) and publishes it to an MQTT broker. Sensors can be connected **locally** (Linux hardware interfaces) or **remotely over TCP** via an [io-to-net](https://github.com/askrejans/io-to-net) bridge — enabling monitoring from any platform including macOS and Windows. Includes an interactive terminal UI with live charts, G-force visualisation, and a systemd-ready daemon mode.

Part of the **to-mqtt** ecosystem — see also [`gps-to-mqtt`](https://github.com/askrejans/gps-to-mqtt) and [`speeduino-to-mqtt`](https://github.com/askrejans/speeduino-to-mqtt).

![sensors](https://github.com/user-attachments/assets/835dcf36-17c0-42ce-aec8-379e9177f768)

---

## Key Capabilities

- **Broad sensor support** — I2C environmental, power, motion, light, and ADC sensors; GPIO digital inputs; particulate matter (PM2.5/PM10); a synthetic test sensor (no hardware required)
- **TCP bridge support** — connect to any sensor remotely via an [io-to-net](https://github.com/askrejans/io-to-net) bridge; all drivers work cross-platform over TCP
- **Real-time MQTT publishing** with automatic reconnection and QoS configuration
- **1-D Kalman filter** on numeric fields with configurable noise/process variance and dead-zone suppression
- **Interactive terminal UI** — tabbed per-sensor views, live sparkline charts, G-meter canvas, keyboard navigation
- **Daemon mode** — auto-detected when stdout is not a TTY; structured JSON logs, systemd-compatible
- **TOML configuration** with environment-variable overrides (`SENSORS_TO_MQTT__*`)

<img width="1324" height="794" alt="Screenshot 2026-03-11 at 00 19 16" src="https://github.com/user-attachments/assets/5dca56e8-aea9-4275-9d88-bc07ef368e33" />


---

## Supported Sensors

| Driver | Bus | Chip | Measurements |
|---|---|---|---|
| `mpu6500` | I2C / TCP | InvenSense MPU-6500 | Acceleration (g), gyroscope (°/s), tilt/lean/bank angles |
| `bmp280` | I2C / TCP | Bosch BMP280 | Temperature (°C), pressure (hPa), altitude (m) |
| `bme280` | I2C / TCP | Bosch BME280 | Temperature (°C), pressure (hPa), humidity (%), altitude (m) |
| `sht31` | I2C / TCP | Sensirion SHT31 | Temperature (°C), humidity (%) |
| `bh1750` | I2C / TCP | ROHM BH1750 | Ambient light (lux), light category |
| `ina219` | I2C / TCP | TI INA219 | Bus voltage (V), shunt voltage (mV), current (A), power (W), state-of-charge (%) |
| `ads1115` | I2C / TCP | TI ADS1115 | 4-channel 16-bit ADC — configurable per-channel gain, sample rate, and linear scaling |
| `gpio_button` | GPIO / TCP | — | State (0/1), press count, press duration; software debounce |
| `sds011` | Serial / TCP | Nova Fitness SDS011 | PM2.5 (μg/m³), PM10 (μg/m³), AQI (EPA) |
| `synthetic` | — | — | 15 simulated fields (g-force, gyro, temperature, pressure, humidity, battery, RPM, speed, throttle); sine/sawtooth waveforms |

> **I2C / TCP** — local hardware on Linux, or remote via TCP bridge on any platform.
> GPS and ECU (Speeduino) are handled by dedicated sibling projects.

---

## Installation

### Debian / Ubuntu

```bash
# Add g86racing APT repository
curl -fsSL https://g86racing.com/packages/deb/g86racing-archive-keyring.gpg \
    | sudo tee /usr/share/keyrings/g86racing-archive-keyring.gpg > /dev/null
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/g86racing-archive-keyring.gpg] \
    https://g86racing.com/packages/deb stable main" \
    | sudo tee /etc/apt/sources.list.d/g86racing.list

sudo apt-get update
sudo apt-get install sensors-to-mqtt
```

### Fedora / RHEL / AlmaLinux / Rocky Linux

```bash
# Add g86racing RPM repository
sudo tee /etc/yum.repos.d/g86racing.repo <<'EOF'
[g86racing]
name=g86racing packages
baseurl=https://g86racing.com/packages/rpm
enabled=1
gpgcheck=0
EOF

sudo dnf install sensors-to-mqtt
```

### macOS (Homebrew)

```bash
brew tap g86racing/g86racing
brew install sensors-to-mqtt
```

> On macOS only TCP bridge sensors are available (no local I2C/GPIO hardware). Use [io-to-net](https://github.com/askrejans/io-to-net) on the device with the physical sensors.

### Windows

Not supported — I2C and GPIO sensors require Linux hardware. Use TCP bridge mode from a Linux/macOS machine to access remote sensors.

### Docker

```bash
# Pull and run
docker pull ghcr.io/askrejans/sensors-to-mqtt:latest

# Or use docker-compose (edit docker-compose.yml for your sensor devices)
docker compose up -d
```

---

## Quick Start

### Requirements

- Rust 1.82+ (`rustup.rs`)
- **Local sensors (Linux):** I2C enabled (`raspi-config` → Interface Options → I2C), user in the `i2c` group
- **Remote sensors (all platforms):** [io-to-net](https://github.com/askrejans/io-to-net) bridge running on the device with the sensors

### Build and run (native)

```bash
cargo build --release
./target/release/sensors-to-mqtt --config config.toml
```

If stdout is a terminal the TUI launches automatically. Pipe or redirect stdout to suppress the TUI and get structured logs instead (daemon mode).

### Synthetic sensor (no hardware)

```toml
# config.toml
[[sensors]]
name    = "Synthetic IMU"
driver  = "synthetic"
enabled = true

[sensors.connection]
type    = "i2c"      # ignored by the synthetic driver — any value is accepted
device  = "/dev/i2c-1"
address = 0x00
```

---

## Configuration Reference

Configuration is loaded in this priority order (highest wins):

1. `SENSORS_TO_MQTT__*` environment variables (double-underscore separator)
2. File specified with `--config <path>`
3. `./config.toml`
4. `/etc/sensors-to-mqtt/config.toml`
5. Built-in defaults

### Top-level

```toml
log_level           = "info"     # trace | debug | info | warn | error
log_json            = false      # emit JSON log lines (useful in daemon mode)
tui_refresh_rate_ms = 100        # TUI redraw interval
```

### MQTT

```toml
[mqtt]
enabled         = true
host            = "localhost"
port            = 1883
base_topic      = "/SENSORS"
client_id       = "sensors-to-mqtt"
keep_alive_secs = 20
# username      = "user"
# password      = "pass"
```

### Sensor entries

Each `[[sensors]]` block configures one sensor instance.

```toml
[[sensors]]
name    = "My Sensor"   # used as MQTT sub-topic and TUI tab name
driver  = "<driver>"    # see table above
enabled = true          # set false to skip without removing the block

[sensors.connection]
# see connection types below

[sensors.settings]
# driver-specific settings (all optional)
```

### Connection types

**I2C** (Linux only — direct hardware)
```toml
[sensors.connection]
type    = "i2c"
device  = "/dev/i2c-1"   # default
address = 0x68           # 7-bit hex address
```

**TCP** (all platforms — connects to an [io-to-net](https://github.com/askrejans/io-to-net) bridge)
```toml
[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"   # IP of the io-to-net bridge device
port    = 9002              # port configured on the bridge
address = 0x68              # I2C device address (required for I2C sensors)
                            # omit for serial-over-TCP sensors (e.g. sds011)
framing = true              # set true when the bridge sends length-prefixed frames
                            # (2-byte big-endian length header before each response)
                            # default: false (raw byte stream)
```

**GPIO** (Linux only — sysfs)
```toml
[sensors.connection]
type        = "gpio"
pin         = 17          # BCM pin number
active_low  = false       # true = LOW means pressed/active
debounce_ms = 50          # software debounce window
```

**Serial** (Linux / macOS — USB or UART)
```toml
[sensors.connection]
type      = "serial"
port      = "/dev/ttyUSB0"   # or /dev/ttyS0, /dev/ttyAMA0, COM3, etc.
baud_rate = 9600
```

---

## Sensor Configuration Examples

### MPU-6500 IMU — local I2C

```toml
[[sensors]]
name   = "Front IMU"
driver = "mpu6500"

[sensors.connection]
type    = "i2c"
address = 0x68
```

### MPU-6500 IMU — remote TCP bridge

```toml
[[sensors]]
name   = "Front IMU"
driver = "mpu6500"

[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"
port    = 9002
framing = true   # required when bridge uses frame_mode = "length_prefix"
# address is not needed — the bridge handles I2C addressing internally
```

> **Bridge config note:** set `read_len = 14` and `read_reg = 0x3B` in the io-to-net bridge.
> The driver reads all six axes in a single 14-byte burst (ACCEL_XYZ + TEMP + GYRO_XYZ).
> Client writes are ignored by the bridge when `read_only = true`.

### BME280 — temperature, pressure, humidity

```toml
[[sensors]]
name   = "Cabin Climate"
driver = "bme280"

[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"
port    = 9003
address = 0x76
```

### SHT31 — temperature and humidity

```toml
[[sensors]]
name   = "Engine Bay Temp"
driver = "sht31"

[sensors.connection]
type    = "i2c"
address = 0x44
```

### BMP280 — temperature and pressure

```toml
[[sensors]]
name   = "Intake Pressure"
driver = "bmp280"

[sensors.connection]
type    = "i2c"
address = 0x77
```

### BH1750 — ambient light

```toml
[[sensors]]
name   = "Dashboard Light"
driver = "bh1750"

[sensors.connection]
type    = "i2c"
address = 0x23    # 0x23 (ADDR=GND) or 0x5C (ADDR=VCC)
```

### INA219 — current, voltage, power

```toml
[[sensors]]
name   = "Battery Monitor"
driver = "ina219"

[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"
port    = 9004
address = 0x40

[sensors.settings]
shunt_ohms    = 0.1     # shunt resistor value in ohms
max_current_a = 3.2     # determines current resolution
battery_min_v = 11.0    # for state-of-charge estimation
battery_max_v = 14.4
```

### ADS1115 — 4-channel ADC (analog sensors)

Each channel has an independent linear scale: `value = (volts - offset) * scale`.

```toml
[[sensors]]
name   = "Analog Inputs"
driver = "ads1115"

[sensors.connection]
type    = "i2c"
address = 0x48    # 0x48–0x4B depending on ADDR pin

[sensors.settings]
gain        = 4.096   # PGA full-scale range (V): 6.144|4.096|2.048|1.024|0.512|0.256
sample_rate = 128     # SPS: 8|16|32|64|128|250|475|860

[[sensors.settings.channels]]
index  = 0
label  = "Throttle"
unit   = "%"
scale  = 25.6    # maps 0–3.9 V → 0–100 %
offset = 0.0

[[sensors.settings.channels]]
index  = 1
label  = "Coolant Temp"
unit   = "°C"
scale  = 100.0
offset = -1.0
```

### GPIO button / switch — local

```toml
[[sensors]]
name   = "Brake Light Switch"
driver = "gpio_button"

[sensors.connection]
type        = "gpio"
pin         = 17
active_low  = false
debounce_ms = 20
```

### GPIO button / switch — remote TCP

```toml
[[sensors]]
name   = "Brake Light Switch"
driver = "gpio_button"

[sensors.connection]
type = "tcp"
host = "192.168.88.58"
port = 9005
```

Published fields: `state` (0.0 / 1.0), `press_count`, `press_duration_ms`.

### SDS011 — PM2.5 / PM10 air quality

Supports two transports.

**USB-serial** — plug the SDS011 in via USB (appears as `/dev/ttyUSB0`).
If permission is denied: `sudo usermod -aG dialout $USER`.

```toml
[[sensors]]
name   = "Air Quality"
driver = "sds011"

[sensors.connection]
type      = "serial"
port      = "/dev/ttyUSB0"
baud_rate = 9600           # always 9600 for SDS011
```

**Serial-over-IP (raw TCP)** — e.g. an io-to-net bridge, ESP8266 / ESP32, or a serial device server.

```toml
[[sensors]]
name   = "Air Quality (remote)"
driver = "sds011"

[sensors.connection]
type = "tcp"
host = "192.168.1.42"   # address of the bridge
port = 8880             # raw-TCP port configured on the bridge
```

Published fields: `pm2_5` (μg/m³), `pm10` (μg/m³), `aqi_pm2_5`, `aqi_pm10` (US EPA index).

### Synthetic test sensor

```toml
[[sensors]]
name   = "Synthetic"
driver = "synthetic"

[sensors.connection]
type    = "i2c"
address = 0x00

[sensors.settings]
rate_hz = 50     # simulated sample rate
speed   = 1.0    # waveform speed multiplier
noise   = 0.02   # noise amplitude
```

---

## TCP Bridge Setup (io-to-net)

To use sensors remotely, run [io-to-net](https://github.com/askrejans/io-to-net) on the device that has the sensors physically connected (e.g. a Raspberry Pi):

```bash
# Install and configure io-to-net on the sensor host
# Then point sensors-to-mqtt at it from any machine
```

Each sensor gets its own TCP port in the bridge config. The I2C `address` field in `sensors-to-mqtt` tells the bridge which device on the bus to talk to.

---

## CLI Flags

```
sensors-to-mqtt [OPTIONS]

  -c, --config <PATH>   Config file (default: ./config.toml)
      --no-mqtt         Disable MQTT publishing (TUI-only mode)
      --log-level       Override log level (trace|debug|info|warn|error)
  -h, --help            Show help
```

---

## TUI Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `←` / `→` or `h` / `l` | Switch sensor tab |
| `↑` / `↓` or `k` / `j` | Scroll data list |
| `r` | Recalibrate active sensor |
| `e` | Toggle sensor enabled/disabled |
| `q` / `Esc` | Quit |
| `?` | Toggle help overlay |

---

## MQTT Topics

Topics follow the pattern `<base_topic>/IMU/<sensor_name>/<subtopic>`.

With `base_topic = "/SENSORS"` and `name = "Front IMU"`:

| Topic | Payload | Description |
|-------|---------|-------------|
| `/SENSORS/IMU/Front IMU/INFO` | `{"sensor":"…","timestamp":"…"}` | Heartbeat on every reading |
| `/SENSORS/IMU/Front IMU/FILTERED` | `{"timestamp":"…","accel_x":…,…}` | Kalman-filtered motion fields |
| `/SENSORS/IMU/Front IMU/DERIVED` | `{"timestamp":"…","g_force_x":…,…}` | Derived fields (G-force, tilt, etc.) |

Payloads are JSON objects. Numeric values are `f64`.

---

## Systemd Service

The installed package creates a `sensors` system user and installs a hardened unit file.

```bash
# Enable and start
sudo systemctl enable --now sensors-to-mqtt

# View logs
sudo journalctl -u sensors-to-mqtt -f

# Check status
sudo systemctl status sensors-to-mqtt
```

Copy the example config and edit for your sensors:

```bash
sudo cp /etc/sensors-to-mqtt/settings.toml.example /etc/sensors-to-mqtt/settings.toml
sudo nano /etc/sensors-to-mqtt/settings.toml
sudo systemctl restart sensors-to-mqtt
```

### I2C / GPIO permissions

The service user `sensors` is automatically added to the `i2c`, `dialout`, and `gpio` groups at install time. If you add sensors after installation, ensure the device group matches:

```bash
# Verify groups
id sensors

# Add manually if needed
sudo usermod -aG i2c sensors
sudo systemctl restart sensors-to-mqtt
```

---

## Docker

```bash
# Build image
docker build -t sensors-to-mqtt .

# Run with docker-compose (edit docker-compose.yml first)
docker compose up -d

# View logs
docker compose logs -f sensors-to-mqtt
```

For I2C sensors, uncomment the `devices` and `group_add` sections in `docker-compose.yml` and set the correct `i2c` group GID:

```bash
getent group i2c | cut -d: -f3   # get GID
```

---

## Building Packages

Requires [`cross`](https://github.com/cross-rs/cross) for Linux cross-compilation:

```bash
cargo install cross
```

Build all platforms:

```bash
./scripts/build_packages.sh
```

Build specific targets:

```bash
./scripts/build_packages.sh --platform linux --arch arm64 --type deb
./scripts/build_packages.sh --platform linux --type rpm
./scripts/build_packages.sh --platform mac
./scripts/build_packages.sh --platform linux --arch x64 --no-cross
```

Packages are written to `./release/<version>/`.

---

## Environment Variables

Any config key can be overridden with an environment variable using double-underscore as the nesting separator:

```bash
SENSORS_TO_MQTT__MQTT__HOST=192.168.1.10
SENSORS_TO_MQTT__MQTT__PORT=1884
SENSORS_TO_MQTT__LOG_LEVEL=debug
```

---

## Adding a New Sensor Driver

1. Create `src/sensors/i2c/<driver>.rs` (or `src/sensors/gpio/<driver>.rs`, `src/sensors/serial/<driver>.rs`)
2. Implement the `Sensor` trait — `init()`, `read()`, `field_descriptors()`
3. For I2C drivers, use `open_i2c(cfg, default_address)` from `crate::transport` to get a `Box<dyn I2cBus>` — this gives local I2C on Linux and TCP on all platforms automatically
4. Add `pub mod <driver>;` in the appropriate `mod.rs`
5. Add a match arm in `src/sensors/registry.rs`
6. Write inline unit tests in the driver file

The TUI renders fields automatically based on the `VizType` in each `FieldDescriptor`:

| `VizType` | Rendering |
|-----------|-----------|
| `Value` | Text value |
| `Numeric { unit }` | Value with unit suffix |
| `GForce` | G-meter canvas + sparkline |
| `AngularRate` | Sparkline (°/s) |
| `Angle` | Sparkline (°) |

---

## Development

```bash
# Run all tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run

# Check for errors without building
cargo check
```

---

## License

MIT — see [LICENSE](LICENSE).

