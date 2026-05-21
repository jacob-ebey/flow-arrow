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
contains     : (Bytes, Bytes) -> Bool
starts_with  : (Bytes, Bytes) -> Bool
ends_with    : (Bytes, Bytes) -> Bool
index_of     : (Bytes, Bytes) -> Int
last_index_of: (Bytes, Bytes) -> Int
slice        : (Bytes, Int, Int) -> Bytes
take         : (Bytes, Int) -> Bytes
drop         : (Bytes, Int) -> Bytes
replace      : (Bytes, Bytes, Bytes) -> Bytes
repeat_bytes : (Bytes, Int) -> Bytes
ascii_lower  : Bytes -> Bytes
ascii_upper  : Bytes -> Bytes
split_on     : (Bytes, Bytes) -> Seq[Bytes]
strip_prefix : (Bytes, Bytes) -> Faultable[Bytes]
strip_suffix : (Bytes, Bytes) -> Faultable[Bytes]
bytes_to_codes : Bytes -> Seq[Int]
codes_to_bytes : Seq[Int] -> Bytes
byte_length    : Bytes -> Int
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

### Search and slicing

`contains`, `starts_with`, and `ends_with` test byte subsequences exactly.
`index_of` returns the first byte offset of a subsequence, or `-1` when it is
not present. `last_index_of` returns the final offset, or `-1`.

`slice(input, start, end)` returns the half-open byte range
`start..end`. `take(input, count)` and `drop(input, count)` clamp counts beyond
the input length. Negative indices or counts are usage faults.

### Replacement and casing

`replace(input, needle, replacement)` replaces non-overlapping occurrences of
`needle`. The needle must be non-empty. `repeat_bytes(input, count)` repeats a
byte sequence `count` times; the count must be non-negative.

`ascii_lower` and `ascii_upper` only transform ASCII letters. All other bytes
are preserved.

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
