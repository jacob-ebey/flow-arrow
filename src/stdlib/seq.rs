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
pub const BROADCAST_LEFT: StdSymbol = node(MODULE, "broadcast_left", "(A,Seq[B])", "Seq[(A,B)]");
pub const BROADCAST_RIGHT: StdSymbol = node(MODULE, "broadcast_right", "(Seq[A],B)", "Seq[(A,B)]");
pub const TRANSPOSE: StdSymbol = node(MODULE, "transpose", "Seq[Seq[V]]", "Seq[Seq[V]]");
pub const FLATTEN: StdSymbol = node(MODULE, "flatten", "Seq[Seq[V]]", "Seq[V]");
pub const INNER_LENGTH: StdSymbol = node(MODULE, "inner_length", "Seq[Seq[V]]", "Int");
pub const SHIFT_RIGHT: StdSymbol = node(MODULE, "shift_right", "(Seq[V],V)", "Seq[V]");
pub const SHIFT_LEFT: StdSymbol = node(MODULE, "shift_left", "(Seq[V],V)", "Seq[V]");
pub const HEAD: StdSymbol = node(MODULE, "head", "Seq[V]", "Faultable[V]");
pub const TAIL: StdSymbol = node(MODULE, "tail", "Seq[V]", "Seq[V]");
