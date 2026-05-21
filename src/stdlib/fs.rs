use super::*;

const MODULE: &str = "std.fs";
pub const C: &str = include_str!("fs.c");

pub const READ_FILE: StdSymbol = io_node(MODULE, "read_file", "Bytes", "Faultable[Bytes]");
pub const WRITE_FILE: StdSymbol = io_node(MODULE, "write_file", "(Bytes,Bytes)", "Faultable[Int]");
