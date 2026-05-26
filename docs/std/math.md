# `std.math`

Pure arithmetic and comparison nodes. These are ordinary pure nodes,
not language built-ins; importing them only affects compile-time name
resolution.

## Nodes

```text
add          : (i64, i64)   -> i64
             | (f64, f64)   -> f64
sub          : (i64, i64)   -> i64
             | (f64, f64)   -> f64
mul          : (i64, i64)   -> i64
             | (f64, f64)   -> f64
div          : (i64, i64)   -> Faultable[i64]
             | (f64, f64)   -> Faultable[f64]
rem          : (i64, i64)   -> Faultable[i64]
             | (f64, f64)   -> Faultable[f64]
neg          : i64          -> i64
             | f64         -> f64
abs          : i64          -> i64
             | f64         -> f64
sqrt         : f64          -> Faultable[f64]
eq           : (i64, i64)   -> Bool
             | (f64, f64)   -> Bool
lt           : (i64, i64)   -> Bool
             | (f64, f64)   -> Bool
gt           : (i64, i64)   -> Bool
             | (f64, f64)   -> Bool
le           : (i64, i64)   -> Bool
             | (f64, f64)   -> Bool
ge           : (i64, i64)   -> Bool
             | (f64, f64)   -> Bool
min          : (i64, i64)   -> i64
             | (f64, f64)   -> f64
max          : (i64, i64)   -> i64
             | (f64, f64)   -> f64
```

## Semantics

### `add`

Adds two values of the same numeric type. `i64` plus `i64` returns `i64`;
`f64` plus `f64` returns `f64`. Mixed numeric operands are rejected.

- Declared associative for the initial portable profile.
- Identities: `0` for `i64` reductions, `0.0` for `f64` reductions.
- Suitable for `reduce add(identity: 0)` and `reduce add(identity: 0.0)`.
- Overflow is a boundary/data validation fault reported by the host
  runtime for integer results.

### `sub`

Subtracts the second numeric value from the first. Both operands must have
the same numeric type.

- Not associative.
- Must not be used with `reduce` or `scan`.
- Overflow is a boundary/data validation fault reported by the host
  runtime for integer results.

### `mul`

Multiplies two numeric values. Both operands must have the same numeric type.

- Not associative in the reduce sense (no identity declared in the
  initial profile; use user-defined wrappers for product reductions).
- Overflow is a boundary/data validation fault reported by the host
  runtime for integer results.

### `div`

Divides the first numeric value by the second. Both operands must have the
same numeric type. `i64` division truncates toward zero.

- Division by zero is reported as `Faultable`.
- Not associative.

### `rem`

Returns the remainder of dividing the first numeric value by the second.
Both operands must have the same numeric type. `i64` remainder has the same
sign as the dividend, matching C `%`; `f64` remainder matches C `fmod`.

- Remainder by zero is reported as `Faultable`.
- Not associative.

### `neg`

Returns the additive inverse. Integer overflow on `i64::MIN` is reported
as a runtime usage fault.

### `abs`

Returns the absolute value. Integer overflow on `i64::MIN` is reported
as a runtime usage fault.

### `sqrt`

Returns the square root as `Faultable[f64]`. Negative inputs are reported as
fault values.

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

Returns the smaller of two numeric values. Both operands must have the same
numeric type.

### `max`

Returns the larger of two numeric values. Both operands must have the same
numeric type.

## Examples

```flow
import std.fault { expect }
import std.math { add, eq, mul, rem, lt, sqrt }

node is_sum_one(x: f64, y: f64, n: i64) -> out: Bool {
    ($x, $y) -> add -> $sum
    ($n, 1) -> eq -> $out
}

node is_divisible(n: i64, d: i64) -> out: Bool {
    ($n, $d) -> rem -> expect -> $r
    ($r, 0) -> eq -> $out
}

node is_positive(n: i64) -> out: Bool {
    ($n, 0) -> gt -> $out
}

node hypotenuse(x: f64, y: f64) -> out: f64 {
    ($x, $x) -> mul -> $xx
    ($y, $y) -> mul -> $yy
    ($xx, $yy) -> add -> sqrt -> expect -> $out
}
```
