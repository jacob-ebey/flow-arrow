use super::*;

const MODULE: &str = "std.int";

pub const PARSE_INT: StdSymbol = node(MODULE, "parse_int", "Bytes", "Faultable[Int]");
pub const FORMAT_INT: StdSymbol = node(MODULE, "format_int", "Int", "Bytes");
