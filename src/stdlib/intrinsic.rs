use super::*;

pub const H: &[&str] = &[super::RUNTIME_H];
pub const C: &str = include_str!("intrinsic.c");

pub const RANGE_STEP: StdSymbol = node(INTRINSIC_MODULE, "range_step", "(i64,i64,i64)", "Seq[i64]");
pub const SELECT: StdSymbol = generic_node(INTRINSIC_MODULE, "select", "(Bool,T,T)", "T");
