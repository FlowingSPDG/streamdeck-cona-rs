# streamdeck-cora-rs

[![crates.io](https://img.shields.io/crates/v/streamdeck-cora-rs.svg)](https://crates.io/crates/streamdeck-cora-rs)
[![docs.rs](https://docs.rs/streamdeck-cora-rs/badge.svg)](https://docs.rs/streamdeck-cora-rs)

Rust library for controlling Elgato Stream Deck Studio devices via TCP using the Cora protocol.

## Workspace

This repository contains:

- **`streamdeck-cora-rs-core`**: no-std compatible protocol parsing/encoding
- **`streamdeck-cora-rs`**: Full async TCP library with binaries


See each crate's README for usage:

- [streamdeck-cora-rs](streamdeck-cora-rs/README.md) - Main library
- [streamdeck-cora-rs-core](streamdeck-cora-rs-core/README.md) - Core protocol library

## Binaries

This workspace provides several command-line tools:

- **`streamdeck-server`**: Emulator server for Stream Deck Studio (single instance)
- **`networkdock-server`**: Emulator server for Stream Deck NetworkDock (single instance)
- **`streamdeck-manager`**: CUI manager for creating and controlling multiple virtual Stream Deck Studio instances
- **`streamdeck-client`**: Command-line client for controlling Stream Deck devices

### Running Binaries

From workspace root:

```bash
# Run streamdeck-manager
cargo run -p streamdeck-cora-rs --bin streamdeck-manager

# Run other binaries
cargo run -p streamdeck-cora-rs --bin streamdeck-server
cargo run -p streamdeck-cora-rs --bin networkdock-server
cargo run -p streamdeck-cora-rs --bin streamdeck-client

# Run examples
cargo run -p streamdeck-cora-rs --example simple_client
```

## Protocol

Cora protocol on TCP port 5343. See [PROTOCOL.md](PROTOCOL.md) for details.

## License

MIT License - see LICENSE file for details
