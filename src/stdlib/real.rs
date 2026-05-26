use super::*;

const MODULE: &str = "std.real";
pub const H: &[&str] = &[super::RUNTIME_H];
pub const C: &str = include_str!("real.c");

pub const PARSE_REAL: StdSymbol = node(MODULE, "parse_real", "Bytes", "Faultable[f64]");
pub const FORMAT_REAL: StdSymbol = node(MODULE, "format_real", "f64", "Bytes");
pub const FROM_INT: StdSymbol = node(MODULE, "from_int", "i64", "f64");
