# `std.stream`

Boundary stream nodes for large file artifacts.

`std.fs.read_file` is still the right node for small byte assets. Use
`std.stream` when a program needs to carry a file across an effectful
boundary without materializing the whole file as `Bytes`.

## Types

```text
Stream
```

`Stream` is an opaque runtime file handle.

## Nodes

```text
open_file    : Bytes          -> Faultable[Stream]
size         : Stream         -> Faultable[Int]
read_at      : (Stream,Int,Int) -> Faultable[Bytes]
copy_to_file : (Stream,Bytes) -> Faultable[Int]
close        : Stream         -> Faultable[Int]
```

## Semantics

### `open_file`

Opens a filesystem path for streaming reads.

- The path is supplied as UTF-8 `Bytes`.
- A path containing a NUL byte returns a fault.
- Open failures return a fault.
- Successful output is a `Stream`, not file contents.

### `size`

Returns the stream size in bytes without consuming the current stream
position.

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
import std.stream { open_file, copy_to_file }

program main(args: Args) -> exit_code: Faultable[Int] {
    "large.bin" -> open_file -> $stream
    ($stream, "copy.bin") -> copy_to_file -> $exit_code
}
```
