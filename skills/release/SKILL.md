---
name: release
description: Prepare a sensors-to-mqtt release — version, changelog, dependency updates, smoke tests, packages (deb/rpm/zip/tar.gz), Docker image, Homebrew, licence.
---

# Release

1. `cargo update`, then check direct dependencies for new majors
   (`cargo search <crate> --limit 1`). Raise the minimums in `Cargo.toml`.
   Keep `rust-version` honest.
2. `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
3. Smoke test against a local broker (`mosquitto -p 18830`):
   `SENSORS_TO_MQTT_MQTT_HOST=127.0.0.1 SENSORS_TO_MQTT_MQTT_PORT=18830 cargo run --release -- --simulate --headless`,
   then check `<base>/sim/telemetry`, `<base>/sim/state`, `<base>/status`, an
   on-demand request on `<base>/telemetry/get`, `/health` and the dashboard.
4. Linux build: `cross build --release --target aarch64-unknown-linux-gnu`
   (the local I2C code compiles only for Linux).
5. Hardware check on a sensor host: `--scan`, then a run with the real
   chips, or say explicitly that it was skipped.
6. Bump `version` in `Cargo.toml` and add a `CHANGELOG.md` entry (contract
   changes first, then added, fixed, changed).
7. `docker build -t sensors-to-mqtt:<v> .` and
   `docker run --rm sensors-to-mqtt:<v> --check-config`.
8. `./scripts/build_packages.sh [--platform …]` writes to `release/<version>/`
   (gitignored). Inspect the packages; they ship `LICENSE`.
9. GitHub release with the packages and `SHA256SUMS`; the package repository
   is refreshed with
   `crowfoundry/codebase/infrastructure/infra/deploy/scripts/publish-packages.sh
   --repo askrejans/sensors-to-mqtt --tag <v>` (run by the maintainer;
   credentials stay in their private locations).
10. Homebrew formula in `../../homebrew/homebrew-g86racing`: URLs, sha256 from
    `release/<v>/mac/sha256sums.txt`, licence (separate repository).

Do not set `CARGO_CACHE_AUTO_CLEAN_FREQUENCY`. Licence: GPL-3.0-only
(`LICENSE`, `Cargo.toml`, `PKG_LICENSE` in the build script).
