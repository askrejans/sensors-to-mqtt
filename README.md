# sensors-to-mqtt

Rust application that reads data from a wide range of sensors (I2C, GPIO) and publishes it to an MQTT broker. Includes an interactive terminal UI with live charts, G-force visualisation, and a systemd-ready daemon mode.

Part of the **to-mqtt** ecosystem — see also [`gps-to-mqtt`](https://github.com/askrejans/gps-to-mqtt) and [`speeduino-to-mqtt`](https://github.com/askrejans/speeduino-to-mqtt).

![sensors](https://github.com/user-attachments/assets/835dcf36-17c0-42ce-aec8-379e9177f768)


## Features

- **Broad sensor support** — I2C environmental, power, motion, light, and ADC sensors; GPIO digital inputs; a synthetic test sensor (no hardware required)
- **Real-time MQTT publishing** with automatic reconnection and QoS configuration
- **1-D Kalman filter** on numeric fields with configurable noise/process variance and dead-zone suppression
- **Interactive terminal UI** — tabbed per-sensor views, live sparkline charts, G-meter canvas, keyboard navigation
- **Daemon mode** — auto-detected when stdout is not a TTY; structured JSON logs, systemd-compatible
- **TOML configuration** with environment-variable overrides (`SENSORS_TO_MQTT__*`)
- **Cross-compilation** and DEB/RPM packaging via `scripts/build_packages.sh`

---

## Supported Sensors

| Driver | Bus | Chip | Measurements |
|---|---|---|---|
| `mpu6500` | I2C | InvenSense MPU-6500 | Acceleration (g), gyroscope (°/s), temperature |
| `bmp280` | I2C | Bosch BMP280 | Temperature (°C), pressure (hPa) |
| `bme280` | I2C | Bosch BME280 | Temperature (°C), pressure (hPa), humidity (%) |
| `sht31` | I2C | Sensirion SHT31 | Temperature (°C), humidity (%) |
| `bh1750` | I2C | ROHM BH1750 | Ambient light (lux), light category |
| `ina219` | I2C | TI INA219 | Bus voltage (V), shunt voltage (mV), current (A), power (W), state-of-charge (%) |
| `ads1115` | I2C | TI ADS1115 | 4-channel 16-bit ADC — configurable per-channel gain, sample rate, and linear scaling |
| `gpio_button` | GPIO | — | State (0/1), press count, press duration; software debounce |
| `synthetic` | — | — | 15 simulated fields (g-force, gyro, temperature, pressure, humidity, battery, RPM, speed, throttle); sine/sawtooth waveforms |

> GPS and ECU (Speeduino) are handled by dedicated sibling projects.

---

## Quick Start

### Requirements

- Rust 1.82+ (`rustup.rs`)
- On Linux target: I2C enabled (`raspi-config` → Interface Options → I2C) and user in the `i2c` group

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

**I2C**
```toml
[sensors.connection]
type    = "i2c"
device  = "/dev/i2c-1"   # default
address = 0x68           # 7-bit hex address
```

**GPIO** (digital input / button / switch)
```toml
[sensors.connection]
type        = "gpio"
pin         = 17          # BCM pin number
active_low  = false       # true = LOW means pressed/active
debounce_ms = 50          # software debounce window
```

---

## Sensor Configuration Examples

### MPU-6500 IMU

```toml
[[sensors]]
name   = "Front IMU"
driver = "mpu6500"

[sensors.connection]
type    = "i2c"
address = 0x68
```

### BME280 — temperature, pressure, humidity

```toml
[[sensors]]
name   = "Cabin Climate"
driver = "bme280"

[sensors.connection]
type    = "i2c"
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
type    = "i2c"
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
gain        = "pga_4_096v"   # ±4.096 V full-scale
sample_rate = "sps_128"

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

**Gain options:** `pga_6_144v` · `pga_4_096v` · `pga_2_048v` (default) · `pga_1_024v` · `pga_0_512v` · `pga_0_256v`

**Sample-rate options:** `sps_8` · `sps_16` · `sps_32` · `sps_64` · `sps_128` (default) · `sps_250` · `sps_475` · `sps_860`

### GPIO button / switch

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

Published fields: `state` (0.0 / 1.0), `press_count`, `press_duration_ms`.

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

## Installation

### From packages (recommended for Raspberry Pi)

Download the latest `.deb` from Releases and install:

```bash
sudo dpkg -i sensors-to-mqtt_*.deb
```

### From source

```bash
cargo install --path .
```

---

## Systemd Service

The installed package includes a ready-made unit file. To enable manually:

```bash
sudo cp sensors-to-mqtt.service /lib/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now sensors-to-mqtt
sudo journalctl -u sensors-to-mqtt -f
```

Edit `ExecStart` in the unit file to point to your config:

```
ExecStart=/usr/bin/sensors-to-mqtt --config /etc/sensors-to-mqtt/config.toml
```

---

## Building Packages (DEB / RPM)

Requires [`cross`](https://github.com/cross-rs/cross) and [`fpm`](https://github.com/jordansissel/fpm):

```bash
cargo install cross
gem install fpm
```

Build all architectures and formats:

```bash
./scripts/build_packages.sh
```

Build a specific target:

```bash
./scripts/build_packages.sh --arch arm64 --type deb
./scripts/build_packages.sh --arch x86-64 --type rpm --no-cross
```

Packages are written to `./dist/`.

---

## Adding a New Sensor Driver

1. Create `src/sensors/i2c/<driver>.rs` (or `src/sensors/gpio/<driver>.rs`)
2. Implement the `Sensor` trait — `init()`, `read()`, `field_descriptors()`
3. Add `#[cfg(target_os = "linux")] pub mod <driver>;` in the appropriate `mod.rs`
4. Add a match arm in `src/sensors/registry.rs`
5. Write inline unit tests in the driver file

The TUI renders fields automatically based on the `VizType` in each `FieldDescriptor`:

| `VizType` | Rendering |
|-----------|-----------|
| `Value` | Text value |
| `Numeric { unit }` | Value with unit suffix |
| `GForce` | G-meter canvas + sparkline |
| `AngularRate` | Sparkline (°/s) |
| `Angle` | Sparkline (°) |

---

## Environment Variables

Any config key can be overridden with an environment variable using double-underscore as the nesting separator:

```bash
SENSORS_TO_MQTT__MQTT__HOST=192.168.1.10
SENSORS_TO_MQTT__MQTT__PORT=1884
SENSORS_TO_MQTT__LOG_LEVEL=debug
```

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

