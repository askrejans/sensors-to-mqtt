# Sensors-to-MQTT

This Rust-based application reads sensor data from an I2C bus and publishes the filtered results to an MQTT broker.

## Features
- I2C sensor data collection (MPU6500 IMU for now only).
- Real-time publishing to an MQTT broker.
- Terminal-based data display.

## Quick Start
1. Install [Rust and Cargo](https://www.rust-lang.org).
2. Clone this repository and navigate to its folder.
3. Adjust `config.yaml` for your device and broker.
4. Compile and run:
    ```sh
    cargo build --release
    cargo run
    ```

## Configuration
- **config.yaml** includes sensor bus paths and MQTT settings (`host`, `port`, `base_topic`).

## Testing
Unit tests are located alongside the source files. Run:
```sh
cargo test
```

## License
Distributed under the MIT License. See `LICENSE` for more details.