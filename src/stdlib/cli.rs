use super::*;

const MODULE: &str = "std.cli";
pub const C: &str = include_str!("cli.c");

pub const ARGS: StdSymbol = ty(MODULE, "Args");
pub const ARGV: StdSymbol = node(MODULE, "argv", "Args", "Seq[Bytes]");
pub const FLAG_PRESENT: StdSymbol =
    unsupported_node(MODULE, "flag_present", "(Args,Bytes)", "Bool");
pub const FLAG_VALUE: StdSymbol = unsupported_node(MODULE, "flag_value", "(Args,Bytes)", "Bytes");
