# streamdeck-cora-rs-core

no-std compatible core library for Stream Deck Studio TCP protocol (Cora protocol).

This crate provides protocol parsing and encoding functionality without requiring the standard library. Use `streamdeck-cora-rs` for full async TCP support.

## Features

- `alloc`: Enable heap allocations (default)
- `std`: Alias for `alloc` feature

## Usage

```toml
[dependencies]
streamdeck-cora-rs-core = { version = "0.1.0", default-features = false }
```

For no-std without alloc:

```toml
[dependencies]
streamdeck-cora-rs-core = { version = "0.1.0", default-features = false }
heapless = "0.8"
```

## License

MIT
