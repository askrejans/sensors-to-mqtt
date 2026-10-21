//! Transport abstractions for cross-platform sensor connectivity.
//!
//! Provides an `I2cBus` trait that works over both local hardware (Linux only)
//! and a TCP bridge (all platforms, compatible with io-to-net).

pub mod framing;
pub mod i2c_bus;
pub use framing::{FramedTcpReader, tcp_read_framed};
pub use i2c_bus::{I2cBus, open_i2c};
