//! Protocol parsing and serialization tests

use streamdeck_rs_tcp::protocol::command::{Command, CommandEncoder};
use streamdeck_rs_tcp::protocol::event::{Event, EventDecoder};
use streamdeck_rs_tcp::protocol::packet::PACKET_SIZE;

#[test]
fn test_brightness_command() {
    let command = Command::SetBrightness(80);
    let packet = CommandEncoder::encode(&command).unwrap();

    assert_eq!(packet[0], 0x03);
    assert_eq!(packet[1], 0x08);
    assert_eq!(packet[2], 80);
    assert_eq!(packet.len(), PACKET_SIZE);
}

#[test]
fn test_serial_command() {
    let command = Command::GetSerial;
    let packet = CommandEncoder::encode(&command).unwrap();

    assert_eq!(packet[0], 0x03);
    assert_eq!(packet[1], 0x84);
    assert_eq!(packet.len(), PACKET_SIZE);
}

#[test]
fn test_keep_alive_command() {
    let command = Command::KeepAlive;
    let packet = CommandEncoder::encode(&command).unwrap();

    assert_eq!(packet[0], 0x03);
    assert_eq!(packet[1], 0x1a);
    assert_eq!(packet.len(), PACKET_SIZE);
}

#[test]
fn test_encoder_color_command() {
    let command = Command::SetEncoderColor {
        encoder_index: 0,
        r: 255,
        g: 0,
        b: 0,
    };
    let packet = CommandEncoder::encode(&command).unwrap();

    assert_eq!(packet[0], 0x02);
    assert_eq!(packet[1], 0x10);
    assert_eq!(packet[2], 0);
    assert_eq!(packet[3], 255);
    assert_eq!(packet[4], 0);
    assert_eq!(packet[5], 0);
    assert_eq!(packet.len(), PACKET_SIZE);
}

#[test]
fn test_button_event_decode() {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = 0x01;
    packet[1] = 0x00;
    packet[4] = 0; // Button 0 not pressed
    packet[5] = 1; // Button 1 pressed
    packet[6] = 0; // Button 2 not pressed

    let event = EventDecoder::decode(&packet).unwrap();
    assert!(event.is_some());
    match event.unwrap() {
        Event::Button { index, pressed } => {
            assert_eq!(index, 1);
            assert!(pressed);
        }
        _ => panic!("Expected Button event"),
    }
}

#[test]
fn test_keep_alive_event_decode() {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = 0x01;
    packet[1] = 0x0a;

    let event = EventDecoder::decode(&packet).unwrap();
    assert!(event.is_some());
    match event.unwrap() {
        Event::KeepAlive => {}
        _ => panic!("Expected KeepAlive event"),
    }
}

#[test]
fn test_non_event_packet() {
    let mut packet = [0u8; PACKET_SIZE];
    packet[0] = 0x03; // Not an event packet
    packet[1] = 0x84;

    let event = EventDecoder::decode(&packet).unwrap();
    assert!(event.is_none());
}
