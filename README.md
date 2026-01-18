# streamdeck-cona-rs

[![crates.io](https://img.shields.io/crates/v/streamdeck-cona-rs.svg)](https://crates.io/crates/streamdeck-cona-rs)
[![docs.rs](https://docs.rs/streamdeck-cona-rs/badge.svg)](https://docs.rs/streamdeck-cona-rs)

Rust library for controlling Elgato Stream Deck Studio devices via TCP using the Cora protocol.

## Workspace

This repository contains:

- **`streamdeck-cona-rs-core`**: no-std compatible protocol parsing/encoding
- **`streamdeck-cona-rs`**: Full async TCP library with binaries


See each crate's README for usage:

- [streamdeck-cona-rs](streamdeck-cona-rs/README.md) - Main library
- [streamdeck-cona-rs-core](streamdeck-cona-rs-core/README.md) - Core protocol library

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
cargo run -p streamdeck-cona-rs --bin streamdeck-manager

# Run other binaries
cargo run -p streamdeck-cona-rs --bin streamdeck-server
cargo run -p streamdeck-cona-rs --bin networkdock-server
cargo run -p streamdeck-cona-rs --bin streamdeck-client

# Run examples
cargo run -p streamdeck-cona-rs --example simple_client
```

## Protocol

Cora protocol on TCP port 5343. See [PROTOCOL.md](PROTOCOL.md) for details.

## License

MIT License - see LICENSE file for details
