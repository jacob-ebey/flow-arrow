# `std.fs`

Boundary file I/O nodes for command-line programs and effectful wrappers.

## Nodes

```text
read_file    : Bytes                  -> Faultable[Bytes]
write_file   : (Bytes,Bytes)          -> Faultable[Int]
open_file    : Bytes                  -> Faultable[Stream[Bytes]]
size         : Stream[Bytes]          -> Faultable[Int]
read_at      : (Stream[Bytes],Int,Int) -> Faultable[Bytes]
copy_to_file : (Stream[Bytes],Bytes)   -> Faultable[Int]
close        : Stream[V]              -> Faultable[Int]
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

### `open_file`

Opens a filesystem path for streaming reads.

- The path is supplied as UTF-8 `Bytes`.
- A path containing a NUL byte returns a fault.
- Open failures return a fault.
- Successful output is a `Stream[Bytes]`, not file contents.

### `size`

Returns the file stream size in bytes without consuming the current
stream position.

### `read_at`

Reads a byte slice without consuming the current stream position.

- The input tuple is `(stream, offset, length)`.
- Negative offsets and lengths return a fault.
- A range that extends past the end of the stream returns a fault.
- The returned `Bytes` contains only the requested slice, not the full
  stream.

### `copy_to_file`

Copies stream contents to a filesystem path using a fixed-size runtime
buffer.

- The input tuple is `(stream, output_path)`.
- The output path replaces any existing file.
- The return value is `0` on success.
- The stream is consumed from its current position.

### `close`

Closes a stream handle.

## Example

```flow
import std.cli { Args }
import std.fs { read_file, write_file }

program main(args: Args) -> exit_code: Faultable[Int] {
    "input.bin" -> read_file -> $contents
    ("copy.bin", $contents) -> write_file -> $exit_code
}
```
