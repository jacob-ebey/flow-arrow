# `std.bytes`

Pure byte/text utilities. `Bytes` is FlowArrow's boundary-safe byte
sequence type: programs receive and produce bytes at the effect
boundary, while all processing inside the graph remains pure.

## Nodes

```text
split_lines  : Bytes -> Seq[Bytes]
concat_bytes : Seq[Bytes] -> Bytes
join_bytes   : (Seq[Bytes], Bytes) -> Bytes
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

## Examples

```flow
import std.bytes { split_lines, concat_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout }

program main(args: Args) -> exit_code: Int {
    () -> read_stdin -> input
    input -> split_lines -> lines
    ["line count not implemented yet", "\n"] -> concat_bytes -> output
    output -> write_stdout -> exit_code
}
```
