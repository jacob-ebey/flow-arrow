use super::*;

const MODULE: &str = "std.predicates";

pub const NOT_EMPTY: StdSymbol = node(MODULE, "not_empty", "Bytes", "Bool");
pub const IS_EMPTY: StdSymbol = node(MODULE, "is_empty", "Bytes", "Bool");
pub const AND: StdSymbol = node(MODULE, "and", "(Bool,Bool)", "Bool");
pub const OR: StdSymbol = node(MODULE, "or", "(Bool,Bool)", "Bool");
pub const XOR: StdSymbol = node(MODULE, "xor", "(Bool,Bool)", "Bool");
pub const NOT: StdSymbol = node(MODULE, "not", "Bool", "Bool");
pub const ALL: StdSymbol = node(MODULE, "all", "Seq[Bool]", "Bool");
pub const ANY: StdSymbol = node(MODULE, "any", "Seq[Bool]", "Bool");
