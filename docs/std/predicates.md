# `std.predicates`

Small reusable pure predicates and boolean logic nodes.

## Nodes

```text
not_empty : Bytes -> Bool
is_empty  : Bytes -> Bool
and       : (Bool, Bool) -> Bool
or        : (Bool, Bool) -> Bool
xor       : (Bool, Bool) -> Bool
not       : Bool -> Bool
all       : Seq[Bool] -> Bool
any       : Seq[Bool] -> Bool
```

## Semantics

### `not_empty`

Returns `true` when a byte sequence has length greater than zero.

- Whitespace is considered content.
- Invalid UTF-8 is irrelevant; this operates on bytes.
- Useful with `filter not_empty` after `split_lines`.

### `is_empty`

Returns `true` when a byte sequence has length zero.

- Exact complement of `not_empty`.
- Useful for validation paths that need to identify missing input.

### `and`

Returns `true` when both inputs are `true`.

- Both inputs are always evaluated (no short-circuit).

### `or`

Returns `true` when at least one input is `true`.

- Both inputs are always evaluated (no short-circuit).

### `xor`

Returns `true` when exactly one input is `true`.

- Both inputs are always evaluated (no short-circuit).

### `not`

Returns the logical negation of its input.

- Can be used as a `filter` or `map` argument since it takes a single
  `Bool` input.

### `all`

Returns `true` when every item in a boolean sequence is `true`.

- Empty input returns `true` (vacuous truth).

### `any`

Returns `true` when at least one item in a boolean sequence is `true`.

- Empty input returns `false`.

## Examples

```flow
import std.bytes { split_lines }
import std.predicates { not_empty, is_empty, and, or, xor, not, all, any }

node non_empty_lines(input: Bytes) -> lines: Seq[Bytes] {
    $input -> split_lines -> $raw_lines
    $raw_lines -> filter not_empty -> $lines
}

node either_flag(a: Bool, b: Bool) -> out: Bool {
    ($a, $b) -> or -> $out
}

node both_flags(a: Bool, b: Bool) -> out: Bool {
    ($a, $b) -> and -> $out
}

node exactly_one_flag(a: Bool, b: Bool) -> out: Bool {
    ($a, $b) -> xor -> $out
}

node sequence_checks(values: Seq[Bool]) -> (all_ok: Bool, any_ok: Bool) {
    $values -> all -> $all_ok
    $values -> any -> $any_ok
}
```
