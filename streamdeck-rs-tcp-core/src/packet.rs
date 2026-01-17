//! Packet reading and writing utilities

use crate::error::{Error, Result};

/// Fixed packet size for TCP protocol
pub const PACKET_SIZE: usize = 1024;

/// Helper for writing data to packets with padding
pub struct PacketWriter;

impl PacketWriter {
    /// Create a packet with data and zero-padding to PACKET_SIZE
    pub fn create_packet(data: &[u8]) -> [u8; PACKET_SIZE] {
        let mut packet = [0u8; PACKET_SIZE];
        let len = data.len().min(PACKET_SIZE);
        packet[..len].copy_from_slice(&data[..len]);
        packet
    }

    #[cfg(feature = "alloc")]
    /// Pad a vector to PACKET_SIZE
    pub fn pad_to_packet_size(mut data: alloc::vec::Vec<u8>) -> alloc::vec::Vec<u8> {
        if data.len() < PACKET_SIZE {
            data.resize(PACKET_SIZE, 0);
        }
        data.truncate(PACKET_SIZE);
        data
    }
}

/// Helper for reading data from packets
pub struct PacketReader;

impl PacketReader {
    /// Read a u16 from packet at offset (Little Endian)
    pub fn read_u16_le(packet: &[u8], offset: usize) -> Result<u16> {
        if offset + 2 > packet.len() {
            return Err(Error::Protocol("Cannot read u16: packet too short"));
        }
        Ok(u16::from_le_bytes([packet[offset], packet[offset + 1]]))
    }

    /// Read a u16 from packet at offset (Big Endian)
    pub fn read_u16_be(packet: &[u8], offset: usize) -> Result<u16> {
        if offset + 2 > packet.len() {
            return Err(Error::Protocol("Cannot read u16: packet too short"));
        }
        Ok(u16::from_be_bytes([packet[offset], packet[offset + 1]]))
    }

    /// Read a slice from packet
    pub fn read_slice<'a>(packet: &'a [u8], offset: usize, length: usize) -> Result<&'a [u8]> {
        if offset + length > packet.len() {
            return Err(Error::Protocol("Cannot read slice: packet too short"));
        }
        Ok(&packet[offset..offset + length])
    }
}
