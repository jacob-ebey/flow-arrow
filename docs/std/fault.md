# `std.fault`

Helpers for graph-visible fault diagnostics.

Faults represent unintended invalid states, not ordinary control flow.
See [`../faults.md`](../faults.md) for the language-level rules.

## Types

```text
Fault
Faultable[T]
```

## Nodes

```text
has_faults    : Seq[Fault] -> Bool
format_faults : Seq[Fault] -> Bytes
```

## Semantics

### `has_faults`

Returns `true` when a fault sequence is non-empty.

### `format_faults`

Formats fault diagnostics as bytes suitable for `write_stderr`.

The initial diagnostic format is intentionally simple and human-readable.
A stable structured diagnostic model is still an open design question.

## Example

```flow
import std.fault { has_faults, format_faults }

node summarize_faults(faults: Seq[Fault]) -> (diagnostics: Bytes, invalid: Bool) {
    faults -> has_faults -> invalid
    faults -> format_faults -> diagnostics
}
```
