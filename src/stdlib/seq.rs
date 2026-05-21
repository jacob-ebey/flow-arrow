use super::*;

const MODULE: &str = "std.seq";

pub const LENGTH: StdSymbol = node(MODULE, "length", "Seq[Var]", "Int");
pub const IS_EMPTY: StdSymbol = node(MODULE, "is_empty", "Seq[Var]", "Bool");
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
pub const REVERSE: StdSymbol = node(MODULE, "reverse", "Seq[V]", "Seq[V]");
pub const TAKE: StdSymbol = node(MODULE, "take", "(Seq[V],Int)", "Seq[V]");
pub const DROP: StdSymbol = node(MODULE, "drop", "(Seq[V],Int)", "Seq[V]");
pub const FILL: StdSymbol = node(MODULE, "fill", "(V,Int)", "Seq[V]");
pub const SLICE: StdSymbol = node(MODULE, "slice", "(Seq[V],Int,Int)", "Seq[V]");
pub const LAST: StdSymbol = node(MODULE, "last", "Seq[V]", "Faultable[V]");
pub const GET: StdSymbol = node(MODULE, "get", "(Seq[V],Int)", "V");
pub const GET_OR: StdSymbol = node(MODULE, "get_or", "(Seq[V],Int,V)", "V");
pub const AT: StdSymbol = node(MODULE, "at", "(Seq[V],Int)", "Faultable[V]");
pub const APPEND: StdSymbol = node(MODULE, "append", "(Seq[V],V)", "Seq[V]");
pub const SET: StdSymbol = node(MODULE, "set", "(Seq[V],Int,V)", "Seq[V]");
pub const CONCAT: StdSymbol = node(MODULE, "concat", "(Seq[V],Seq[V])", "Seq[V]");
