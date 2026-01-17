//! Command encoding for sending to Stream Deck Studio

use crate::error::Result;
use crate::packet::PACKET_SIZE;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Command types that can be sent to the device
#[derive(Debug, Clone)]
pub enum Command {
    /// Request serial number (0x03 0x84)
    GetSerial,
    /// Set brightness (0x03 0x08 <value>)
    SetBrightness(u8),
    /// Set button image (0x02 0x07 <button_index> <last_page_flag> <length> <page_number> <image_data>)
    #[cfg(feature = "alloc")]
    SetButtonImage {
        button_index: u8,
        is_last_page: bool,
        page_number: u16,
        data: Vec<u8>,
    },
    /// Set encoder LED color (0x02 0x10 <encoder_index> <R> <G> <B>)
    SetEncoderColor {
        encoder_index: u8,
        r: u8,
        g: u8,
        b: u8,
    },
    /// Set encoder ring LED colors (0x02 0x0f <encoder_index> <RGB_data...>)
    #[cfg(feature = "alloc")]
    SetEncoderRing {
        encoder_index: u8,
        colors: Vec<(u8, u8, u8)>, // Up to 24 RGB tuples
    },
    /// Set display area image (0x02 0x0c <x> <y> <width> <height> <last_page> <page> <length> <image_data>)
    #[cfg(feature = "alloc")]
    SetDisplayAreaImage {
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        is_last_page: bool,
        page_number: u16,
        data: Vec<u8>,
    },
    /// Keep-alive response (0x03 0x1a)
    KeepAlive,
    /// Reset device (0x03 0x02)
    Reset,
}

/// Encoder for converting commands to packet format
pub struct CommandEncoder;

impl CommandEncoder {
    /// Encode a command into a 1024-byte packet
    pub fn encode(command: &Command) -> Result<[u8; PACKET_SIZE]> {
        match command {
            Command::GetSerial => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x03;
                packet[1] = 0x84;
                Ok(packet)
            }

            Command::SetBrightness(brightness) => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x03;
                packet[1] = 0x08;
                packet[2] = *brightness;
                Ok(packet)
            }

            #[cfg(feature = "alloc")]
            Command::SetButtonImage {
                button_index,
                is_last_page,
                page_number,
                data,
            } => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x02;
                packet[1] = 0x07;
                packet[2] = *button_index;
                packet[3] = if *is_last_page { 0x01 } else { 0x00 };

                let data_length = data.len().min(PACKET_SIZE - 8) as u16;
                packet[4] = (data_length & 0xff) as u8;
                packet[5] = ((data_length >> 8) & 0xff) as u8;

                packet[6] = (*page_number & 0xff) as u8;
                packet[7] = ((*page_number >> 8) & 0xff) as u8;

                let data_end = 8 + data_length as usize;
                if data_end <= PACKET_SIZE {
                    packet[8..data_end].copy_from_slice(&data[..data_length as usize]);
                }

                Ok(packet)
            }

            Command::SetEncoderColor {
                encoder_index,
                r,
                g,
                b,
            } => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x02;
                packet[1] = 0x10;
                packet[2] = *encoder_index;
                packet[3] = *r;
                packet[4] = *g;
                packet[5] = *b;
                Ok(packet)
            }

            #[cfg(feature = "alloc")]
            Command::SetEncoderRing {
                encoder_index,
                colors,
            } => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x02;
                packet[1] = 0x0f;
                packet[2] = *encoder_index;

                // Each color is 3 bytes (RGB), up to 24 colors (72 bytes total)
                let max_colors = 24.min(colors.len());
                for (i, (r, g, b)) in colors.iter().take(max_colors).enumerate() {
                    let offset = 3 + (i * 3);
                    if offset + 2 < PACKET_SIZE {
                        packet[offset] = *r;
                        packet[offset + 1] = *g;
                        packet[offset + 2] = *b;
                    }
                }
                Ok(packet)
            }

            #[cfg(feature = "alloc")]
            Command::SetDisplayAreaImage {
                x,
                y,
                width,
                height,
                is_last_page,
                page_number,
                data,
            } => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x02;
                packet[1] = 0x0c;

                // X coordinate (Little Endian)
                packet[2] = (*x & 0xff) as u8;
                packet[3] = ((*x >> 8) & 0xff) as u8;

                // Y coordinate (Little Endian) - usually 0
                packet[4] = (*y & 0xff) as u8;
                packet[5] = ((*y >> 8) & 0xff) as u8;

                // Width (Little Endian)
                packet[6] = (*width & 0xff) as u8;
                packet[7] = ((*width >> 8) & 0xff) as u8;

                // Height (Little Endian)
                packet[8] = (*height & 0xff) as u8;
                packet[9] = ((*height >> 8) & 0xff) as u8;

                // Last page flag
                packet[10] = if *is_last_page { 0x01 } else { 0x00 };

                // Page number (Little Endian)
                packet[11] = (*page_number & 0xff) as u8;
                packet[12] = ((*page_number >> 8) & 0xff) as u8;

                // Length (Little Endian)
                let data_length = data.len().min(PACKET_SIZE - 16) as u16;
                packet[13] = (data_length & 0xff) as u8;
                packet[14] = ((data_length >> 8) & 0xff) as u8;

                // Padding byte at offset 15
                packet[15] = 0x00;

                // Image data starting at offset 16
                let data_end = 16 + data_length as usize;
                if data_end <= PACKET_SIZE {
                    packet[16..data_end].copy_from_slice(&data[..data_length as usize]);
                }

                Ok(packet)
            }

            Command::KeepAlive => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x03;
                packet[1] = 0x1a;
                Ok(packet)
            }

            Command::Reset => {
                let mut packet = [0u8; PACKET_SIZE];
                packet[0] = 0x03;
                packet[1] = 0x02;
                Ok(packet)
            }
        }
    }

    /// Encode command with a buffer (no-std friendly, no allocation)
    /// 
    /// This method allows encoding commands without requiring Vec allocation.
    /// For commands that require data (SetButtonImage, SetDisplayAreaImage, SetEncoderRing),
    /// use the alloc version or provide a pre-allocated buffer.
    pub fn encode_to_buffer(command: &Command, buffer: &mut [u8; PACKET_SIZE]) -> Result<()> {
        buffer.fill(0);
        
        match command {
            Command::GetSerial => {
                buffer[0] = 0x03;
                buffer[1] = 0x84;
            }
            Command::SetBrightness(brightness) => {
                buffer[0] = 0x03;
                buffer[1] = 0x08;
                buffer[2] = *brightness;
            }
            Command::SetEncoderColor {
                encoder_index,
                r,
                g,
                b,
            } => {
                buffer[0] = 0x02;
                buffer[1] = 0x10;
                buffer[2] = *encoder_index;
                buffer[3] = *r;
                buffer[4] = *g;
                buffer[5] = *b;
            }
            Command::KeepAlive => {
                buffer[0] = 0x03;
                buffer[1] = 0x1a;
            }
            Command::Reset => {
                buffer[0] = 0x03;
                buffer[1] = 0x02;
            }
            #[cfg(feature = "alloc")]
            _ => {
                // For commands requiring Vec, use encode() instead
                return Err(crate::error::Error::InvalidParameter(
                    "This command requires allocation, use encode() instead"
                ));
            }
        }
        Ok(())
    }
}
