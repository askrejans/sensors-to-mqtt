# Changelog

## 0.7.0

- **Migration:** an unconfigured installation now publishes below `/telemetry/sensors` instead of `/SENSORS`. Set `mqtt.base_topic` explicitly to `/SENSORS` before upgrading if consumers need the former default. Explicit settings and optional `car` namespaces keep their topics.

- Declare the Linux runtime libraries required by the shipped Debian binary.
- Document standalone use with arbitrary MQTT brokers and explicit topic prefixes; preserve explicit topic prefixes and telemetry packet contracts.

## 0.6.1

### Added
- `car` setting (`SENSORS_TO_MQTT_CAR`): with a G86 car slug and no explicit
  `mqtt.base_topic`, the base becomes `/g86/<car>/sensors`, the car computer
  namespace. An explicit `mqtt.base_topic` still wins, and without `car` the
  default stays `/SENSORS`, so existing installs are unchanged. The startup
  log names the base topic and where it came from.
- `<base>/health`: a retained bridge health document (schema 1) for the car
  computer and the G86 app — `state` (`ok`/`degraded`/`down`), `link`,
  `sources`, `rateHz`, `ageMs`, fresh `channels`, `errors` (`read`, `init`,
  `reconnects`), `clockSynced` and per-sensor `sensors`. Published every 5 s,
  at once when the state or link changes, and as a final `down` on a clean
  shutdown before `offline`.
- Prometheus counter of failed driver opens (`init` in the health errors).

## 0.6.0

A rework in line with ecu-to-mqtt and gps-to-mqtt.

### Contract changes
- Licence: MIT → GPL-3.0-only.
- Canonical envelopes (`<base>/<sensor>/telemetry`, schema 1) now come from
  every sensor with channels, not only the MPU-6500. The source is
  `sensors/<driver>/<name>`, `partial` is `false`, and `sequence` counts
  every reading of the process. The MPU-6500 keeps `lateralG`,
  `longitudinalG`, `verticalG` and `yawRateDegS`, and adds `combinedG`,
  `peakG`, `rollRateDegS`, `pitchRateDegS`, `leanDeg`, `bankDeg` and
  `tiltDeg`. The environment, power, light and particulate sensors publish
  `ambientC`, `baroKpa` (hPa ÷ 10), `baroAltitudeM`, `humidityPct`,
  `lightLux`, `busV`, `currentA`, `powerW`, `batterySocPct`, `pm25`, `pm10`,
  `aqiPm25` and `aqiPm10`.
- The `IMU/<sensor>/{INFO,FILTERED,DERIVED}` topics are now the opt-in
  `legacy` format, at most `mqtt.max_rate_hz` (10) per sensor. 0.5 published
  them on every reading. Default formats: `canonical` and `json`.
- MQTT QoS of live data defaults to 0 (0.5 used 1). The client ID is unique
  per process unless `mqtt.client_id` is set.
- The service and its package user are `sensors-to-mqtt` (was `sensors`), and
  the unit starts with `--headless`.
- CLI: clap replaces gumdrop. `-c`, `--no-mqtt` and `--log-level` are
  unchanged.

### Added
- On-demand telemetry for the G86 displays. Publish a reply topic to
  `<base>/telemetry/get` and receive every fresh channel of every sensor,
  each answer carrying a reading the display hasn't had yet. Same long-poll
  contract as ecu-to-mqtt 0.7.3 and gps-to-mqtt 0.7.2.
- `json` format: a retained `<base>/<sensor>/state` document with every field
  and channel. `homeassistant` format: discovery for every channel.
  `<base>/status` online/offline with a last will.
- HTTP API and web dashboard (port 9092, loopback by default, optional bearer
  token):
  - Routes: `/health`, Prometheus `/metrics`, `/api/v1/status`, `live`,
    `channels`, `config`, and the `stream` (SSE).
  - Dashboard: G-ball, gauges, per-sensor fields, charts, diagnostics,
    configuration.
- `--scan`: finds the chips on a local I2C bus by their ID registers and
  prints a configuration block for each (reads only).
- `--check-config`, `--simulate`, `--headless`, `--healthcheck`, `--version`.
- `[sensors.channels]` renames or drops canonical channels per sensor.
  `settings.interval_ms` sets the read interval (per-driver defaults).
- MQTT TLS (`use_tls`, CA and client certificates) and `password_file`.
- Configuration validation at startup:
  - log level, MQTT and HTTP settings,
  - sensor names (unique, no topic wildcards) and drivers,
  - two enabled sensors publishing the same channel.
- Environment overrides in the siblings' form, `SENSORS_TO_MQTT_MQTT_HOST`.
  The nested `SENSORS_TO_MQTT__MQTT__HOST` still works.
- Windows build (TCP bridge and serial sensors).

### Fixed
- Drivers other than the MPU-6500 were never initialised, so the BMP280,
  BME280, SHT31, BH1750, INA219 and ADS1115 failed every read ("not
  initialised") or read unconfigured chips.
- A sensor that failed to open stayed down until restart; it is now reopened
  with backoff (1 s to 30 s), also after 10 read errors in a row.
- Blocking driver reads ran on the async runtime: a 2 s SDS011 timeout or the
  SHT31's conversion wait stalled MQTT and the other sensors. Every sensor
  now reads on its own thread, on a fixed schedule.
- The example configurations gave the ADS1115 gain and rate as strings.

### Changed
- Every dependency at its latest release (tokio 1.54, rumqttc 0.25, ratatui
  0.30, crossterm 0.29, axum 0.8, clap 4.6). Edition 2024, Rust 1.89.
- Release builds use thin LTO and strip symbols.
- Packages follow the sibling gateways: deb, rpm, Windows zip, macOS tar.gz
  (`sensors-to-mqtt_<v>_<platform>.<ext>`), with the licence included.

## 0.5.0

See [RELEASE-0.5.0.md](RELEASE-0.5.0.md); earlier versions in the Git history.
