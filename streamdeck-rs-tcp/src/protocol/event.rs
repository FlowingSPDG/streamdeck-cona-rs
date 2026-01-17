//! Event decoding for receiving from Stream Deck Studio

use crate::error::{Error, Result};
use crate::protocol::packet::PacketReader;

/// Event types that can be received from the device
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Button press/release event (0x01 0x00)
    Button {
        /// Button index (0-31)
        index: u8,
        /// True if pressed, false if released
        pressed: bool,
    },
    /// Encoder rotation event (0x01 0x03, subtype 0x01)
    EncoderRotate {
        /// Encoder index (0 or 1)
        encoder_index: u8,
        /// Rotation delta (signed, -128 to 127)
        delta: i8,
    },
    /// Encoder press event (0x01 0x03, subtype 0x00)
    EncoderPress {
        /// Encoder index (0 or 1)
        encoder_index: u8,
        /// True if pressed, false if released
        pressed: bool,
    },
    /// Touch event (0x01 0x02)
    Touch {
        /// Touch type: 0x01 = tap, 0x02 = press/hold, 0x03 = swipe
        touch_type: TouchType,
        /// X coordinate
        x: u16,
        /// Y coordinate
        y: u16,
        /// For swipe: end X coordinate
        end_x: Option<u16>,
        /// For swipe: end Y coordinate
        end_y: Option<u16>,
    },
    /// NFC event (0x01 0x04)
    Nfc {
        /// NFC data
        data: Vec<u8>,
    },
    /// Keep-alive request from device (0x01 0x0a)
    KeepAlive,
}

/// Touch event types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchType {
    /// Single tap
    Tap,
    /// Press and hold
    Press,
    /// Swipe gesture
    Swipe,
}

/// Decoder for converting packets to events
pub struct EventDecoder;

impl EventDecoder {
    /// Decode a 1024-byte packet into an event
    pub fn decode(packet: &[u8]) -> Result<Option<Event>> {
        if packet.len() < 2 {
            return Err(Error::Protocol("Packet too short".to_string()));
        }

        // All events start with 0x01
        if packet[0] != 0x01 {
            return Ok(None);
        }

        match packet[1] {
            0x00 => {
                // Button event
                // Buttons state starts at offset 4, 32 buttons total
                let mut events = Vec::new();
                for i in 0..32 {
                    let offset = 4 + i as usize;
                    if offset < packet.len() {
                        match packet[offset] {
                            0 => continue,
                            1 => events.push(Event::Button {
                                index: i,
                                pressed: true,
                            }),
                            _ => {}
                        }
                    }
                }
                // Return first button event if any (protocol may send multiple button states)
                Ok(events.first().cloned())
            }

            0x02 => {
                // Touch event
                if packet.len() < 10 {
                    return Err(Error::Protocol("Touch event packet too short".to_string()));
                }

                let touch_type = match packet[4] {
                    0x01 => TouchType::Tap,
                    0x02 => TouchType::Press,
                    0x03 => TouchType::Swipe,
                    _ => {
                        return Err(Error::Protocol(format!(
                            "Unknown touch type: 0x{:02x}",
                            packet[4]
                        )));
                    }
                };

                let x = PacketReader::read_u16_le(packet, 6)?;
                let y = PacketReader::read_u16_le(packet, 8)?;

                let (end_x, end_y) = if touch_type == TouchType::Swipe {
                    if packet.len() < 14 {
                        return Err(Error::Protocol("Swipe event packet too short".to_string()));
                    }
                    (
                        Some(PacketReader::read_u16_le(packet, 10)?),
                        Some(PacketReader::read_u16_le(packet, 12)?),
                    )
                } else {
                    (None, None)
                };

                Ok(Some(Event::Touch {
                    touch_type,
                    x,
                    y,
                    end_x,
                    end_y,
                }))
            }

            0x03 => {
                // Encoder event
                if packet.len() < 9 {
                    return Err(Error::Protocol("Encoder event packet too short".to_string()));
                }

                match packet[4] {
                    0x00 => {
                        // Encoder press
                        // Encoder states at offset 5 (2 encoders)
                        if packet.len() < 7 {
                            return Err(Error::Protocol(
                                "Encoder press event packet too short".to_string(),
                            ));
                        }

                        // Process both encoders
                        for i in 0..2u8 {
                            let offset = 5 + i as usize;
                            if offset < packet.len() {
                                match packet[offset] {
                                    0 => continue,
                                    1 => {
                                        return Ok(Some(Event::EncoderPress {
                                            encoder_index: i,
                                            pressed: true,
                                        }));
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(None)
                    }

                    0x01 => {
                        // Encoder rotation
                        // Rotation values at offset 5 (2 encoders, signed 8-bit)
                        if packet.len() < 7 {
                            return Err(Error::Protocol(
                                "Encoder rotate event packet too short".to_string(),
                            ));
                        }

                        // Process both encoders
                        for i in 0..2u8 {
                            let offset = 5 + i as usize;
                            if offset < packet.len() {
                                let value = packet[offset] as i8;
                                if value != 0 {
                                    return Ok(Some(Event::EncoderRotate {
                                        encoder_index: i,
                                        delta: value,
                                    }));
                                }
                            }
                        }
                        Ok(None)
                    }

                    _ => Err(Error::Protocol(format!(
                        "Unknown encoder event subtype: 0x{:02x}",
                        packet[4]
                    ))),
                }
            }

            0x04 => {
                // NFC event
                let length = PacketReader::read_u16_le(packet, 2)? as usize;
                if packet.len() < 4 + length {
                    return Err(Error::Protocol(format!(
                        "NFC event packet too short: need {} bytes, got {}",
                        4 + length,
                        packet.len()
                    )));
                }

                let data = packet[4..4 + length].to_vec();
                Ok(Some(Event::Nfc { data }))
            }

            0x0a => {
                // Keep-alive request
                Ok(Some(Event::KeepAlive))
            }

            _ => Ok(None),
        }
    }
}
