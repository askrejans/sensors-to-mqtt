---
name: output-contracts
description: Add or change any sensors-to-mqtt output — canonical channels, MQTT topics/formats (canonical, json, legacy, homeassistant, on demand), HTTP/REST/SSE routes, Prometheus metrics — without breaking G86 consumers.
---

# Output contracts

## Consumers that break silently

| Contract | Consumer | Rule |
|----------|----------|------|
| `<base>/<sensor>/telemetry` envelope: exactly `schema, source, partial, bootId, timestampMs, sequence, channels`; never retained | G86 Flutter app `lib/services/ecu/ecu_telemetry.dart`: subscribes `<sensorsTopic>/+/telemetry`, accepts sources starting with `sensors/mpu6500/`, 2 s freshness, sequence must rise per `bootId`, ignores unknown channels | Frozen. Adding a channel is fine; renaming, rescaling or changing a unit needs a new `schema` and coordinated app, API and dashboard work (`../../skills/automotive-telemetry`). `service.rs` tests pin the key set. |
| Channel names `lateralG`, `longitudinalG`, `verticalG`, `yawRateDegS` | the app's range table, `g86-api/internal/services/ecu_telemetry.go`, `g86-web-dashboard/src/console/channels.mjs` | Keep names and units (g, °/s). |
| `<base>/telemetry/get` on demand: reply topic in, merged envelope out, a reading the consumer hasn't had, held ≤ 2 s, burst 12 / 100 s⁻¹ | G86-info, G86-ESP-DSP | Same contract as ecu-to-mqtt and gps-to-mqtt; change all three together. |
| `/metrics` names | Grafana | Add, never rename. Counters end in `_total`. |
| `/health` 200/503 semantics | Docker `HEALTHCHECK`, `--healthcheck` | `ok` only when every enabled sensor delivers. |

Check consumers before editing:
`grep -rn "<topic or channel>" ../../g86-web-dashboard/src ../../g86-racing-app*/lib ../../g86-api ../../displays`.

## Adding a channel

1. Map the field in `DEFAULTS` in `src/channels.rs` (driver, field, channel,
   scale). Units in the name (`C`, `Kpa`, `DegS`, `Pct`); reuse an existing
   ecosystem name when the quantity is the same.
2. Add a `CATALOG` row (label, unit, group, range); a test fails without it.
3. The channel then appears in the envelope, the on-demand answer,
   `/api/v1/live`, SSE, the `json` state, Home Assistant discovery and
   `sensor_channel{name}`.
4. Document it in the README sensor table and the CHANGELOG.

## Adding an MQTT format

Add a `MqttFormat` variant (list parsing is automatic), publish from
`Publisher::on_sample`/`publish_state`, re-send retained state in
`on_connect`, and use `try_publish` (drops counted). Never block the
publisher. `max_rate_hz` applies to the per-sensor state formats, never to
`canonical`.

## HTTP

Routes under `/api/v1/*` sit behind the token middleware automatically;
`/health` and `/metrics` stay open. Test with `tower::ServiceExt::oneshot`
(see `output/http.rs` tests). Config goes out only via
`AppConfig::redacted()`.
