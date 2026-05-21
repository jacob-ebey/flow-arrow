# `std.fs`

Boundary file I/O nodes for command-line programs and effectful wrappers.

## Nodes

```text
read_file  : Bytes         -> Faultable[Bytes]
write_file : (Bytes,Bytes) -> Faultable[Int]
```

`write_file` takes `(path, contents)` and returns `0` on success.

## Semantics

### `read_file`

Reads all bytes from a filesystem path.

- The path is supplied as UTF-8 `Bytes`.
- A path containing a NUL byte returns a fault.
- Open, read, and close failures return a fault.
- Successful reads preserve binary data, including NUL bytes.

### `write_file`

Writes bytes to a filesystem path, replacing the file if it already
exists.

- The input tuple is `(path, contents)`.
- A path containing a NUL byte returns a fault.
- Open, write, and close failures return a fault.
- Successful writes produce `0`.

## Example

```flow
import std.cli { Args }
import std.fs { read_file, write_file }

program main(args: Args) -> exit_code: Faultable[Int] {
    "input.bin" -> read_file -> $contents
    ("copy.bin", $contents) -> write_file -> $exit_code
}
```
