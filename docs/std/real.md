# `std.real`

Pure parsing and formatting utilities for `Real` values.

## Nodes

```text
parse_real  : Bytes -> Faultable[Real]
format_real : Real  -> Bytes
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

## Examples

```flow
import std.real { parse_real, format_real }

node parse_then_format(input: Bytes) -> output: Faultable[Bytes] {
    input -> parse_real -> n
    n -> format_real -> output
}
```
