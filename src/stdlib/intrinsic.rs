use super::*;

pub const RANGE_STEP: StdSymbol = node(INTRINSIC_MODULE, "range_step", "(Int,Int,Int)", "Seq[Int]");
pub const SELECT: StdSymbol = generic_node(INTRINSIC_MODULE, "select", "(Bool,T,T)", "T");
