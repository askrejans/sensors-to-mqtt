# sensors-to-mqtt 0.5.0

Adds non-retained, timestamped G86 motion packets on
`<base_topic>/<sensor_name>/telemetry` for the MPU-6500. Existing topics remain.
Packets include schema 1, source, capture timestamp, boot ID and sequence.
Channels are lateralG, longitudinalG, verticalG and yawRateDegS.

Mount X right, Y forward and Z up. Keep the sensor still during startup calibration.
Vertical load includes gravity (1 G stationary); lateral/longitudinal acceleration
preserves sustained cornering and braking. This is a fixed mounting reference,
not attitude fusion for arbitrary mounting or banked circuits.

No motion is inferred from unrelated I2C sensors. The G86 Linux logger wizard
configures hardware, MQTT credentials and services independently.

Validation: 46 library tests pass, including measured units, capture timestamps,
missing channels and sustained acceleration. Linux ARM64 and AMD64 packages are
built separately. Physical sensor/vehicle validation remains required.
