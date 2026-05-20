# `std.int`

Pure parsing and formatting utilities for `Int` values.

## Nodes

```text
parse_int  : Bytes -> Int
format_int : Int   -> Bytes
```

## Semantics

### `parse_int`

Parses ASCII decimal bytes into an `Int`.

- Leading and trailing ASCII whitespace are ignored.
- A leading `-` is accepted.
- Overflow is a boundary/data validation failure reported by the host
  runtime; it is not a FlowArrow exception.

### `format_int`

Formats an `Int` as deterministic ASCII decimal bytes.

- No leading `+`.
- No leading zeroes except for the value `0`.

## Examples

```flow
import std.int { parse_int, format_int }

node parse_then_format(input: Bytes) -> output: Bytes {
    input -> parse_int -> n
    n -> format_int -> output
}
```
