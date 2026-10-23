# sensors-to-mqtt

Reads I2C, GPIO and serial sensors and publishes them to MQTT, an HTTP API and
a web dashboard, alongside
[`ecu-to-mqtt`](https://github.com/askrejans/ecu-to-mqtt) and
[`gps-to-mqtt`](https://github.com/askrejans/gps-to-mqtt). Sensors can be
connected **locally** (Linux I2C/GPIO/serial) or **remotely over TCP** through an
[io-to-net](https://github.com/askrejans/io-to-net) bridge, which works from any
platform. An interactive terminal UI shows live charts and a G-meter; on a
service it runs headless.

![sensors](https://github.com/user-attachments/assets/835dcf36-17c0-42ce-aec8-379e9177f768)

## Standalone MQTT use

This bridge can publish to any MQTT broker. Configure the broker host, credentials and `mqtt.base_topic` for your own installation; use TLS for a broker reached across an untrusted network. The source distribution and ordinary binaries have no account, subscription or activation dependency. Product-specific subscription checks belong to separately built managed distributions.

For a custom namespace, set `mqtt.base_topic` explicitly (for example `/telemetry/car-1/sensors`). An explicit prefix takes precedence over `car`. The optional legacy `car` shortcut keeps its existing `/g86/<car>/…` namespace for compatibility; omitting it does not create an account or select a vehicle profile. Existing defaults and packet fields remain compatible.

## Features

- **Canonical telemetry.** Every reading becomes one timestamped envelope
  (schema 1, `bootId`, `sequence`) on `<base>/<sensor>/telemetry`, with
  channel names shared across the ecosystem: `lateralG`, `longitudinalG`,
  `yawRateDegS`, `ambientC`, `baroKpa` and so on.
- **On demand for slow links.** A display publishes a reply topic to
  `<base>/telemetry/get` and receives every fresh channel of every sensor.
  This is the same long-poll contract as ecu-to-mqtt 0.7.3 and gps-to-mqtt
  0.7.2: each request is answered with a reading the display hasn't had, so
  it never builds a backlog.
- **Stable acquisition.**
  - Each sensor reads on its own thread, on a fixed schedule.
  - A sensor that fails to open, or keeps failing, is reopened with backoff
    (1 s up to 30 s).
  - A slow sensor, the broker or a browser can never stall another sensor.
- **More MQTT formats.**
  - `json`: a retained per-sensor state document with every field.
  - `homeassistant`: discovery for every channel.
  - `legacy`: the IMU topics of 0.5.
  - An `online`/`offline` status with a last will, and a retained
    `<base>/health` document (rate, sample age, fresh channels, errors,
    clock state) for monitoring consumers.
  - TLS, and the password from a file.
- **HTTP API and web dashboard** on port 9092:
  - Dashboard: G-ball, gauges, per-sensor fields, charts, diagnostics and
    configuration.
  - API: `/health`, Prometheus `/metrics`, REST and Server-Sent Events.
- **`--scan`** finds the chips on an I2C bus and prints a ready configuration
  block for each. Chips are identified by their ID registers.
- **Broad sensor support**: IMU, environmental, power, light, ADC, GPIO
  buttons, particulate matter, and a synthetic sensor for testing without
  hardware.
- **1-D Kalman filtering** with dead-zone suppression on the IMU.

<img width="1324" height="794" alt="Terminal UI" src="https://github.com/user-attachments/assets/5dca56e8-aea9-4275-9d88-bc07ef368e33" />

## Supported sensors

| Driver | Bus | Chip | Fields | Canonical channels |
|---|---|---|---|---|
| `mpu6500` | I2C / TCP | InvenSense MPU-6500 (also 6050/9250) | accel, gyro, g-force, angles | `lateralG` `longitudinalG` `verticalG` `combinedG` `peakG` `rollRateDegS` `pitchRateDegS` `yawRateDegS` `leanDeg` `bankDeg` `tiltDeg` |
| `bmp280` | I2C / TCP | Bosch BMP280 | temperature, pressure_hpa, altitude_m | `ambientC` `baroKpa` `baroAltitudeM` |
| `bme280` | I2C / TCP | Bosch BME280 | BMP280 + humidity_pct | + `humidityPct` |
| `sht31` | I2C / TCP | Sensirion SHT31 | temperature, humidity_pct | `ambientC` `humidityPct` |
| `bh1750` | I2C / TCP | ROHM BH1750 | lux, lux_category | `lightLux` |
| `ina219` | I2C / TCP | TI INA219 | bus_voltage_v, shunt_mv, current_a, power_w, soc_pct | `busV` `currentA` `powerW` `batterySocPct` |
| `ads1115` | I2C / TCP | TI ADS1115 | ch0–3 volts and mapped values | `<name>Ch0` … `<name>Ch3` (mapped values) |
| `gpio_button` | GPIO / TCP | — | state, press_count, press_duration_ms | `<name>Pressed` `<name>Presses` |
| `sds011` | Serial / TCP | Nova Fitness SDS011 | pm2_5, pm10, aqi_pm2_5, aqi_pm10 | `pm25` `pm10` `aqiPm25` `aqiPm10` |
| `synthetic` | — | — | 15 simulated fields | the IMU and environment channels (its rpm, speed and throttle stay out) |

- **I2C / TCP:** local hardware on Linux, or remote through a TCP bridge on
  any platform.
- **IMU mounting:** X right, Y forward, Z up. Keep the sensor still during
  the startup calibration (3 s). `verticalG` includes gravity (1 g at rest).
- **Units:** pressure is published as kPa on `baroKpa`, the unit
  ecu-to-mqtt uses.
- **Renaming channels:** rename or drop one per sensor with
  `[sensors.channels]` (see Configuration). Two enabled sensors that would
  publish the same channel are rejected at startup.

## Installation

### Debian / Ubuntu / Raspberry Pi OS

```bash
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
sudo tee /etc/yum.repos.d/g86racing.repo <<'REPO'
[g86racing]
name=g86racing packages
baseurl=https://g86racing.com/packages/rpm
enabled=1
gpgcheck=0
REPO
sudo dnf install sensors-to-mqtt
```

### macOS (Homebrew) and Windows

```bash
brew tap askrejans/g86racing
brew install sensors-to-mqtt
```

macOS and Windows have no local I2C or GPIO. They can use TCP bridge
sensors, and serial sensors such as the SDS011. Windows builds are a zip on
the GitHub release.

### Docker

```bash
docker compose up -d     # edit docker-compose.yml for devices and broker first
```

## Quick start

```bash
sensors-to-mqtt --scan                       # what is on /dev/i2c-1 (Linux)
sensors-to-mqtt --simulate                   # no hardware: one synthetic sensor
sensors-to-mqtt -c settings.toml             # TUI on a terminal
sensors-to-mqtt -c settings.toml --headless  # logs only (systemd, Docker)
sensors-to-mqtt -c settings.toml --check-config
```

The dashboard is at <http://127.0.0.1:9092/>. Example `--scan` output on a
Raspberry Pi with an MPU-6500:

```
/dev/i2c-1:
  0x68  MPU-6500                                     driver = "mpu6500"

Suggested configuration:

[[sensors]]
name    = "imu"
driver  = "mpu6500"
...
```

The scan only reads: a one-byte probe per address, then the ID registers of
the responders.

## Configuration

Configuration is loaded in this order (highest wins):

1. Environment variables. Use `SENSORS_TO_MQTT_<SECTION>_<KEY>`, for example
   `SENSORS_TO_MQTT_MQTT_HOST`, `SENSORS_TO_MQTT_HTTP_PORT` or
   `SENSORS_TO_MQTT_LOG_LEVEL`. The nested form of 0.5,
   `SENSORS_TO_MQTT__MQTT__HOST`, still works.
2. The file given with `--config`.
3. `./settings.toml`, `./config.toml`, `/etc/sensors-to-mqtt/settings.toml`,
   `/etc/sensors-to-mqtt/config.toml`.
4. Built-in defaults.

[example.settings.toml](example.settings.toml) documents every key. The main
ones:

| Key | Default | |
|---|---|---|
| `log_level` | `info` | trace, debug, info, warn, error |
| `log_json` | `false` | JSON log lines |
| `car` | — | G86 car slug (`^[a-z0-9][a-z0-9-]{0,31}$`); without an explicit `mqtt.base_topic` the base becomes `/g86/<car>/sensors`. Env `SENSORS_TO_MQTT_CAR` |
| `mqtt.enabled` | `true` | |
| `mqtt.host` / `mqtt.port` | `localhost` / `1883` | |
| `mqtt.base_topic` | `/SENSORS`, or `/g86/<car>/sensors` when `car` is set | an explicit value always wins |
| `mqtt.client_id` | unique per process | |
| `mqtt.username` / `mqtt.password` / `mqtt.password_file` | — | |
| `mqtt.qos` | `0` | live data |
| `mqtt.use_tls`, `mqtt.ca_cert_path`, `mqtt.client_cert_path`, `mqtt.client_key_path` | — | TLS |
| `mqtt.formats` | `["canonical", "json"]` | also `legacy`, `homeassistant` |
| `mqtt.max_rate_hz` | `10` | json and legacy, per sensor; 0 = every reading |
| `mqtt.homeassistant_prefix` | `homeassistant` | |
| `http.enabled` | `true` | |
| `http.bind` / `http.port` | `127.0.0.1` / `9092` | `0.0.0.0` for the network |
| `http.token` | — | bearer token for `/api/*` |
| `http.cors_origin` | — | |
| `http.ui` | `true` | web dashboard |

### Sensors

```toml
[[sensors]]
name    = "imu"        # MQTT topic level and TUI tab; no '/', '+' or '#'
driver  = "mpu6500"
enabled = true

[sensors.connection]   # i2c | tcp | gpio | serial, see below
type    = "i2c"
device  = "/dev/i2c-1"
address = 0x68

[sensors.settings]     # optional
interval_ms  = 20      # time between reads; per-driver default
history_size = 600     # TUI sparkline window

[sensors.channels]     # optional: rename or drop canonical channels
lean_angle = "chassisLeanDeg"
peak_g     = ""
```

Default read intervals:

| Driver | Interval |
|---|---|
| IMU, synthetic | 20 ms |
| button | 10 ms |
| BH1750 | 200 ms |
| SDS011 | 1 s |
| others | 100 ms |

### Connection types

**I2C** (Linux only, direct hardware)
```toml
[sensors.connection]
type    = "i2c"
device  = "/dev/i2c-1"   # default
address = 0x68           # 7-bit address
```

**TCP** (all platforms, through an [io-to-net](https://github.com/askrejans/io-to-net) bridge)
```toml
[sensors.connection]
type    = "tcp"
host    = "192.168.88.58"   # the io-to-net bridge
port    = 9002              # port configured on the bridge
address = 0x68              # I2C device address (I2C sensors); omit for serial-over-TCP
framing = true              # bridge sends 2-byte big-endian length-prefixed frames
```

**GPIO** (Linux only)
```toml
[sensors.connection]
type        = "gpio"
pin         = 17          # BCM pin number
active_low  = false       # true = LOW means pressed
debounce_ms = 50
```

**Serial** (USB or UART)
```toml
[sensors.connection]
type      = "serial"
port      = "/dev/ttyUSB0"   # or /dev/ttyAMA0, COM3 …
baud_rate = 9600
```

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

## MQTT topics

The base topic is chosen in this order:

| Configuration | Base topic |
|---|---|
| `mqtt.base_topic` set (file or `SENSORS_TO_MQTT_MQTT_BASE_TOPIC`) | that value, unchanged |
| only `car = "golf86"` (or `SENSORS_TO_MQTT_CAR`) | `/g86/golf86/sensors` (G86 car computer namespace) |
| neither | `/SENSORS` (legacy default; existing installs keep it) |

The startup log names the base and where it came from. g86-logger writes
`car` for you.

With `car = "golf86"` and a sensor named `imu`:

| Topic | Format | Retained | Payload |
|---|---|---|---|
| `/g86/golf86/sensors/imu/telemetry` | `canonical` | no | G86 envelope, every reading |
| `/g86/golf86/sensors/imu/state` | `json` | yes | `{"timestampMs", "sequence", "sensor", "driver", "fields", "channels"}`, ≤ `max_rate_hz` |
| `/g86/golf86/sensors/IMU/imu/INFO`, `FILTERED`, `DERIVED` | `legacy` | no | the topics of 0.5, ≤ `max_rate_hz` |
| `homeassistant/sensor/<node>/<channel>/config` | `homeassistant` | yes | discovery, on connect |
| `/g86/golf86/sensors/status` | always | yes | `online` / `offline` (last will) |
| `/g86/golf86/sensors/health` | always | yes | bridge health (below), every 5 s and on a state change |
| `/g86/golf86/sensors/telemetry/get` | always (in) | — | on demand: the payload is a reply topic |

Bridge health (G86 car computer contract):

```json
{"schema":1,"bridge":"sensors-to-mqtt","version":"0.6.1","bootId":"4f2a-1c9e0b7d3a21",
 "timestampMs":1792621857245,"state":"ok","link":"connected",
 "sources":["sensors/mpu6500/imu"],"rateHz":50.0,"ageMs":12,
 "channels":["lateralG","longitudinalG","verticalG","yawRateDegS"],
 "errors":{"read":0,"init":0,"reconnects":0},"clockSynced":true,"sensors":{"imu":"ok"}}
```

`state` is `down` when no sensor delivers (nothing for 10 s), `degraded` when
a sensor is down, the newest reading is older than 2 s or an error counter
rose in the last 5 s, else `ok`. `link` is `connected` while a sensor reads,
`disconnected` after a failed open/read, `connecting` before. `rateHz` counts
canonical readings over 5 s (all sensors), `ageMs` is the age of the newest
capture time, `channels` the ones updated in the last 2 s. `clockSynced`
reads the kernel clock state on Linux (NTP/chrony synchronised); it is
always `true` elsewhere. A clean shutdown publishes a final `down` document,
then `offline`.

The canonical envelope:

```json
{"schema":1,"source":"sensors/mpu6500/imu","partial":false,"bootId":"4f2a-1c9e0b7d3a21",
 "timestampMs":1792621857245,"sequence":1094,
 "channels":{"lateralG":0.42,"longitudinalG":-0.31,"verticalG":1.01,"yawRateDegS":12.5, "…": 0}}
```

- `timestampMs` is the capture time.
- `sequence` increases with every reading of the process.
- `bootId` changes on every start.
- The G86 app reads the MPU-6500's envelope (`sensors/mpu6500/…`) through
  its Sensors topic setting.

**On demand:**
- Publish a reply topic (1–128 bytes, no wildcards) to `<base>/telemetry/get`.
- The answer on that topic is an envelope with `"source": "sensors"` and
  every channel of every sensor updated within the last 2 s.
- A request is answered at once if there is a reading the consumer hasn't
  had yet; otherwise it is held for the next reading, up to 2 s.
- Per reply topic: bursts of 12, 100 answers a second on average, and at
  most 32 reply topics.

## HTTP API

| Route | Auth | |
|---|---|---|
| `GET /` | open | web dashboard (strict CSP, no remote resources) |
| `GET /health` | open | 200 when every enabled sensor delivers, else 503 (`degraded`, `no_data`) |
| `GET /metrics` | open | Prometheus: per-sensor connected and rate, channels, counters |
| `GET /api/v1/status` | token | wiring, health, sensors, counters |
| `GET /api/v1/live` | token | channels with age and sensor, every sensor's fields |
| `GET /api/v1/channels` | token | channel catalogue (label, unit, group, range) |
| `GET /api/v1/config` | token | effective configuration, secrets masked |
| `GET /api/v1/stream` | token | Server-Sent Events, ≤ 25 per second per sensor |

`sensors-to-mqtt --healthcheck` queries `/health` (for container health checks).

## CLI

```
sensors-to-mqtt [OPTIONS]

  -c, --config <FILE>        Config file
      --check-config         Validate, print the effective settings and exit
      --scan                 Find the chips on a local I2C bus and exit
      --i2c-device <DEVICE>  Bus for --scan [default: /dev/i2c-1]
      --simulate             One synthetic sensor instead of the configured ones
      --no-mqtt              Do not publish to MQTT
      --headless             No terminal UI, even on a terminal
      --log-level <LEVEL>    trace | debug | info | warn | error
      --healthcheck          Query the local /health and exit 0 or 1
  -V, --version
  -h, --help
```

TUI keys: `←` / `→` or `1`–`9` switch tabs, `q` / `Esc` quits.

## Systemd service

The packages install a hardened unit and create the `sensors-to-mqtt` user in
the `i2c`, `dialout` and `gpio` groups (created when missing).

```bash
sudo cp /etc/sensors-to-mqtt/settings.toml.example /etc/sensors-to-mqtt/settings.toml
sudo nano /etc/sensors-to-mqtt/settings.toml
sudo systemctl enable --now sensors-to-mqtt
sudo journalctl -u sensors-to-mqtt -f
```

## TCP bridge setup (io-to-net)

Run [io-to-net](https://github.com/askrejans/io-to-net) on the device with the
sensors. Each sensor gets its own TCP port in the bridge configuration. For the
MPU-6500, set `read_len = 14` and `read_reg = 0x3B` on the bridge: the driver
reads all six axes in one 14-byte burst.

## Building packages

```bash
./scripts/build_packages.sh                                  # everything
./scripts/build_packages.sh --platform linux --arch arm64 --type deb
./scripts/build_packages.sh --platform mac
```

Linux and Windows builds use [`cross`](https://github.com/cross-rs/cross)
(Docker). Packages go to `release/<version>/`.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- --simulate
```

Adding a driver:
1. Implement the `Sensor` trait (`init`, `read`, `field_descriptors`) in
   `src/sensors/<bus>/<driver>.rs`. For I2C, use `open_i2c(cfg, default_address)`:
   local on Linux, TCP everywhere.
2. Register it in `src/sensors/registry.rs` and `config::DRIVERS`.
3. Map its fields to canonical channels in `src/channels.rs`, with catalogue
   entries.
4. Add tests next to the code.

[AGENTS.md](AGENTS.md) has the architecture and the contracts.

## Licence

GPL-3.0-only — see [LICENSE](LICENSE). Versions up to 0.5.0 were MIT.
