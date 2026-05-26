# `std.vector`

`std.vector` is backed by bundled FlowArrow source instead of native
runtime code. Importing it expands its FlowArrow nodes into the program
under compiler-internal names, so the LLVM backend compiles the same
graph structure it would compile for user-defined nodes.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `sum_f64` | `Seq[f64]` | `f64` | Reduces vector items with `std.math.add_f64` |
| `mean_f64` | `Seq[f64]` | `f64` | Divides `sum_f64(values)` by `std.seq.length(values)` |
| `min_f64` | `Seq[f64]` | `f64` | Minimum item; requires a non-empty vector |
| `max_f64` | `Seq[f64]` | `f64` | Maximum item; requires a non-empty vector |
| `neg_f64` | `Seq[f64]` | `Seq[f64]` | Elementwise numeric negation |
| `abs_f64` | `Seq[f64]` | `Seq[f64]` | Elementwise absolute value |
| `exp_f64` | `Seq[f64]` | `Seq[f64]` | Elementwise exponent |
| `add_f64` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise addition |
| `sub_f64` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise subtraction |
| `mul_f64` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise multiplication |
| `div_f64` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise division |
| `add_scalar_f64` | `(Seq[f64],f64)` | `Seq[f64]` | Adds a scalar to each item |
| `sub_scalar_f64` | `(Seq[f64],f64)` | `Seq[f64]` | Subtracts a scalar from each item |
| `scalar_sub_f64` | `(f64,Seq[f64])` | `Seq[f64]` | Subtracts each item from a scalar |
| `mul_scalar_f64` | `(Seq[f64],f64)` | `Seq[f64]` | Multiplies each item by a scalar |
| `scalar_mul_f64` | `(f64,Seq[f64])` | `Seq[f64]` | Multiplies each item by a scalar |
| `div_scalar_f64` | `(Seq[f64],f64)` | `Seq[f64]` | Divides each item by a scalar |
| `scalar_div_f64` | `(f64,Seq[f64])` | `Seq[f64]` | Divides a scalar by each item |
| `equals_f64` | `(Seq[f64],Seq[f64])` | `Bool` | Pairwise equality followed by `std.predicates.all` |
| `dot_f64` | `(Seq[f64],Seq[f64])` | `f64` | Pairwise multiplication followed by `sum_f64` |
| `squared_norm_f64` | `Seq[f64]` | `f64` | Sum of squared values |
| `l1_norm_f64` | `Seq[f64]` | `f64` | Sum of absolute values |
| `norm_f64` | `Seq[f64]` | `f64` | Square root of `squared_norm_f64` |
| `normalize_f64` | `Seq[f64]` | `Seq[f64]` | Divides values by their Euclidean norm |
| `relu_f64` | `Seq[f64]` | `Seq[f64]` | Elementwise rectified linear unit |
| `sigmoid_f64` | `Seq[f64]` | `Seq[f64]` | Elementwise logistic activation |
| `silu_f64` | `Seq[f64]` | `Seq[f64]` | Elementwise SiLU activation |
| `softmax_f64` | `Seq[f64]` | `Seq[f64]` | Stable vector softmax |
| `cosine_similarity_f64` | `(Seq[f64],Seq[f64])` | `f64` | Dot product divided by both norms |
| `squared_distance_f64` | `(Seq[f64],Seq[f64])` | `f64` | Sum of squared pairwise differences |
| `distance_f64` | `(Seq[f64],Seq[f64])` | `f64` | Square root of `squared_distance_f64` |
| `sum_f32` | `Seq[f32]` | `f32` | Reduces vector items with `std.math.add_f32` |
| `mean_f32` | `Seq[f32]` | `f32` | Divides `sum_f32(values)` by `std.seq.length(values)` |
| `min_f32` | `Seq[f32]` | `f32` | Minimum item; requires a non-empty vector |
| `max_f32` | `Seq[f32]` | `f32` | Maximum item; requires a non-empty vector |
| `neg_f32` | `Seq[f32]` | `Seq[f32]` | Elementwise numeric negation |
| `abs_f32` | `Seq[f32]` | `Seq[f32]` | Elementwise absolute value |
| `exp_f32` | `Seq[f32]` | `Seq[f32]` | Elementwise exponent |
| `add_f32` | `(Seq[f32],Seq[f32])` | `Seq[f32]` | Pairwise addition |
| `sub_f32` | `(Seq[f32],Seq[f32])` | `Seq[f32]` | Pairwise subtraction |
| `mul_f32` | `(Seq[f32],Seq[f32])` | `Seq[f32]` | Pairwise multiplication |
| `div_f32` | `(Seq[f32],Seq[f32])` | `Seq[f32]` | Pairwise division |
| `add_scalar_f32` | `(Seq[f32],f32)` | `Seq[f32]` | Adds a scalar to each item |
| `sub_scalar_f32` | `(Seq[f32],f32)` | `Seq[f32]` | Subtracts a scalar from each item |
| `scalar_sub_f32` | `(f32,Seq[f32])` | `Seq[f32]` | Subtracts each item from a scalar |
| `mul_scalar_f32` | `(Seq[f32],f32)` | `Seq[f32]` | Multiplies each item by a scalar |
| `scalar_mul_f32` | `(f32,Seq[f32])` | `Seq[f32]` | Multiplies each item by a scalar |
| `div_scalar_f32` | `(Seq[f32],f32)` | `Seq[f32]` | Divides each item by a scalar |
| `scalar_div_f32` | `(f32,Seq[f32])` | `Seq[f32]` | Divides a scalar by each item |
| `equals_f32` | `(Seq[f32],Seq[f32])` | `Bool` | Pairwise equality followed by `std.predicates.all` |
| `dot_f32` | `(Seq[f32],Seq[f32])` | `f32` | Pairwise multiplication followed by `sum_f32` |
| `squared_norm_f32` | `Seq[f32]` | `f32` | Sum of squared values |
| `l1_norm_f32` | `Seq[f32]` | `f32` | Sum of absolute values |
| `norm_f32` | `Seq[f32]` | `f32` | Square root of `squared_norm_f32` |
| `normalize_f32` | `Seq[f32]` | `Seq[f32]` | Divides values by their Euclidean norm |
| `relu_f32` | `Seq[f32]` | `Seq[f32]` | Elementwise rectified linear unit |
| `sigmoid_f32` | `Seq[f32]` | `Seq[f32]` | Elementwise logistic activation |
| `silu_f32` | `Seq[f32]` | `Seq[f32]` | Elementwise SiLU activation |
| `softmax_f32` | `Seq[f32]` | `Seq[f32]` | Stable vector softmax |
| `cosine_similarity_f32` | `(Seq[f32],Seq[f32])` | `f32` | Dot product divided by both norms |
| `squared_distance_f32` | `(Seq[f32],Seq[f32])` | `f32` | Sum of squared pairwise differences |
| `distance_f32` | `(Seq[f32],Seq[f32])` | `f32` | Square root of `squared_distance_f32` |

