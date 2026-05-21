use super::*;

const MODULE: &str = "std.stream";
pub const C: &str = include_str!("stream.c");

pub const STREAM: StdSymbol = ty(MODULE, "Stream");
pub const OPEN_FILE: StdSymbol = io_node(MODULE, "open_file", "Bytes", "Faultable[Stream]");
pub const SIZE: StdSymbol = io_node(MODULE, "size", "Stream", "Faultable[Int]");
pub const READ_AT: StdSymbol =
    io_node(MODULE, "read_at", "(Stream,Int,Int)", "Faultable[Bytes]");
pub const COPY_TO_FILE: StdSymbol =
    io_node(MODULE, "copy_to_file", "(Stream,Bytes)", "Faultable[Int]");
pub const CLOSE: StdSymbol = io_node(MODULE, "close", "Stream", "Faultable[Int]");
