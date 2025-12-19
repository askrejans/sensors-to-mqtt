//! Custom error types for the sensors-to-mqtt application.
//!
//! This module defines domain-specific error types using thiserror,
//! providing clear error messages and proper error context propagation.

use thiserror::Error;

/// Errors related to sensor operations
#[derive(Debug, Error)]
pub enum SensorError {
    #[error("I2C communication failed: {0}")]
    I2cError(String),

    #[error("Sensor initialization failed: {0}")]
    InitializationError(String),

    #[error("Sensor calibration failed: {0}")]
    CalibrationError(String),

    #[error("Invalid sensor configuration: {0}")]
    ConfigError(String),

    #[error("Sensor read failed: {0}")]
    ReadError(String),

    #[error("Unsupported sensor driver: {0}")]
    UnsupportedDriver(String),
}

/// Errors related to MQTT operations
#[derive(Debug, Error)]
pub enum MqttError {
    #[error("MQTT connection failed: {0}")]
    ConnectionError(String),

    #[error("MQTT publish failed: {0}")]
    PublishError(String),

    #[error("MQTT subscription failed: {0}")]
    SubscriptionError(String),

    #[error("MQTT disconnection failed: {0}")]
    DisconnectionError(String),

    #[error("Invalid MQTT configuration: {0}")]
    ConfigError(String),
}

/// Errors related to application configuration
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),

    #[error("Failed to parse config: {0}")]
    ParseError(String),

    #[error("Invalid configuration: {0}")]
    ValidationError(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid value for {field}: {message}")]
    InvalidValue { field: String, message: String },
}

/// Errors related to the UI
#[derive(Debug, Error)]
pub enum UiError {
    #[error("Terminal initialization failed: {0}")]
    InitializationError(String),

    #[error("Terminal rendering failed: {0}")]
    RenderError(String),

    #[error("Input handling failed: {0}")]
    InputError(String),
}

/// Errors related to service/daemon operations
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("Service initialization failed: {0}")]
    InitializationError(String),

    #[error("Service runtime error: {0}")]
    RuntimeError(String),

    #[error("Signal handling error: {0}")]
    SignalError(String),
}

/// Application-level errors that can wrap other error types
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Sensor error: {0}")]
    Sensor(#[from] SensorError),

    #[error("MQTT error: {0}")]
    Mqtt(#[from] MqttError),

    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("UI error: {0}")]
    Ui(#[from] UiError),

    #[error("Service error: {0}")]
    Service(#[from] ServiceError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Convenience type alias for Results using AppError
pub type Result<T> = std::result::Result<T, AppError>;
