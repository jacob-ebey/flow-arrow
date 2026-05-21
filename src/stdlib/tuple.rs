use super::*;

const MODULE: &str = "std.tuple";

pub const FIRST: StdSymbol = node(MODULE, "first", "(A,B)", "A");
pub const SECOND: StdSymbol = node(MODULE, "second", "(A,B)", "B");
pub const SWAP: StdSymbol = node(MODULE, "swap", "(A,B)", "(B,A)");
