# `std.int`

Pure parsing and formatting utilities for `Int` values.

## Nodes

```text
parse_int  : Bytes -> Faultable[Int]
format_int : Int   -> Bytes
```

## Semantics

### `parse_int`

Parses ASCII decimal bytes into an `Int`.

- Leading and trailing ASCII whitespace are ignored.
- A leading `-` is accepted.
- Invalid input and overflow are data validation faults. If unhandled,
  they propagate through the surrounding definition as `Faultable[...]`;
  they are not exceptions or control-flow mechanisms.

### `format_int`

Formats an `Int` as deterministic ASCII decimal bytes.

- No leading `+`.
- No leading zeroes except for the value `0`.

## Examples

```flow
import std.int { parse_int, format_int }

node parse_then_format(input: Bytes) -> output: Faultable[Bytes] {
    $input -> parse_int -> $n
    $n -> format_int -> $output
}
```
