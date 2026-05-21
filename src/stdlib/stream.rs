use super::*;

const MODULE: &str = "std.stream";

pub const STREAM: StdSymbol = ty(MODULE, "Stream");
pub const TO_SEQ: StdSymbol = io_node(MODULE, "to_seq", "Stream[V]", "Faultable[Seq[V]]");
pub const DRAIN: StdSymbol = io_node(MODULE, "drain", "Stream[V]", "Faultable[Int]");
