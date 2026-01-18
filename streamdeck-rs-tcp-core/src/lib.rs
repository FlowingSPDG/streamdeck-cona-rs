//! Stream Deck Studio TCP Protocol Core Library (no-std compatible)
//!
//! This is a no-std compatible core library for the Stream Deck Studio TCP protocol.
//! It provides protocol parsing and serialization without requiring std or async runtime.
//!
//! ## Features
//!
//! - `alloc`: Enable allocation support (Vec, String, etc.) - enabled by default
//! - `std`: Enable std support (for testing)
//!
//! ## Example
//!
//! ```no_run
//! use streamdeck_cona_rs_core::{Command, CommandEncoder, Event, EventDecoder};
//!
//! // Encode a command
//! let command = Command::SetBrightness(80);
//! let packet = CommandEncoder::encode(&command).unwrap();
//!
//! // Decode an event
//! let event = EventDecoder::decode(&packet).unwrap();
//! ```

#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

// String and Vec are used through command and event modules when alloc is enabled

pub mod command;
pub mod cora;
pub mod error;
pub mod event;
pub mod packet;

pub use command::{Command, CommandEncoder};
pub use cora::{CoraMessage, CoraMessageFlags, CoraHidOp, CORA_MAGIC, CORA_HEADER_SIZE, is_cora_magic, is_legacy_keep_alive};
pub use error::{Error, Result};
pub use event::{Event, EventDecoder, TouchType};
pub use packet::{PACKET_SIZE, PacketReader, PacketWriter};
