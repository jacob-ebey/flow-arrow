# FlowArrow examples

Small programs intended to stress the language design.

Every example imports the small set of standard-library nodes it uses.
The initial stdlib surface is documented in [`docs/std/`](../docs/std/):

```flow
import std.bytes { split_lines, concat_bytes, join_bytes, trim, split_on, strip_prefix, strip_suffix }
import std.cli { Args, argv }
import std.io { read_stdin, write_stdout }
import std.http as http
import std.real { parse_real, format_real, from_int }
import std.int { parse_int, format_int }
import std.math { add, sub, mul, div, rem, neg, abs, sqrt, eq, lt, gt, le, ge, min, max }
import std.predicates { not_empty, is_empty, and, or, xor, not, all, any }
import std.fault { Fault, has_faults, format_faults, collect, expect }
import std.seq { head, tail, length }
import std.fs { walk_files, read_files }
import std.cv { load, save_jpeg, grayscale }
import std.sqlite as sqlite
import std.stream as stream
```

```text
# Byte / text
split_lines       : Bytes -> Seq[Bytes]
parse_int         : Bytes -> Faultable[Int]
parse_real        : Bytes -> Faultable[Real]
format_int        : Int   -> Bytes                 # propagates Faultable[Int] -> Faultable[Bytes]
format_real       : Real  -> Bytes                 # propagates Faultable[Real] -> Faultable[Bytes]
from_int          : Int   -> Real
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
http.listen       : http.ServerConfig -> Faultable[http.Listener]
http.requests     : http.Listener -> Stream[http.Request]
http.serve        : (http.Listener, Stream[http.Response]) -> Faultable[Int]
sqlite.open       : Bytes -> Faultable[sqlite.Connection]
sqlite.exec       : (sqlite.Connection, Bytes, Seq[sqlite.Value]) -> Faultable[(sqlite.Connection, Int)]
sqlite.query      : (sqlite.Connection, Bytes, Seq[sqlite.Value]) -> Faultable[(sqlite.Connection, Stream[sqlite.Row])]
stream.to_seq     : Stream[V] -> Faultable[Seq[V]]
walk_files        : Bytes -> Faultable[Seq[Bytes]]
read_files        : Seq[Bytes] -> Faultable[Seq[(Bytes,Bytes)]]

# Arithmetic
add               : (Int|Real, Int|Real) -> Int|Real # associative
sub               : (Int|Real, Int|Real) -> Int|Real
mul               : (Int|Real, Int|Real) -> Int|Real
div               : (Int|Real, Int|Real) -> Int|Real # truncates toward zero for Int
rem               : (Int|Real, Int|Real) -> Int|Real # same sign as dividend for Int
neg               : Int|Real -> Int|Real
abs               : Int|Real -> Int|Real
sqrt              : Int|Real -> Real
min               : (Int|Real, Int|Real) -> Int|Real
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
collect           : Seq[Faultable[V]] -> Faultable[Seq[V]]
expect            : Faultable[V] -> V
```

These are the stdlib primitives currently backed by the compiler and
runtime. Native primitives live in the compiler's stdlib registry;
source-backed modules such as `std.vector`, `std.matrix`, and `std.cv`
live as bundled `.flow` modules with an explicit export list.

Use `flowarrow typecheck <path.flow>` to validate imports and graph
types without emitting LLVM or invoking the native backend.

Use `flowarrow graph <path.flow>` to print the typed execution graph as a
Mermaid `flowchart TD` diagram. The graph uses shapes and classes to separate
values, pure operations, boundary operations, collection operators, decisions,
and fault paths. Use `flowarrow graph --compact <path.flow>` to collapse
intermediate bindings into edge labels for a denser operation-first view.

| Example                       | What it shows                                          |
| ----------------------------- | ------------------------------------------------------ |
| `add-numbers-from-stdin/`     | Boundary I/O, dynamic-size sequences, parallel reduce. |
| `add-numbers-from-args/`      | Command-line argument parsing and parallel reduce.     |
| `concurrency/`                | Pure parallel map, independent reductions, deterministic join. |
| `gpu-accumulator-benchmark/`  | GPU-favorable repeated vector accumulator workload. |
| `parse-and-sum-lines/`        | Minimal pressure test for parse faults and graph-visible fault semantics. |
| `99-bottles/`                 | Pure string generation via `range_step` + `map` + concat reduce. |
| `fibonacci/`                  | Stdin integer parsing and FlowArrow Fibonacci iteration. |
| `higher-order-nodes/`         | Static node parameters lowered to concrete graph calls before codegen. |
| `wasm-fib/`                   | Pure FlowArrow `fib` node exported to WASM and called from Node.js. |
| `typescript-fib/`             | Pure FlowArrow `fib` node emitted as TypeScript and called from Node.js. |
| `typescript-concurrency-benchmark/` | Compares TypeScript sequential and worker-enabled builds for a pure map workload. |
| `ts-interop/`                 | TS/JS foreign imports from ESM modules and globals. |
| `c-interop/`                  | Native LLVM build importing and linking C ABI functions. |
| `c-library/`                  | Native shared library export consumed from a C application. |
| `json-parser/`                | Flat JSON array of numbers → JSON summary object, with bracket framing and fault routing. |
| `grep/`                       | Literal byte search over multiple file, directory, or glob targets. |
| `grayscale-image/`            | Filepath arguments plus `std.cv` image auto-detect, grayscale conversion, and JPEG encode. |

## Boundary API sketches

These examples exercise newer boundary APIs. Some may require optional system
libraries to build.

| Example | What it explores |
| --- | --- |
| `http-server/` | `std.http` server shape backed by H2O: listener boundary, request stream, pure response mapping, and explicit serving boundary. |
| `sqlite-todos/` | `std.sqlite` database boundary: prepared statements, row streams, row/value extraction, and explicit stream materialization. |
