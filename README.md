# streamdeck-rs-tcp

[![crates.io](https://img.shields.io/crates/v/streamdeck-rs-tcp.svg)](https://crates.io/crates/streamdeck-rs-tcp)
[![docs.rs](https://docs.rs/streamdeck-rs-tcp/badge.svg)](https://docs.rs/streamdeck-rs-tcp)

Rust library for communicating with Elgato Stream Deck Studio devices via TCP/IP protocol.

## Overview

This library provides a Rust implementation for controlling Elgato Stream Deck Studio devices over the network. The Stream Deck Studio uses a fixed 1024-byte binary protocol over TCP port 5343, which is essentially the USB HID protocol implemented over TCP/IP.

## Features

- ✅ TCP connection management
- ✅ Brightness control
- ✅ Event handling (button, encoder, touch, NFC)
- ✅ Keep-alive management
- 🚧 Device discovery (not ready yet)
- 🚧 no-std support for embedded development


## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
streamdeck-rs-tcp = "0.1.0"
tokio = { version = "1", features = ["full"] }
```

## Example

See `examples/simple_client.rs` for a complete example:

```rust
use streamdeck_rs_tcp::Device;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to Stream Deck Studio
    let device = Device::connect_tcp("192.168.1.100:5343").await?;
    
    // Get and print serial number
    if let Some(serial) = device.serial_number().await {
        println!("Connected to device with serial: {}", serial);
    }
    
    // Set brightness to 80%
    device.set_brightness(80).await?;
    
    // Set button image
    let image_data = include_bytes!("image.jpg");
    device.set_button_image(5, image_data.to_vec()).await?;
    
    Ok(())
}
```

## Protocol

This library implements the TCP protocol for Stream Deck Studio as documented in `TCP_RAW_PANEL_PROTOCOL.md`.

Key characteristics:
- Port: 5343
- Packet size: 1024 bytes (fixed)
- Protocol: Binary (USB HID over TCP/IP)
- Keep-alive: 5 second timeout

## Crates

This workspace contains two crates:

- **`streamdeck-rs-tcp-core`**: no-std compatible core library for protocol parsing/encoding
- **`streamdeck-rs-tcp`**: Full-featured async TCP library (depends on core)

## Useful Links

- [SKAARHOJ Wiki - Stream Deck on Raw Panel](https://wiki.skaarhoj.com/books/raw-panel/page/stream-deck-on-raw-panel) - Comprehensive documentation about Stream Deck Studio protocol and usage
- [YouTube Video](https://www.youtube.com/watch?v=lrTc9Ogmh8s) - Video demonstration of Stream Deck integration

## License

MIT License - see LICENSE file for details
