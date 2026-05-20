# `std.bytes`

Pure byte/text utilities. `Bytes` is FlowArrow's boundary-safe byte
sequence type: programs receive and produce bytes at the effect
boundary, while all processing inside the graph remains pure.

## Nodes

```text
split_lines  : Bytes -> Seq[Bytes]
concat_bytes : Seq[Bytes] -> Bytes
join_bytes   : (Seq[Bytes], Bytes) -> Bytes
trim         : Bytes -> Bytes
split_on     : (Bytes, Bytes) -> Seq[Bytes]
strip_prefix : (Bytes, Bytes) -> Faultable[Bytes]
strip_suffix : (Bytes, Bytes) -> Faultable[Bytes]
```

## Semantics

### `split_lines`

Splits a byte sequence into lines.

- Line terminators are not included in returned lines.
- Both `\n` and `\r\n` are accepted as line terminators.
- If the input ends with a line terminator, no extra trailing empty line
  is produced.
- Invalid UTF-8 is allowed; this operates on bytes, not Unicode scalar
  values.

### `concat_bytes`

Concatenates a sequence of byte chunks in order.

- Associative.
- Identity: `""`.
- Not commutative; ordering of the input sequence is preserved.
- Suitable for `reduce concat_bytes(identity: "")`.

### `join_bytes`

Concatenates byte chunks with a separator between adjacent chunks.

- The second input is the separator.
- Empty input sequence produces `""`.
- A one-element input sequence returns that element unchanged.

### `trim`

Returns the input with leading and trailing ASCII whitespace removed.

- ASCII whitespace is ` `, `\t`, `\n`, `\r`, `\v`, and `\f`.
- Interior bytes are preserved verbatim.
- Operates on bytes; non-ASCII whitespace is not recognised.
- Safe to use as a `map` argument over `Seq[Bytes]`.

### `split_on`

Splits the first input on every occurrence of the second input.

- The separator must be a non-empty byte sequence.
- Adjacent or boundary separators produce empty segments; the output
  always has length `occurrences + 1`.
- The separator itself is not included in any output segment.
- Useful for tokenising boundary-delimited formats such as JSON arrays
  after removing structural framing.

### `strip_prefix`

Removes a required prefix from the input.

- Returns the remaining bytes on success.
- Returns a graph-visible `Fault` when the input does not start with the
  prefix. If unhandled, propagates through the surrounding declaration
  as `Faultable[Bytes]`.

### `strip_suffix`

Removes a required suffix from the input.

- Returns the leading bytes on success.
- Returns a graph-visible `Fault` when the input does not end with the
  suffix. If unhandled, propagates through the surrounding declaration
  as `Faultable[Bytes]`.

## Examples

```flow
import std.bytes { split_lines, concat_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout }

program main(args: Args) -> exit_code: Int {
    () -> read_stdin -> $input
    $input -> split_lines -> $lines
    ["line count not implemented yet", "\n"] -> concat_bytes -> $output
    $output -> write_stdout -> $exit_code
}
```
