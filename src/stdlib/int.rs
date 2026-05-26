use super::*;

const MODULE: &str = "std.int";
pub const H: &[&str] = &[super::RUNTIME_H];
pub const C: &str = include_str!("int.c");

pub const PARSE_INT: StdSymbol = node(MODULE, "parse_int", "Bytes", "Faultable[i64]");
pub const FORMAT_INT: StdSymbol = node(MODULE, "format_int", "i64", "Bytes");
pub const BIT_AND: StdSymbol = node(MODULE, "bit_and", "(i64,i64)", "i64");
pub const BIT_OR: StdSymbol = node(MODULE, "bit_or", "(i64,i64)", "i64");
pub const BIT_XOR: StdSymbol = node(MODULE, "bit_xor", "(i64,i64)", "i64");
pub const BIT_SHL: StdSymbol = node(MODULE, "bit_shl", "(i64,i64)", "i64");
pub const BIT_SHR: StdSymbol = node(MODULE, "bit_shr", "(i64,i64)", "i64");
