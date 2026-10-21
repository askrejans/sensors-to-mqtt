# sensors-to-mqtt — agent guide

Sensor MQTT producer (Rust, tokio): I2C, GPIO and serial sensors, local or
through an io-to-net TCP bridge, to MQTT, HTTP and a web dashboard. This
component belongs to `g86racing`. Read [project guidance](../../AGENTS.md) and
the relevant skill before changing a cross-component contract. Read this
checkout's `README.md`, any `CLAUDE.md`, and actual manifests for current
commands and architecture. Preserve its existing branch and local edits.

## Skills

| Task | Skill |
|------|-------|
| New or changed output: MQTT format/topic, channel name, HTTP route, metric | [skills/output-contracts](skills/output-contracts/SKILL.md) |
| Version bump, packages, Docker, dependencies, Homebrew | [skills/release](skills/release/SKILL.md) |
| Cross-component telemetry contracts (G86 app, dashboard, API) | [../../skills/automotive-telemetry](../../skills/automotive-telemetry/SKILL.md) |
| Lap/sector timing consumers of this data | [../../skills/race-timing-analysis](../../skills/race-timing-analysis/SKILL.md) |

## Map

```
src/sensors/       Sensor trait, registry, drivers (i2c/, gpio/, serial/, synthetic)
src/transport/     I2cBus: local Linux I2C or the io-to-net TCP bridge
src/channels.rs    driver fields → canonical channels, catalogue, per-sensor overrides
src/service.rs     one thread per sensor (schedule, reopen with backoff) and the hub
src/models.rs      Shared: snapshot (std RwLock), counters, bus of Stamped readings
src/output/        mqtt (formats, on-demand long-poll, discovery), http, web.rs
src/scan.rs        --scan: I2C probe and chip identification (reads only)
src/tui/           ratatui terminal UI
web/               browser dashboard, embedded with include_str!
scripts/           build_packages.sh
```

## Commands

```bash
cargo test                                    # must pass
cargo clippy --all-targets -- -D warnings     # must be clean
cargo fmt
cargo run -- --simulate                       # no hardware needed
cargo run -- --check-config -c example.settings.toml
sensors-to-mqtt --scan                        # on the sensor host (Linux)
```

## Invariants

1. **Sensors never wait on an output.** Each sensor reads on its own thread;
   the hub never awaits; outputs subscribe to the lossy bus and count drops.
2. **One mapping path.** Driver fields become channels only through
   `channels.rs`; the hub alone sequences readings and builds the envelope.
3. **Contracts are frozen unless versioned.** The envelope (schema 1, seven
   keys) on `<base>/<sensor>/telemetry`, source `sensors/<driver>/<name>`,
   channel names and units. The G86 app accepts `sensors/mpu6500/*` sources
   and knows `lateralG`, `longitudinalG`, `verticalG` and `yawRateDegS`.
   The on-demand contract (`<base>/telemetry/get`) matches ecu-to-mqtt and
   gps-to-mqtt.
4. **Untrusted input is bounded and never panics**: bridge peers, serial
   frames, MQTT requests, HTTP clients.
5. **The web dashboard has no remote resources and no `innerHTML`.** A test
   enforces this. Configuration is read-only.
6. **`--scan` only reads.** It must never write to a chip.
7. New config keys need a serde default, validation, `example.settings.toml`
   and `config.toml`, the README table, and a test.

## Conventions

- Edition 2024, `anyhow` for application errors, `tracing` for logs.
- std `RwLock` for the snapshot, never held across `.await`.
- Tests sit next to the code (`#[cfg(test)] mod tests`); integration tests in
  `tests/` use the synthetic driver.
- Hardware tests (real chips, mounting, calibration) need the device; say when
  they were not run.
- Licence: GPL-3.0-only.

Choose validation for the component changed; downloaded dependencies and
compiled artifacts may need to be regenerated after cleanup. Runtime data,
source recordings, signing files and release receipts remain separate from
disposable build output.
