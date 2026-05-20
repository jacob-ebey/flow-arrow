# `std.io`

Boundary I/O nodes for command-line programs.

Unlike most standard-library nodes, `std.io` nodes are effect boundary
nodes. They are still explicit graph vertices, but they may only appear
inside a `program` body. Pure `node` declarations cannot read stdin,
write stdout, write stderr, or observe process I/O.

## Nodes

```text
read_stdin   : ()    -> Bytes
write_stdout : Bytes -> Int
write_stderr : Bytes -> Int
```

## Semantics

### `read_stdin`

Reads all bytes from standard input.

- Legal only in a `program` body.
- The read is explicit in the dependency graph:

  ```flow
  () -> read_stdin -> input
  ```

- The node has no FlowArrow data input because stdin is supplied by the
  host process boundary.
- A program may contain at most one `read_stdin` node.

### `write_stdout`

Writes bytes to standard output and produces an exit code.

- Legal only in a `program` body.
- Produces `0` on success.
- A boundary fault produces a non-zero host-defined exit code.
- The program's returned `Int` exit code should depend on this node if
  stdout must be written before process exit.

### `write_stderr`

Writes bytes to standard error and produces an exit code.

- Legal only in a `program` body.
- Produces `0` on success.
- A boundary fault produces a non-zero host-defined exit code.

## Example

```flow
import std.cli { Args }
import std.io { read_stdin, write_stdout }

program main(args: Args) -> exit_code: Int {
    () -> read_stdin -> input
    input -> transform -> output
    output -> write_stdout -> exit_code
}
```
