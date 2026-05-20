# `std.math`

Pure arithmetic and comparison nodes. These are ordinary pure nodes,
not language built-ins; importing them only affects compile-time name
resolution.

## Nodes

```text
add          : (Real, Real) -> Real
add_int      : (Int, Int)   -> Int
sub_int      : (Int, Int)   -> Int
eq_int       : (Int, Int)   -> Bool
```

## Semantics

### `add`

Adds two `Real` values.

- Declared associative for the initial portable profile.
- Identity: `0.0`.
- Suitable for `reduce add(identity: 0.0)`.

### `add_int`

Adds two `Int` values.

- Associative.
- Identity: `0`.
- Suitable for `reduce add_int(identity: 0)`.
- Overflow is a boundary/data validation fault reported by the host
  runtime.

### `sub_int`

Subtracts the second `Int` from the first.

- Not associative.
- Must not be used with `reduce` or `scan`.

### `eq_int`

Returns whether two `Int` values are equal.

## Examples

```flow
import std.math { add, eq_int }

node is_sum_one(x: Real, y: Real, n: Int) -> out: Bool {
    (x, y) -> add -> sum
    (n, 1) -> eq_int -> out
}
```
