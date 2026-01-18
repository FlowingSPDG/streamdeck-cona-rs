//! Error types for the library

use thiserror::Error;

/// Result type alias for the library
pub type Result<T> = std::result::Result<T, Error>;

/// Error types that can occur in the library
#[derive(Error, Debug)]
pub enum Error {
    /// IO errors (network, file system, etc.)
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Network connection errors
    #[error("Connection error: {0}")]
    Connection(String),

    /// Protocol errors (invalid packet format, unexpected response, etc.)
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Timeout errors
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Invalid parameter errors
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Image processing errors
    #[error("Image processing error: {0}")]
    Image(String),

    /// Device not found or unavailable
    #[error("Device error: {0}")]
    Device(String),
}

impl From<streamdeck_cona_rs_core::Error> for Error {
    fn from(err: streamdeck_cona_rs_core::Error) -> Self {
        match err {
            streamdeck_cona_rs_core::Error::Protocol(msg) => Error::Protocol(msg.to_string()),
            streamdeck_cona_rs_core::Error::InvalidParameter(msg) => Error::InvalidParameter(msg.to_string()),
            streamdeck_cona_rs_core::Error::BufferTooSmall => Error::Protocol("Buffer too small".to_string()),
        }
    }
}