There are no unsuffixed numeric exports. Use the explicit `_f32` or `_f64`
variant so the selected numeric width is visible at the call site.
Shape-specific types such as `Vec[N, f64]` are syntax-level design
targets, but they are not represented by the current checker yet.
Binary vector operations use `std.seq.zip`, so mismatched lengths
propagate the same runtime fault as `zip`. Scalar operations use
`std.seq.broadcast_left` or `std.seq.broadcast_right`. Mean, normalization,
cosine similarity, and division nodes inherit the matching `std.math.div_f32`
or `std.math.div_f64` division-by-zero behavior. Norm and distance nodes use
the matching `std.math.sqrt_f32` or `std.math.sqrt_f64`.

When the TypeScript or JavaScript backend is built with `--gpu`, the compiler
lowers vector unary operations, pairwise operations, scalar broadcasts,
`sum_*`/`min_*`/`max_*`, `dot_*`, `squared_norm_*`, and `softmax_*` to
generated WebGPU runtime kernels for both `f32` and `f64` variants. Those
kernels preserve the declared numeric width; `f64` variants still require
device `SHADER_F64` support.

Example:

```flow
import std.cli { Args }
import std.math { eq_f64 as eq }
import std.vector {
    add_f64 as vector_add,
    distance_f64,
    dot_f64,
    equals_f64,
    norm_f64,
    sum_f64,
}

program main(args: Args) -> exit_code: i64 {
    [1.0, 2.0, 3.0] -> sum_f64 -> $total
    ([1.0, 2.0], [3.0, 4.0]) -> vector_add -> $added
    ([1.0, 2.0], [3.0, 4.0]) -> dot_f64 -> $dot_total
    [3.0, 4.0] -> norm_f64 -> $magnitude
    ([1.0, 2.0], [4.0, 6.0]) -> distance_f64 -> $gap
    ($added, [4.0, 6.0]) -> equals_f64 -> $add_ok
    ($dot_total, 11.0) -> eq -> $ok
    ($add_ok, $ok, false) -> select -> $all_ok
    ($all_ok, 0, 1) -> select -> $exit_code
}
```
