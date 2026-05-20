# FlowArrow examples

Small programs intended to stress the language design.

Every example imports the small set of standard-library nodes it uses.
The initial stdlib surface is documented in [`docs/std/`](../docs/std/):

```flow
import std.bytes { split_lines, concat_bytes, join_bytes, trim, split_on, strip_prefix, strip_suffix }
import std.cli { Args, argv }
import std.io { read_stdin, write_stdout }
import std.real { parse_real, format_real }
import std.int { parse_int, format_int }
import std.math { add, sub, mul, div, rem, eq, lt, gt, le, ge, max }
import std.predicates { not_empty, is_empty, and, or, xor, not, all, any }
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
join_bytes        : (Seq[Bytes], Bytes) -> Bytes     # joins with separator
trim              : Bytes -> Bytes                   # strips leading/trailing ASCII whitespace
split_on          : (Bytes, Bytes) -> Seq[Bytes]     # splits on a non-empty byte separator
strip_prefix      : (Bytes, Bytes) -> Faultable[Bytes] # faults if input does not start with prefix
strip_suffix      : (Bytes, Bytes) -> Faultable[Bytes] # faults if input does not end with suffix

# Boundary I/O
Args              # CLI argument/flag input type
argv              : Args -> Seq[Bytes]      # excludes executable name
read_stdin        : ()    -> Bytes
write_stdout      : Bytes -> Int
write_stderr      : Bytes -> Int

# Arithmetic
add               : (Int|Real, Int|Real) -> Int|Real # associative
sub               : (Int|Real, Int|Real) -> Int|Real
mul               : (Int|Real, Int|Real) -> Int|Real
div               : (Int|Real, Int|Real) -> Int|Real # truncates toward zero for Int
rem               : (Int|Real, Int|Real) -> Int|Real # same sign as dividend for Int
max               : (Int|Real, Int|Real) -> Int|Real

# Comparisons
eq                : (Int|Real, Int|Real) -> Bool
lt                : (Int|Real, Int|Real) -> Bool
gt                : (Int|Real, Int|Real) -> Bool
le                : (Int|Real, Int|Real) -> Bool
ge                : (Int|Real, Int|Real) -> Bool

# Boolean logic
and               : (Bool, Bool) -> Bool
or                : (Bool, Bool) -> Bool
xor               : (Bool, Bool) -> Bool
not               : Bool -> Bool                     # usable as map/filter argument
not_empty         : Bytes -> Bool                    # usable as filter argument
is_empty          : Bytes -> Bool
all               : Seq[Bool] -> Bool
any               : Seq[Bool] -> Bool

# Faults
Fault
has_faults        : Seq[Fault] -> Bool
format_faults     : Seq[Fault] -> Bytes
```

These are the stdlib primitives currently backed by the compiler and
runtime. Native primitives live in the compiler's stdlib registry;
source-backed modules such as `std.vector` live as bundled `.flow`
modules with an explicit export list.

Use `flowarrow typecheck <path.flow>` to validate imports and graph
types without emitting LLVM or invoking the native backend.

Use `flowarrow graph <path.flow>` to print the typed execution graph as a
Mermaid `flowchart TD` diagram.

| Example                       | What it shows                                          |
| ----------------------------- | ------------------------------------------------------ |
| `add-numbers-from-stdin/`     | Boundary I/O, dynamic-size sequences, parallel reduce. |
| `add-numbers-from-args/`      | Command-line argument parsing and parallel reduce.     |
| `parse-and-sum-lines/`        | Minimal pressure test for parse faults and graph-visible fault semantics. |
| `99-bottles/`                 | Pure string generation via `range_step` + `map` + concat reduce. |
| `fibonacci/`                  | Stdin integer parsing and FlowArrow Fibonacci iteration. |
| `json-parser/`                | Flat JSON array of numbers → JSON summary object, with bracket framing and fault routing. |
