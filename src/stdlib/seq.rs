use super::*;

const MODULE: &str = "std.seq";

pub const LENGTH: StdSymbol = node(MODULE, "length", "Seq[Var]", "Int");
pub const GROUP_BY_ID: StdSymbol = node(
    MODULE,
    "group_by_id",
    "(Seq[Var],Seq[Int])",
    "Seq[Seq[Var]]",
);
pub const ZIP: StdSymbol = node(MODULE, "zip", "(Seq[A],Seq[B])", "Seq[(A,B)]");
pub const SHIFT_RIGHT: StdSymbol = node(MODULE, "shift_right", "(Seq[V],V)", "Seq[V]");
pub const HEAD: StdSymbol = node(MODULE, "head", "Seq[V]", "Faultable[V]");
