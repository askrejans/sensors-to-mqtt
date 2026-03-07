//! Transport abstractions for cross-platform sensor connectivity.
//!
//! Provides an `I2cBus` trait that works over both local hardware (Linux only)
//! and a TCP bridge (all platforms, compatible with io-to-net).

pub mod framing;
pub mod i2c_bus;
pub use framing::{tcp_read_framed, FramedTcpReader};
pub use i2c_bus::{open_i2c, I2cBus};
