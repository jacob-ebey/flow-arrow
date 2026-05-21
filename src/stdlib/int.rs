use super::*;

const MODULE: &str = "std.int";
pub const H: &[&str] = &[super::RUNTIME_H];
pub const C: &str = include_str!("int.c");

pub const PARSE_INT: StdSymbol = node(MODULE, "parse_int", "Bytes", "Faultable[Int]");
pub const FORMAT_INT: StdSymbol = node(MODULE, "format_int", "Int", "Bytes");
pub const BIT_AND: StdSymbol = node(MODULE, "bit_and", "(Int,Int)", "Int");
pub const BIT_OR: StdSymbol = node(MODULE, "bit_or", "(Int,Int)", "Int");
pub const BIT_XOR: StdSymbol = node(MODULE, "bit_xor", "(Int,Int)", "Int");
pub const BIT_SHL: StdSymbol = node(MODULE, "bit_shl", "(Int,Int)", "Int");
pub const BIT_SHR: StdSymbol = node(MODULE, "bit_shr", "(Int,Int)", "Int");
