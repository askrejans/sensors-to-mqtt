//! GPIO sensor drivers (Linux only — uses Linux character device / sysfs GPIO).

#[cfg(target_os = "linux")]
pub mod button;
