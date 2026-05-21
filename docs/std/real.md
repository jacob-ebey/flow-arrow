# `std.real`

Pure parsing and formatting utilities for `Real` values.

## Nodes

```text
parse_real  : Bytes -> Faultable[Real]
format_real : Real  -> Bytes
from_int    : Int   -> Real
```

## Semantics

### `parse_real`

Parses ASCII decimal bytes into a `Real`.

- Leading and trailing ASCII whitespace are ignored.
- Accepted syntax is the language's `REAL` literal form, plus integer
  text as a convenience (`"1"` parses as `1.0`).
- Invalid input is a data validation fault. If unhandled, it propagates
  through the surrounding definition as `Faultable[...]`; if handled with
  `fault map`, it becomes graph-visible `Fault` data.

### `format_real`

Formats a `Real` as deterministic ASCII bytes.

- Output must be stable across supported targets for the same value.
- Finite values use a shortest round-trippable decimal representation.
- `NaN` and infinities are not part of the initial portable profile and
  must be rejected by earlier validation or target-specific extensions.

### `from_int`

Converts an `Int` to the corresponding `Real`. This is intended for
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

node byte_to_unit(value: Int) -> out: Real {
    $value -> from_int -> $real
    ($real, 255.0) -> div -> $out
}
```
