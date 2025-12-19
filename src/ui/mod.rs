//! Terminal UI module using ratatui.
//!
//! This module provides an interactive terminal interface for monitoring
//! sensor data with real-time visualization and keyboard controls.

pub mod app;
pub mod input;
pub mod widgets;

pub use app::App;
pub use input::{handle_input, InputAction};
