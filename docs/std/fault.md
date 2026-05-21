# `std.fault`

Helpers for graph-visible fault diagnostics.

Faults represent unintended invalid states, not ordinary control flow.
See [`../faults.md`](../faults.md) for the language-level rules.

## Types

```text
Fault
Faultable[T]
```

Plain values flow into matching `Faultable[T]` outputs implicitly. A node or
match arm that produces `T` can satisfy a declared `Faultable[T]` result; the
compiler emits the successful faultable wrapper.

## Nodes

```text
has_faults    : Seq[Fault] -> Bool
format_faults : Seq[Fault] -> Bytes
expect        : Faultable[V] -> V
collect       : Seq[Faultable[V]] -> Faultable[Seq[V]]
collect       : Faultable[Seq[Faultable[V]]] -> Faultable[Seq[V]]
```

## Semantics

### `has_faults`

Returns `true` when a fault sequence is non-empty.

### `format_faults`

Formats fault diagnostics as bytes suitable for `write_stderr`.

The initial diagnostic format is intentionally simple and human-readable.
A stable structured diagnostic model is still an open design question.

### `expect`

Unwraps a successful `Faultable[V]`. If the input is a fault, the runtime emits
the fault diagnostic and exits non-zero.

### `collect`

Converts `Seq[Faultable[V]]` into `Faultable[Seq[V]]`. If the input sequence is
itself faultable, the outer fault is propagated first. The result is successful
when every item is successful; otherwise the first item fault is propagated.

## Example

```flow
import std.fault { has_faults, format_faults }

node summarize_faults(faults: Seq[Fault]) -> (diagnostics: Bytes, invalid: Bool) {
    $faults -> has_faults -> $invalid
    $faults -> format_faults -> $diagnostics
}
```
