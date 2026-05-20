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
pub const TRIM: StdSymbol = node(MODULE, "trim", "Bytes", "Bytes");
pub const SPLIT_ON: StdSymbol = node(MODULE, "split_on", "(Bytes,Bytes)", "Seq[Bytes]");
pub const STRIP_PREFIX: StdSymbol =
    node(MODULE, "strip_prefix", "(Bytes,Bytes)", "Faultable[Bytes]");
pub const STRIP_SUFFIX: StdSymbol =
    node(MODULE, "strip_suffix", "(Bytes,Bytes)", "Faultable[Bytes]");
pub const BYTES_TO_CODES: StdSymbol = node(MODULE, "bytes_to_codes", "Bytes", "Seq[Int]");
pub const CODES_TO_BYTES: StdSymbol = node(MODULE, "codes_to_bytes", "Seq[Int]", "Bytes");
pub const BYTE_LENGTH: StdSymbol = node(MODULE, "byte_length", "Bytes", "Int");
