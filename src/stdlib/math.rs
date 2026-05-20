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
pub const DIV: StdSymbol = node(MODULE, "div", "(Number,Number)", "Number");
pub const REM: StdSymbol = node(MODULE, "rem", "(Number,Number)", "Number");
pub const EQ: StdSymbol = node(MODULE, "eq", "(Number,Number)", "Bool");
pub const LT: StdSymbol = node(MODULE, "lt", "(Number,Number)", "Bool");
pub const GT: StdSymbol = node(MODULE, "gt", "(Number,Number)", "Bool");
pub const LE: StdSymbol = node(MODULE, "le", "(Number,Number)", "Bool");
pub const GE: StdSymbol = node(MODULE, "ge", "(Number,Number)", "Bool");
pub const MAX: StdSymbol = node(MODULE, "max", "(Number,Number)", "Number");
