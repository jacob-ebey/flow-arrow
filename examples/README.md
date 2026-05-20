# FlowArrow examples

Small programs intended to stress the language design.

Every example imports the small set of standard-library nodes it uses.
The initial stdlib surface is documented in [`docs/std/`](../docs/std/):

```flow
import std.bytes { split_lines, concat_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout }
import std.real { parse_real, format_real }
import std.int { format_int }
import std.math { add, sub_int, eq_int }
import std.predicates { not_empty }
```

```text
# Byte / text
split_lines       : Bytes -> Seq[Bytes]
parse_real        : Bytes -> Real
format_int        : Int   -> Bytes
format_real       : Real  -> Bytes
concat_bytes      : Seq[Bytes] -> Bytes              # associative; identity: ""

# Boundary I/O
Args              # CLI argument/flag input type
read_stdin        : ()    -> Bytes
write_stdout      : Bytes -> Int

# Predicates / arithmetic
not_empty         : Bytes -> Bool
add               : (Real, Real) -> Real             # associative
sub_int           : (Int, Int)   -> Int
eq_int            : (Int, Int)   -> Bool
```

These are the stdlib primitives currently backed by the compiler and
runtime; adding more should start in the compiler's stdlib registry.

Use `flowarrow typecheck <path.flow>` to validate imports and graph
types without emitting LLVM or invoking the native backend.

| Example                       | What it shows                                          |
| ----------------------------- | ------------------------------------------------------ |
| `add-numbers-from-stdin/`     | Boundary I/O, dynamic-size sequences, parallel reduce. |
| `99-bottles/`                 | Pure string generation via `range_step` + `map` + concat reduce. |
