use super::*;

const MODULE: &str = "std.sqlite";
pub const H: &[&str] = &[super::RUNTIME_H, include_str!("sqlite.h")];
pub const C: &str = include_str!("sqlite.c");

pub const CONNECTION: StdSymbol = ty(MODULE, "Connection");
pub const ROW: StdSymbol = ty(MODULE, "Row");
pub const VALUE: StdSymbol = ty(MODULE, "Value");

pub const OPEN: StdSymbol = io_node(MODULE, "open", "Bytes", "Faultable[sqlite.Connection]");
pub const OPEN_READONLY: StdSymbol = io_node(
    MODULE,
    "open_readonly",
    "Bytes",
    "Faultable[sqlite.Connection]",
);
pub const OPEN_MEMORY: StdSymbol =
    io_node(MODULE, "open_memory", "()", "Faultable[sqlite.Connection]");
pub const CLOSE: StdSymbol = io_node(MODULE, "close", "sqlite.Connection", "Faultable[Int]");

pub const BUSY_TIMEOUT: StdSymbol = io_node(
    MODULE,
    "busy_timeout",
    "(sqlite.Connection,Int)",
    "Faultable[sqlite.Connection]",
);
pub const FOREIGN_KEYS: StdSymbol = io_node(
    MODULE,
    "foreign_keys",
    "(sqlite.Connection,Bool)",
    "Faultable[sqlite.Connection]",
);
pub const BEGIN: StdSymbol = io_node(
    MODULE,
    "begin",
    "sqlite.Connection",
    "Faultable[sqlite.Connection]",
);
pub const BEGIN_IMMEDIATE: StdSymbol = io_node(
    MODULE,
    "begin_immediate",
    "sqlite.Connection",
    "Faultable[sqlite.Connection]",
);
pub const COMMIT: StdSymbol = io_node(
    MODULE,
    "commit",
    "sqlite.Connection",
    "Faultable[sqlite.Connection]",
);
pub const ROLLBACK: StdSymbol = io_node(
    MODULE,
    "rollback",
    "sqlite.Connection",
    "Faultable[sqlite.Connection]",
);

pub const NULL: StdSymbol = node(MODULE, "null", "()", "sqlite.Value");
pub const INT: StdSymbol = node(MODULE, "int", "Int", "sqlite.Value");
pub const REAL: StdSymbol = node(MODULE, "real", "Real", "sqlite.Value");
pub const TEXT: StdSymbol = node(MODULE, "text", "Bytes", "sqlite.Value");
pub const BLOB: StdSymbol = node(MODULE, "blob", "Bytes", "sqlite.Value");

pub const EXEC: StdSymbol = io_node(
    MODULE,
    "exec",
    "(sqlite.Connection,Bytes,Seq[sqlite.Value])",
    "Faultable[(sqlite.Connection,Int)]",
);
pub const QUERY: StdSymbol = io_node(
    MODULE,
    "query",
    "(sqlite.Connection,Bytes,Seq[sqlite.Value])",
    "Faultable[(sqlite.Connection,Stream[sqlite.Row])]",
);
pub const QUERY_ALL: StdSymbol = io_node(
    MODULE,
    "query_all",
    "(sqlite.Connection,Bytes,Seq[sqlite.Value])",
    "Faultable[(sqlite.Connection,Seq[sqlite.Row])]",
);

pub const COLUMN_COUNT: StdSymbol = node(MODULE, "column_count", "sqlite.Row", "Int");
pub const COLUMN_NAME: StdSymbol = node(
    MODULE,
    "column_name",
    "(sqlite.Row,Int)",
    "Faultable[Bytes]",
);
pub const VALUE_AT: StdSymbol = node(
    MODULE,
    "value_at",
    "(sqlite.Row,Int)",
    "Faultable[sqlite.Value]",
);
pub const VALUE_NAMED: StdSymbol = node(
    MODULE,
    "value_named",
    "(sqlite.Row,Bytes)",
    "Faultable[sqlite.Value]",
);

pub const KIND: StdSymbol = node(MODULE, "kind", "sqlite.Value", "Bytes");
pub const IS_NULL: StdSymbol = node(MODULE, "is_null", "sqlite.Value", "Bool");
pub const AS_INT: StdSymbol = node(MODULE, "as_int", "sqlite.Value", "Faultable[Int]");
pub const AS_REAL: StdSymbol = node(MODULE, "as_real", "sqlite.Value", "Faultable[Real]");
pub const AS_TEXT: StdSymbol = node(MODULE, "as_text", "sqlite.Value", "Faultable[Bytes]");
pub const AS_BLOB: StdSymbol = node(MODULE, "as_blob", "sqlite.Value", "Faultable[Bytes]");
