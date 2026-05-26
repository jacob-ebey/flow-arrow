use super::*;

const MODULE: &str = "std.fs";
pub const H: &[&str] = &[super::RUNTIME_H];
pub const C: &str = include_str!("fs.c");

pub const READ_FILE: StdSymbol = io_node(MODULE, "read_file", "Bytes", "Faultable[Bytes]");
pub const WRITE_FILE: StdSymbol = io_node(MODULE, "write_file", "(Bytes,Bytes)", "Faultable[i64]");
pub const EXISTS: StdSymbol = io_node(MODULE, "exists", "Bytes", "Bool");
pub const IS_FILE: StdSymbol = io_node(MODULE, "is_file", "Bytes", "Bool");
pub const IS_DIR: StdSymbol = io_node(MODULE, "is_dir", "Bytes", "Bool");
pub const FILE_SIZE: StdSymbol = io_node(MODULE, "file_size", "Bytes", "Faultable[i64]");
pub const JOIN_PATH: StdSymbol = node(MODULE, "join_path", "(Bytes,Bytes)", "Bytes");
pub const BASENAME: StdSymbol = node(MODULE, "basename", "Bytes", "Bytes");
pub const DIRNAME: StdSymbol = node(MODULE, "dirname", "Bytes", "Bytes");
pub const LIST_DIR: StdSymbol = io_node(MODULE, "list_dir", "Bytes", "Faultable[Seq[Bytes]]");
pub const WALK_FILES: StdSymbol = io_node(MODULE, "walk_files", "Bytes", "Faultable[Seq[Bytes]]");
pub const READ_FILES: StdSymbol = io_node(
    MODULE,
    "read_files",
    "Seq[Bytes]",
    "Faultable[Seq[(Bytes,Bytes)]]",
);
pub const OPEN_FILE: StdSymbol = io_node(MODULE, "open_file", "Bytes", "Faultable[Stream[Bytes]]");
pub const SIZE: StdSymbol = io_node(MODULE, "size", "Stream[Bytes]", "Faultable[i64]");
pub const READ_AT: StdSymbol = io_node(
    MODULE,
    "read_at",
    "(Stream[Bytes],i64,i64)",
    "Faultable[Bytes]",
);
pub const COPY_TO_FILE: StdSymbol = io_node(
    MODULE,
    "copy_to_file",
    "(Stream[Bytes],Bytes)",
    "Faultable[i64]",
);
pub const CLOSE: StdSymbol = io_node(MODULE, "close", "Stream[V]", "Faultable[i64]");
