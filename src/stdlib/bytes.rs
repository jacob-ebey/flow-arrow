use super::*;

const MODULE: &str = "std.bytes";

pub const SPLIT_LINES: StdSymbol = node(MODULE, "split_lines", "Bytes", "Seq[Bytes]");
pub const CONCAT_BYTES: StdSymbol = reducible_node(
    MODULE,
    "concat_bytes",
    "Seq[Bytes]",
    "Bytes",
    "(Bytes,Bytes)",
    "Bytes",
);
pub const JOIN_BYTES: StdSymbol = node(MODULE, "join_bytes", "(Seq[Bytes],Bytes)", "Bytes");
