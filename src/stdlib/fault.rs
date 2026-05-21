use super::*;

const MODULE: &str = "std.fault";
pub const C: &str = include_str!("fault.c");

pub const FAULT: StdSymbol = ty(MODULE, "Fault");
pub const HAS_FAULTS: StdSymbol = node(MODULE, "has_faults", "Seq[Fault]", "Bool");
pub const FORMAT_FAULTS: StdSymbol = node(MODULE, "format_faults", "Seq[Fault]", "Bytes");
