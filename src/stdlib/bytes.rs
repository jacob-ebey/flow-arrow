use super::*;

const MODULE: &str = "std.bytes";
pub const H: &[&str] = &[super::RUNTIME_H];
pub const C: &str = include_str!("bytes.c");

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
pub const CONTAINS: StdSymbol = node(MODULE, "contains", "(Bytes,Bytes)", "Bool");
pub const STARTS_WITH: StdSymbol = node(MODULE, "starts_with", "(Bytes,Bytes)", "Bool");
pub const ENDS_WITH: StdSymbol = node(MODULE, "ends_with", "(Bytes,Bytes)", "Bool");
pub const INDEX_OF: StdSymbol = node(MODULE, "index_of", "(Bytes,Bytes)", "i64");
pub const LAST_INDEX_OF: StdSymbol = node(MODULE, "last_index_of", "(Bytes,Bytes)", "i64");
pub const BYTE_SLICE: StdSymbol = node(MODULE, "slice", "(Bytes,i64,i64)", "Bytes");
pub const TAKE: StdSymbol = node(MODULE, "take", "(Bytes,i64)", "Bytes");
pub const DROP: StdSymbol = node(MODULE, "drop", "(Bytes,i64)", "Bytes");
pub const REPLACE: StdSymbol = node(MODULE, "replace", "(Bytes,Bytes,Bytes)", "Bytes");
pub const REPEAT_BYTES: StdSymbol = node(MODULE, "repeat_bytes", "(Bytes,i64)", "Bytes");
pub const ASCII_LOWER: StdSymbol = node(MODULE, "ascii_lower", "Bytes", "Bytes");
pub const ASCII_UPPER: StdSymbol = node(MODULE, "ascii_upper", "Bytes", "Bytes");
pub const SPLIT_ON: StdSymbol = node(MODULE, "split_on", "(Bytes,Bytes)", "Seq[Bytes]");
pub const STRIP_PREFIX: StdSymbol =
    node(MODULE, "strip_prefix", "(Bytes,Bytes)", "Faultable[Bytes]");
pub const STRIP_SUFFIX: StdSymbol =
    node(MODULE, "strip_suffix", "(Bytes,Bytes)", "Faultable[Bytes]");
pub const BYTES_TO_CODES: StdSymbol = node(MODULE, "bytes_to_codes", "Bytes", "Seq[i64]");
pub const CODES_TO_BYTES: StdSymbol = node(MODULE, "codes_to_bytes", "Seq[i64]", "Bytes");
pub const BYTE_LENGTH: StdSymbol = node(MODULE, "byte_length", "Bytes", "i64");
