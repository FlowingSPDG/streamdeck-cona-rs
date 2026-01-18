//! Cora protocol implementation for Stream Deck Studio
//!
//! The Cora protocol is a newer protocol used by Stream Deck Studio devices.
//! It uses a 16-byte header followed by variable-length payload.

use crate::error::{Error, Result};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Cora protocol magic bytes
pub const CORA_MAGIC: [u8; 4] = [0x43, 0x93, 0x8a, 0x41];

/// Cora message header size
pub const CORA_HEADER_SIZE: usize = 16;

/// Cora message flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoraMessageFlags {
    /// No flags set
    None = 0x0000,
    /// Payload for child HID device (secondary port)
    Verbatim = 0x8000,
    /// Host requests an ACK
    ReqAck = 0x4000,
    /// Unit response to REQ_ACK
    AckNak = 0x0200,
    /// Unit response to GET_REPORT op
    Result = 0x0100,
}

impl From<u16> for CoraMessageFlags {
    fn from(value: u16) -> Self {
        if value & CoraMessageFlags::Verbatim as u16 != 0 {
            CoraMessageFlags::Verbatim
        } else if value & CoraMessageFlags::ReqAck as u16 != 0 {
            CoraMessageFlags::ReqAck
        } else if value & CoraMessageFlags::AckNak as u16 != 0 {
            CoraMessageFlags::AckNak
        } else if value & CoraMessageFlags::Result as u16 != 0 {
            CoraMessageFlags::Result
        } else {
            CoraMessageFlags::None
        }
    }
}

/// HID operation types for Cora protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoraHidOp {
    /// HID write (button image transmission, etc.)
    Write = 0x00,
    /// Feature report send (brightness setting, etc.)
    SendReport = 0x01,
    /// Feature report get (device information retrieval, etc.)
    GetReport = 0x02,
}

impl From<u8> for CoraHidOp {
    fn from(value: u8) -> Self {
        match value {
            0x00 => CoraHidOp::Write,
            0x01 => CoraHidOp::SendReport,
            0x02 => CoraHidOp::GetReport,
            _ => CoraHidOp::Write, // Default fallback
        }
    }
}

/// Cora protocol message structure
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub struct CoraMessage {
    pub flags: CoraMessageFlags,
    pub hid_op: CoraHidOp,
    pub message_id: u32,
    pub payload: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl CoraMessage {
    /// Create a new Cora message
    pub fn new(flags: CoraMessageFlags, hid_op: CoraHidOp, message_id: u32, payload: Vec<u8>) -> Self {
        Self {
            flags,
            hid_op,
            message_id,
            payload,
        }
    }

    /// Encode message to bytes
    pub fn encode(&self) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(CORA_HEADER_SIZE + self.payload.len());
        
        // Magic bytes
        buffer.extend_from_slice(&CORA_MAGIC);
        
        // Flags (Little Endian)
        buffer.extend_from_slice(&(self.flags as u16).to_le_bytes());
        
        // HID operation
        buffer.push(self.hid_op as u8);
        
        // Reserved byte
        buffer.push(0x00);
        
        // Message ID (Little Endian)
        buffer.extend_from_slice(&self.message_id.to_le_bytes());
        
        // Payload length (Little Endian)
        buffer.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        
        // Payload
        buffer.extend_from_slice(&self.payload);
        
        buffer
    }

    /// Decode message from bytes
    pub fn decode(buffer: &[u8]) -> Result<Self> {
        if buffer.len() < CORA_HEADER_SIZE {
            return Err(Error::Protocol("Cora message too short"));
        }

        // Check magic bytes
        if buffer[0..4] != CORA_MAGIC {
            return Err(Error::Protocol("Invalid Cora magic bytes"));
        }

        // Read flags (Little Endian)
        let flags = u16::from_le_bytes([buffer[4], buffer[5]]).into();
        
        // Read HID operation
        let hid_op = buffer[6].into();
        
        // Reserved byte at 7 (ignored)
        
        // Read message ID (Little Endian)
        let message_id = u32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]);
        
        // Read payload length (Little Endian)
        let payload_length = u32::from_le_bytes([buffer[12], buffer[13], buffer[14], buffer[15]]) as usize;
        
        // Check if we have the full payload
        if buffer.len() < CORA_HEADER_SIZE + payload_length {
            return Err(Error::Protocol("Cora message incomplete"));
        }
        
        // Extract payload
        let payload = buffer[CORA_HEADER_SIZE..CORA_HEADER_SIZE + payload_length].to_vec();
        
        Ok(Self {
            flags,
            hid_op,
            message_id,
            payload,
        })
    }

    /// Check if this is a keep-alive message
    pub fn is_keep_alive(&self) -> bool {
        self.payload.len() > 4 && self.payload[0] == 0x01 && self.payload[1] == 0x0a
    }

    /// Get connection number from keep-alive payload
    pub fn keep_alive_connection_no(&self) -> Option<u8> {
        if self.is_keep_alive() && self.payload.len() > 5 {
            Some(self.payload[5])
        } else {
            None
        }
    }
}

/// Check if buffer starts with Cora magic bytes
pub fn is_cora_magic(buffer: &[u8]) -> bool {
    buffer.len() >= 4 && buffer[0..4] == CORA_MAGIC
}

/// Check if buffer starts with Legacy keep-alive packet
pub fn is_legacy_keep_alive(buffer: &[u8]) -> bool {
    buffer.len() >= 2 && buffer[0] == 0x01 && buffer[1] == 0x0a
}
