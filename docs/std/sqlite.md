# `std.sqlite`

SQLite boundary nodes backed by system SQLite 3.

`std.sqlite` exposes explicit database I/O while keeping row and value
inspection as ordinary typed FlowArrow nodes. SQL execution always uses prepared
statements with positional parameters.

## Types

```text
sqlite.Connection
sqlite.Row
sqlite.Value
```

## Nodes

```text
open            : Bytes -> Faultable[sqlite.Connection]
open_readonly   : Bytes -> Faultable[sqlite.Connection]
open_memory     : () -> Faultable[sqlite.Connection]
close           : sqlite.Connection -> Faultable[i64]

busy_timeout    : (sqlite.Connection, i64) -> Faultable[sqlite.Connection]
foreign_keys    : (sqlite.Connection, Bool) -> Faultable[sqlite.Connection]

begin           : sqlite.Connection -> Faultable[sqlite.Connection]
begin_immediate : sqlite.Connection -> Faultable[sqlite.Connection]
commit          : sqlite.Connection -> Faultable[sqlite.Connection]
rollback        : sqlite.Connection -> Faultable[sqlite.Connection]

null            : () -> sqlite.Value
int             : i64 -> sqlite.Value
real            : f64 -> sqlite.Value
text            : Bytes -> sqlite.Value
blob            : Bytes -> sqlite.Value

exec            : (sqlite.Connection, Bytes, Seq[sqlite.Value]) -> Faultable[(sqlite.Connection, i64)]
query           : (sqlite.Connection, Bytes, Seq[sqlite.Value]) -> Faultable[(sqlite.Connection, Stream[sqlite.Row])]
query_all       : (sqlite.Connection, Bytes, Seq[sqlite.Value]) -> Faultable[(sqlite.Connection, Seq[sqlite.Row])]

column_count    : sqlite.Row -> i64
column_name     : (sqlite.Row, i64) -> Faultable[Bytes]
value_at        : (sqlite.Row, i64) -> Faultable[sqlite.Value]
value_named     : (sqlite.Row, Bytes) -> Faultable[sqlite.Value]

kind            : sqlite.Value -> Bytes
is_null         : sqlite.Value -> Bool
as_int          : sqlite.Value -> Faultable[i64]
as_real         : sqlite.Value -> Faultable[f64]
as_text         : sqlite.Value -> Faultable[Bytes]
as_blob         : sqlite.Value -> Faultable[Bytes]
```

## Semantics

`open` creates a read/write database with `SQLITE_OPEN_CREATE` and
`SQLITE_OPEN_FULLMUTEX`. `open_readonly` opens an existing database readonly.
`open_memory` opens an in-memory database.

New connections set a 5000 ms busy timeout and enable foreign keys. The
`busy_timeout` and `foreign_keys` nodes can change those settings.

`exec`, `query`, and `query_all` accept one SQL statement and a sequence of
positional parameter values. SQL containing a NUL byte, trailing non-whitespace
after the first statement, or a bind count mismatch returns a fault. `exec`
faults if the statement returns rows; use `query` or `query_all` for row output.

`query` returns a single-pass `Stream[sqlite.Row]`. Rows are owned snapshots:
column names and values remain valid after the statement steps again or
finalizes. Consuming a stream to EOF, `stream.to_seq`, or `stream.drain`
finalizes the statement. `query_all` materializes all rows and finalizes before
returning.

Typed getters are exact: `as_int` accepts integer values, `as_real` accepts real
values, `as_text` accepts text values, and `as_blob` accepts blob values. Use
`kind` or `is_null` before extracting optional values.

## Example

```flow
import std.cli { Args }
import std.sqlite as sqlite
import std.stream as stream
import std.tuple { first, second }

program main(args: Args) -> exit_code: Faultable[i64] {
    () -> sqlite.open_memory -> $conn0

    ($conn0, "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL)", []) -> sqlite.exec -> first -> $conn1
    ($conn1, "INSERT INTO todos (title) VALUES (?)", ["ship sqlite" -> sqlite.text]) -> sqlite.exec -> first -> $conn2
    ($conn2, "SELECT title FROM todos ORDER BY id", []) -> sqlite.query -> $query
    $query -> first -> $conn3
    $query -> second -> stream.to_seq -> $rows
    $conn3 -> sqlite.close -> $exit_code
}
```
