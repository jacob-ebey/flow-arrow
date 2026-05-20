# `std.math`

Pure arithmetic and comparison nodes. These are ordinary pure nodes,
not language built-ins; importing them only affects compile-time name
resolution.

## Nodes

```text
add          : (Int, Int)   -> Int
             | (Int, Real)  -> Real
             | (Real, Int)  -> Real
             | (Real, Real) -> Real
sub          : (Int, Int)   -> Int
             | (Int, Real)  -> Real
             | (Real, Int)  -> Real
             | (Real, Real) -> Real
eq           : (Int, Int)   -> Bool
             | (Int, Real)  -> Bool
             | (Real, Int)  -> Bool
             | (Real, Real) -> Bool
max          : (Int, Int)   -> Int
             | (Int, Real)  -> Real
             | (Real, Int)  -> Real
             | (Real, Real) -> Real
```

## Semantics

### `add`

Adds two numeric values. Integer plus integer returns `Int`; any
combination involving `Real` returns `Real`.

- Declared associative for the initial portable profile.
- Identities: `0` for integer reductions, `0.0` for real reductions.
- Suitable for `reduce add(identity: 0)` and `reduce add(identity: 0.0)`.
- Overflow is a boundary/data validation fault reported by the host
  runtime for integer results.

### `sub`

Subtracts the second numeric value from the first. Integer minus integer
returns `Int`; any combination involving `Real` returns `Real`.

- Not associative.
- Must not be used with `reduce` or `scan`.
- Overflow is a boundary/data validation fault reported by the host
  runtime for integer results.

### `eq`

Returns whether two numeric values are equal.

### `max`

Returns the larger of two numeric values. Integer with integer returns
`Int`; any combination involving `Real` returns `Real`.

## Examples

```flow
import std.math { add, eq }

node is_sum_one(x: Real, y: Real, n: Int) -> out: Bool {
    (x, y) -> add -> sum
    (n, 1) -> eq -> out
}
```
