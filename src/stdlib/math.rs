use super::*;

const MODULE: &str = "std.math";

pub const ADD: StdSymbol = reducible_node(MODULE, "add", "(V,V)", "V", "(V,V)", "V");
pub const SUB: StdSymbol = node(MODULE, "sub", "(V,V)", "V");
pub const MUL: StdSymbol = node(MODULE, "mul", "(V,V)", "V");
pub const DIV: StdSymbol = node(MODULE, "div", "(V,V)", "Faultable[V]");
pub const REM: StdSymbol = node(MODULE, "rem", "(V,V)", "Faultable[V]");
pub const NEG: StdSymbol = node(MODULE, "neg", "V", "V");
pub const ABS: StdSymbol = node(MODULE, "abs", "V", "V");
pub const SQRT: StdSymbol = node(MODULE, "sqrt", "f64", "Faultable[f64]");
pub const EXP: StdSymbol = node(MODULE, "exp", "f64", "f64");
pub const SIN: StdSymbol = node(MODULE, "sin", "f64", "f64");
pub const COS: StdSymbol = node(MODULE, "cos", "f64", "f64");
pub const EQ: StdSymbol = node(MODULE, "eq", "(V,V)", "Bool");
pub const LT: StdSymbol = node(MODULE, "lt", "(V,V)", "Bool");
pub const GT: StdSymbol = node(MODULE, "gt", "(V,V)", "Bool");
pub const LE: StdSymbol = node(MODULE, "le", "(V,V)", "Bool");
pub const GE: StdSymbol = node(MODULE, "ge", "(V,V)", "Bool");
pub const MIN: StdSymbol = reducible_node(MODULE, "min", "(V,V)", "V", "(V,V)", "V");
pub const MAX: StdSymbol = reducible_node(MODULE, "max", "(V,V)", "V", "(V,V)", "V");
