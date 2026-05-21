use super::*;

const MODULE: &str = "std.io";
pub const H: &[&str] = &[super::RUNTIME_H];
pub const C: &str = include_str!("io.c");

pub const READ_STDIN: StdSymbol = io_node(MODULE, "read_stdin", "()", "Bytes");
pub const WRITE_STDOUT: StdSymbol = io_node(MODULE, "write_stdout", "Bytes", "Int");
pub const WRITE_STDERR: StdSymbol = io_node(MODULE, "write_stderr", "Bytes", "Int");
