//! Timestamped, non-retained motion packets for the G86 recorder.
use crate::sensors::SensorData;
use serde_json::{json, Map, Value};

pub fn motion_packet(data: &SensorData, name: &str, boot: &str, sequence: u64) -> Option<Value> {
    let mut channels = Map::new();
    // MPU-6500 is calibrated stationary with Z up, X right and Y forward.
    // Do not infer vehicle motion for other sensor drivers.
    for (field, channel, scale) in [
        ("accel_raw_x", "lateralG", 1.0),
        ("accel_raw_y", "longitudinalG", 1.0),
        ("accel_raw_z", "verticalG", 1.0),
        ("gyro_z", "yawRateDegS", 1.0),
    ] {
        if let Some(value) = data.fields.get(field).filter(|v| v.is_finite()) {
            channels.insert(channel.into(), json!(value * scale));
        }
    }
    if channels.is_empty() { return None; }
    Some(json!({"schema": 1, "source": format!("sensors/mpu6500/{name}"),
        "timestampMs": data.timestamp.timestamp_millis(), "sequence": sequence,
        "bootId": boot, "channels": channels}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_measured_acceleration_and_capture_time() {
        let data = SensorData { timestamp: chrono::Utc::now(), fields: [
            ("accel_raw_x".into(), 0.6), ("accel_raw_y".into(), -0.4),
            ("accel_raw_z".into(), 1.0), ("gyro_z".into(), 15.0)].into() };
        let packet = motion_packet(&data, "imu", "boot", 2).unwrap();
        assert_eq!(packet["channels"]["lateralG"], 0.6);
        assert_eq!(packet["channels"]["longitudinalG"], -0.4);
        assert_eq!(packet["channels"]["verticalG"], 1.0);
        assert_eq!(packet["timestampMs"], data.timestamp.timestamp_millis());
        assert!(motion_packet(&SensorData { timestamp: data.timestamp, fields: Default::default() }, "imu", "boot", 3).is_none());
    }
}
