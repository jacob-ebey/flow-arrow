# FlowArrow standard library

The standard library is imported through the reserved `std` module
root. Most stdlib declarations are pure nodes and types; boundary
modules such as `std.io` expose explicit program-only I/O nodes.
Importing any stdlib module never creates hidden ordering, hidden
effects, or dynamic dispatch.

Initial modules:

| Module | Purpose |
| --- | --- |
| [`std.bytes`](./bytes.md) | Byte/text splitting, concatenation, joining |
| [`std.cli`](./cli.md) | Command-line argument and flag helpers |
| [`std.io`](./io.md) | Stdin/stdout/stderr boundary I/O |
| [`std.real`](./real.md) | `Real` parsing and formatting |
| [`std.int`](./int.md) | `Int` parsing and formatting |
| [`std.math`](./math.md) | Arithmetic and comparisons |
| [`std.predicates`](./predicates.md) | Reusable predicates |
| [`std.fault`](./fault.md) | Fault diagnostics and fault-status helpers |

Example:

```flow
import std.bytes { split_lines, concat_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout }
import std.real { parse_real, format_real }
import std.math { add }
import std.predicates { not_empty }

program main(args: Args) -> exit_code: Int {
    () -> read_stdin -> input
    input -> split_lines -> lines
    lines -> transform -> output
    output -> write_stdout -> exit_code
}
```

These docs define the first stable surface area needed by the current
examples. More modules should be added only when they preserve
FlowArrow's core invariant:

```text
syntax = data dependency
```
