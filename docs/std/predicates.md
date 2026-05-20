# `std.predicates`

Small reusable pure predicates.

## Nodes

```text
not_empty : Bytes -> Bool
```

## Semantics

### `not_empty`

Returns `true` when a byte sequence has length greater than zero.

- Whitespace is considered content.
- Invalid UTF-8 is irrelevant; this operates on bytes.
- Useful with `filter not_empty` after `split_lines`.

## Examples

```flow
import std.bytes { split_lines }
import std.predicates { not_empty }

node non_empty_lines(input: Bytes) -> lines: Seq[Bytes] {
    input -> split_lines -> raw_lines
    raw_lines -> filter not_empty -> lines
}
```
