# `std.math`

Pure arithmetic and comparison nodes. These are ordinary pure nodes,
not language built-ins; importing them only affects compile-time name
resolution.

## Nodes

```text
add_i32      : (i32, i32) -> Faultable[i32]
add_i64      : (i64, i64) -> Faultable[i64]
add_f32      : (f32, f32) -> f32
add_f64      : (f64, f64) -> f64
sub_*        : same typed variants and outputs as add_*
mul_*        : same typed variants and outputs as add_*
div_i32      : (i32, i32) -> Faultable[i32]
div_i64      : (i64, i64) -> Faultable[i64]
div_f32      : (f32, f32) -> Faultable[f32]
div_f64      : (f64, f64) -> Faultable[f64]
rem_*        : same typed variants and outputs as div_*
neg_i32      : i32 -> Faultable[i32]
neg_i64      : i64 -> Faultable[i64]
neg_f32      : f32 -> f32
neg_f64      : f64 -> f64
abs_*        : same typed variants and outputs as neg_*
sqrt_f32     : f32 -> Faultable[f32]
sqrt_f64     : f64 -> Faultable[f64]
exp_f32      : f32 -> f32
exp_f64      : f64 -> f64
sin_f32      : f32 -> f32
sin_f64      : f64 -> f64
cos_f32      : f32 -> f32
cos_f64      : f64 -> f64
eq_*         : (T, T) -> Bool for i32, i64, f32, f64
lt_*         : (T, T) -> Bool for i32, i64, f32, f64
gt_*         : (T, T) -> Bool for i32, i64, f32, f64
le_*         : (T, T) -> Bool for i32, i64, f32, f64
ge_*         : (T, T) -> Bool for i32, i64, f32, f64
min_*        : (T, T) -> T for i32, i64, f32, f64
max_*        : (T, T) -> T for i32, i64, f32, f64
```

## Semantics

### `add_i32`, `add_i64`, `add_f32`, `add_f64`

Adds two values of the named numeric type. Integer variants return
`Faultable[...]` because fixed-width addition can overflow; floating-point
variants return plain floating-point values. Mixed numeric operands are
rejected by construction because there is no generic `add` export.

- Declared associative for the initial portable profile.
- Identities: `0` for integer reductions, `0.0` for `f64`, and f32 values for `f32`.
- `reduce add_i64(identity: 0)` over `Seq[i64]` returns `Faultable[i64]`.
  `reduce add_f64(identity: 0.0)` over `Seq[f64]` returns `f64`.
- `scan add_i64(identity: 0)` over `Seq[i64]` returns `Seq[Faultable[i64]]`.
- Integer overflow is reported as a recoverable fault value.

### `sub_*`

Subtracts the second numeric value from the first. Both operands must have
the same numeric type.

- Not associative.
- Must not be used with `reduce` or `scan`.
- Integer overflow is reported as a recoverable fault value.

### `mul_*`

Multiplies two numeric values. Both operands must have the same numeric type.

- Not associative in the reduce sense (no identity declared in the
  initial profile; use user-defined wrappers for product reductions).
- Integer overflow is reported as a recoverable fault value.

### `div_*`

Divides the first numeric value by the second. Both operands must have the
same numeric type. `i64` division truncates toward zero.

- Division by zero is reported as `Faultable`.
- Not associative.

### `rem_*`

Returns the remainder of dividing the first numeric value by the second.
Both operands must have the same numeric type. `i64` remainder has the same
sign as the dividend, matching C `%`; `f64` remainder matches C `fmod`.

- Remainder by zero is reported as `Faultable`.
- Not associative.

### `neg_*`

Returns the additive inverse. Integer overflow on `i64::MIN` is reported
as a recoverable fault value.

### `abs_*`

Returns the absolute value. Integer overflow on `i64::MIN` is reported
as a recoverable fault value.

### `sqrt_f32`, `sqrt_f64`

Returns the square root as `Faultable[f32]` or `Faultable[f64]`. Negative
inputs are reported as fault values.

### `eq_*`

Returns whether two numeric values are equal.

### `lt_*`

Returns whether the first numeric value is strictly less than the second.

### `gt_*`

Returns whether the first numeric value is strictly greater than the second.

### `le_*`

Returns whether the first numeric value is less than or equal to the second.

### `ge_*`

Returns whether the first numeric value is greater than or equal to the second.

### `min_*`

Returns the smaller of two numeric values. Both operands must have the same
numeric type.

### `max_*`

Returns the larger of two numeric values. Both operands must have the same
numeric type.

## Examples

```flow
import std.fault { expect }
import std.math { add_f64, eq_i64, gt_i64, mul_f64, rem_i64, sqrt_f64 }

node is_sum_one(x: f64, y: f64, n: i64) -> out: Bool {
    ($x, $y) -> add_f64 -> $sum
    ($n, 1) -> eq_i64 -> $out
}

node is_divisible(n: i64, d: i64) -> out: Bool {
    ($n, $d) -> rem_i64 -> expect -> $r
    ($r, 0) -> eq_i64 -> $out
}

node is_positive(n: i64) -> out: Bool {
    ($n, 0) -> gt_i64 -> $out
}

node hypotenuse(x: f64, y: f64) -> out: f64 {
    ($x, $x) -> mul_f64 -> $xx
    ($y, $y) -> mul_f64 -> $yy
    ($xx, $yy) -> add_f64 -> sqrt_f64 -> expect -> $out
}
```
