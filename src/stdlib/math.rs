use super::*;

const MODULE: &str = "std.math";

pub const ADD: StdSymbol = reducible_node(
    MODULE,
    "add",
    "(Number,Number)",
    "Number",
    "(Number,Number)",
    "Number",
);
pub const SUB: StdSymbol = node(MODULE, "sub", "(Number,Number)", "Number");
pub const MUL: StdSymbol = node(MODULE, "mul", "(Number,Number)", "Number");
pub const DIV: StdSymbol = node(MODULE, "div", "(Number,Number)", "Faultable[Number]");
pub const REM: StdSymbol = node(MODULE, "rem", "(Number,Number)", "Faultable[Number]");
pub const NEG: StdSymbol = node(MODULE, "neg", "Number", "Number");
pub const ABS: StdSymbol = node(MODULE, "abs", "Number", "Number");
pub const SQRT: StdSymbol = node(MODULE, "sqrt", "Number", "Faultable[Real]");
pub const EXP: StdSymbol = node(MODULE, "exp", "Number", "Real");
pub const SIN: StdSymbol = node(MODULE, "sin", "Number", "Real");
pub const COS: StdSymbol = node(MODULE, "cos", "Number", "Real");
pub const EQ: StdSymbol = node(MODULE, "eq", "(Number,Number)", "Bool");
pub const LT: StdSymbol = node(MODULE, "lt", "(Number,Number)", "Bool");
pub const GT: StdSymbol = node(MODULE, "gt", "(Number,Number)", "Bool");
pub const LE: StdSymbol = node(MODULE, "le", "(Number,Number)", "Bool");
pub const GE: StdSymbol = node(MODULE, "ge", "(Number,Number)", "Bool");
pub const MIN: StdSymbol = reducible_node(
    MODULE,
    "min",
    "(Number,Number)",
    "Number",
    "(Number,Number)",
    "Number",
);
pub const MAX: StdSymbol = reducible_node(
    MODULE,
    "max",
    "(Number,Number)",
    "Number",
    "(Number,Number)",
    "Number",
);
