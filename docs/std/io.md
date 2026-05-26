# `std.io`

Boundary I/O nodes for command-line programs.

Unlike most standard-library nodes, `std.io` nodes are effect boundary
nodes. They are still explicit graph vertices. A reusable `node` that
calls one becomes effectful by composition, and effectful nodes cannot be
used as higher-order `map`, `filter`, `reduce`, or `scan` functions.

## Nodes

```text
read_stdin   : ()    -> Bytes
write_stdout : Bytes -> i64
write_stderr : Bytes -> i64
```

## Semantics

### `read_stdin`

Reads all bytes from standard input.

- The read is explicit in the dependency graph:

  ```flow
  () -> read_stdin -> $input
  ```

- The node has no FlowArrow data input because stdin is supplied by the
  host process boundary.
- A program may contain at most one `read_stdin` node.

### `write_stdout`

Writes bytes to standard output and produces an exit code.

- Produces `0` on success.
- A boundary fault produces a non-zero host-defined exit code.
- The program's returned `i64` exit code should depend on this node if
  stdout must be written before process exit.

### `write_stderr`

Writes bytes to standard error and produces an exit code.

- Produces `0` on success.
- A boundary fault produces a non-zero host-defined exit code.

## Example

```flow
import std.cli { Args }
import std.io { read_stdin, write_stdout }

program main(args: Args) -> exit_code: i64 {
    () -> read_stdin -> $input
    $input -> transform -> $output
    $output -> write_stdout -> $exit_code
}
```
