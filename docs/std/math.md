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
mul          : (Int, Int)   -> Int
             | (Int, Real)  -> Real
             | (Real, Int)  -> Real
             | (Real, Real) -> Real
div          : (Int, Int)   -> Int
             | (Int, Real)  -> Real
             | (Real, Int)  -> Real
             | (Real, Real) -> Real
rem          : (Int, Int)   -> Int
             | (Int, Real)  -> Real
             | (Real, Int)  -> Real
             | (Real, Real) -> Real
neg          : Int          -> Int
             | Real         -> Real
abs          : Int          -> Int
             | Real         -> Real
sqrt         : Int          -> Real
             | Real         -> Real
eq           : (Int, Int)   -> Bool
             | (Int, Real)  -> Bool
             | (Real, Int)  -> Bool
             | (Real, Real) -> Bool
lt           : (Int, Int)   -> Bool
             | (Int, Real)  -> Bool
             | (Real, Int)  -> Bool
             | (Real, Real) -> Bool
gt           : (Int, Int)   -> Bool
             | (Int, Real)  -> Bool
             | (Real, Int)  -> Bool
             | (Real, Real) -> Bool
le           : (Int, Int)   -> Bool
             | (Int, Real)  -> Bool
             | (Real, Int)  -> Bool
             | (Real, Real) -> Bool
ge           : (Int, Int)   -> Bool
             | (Int, Real)  -> Bool
             | (Real, Int)  -> Bool
             | (Real, Real) -> Bool
min          : (Int, Int)   -> Int
             | (Int, Real)  -> Real
             | (Real, Int)  -> Real
             | (Real, Real) -> Real
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

### `mul`

Multiplies two numeric values. Integer times integer returns `Int`; any
combination involving `Real` returns `Real`.

- Not associative in the reduce sense (no identity declared in the
  initial profile; use user-defined wrappers for product reductions).
- Overflow is a boundary/data validation fault reported by the host
  runtime for integer results.

### `div`

Divides the first numeric value by the second. Integer divided by integer
returns `Int` (truncating toward zero); any combination involving `Real`
returns `Real`.

- Division by zero is a boundary/data validation fault.
- Not associative.

### `rem`

Returns the remainder of dividing the first numeric value by the second.
Integer remainder by integer returns `Int` (same sign as the dividend,
matching C `%`); any combination involving `Real` returns `Real` (matching
C `fmod`).

- Remainder by zero is a boundary/data validation fault.
- Not associative.

### `neg`

Returns the additive inverse. Integer overflow on `Int::MIN` is reported
as a runtime usage fault.

### `abs`

Returns the absolute value. Integer overflow on `Int::MIN` is reported
as a runtime usage fault.

### `sqrt`

Returns the square root as `Real`. Negative inputs are reported as a
runtime usage fault.

### `eq`

Returns whether two numeric values are equal.

### `lt`

Returns whether the first numeric value is strictly less than the second.

### `gt`

Returns whether the first numeric value is strictly greater than the second.

### `le`

Returns whether the first numeric value is less than or equal to the second.

### `ge`

Returns whether the first numeric value is greater than or equal to the second.

### `min`

Returns the smaller of two numeric values. Integer with integer returns
`Int`; any combination involving `Real` returns `Real`.

### `max`

Returns the larger of two numeric values. Integer with integer returns
`Int`; any combination involving `Real` returns `Real`.

## Examples

```flow
import std.math { add, eq, mul, rem, lt, sqrt }

node is_sum_one(x: Real, y: Real, n: Int) -> out: Bool {
    ($x, $y) -> add -> $sum
    ($n, 1) -> eq -> $out
}

node is_divisible(n: Int, d: Int) -> out: Bool {
    ($n, $d) -> rem -> $r
    ($r, 0) -> eq -> $out
}

node is_positive(n: Int) -> out: Bool {
    ($n, 0) -> gt -> $out
}

node hypotenuse(x: Real, y: Real) -> out: Real {
    ($x, $x) -> mul -> $xx
    ($y, $y) -> mul -> $yy
    ($xx, $yy) -> add -> sqrt -> $out
}
```
