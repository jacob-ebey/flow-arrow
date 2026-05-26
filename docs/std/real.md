# `std.real`

Pure parsing and formatting utilities for `f64` values.

## Nodes

```text
parse_real  : Bytes -> Faultable[f64]
format_real : f64  -> Bytes
from_int    : i64   -> f64
```

## Semantics

### `parse_real`

Parses ASCII decimal bytes into a `f64`.

- Leading and trailing ASCII whitespace are ignored.
- Accepted syntax is the language's `REAL` literal form, plus integer
  text as a convenience (`"1"` parses as `1.0`).
- Invalid input is a data validation fault. If unhandled, it propagates
  through the surrounding definition as `Faultable[...]`; if handled with
  `fault map`, it becomes graph-visible `Fault` data.

### `format_real`

Formats a `f64` as deterministic ASCII bytes.

- Output must be stable across supported targets for the same value.
- Finite values use a shortest round-trippable decimal representation.
- `NaN` and infinities are not part of the initial portable profile and
  must be rejected by earlier validation or target-specific extensions.

### `from_int`

Converts an `i64` to the corresponding `f64`. This is intended for
explicit numeric normalization, for example turning byte channel samples
into `0.0..1.0` values.

## Examples

```flow
import std.math { div }
import std.real { parse_real, format_real, from_int }

node parse_then_format(input: Bytes) -> output: Faultable[Bytes] {
    $input -> parse_real -> $n
    $n -> format_real -> $output
}

node byte_to_unit(value: i64) -> out: f64 {
    $value -> from_int -> $real
    ($real, 255.0) -> div -> $out
}
```
