# streamdeck-cora-rs

Rust library for controlling Elgato Stream Deck Studio via TCP protocol using the Cora protocol.

## Usage

```toml
[dependencies]
streamdeck-cora-rs = "0.1.0"
tokio = { version = "1", features = ["full"] }
```

```rust
use streamdeck_cora_rs::Device;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let device = Device::connect_tcp("192.168.1.100:5343").await?;
    device.set_brightness(80).await?;
    Ok(())
}
```

## Binaries

- `streamdeck-server` - Emulator server (Studio)
- `networkdock-server` - Emulator server (NetworkDock)
- `streamdeck-manager` - Multi-instance manager
- `streamdeck-client` - Interactive CLI client

Run from workspace root:
```bash
cargo run -p streamdeck-cora-rs --bin streamdeck-manager
```

## License

MIT
