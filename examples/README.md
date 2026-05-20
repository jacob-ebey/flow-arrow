# FlowArrow examples

Small programs intended to stress the language design.

Every example imports the small set of standard-library nodes it uses.
The initial stdlib surface is documented in [`docs/std/`](../docs/std/):

```flow
import std.bytes { split_lines, concat_bytes, join_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout, write_stderr }
import std.real { parse_real, format_real }
import std.int { parse_int, format_int }
import std.math { add, add_int, sub_int, eq_int }
import std.predicates { not_empty }
```

```text
# Byte / text
split_lines       : Bytes -> Seq[Bytes]
parse_real        : Bytes -> Real
parse_int         : Bytes -> Int
format_int        : Int   -> Bytes
format_real       : Real  -> Bytes
concat_bytes      : Seq[Bytes] -> Bytes              # associative; identity: ""
join_bytes        : (Seq[Bytes], Bytes) -> Bytes     # second arg is separator

# Boundary I/O
Args              # CLI argument/flag input type
read_stdin        : ()    -> Bytes
write_stdout      : Bytes -> Int
write_stderr      : Bytes -> Int

# Predicates / arithmetic
not_empty         : Bytes -> Bool
add               : (Real, Real) -> Real             # associative
add_int           : (Int, Int)   -> Int              # associative
sub_int           : (Int, Int)   -> Int
eq_int            : (Int, Int)   -> Bool
```

These are stdlib primitives, not language built-ins; the examples
work as soon as a stdlib provides them.

| Example                       | What it shows                                          |
| ----------------------------- | ------------------------------------------------------ |
| `add-numbers-from-stdin/`     | Boundary I/O, dynamic-size sequences, parallel reduce. |
| `99-bottles/`                 | Pure string generation via `range_step` + `map` + concat reduce. |
