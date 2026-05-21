use super::*;

const MODULE: &str = "std.fs";
pub const C: &str = include_str!("fs.c");

pub const READ_FILE: StdSymbol = io_node(MODULE, "read_file", "Bytes", "Faultable[Bytes]");
pub const WRITE_FILE: StdSymbol = io_node(MODULE, "write_file", "(Bytes,Bytes)", "Faultable[Int]");
pub const OPEN_FILE: StdSymbol = io_node(MODULE, "open_file", "Bytes", "Faultable[Stream[Bytes]]");
pub const SIZE: StdSymbol = io_node(MODULE, "size", "Stream[Bytes]", "Faultable[Int]");
pub const READ_AT: StdSymbol = io_node(
    MODULE,
    "read_at",
    "(Stream[Bytes],Int,Int)",
    "Faultable[Bytes]",
);
pub const COPY_TO_FILE: StdSymbol = io_node(
    MODULE,
    "copy_to_file",
    "(Stream[Bytes],Bytes)",
    "Faultable[Int]",
);
pub const CLOSE: StdSymbol = io_node(MODULE, "close", "Stream[V]", "Faultable[Int]");
