# `std.fs`

Boundary file I/O nodes for command-line programs and effectful wrappers.

## Nodes

```text
read_file    : Bytes                  -> Faultable[Bytes]
write_file   : (Bytes,Bytes)          -> Faultable[Int]
exists       : Bytes                  -> Bool
is_file      : Bytes                  -> Bool
is_dir       : Bytes                  -> Bool
file_size    : Bytes                  -> Faultable[Int]
join_path    : (Bytes,Bytes)          -> Bytes
basename     : Bytes                  -> Bytes
dirname      : Bytes                  -> Bytes
list_dir     : Bytes                  -> Faultable[Seq[Bytes]]
walk_files   : Bytes                  -> Faultable[Seq[Bytes]]
read_files   : Seq[Bytes]             -> Faultable[Seq[(Bytes,Bytes)]]
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

### Metadata helpers

`exists`, `is_file`, and `is_dir` query the filesystem and return `false` for
missing paths and invalid paths. They do not expose the underlying I/O
diagnostic.

`file_size` returns the size of a regular file in bytes. Missing paths, invalid
paths, and non-file paths return a fault.

### Path helpers

`join_path(base, child)` joins two path byte strings with `/` when the base does
not already end with `/`. `basename` returns the final path component.
`dirname` returns the parent path, or `"."` when the input has no parent
component. These helpers are byte-oriented and do not canonicalize symlinks,
relative components, or platform-specific aliases.

### Directory discovery

`list_dir(path)` returns non-recursive directory entries as names, excluding `.`
and `..`. Entries are sorted by byte value so output is deterministic.

`walk_files(path)` returns sorted regular-file paths under `path`. If `path` is
itself a regular file, the result is a one-item sequence. If `path` contains
glob metacharacters (`*`, `?`, or `[`), matching paths are expanded first and
then walked. Symlinks and other non-regular, non-directory filesystem objects
are skipped. A glob pattern with no matches returns a fault.

`read_files(paths)` reads a sequence of paths at the boundary and returns
`Seq[(path, contents)]`. This is the batch form needed by dataflow programs that
cannot use effectful `read_file` as a `map` function. Pure nodes can then map,
filter, split, and format the returned file records.

### `open_file`

Opens a filesystem path for streaming reads.

- The path is supplied as UTF-8 `Bytes`.
- A path containing a NUL byte returns a fault.
- Open failures return a fault.
- Successful output is a pull-readable `Stream[Bytes]` of fixed-size byte
  chunks, not one materialized file value.
- Consuming the stream to EOF with `std.stream.to_seq` or `std.stream.drain`
  closes the underlying file handle.

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
