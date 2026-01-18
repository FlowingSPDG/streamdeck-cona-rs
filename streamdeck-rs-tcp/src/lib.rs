//! Stream Deck Studio TCP Protocol Library
//!
//! This library provides a Rust implementation for controlling Elgato Stream Deck Studio
//! devices via TCP/IP. The Stream Deck Studio uses a fixed 1024-byte binary protocol
//! over TCP port 5343, which is essentially the USB HID protocol implemented over TCP/IP.
//!
//! ## Example
//!
//! ```no_run
//! use streamdeck_cona_rs::Device;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let device = Device::connect_tcp("192.168.1.100:5343").await?;
//!     device.set_brightness(80).await?;
//!     Ok(())
//! }
//! ```

#![allow(dead_code)]

pub mod connection;
pub mod device;
pub mod error;
pub mod image;
pub mod protocol;

pub use device::Device;
pub use error::{Error, Result};
