//! Error types for the core library

use core::fmt;

/// Result type alias for the core library
pub type Result<T> = core::result::Result<T, Error>;

/// Error types that can occur in the core library
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Protocol errors (invalid packet format, unexpected response, etc.)
    Protocol(&'static str),
    /// Invalid parameter errors
    InvalidParameter(&'static str),
    /// Buffer too small
    BufferTooSmall,
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Protocol(msg) => write!(f, "Protocol error: {}", msg),
            Error::InvalidParameter(msg) => write!(f, "Invalid parameter: {}", msg),
            Error::BufferTooSmall => write!(f, "Buffer too small"),
        }
    }
}
