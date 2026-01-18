//! Protocol re-exports from core library

pub use streamdeck_cona_rs_core::{
    Command, CommandEncoder,
    CoraMessage, CoraMessageFlags, CoraHidOp, CORA_MAGIC, CORA_HEADER_SIZE, is_cora_magic, is_legacy_keep_alive,
    Event, EventDecoder,
    PacketReader, PacketWriter,
};
