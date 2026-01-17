//! Protocol implementation for Stream Deck Studio TCP protocol

pub mod command;
pub mod cora;
pub mod event;
pub mod packet;

pub use command::{Command, CommandEncoder};
pub use cora::{CoraMessage, CoraMessageFlags, CoraHidOp, is_cora_magic, is_legacy_keep_alive};
pub use event::{Event, EventDecoder};
pub use packet::{PacketReader, PacketWriter};
