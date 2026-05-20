# FlowArrow examples

Small programs intended to stress the language design.

Every example imports the small set of standard-library nodes it uses.
The initial stdlib surface is documented in [`docs/std/`](../docs/std/):

```flow
import std.bytes { split_lines, concat_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout }
import std.real { parse_real, format_real }
import std.int { parse_int, format_int }
import std.math { add, add_int, sub_int, eq_int, max_int }
import std.predicates { not_empty }
import std.fault { Fault, has_faults, format_faults }
```

```text
# Byte / text
split_lines       : Bytes -> Seq[Bytes]
parse_int         : Bytes -> Faultable[Int]
parse_real        : Bytes -> Faultable[Real]
format_int        : Int   -> Bytes                 # propagates Faultable[Int] -> Faultable[Bytes]
format_real       : Real  -> Bytes                 # propagates Faultable[Real] -> Faultable[Bytes]
concat_bytes      : Seq[Bytes] -> Bytes              # associative; identity: ""

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
max_int           : (Int, Int)   -> Int

# Faults
Fault
has_faults        : Seq[Fault] -> Bool
format_faults     : Seq[Fault] -> Bytes
```

These are the stdlib primitives currently backed by the compiler and
runtime; adding more should start in the compiler's stdlib registry.

Use `flowarrow typecheck <path.flow>` to validate imports and graph
types without emitting LLVM or invoking the native backend.

Use `flowarrow graph <path.flow>` to print the typed execution graph as a
Mermaid `flowchart TD` diagram.

| Example                       | What it shows                                          |
| ----------------------------- | ------------------------------------------------------ |
| `add-numbers-from-stdin/`     | Boundary I/O, dynamic-size sequences, parallel reduce. |
| `parse-and-sum-lines/`        | Minimal pressure test for parse faults and graph-visible fault semantics. |
| `99-bottles/`                 | Pure string generation via `range_step` + `map` + concat reduce. |
| `fibonacci/`                  | Stdin integer parsing and FlowArrow Fibonacci iteration. |
