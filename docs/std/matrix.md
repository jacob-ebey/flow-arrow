# `std.matrix`

`std.matrix` is a source-backed matrix library built on `Seq[Seq[f32]]` and
`Seq[Seq[f64]]`. Rows are the outer sequence and columns are positions inside
each row.

| Export families | Input | Output | Description |
| --- | --- | --- | --- |
| `rows_f32`, `rows_f64` | `Seq[Seq[T]]` | `i64` | Outer sequence length |
| `cols_f32`, `cols_f64` | `Seq[Seq[T]]` | `i64` | First row length, or `0` for an empty matrix |
| `flatten_f32`, `flatten_f64` | `Seq[Seq[T]]` | `Seq[T]` | Row-major flattening |
| `transpose_f32`, `transpose_f64` | `Seq[Seq[T]]` | `Seq[Seq[T]]` | Row/column transpose |
| `neg_f32`, `neg_f64`, `abs_f32`, `abs_f64` | `Seq[Seq[T]]` | `Seq[Seq[T]]` | Elementwise unary operations |
| `add_f32`, `add_f64`, `sub_f32`, `sub_f64`, `mul_f32`, `mul_f64`, `div_f32`, `div_f64` | `(Seq[Seq[T]],Seq[Seq[T]])` | `Seq[Seq[T]]` | Elementwise binary operations |
| `add_scalar_f32`, `add_scalar_f64`, `sub_scalar_f32`, `sub_scalar_f64`, `mul_scalar_f32`, `mul_scalar_f64`, `div_scalar_f32`, `div_scalar_f64` | `(Seq[Seq[T]],T)` | `Seq[Seq[T]]` | Matrix-scalar operations |
| `scalar_sub_f32`, `scalar_sub_f64`, `scalar_mul_f32`, `scalar_mul_f64`, `scalar_div_f32`, `scalar_div_f64` | `(T,Seq[Seq[T]])` | `Seq[Seq[T]]` | Scalar-matrix operations |
| `add_row_f32`, `add_row_f64`, `sub_row_f32`, `sub_row_f64`, `mul_row_f32`, `mul_row_f64`, `div_row_f32`, `div_row_f64` | `(Seq[Seq[T]],Seq[T])` | `Seq[Seq[T]]` | Broadcast a row vector across rows |
| `equals_f32`, `equals_f64` | `(Seq[Seq[T]],Seq[Seq[T]])` | `Bool` | Rowwise equality followed by `all` |
| `sum_f32`, `sum_f64`, `mean_f32`, `mean_f64` | `Seq[Seq[T]]` | `T` | Matrix-wide reductions |
| `row_sums_f32`, `row_sums_f64`, `row_means_f32`, `row_means_f64`, `row_norms_f32`, `row_norms_f64`, `row_mean_squares_f32`, `row_mean_squares_f64`, `row_rms_f32`, `row_rms_f64` | `Seq[Seq[T]]` or `(Seq[Seq[T]],T)` for RMS | `Seq[T]` | Per-row reductions |
| `column_sums_f32`, `column_sums_f64`, `column_means_f32`, `column_means_f64`, `column_norms_f32`, `column_norms_f64`, `column_mean_squares_f32`, `column_mean_squares_f64`, `column_rms_f32`, `column_rms_f64` | `Seq[Seq[T]]` or `(Seq[Seq[T]],T)` for RMS | `Seq[T]` | Per-column reductions via transpose |
| `squared_norm_f32`, `squared_norm_f64`, `l1_norm_f32`, `l1_norm_f64`, `norm_f32`, `norm_f64`, `frobenius_norm_f32`, `frobenius_norm_f64` | `Seq[Seq[T]]` | `T` | Matrix norms |
| `normalize_rows_f32`, `normalize_rows_f64` | `Seq[Seq[T]]` | `Seq[Seq[T]]` | Normalize each row by its Euclidean norm |
| `row_rms_norm_f32`, `row_rms_norm_f64` | `(Seq[Seq[T]],Seq[T],T)` | `Seq[Seq[T]]` | RMS-normalize each row and multiply by weights |
| `row_softmax_f32`, `row_softmax_f64` | `Seq[Seq[T]]` | `Seq[Seq[T]]` | Softmax each row independently |
| `row_swiglu_f32`, `row_swiglu_f64` | `(Seq[Seq[T]],Seq[Seq[T]])` | `Seq[Seq[T]]` | Row-wise `silu(gate) * up` activation |
| `squared_distance_f32`, `squared_distance_f64`, `distance_f32`, `distance_f64` | `(Seq[Seq[T]],Seq[Seq[T]])` | `T` | Frobenius distance |
| `matvec_f32`, `matvec_f64` | `(Seq[Seq[T]],Seq[T])` | `Seq[T]` | Matrix-vector multiplication |
| `vecmat_f32`, `vecmat_f64` | `(Seq[T],Seq[Seq[T]])` | `Seq[T]` | Vector-matrix multiplication |
| `matmul_f32`, `matmul_f64` | `(Seq[Seq[T]],Seq[Seq[T]])` | `Seq[Seq[T]]` | Matrix multiplication |
| `outer_f32`, `outer_f64` | `(Seq[T],Seq[T])` | `Seq[Seq[T]]` | Outer product |
| `gram_f32`, `gram_f64` | `Seq[Seq[T]]` | `Seq[Seq[T]]` | `matrix * transpose(matrix)` |

`T` is either `f32` or `f64`, selected by the suffix. There are no unsuffixed
numeric exports.

Binary row and matrix operations use `std.seq.zip`, so mismatched row
counts or row lengths propagate the same runtime fault as `zip`.
`transpose` checks that every row has the same length. Division and
normalization inherit the matching `std.math.div_f32` or `std.math.div_f64`
division-by-zero behavior.

When the TypeScript or JavaScript backend is built with `--gpu`, the compiler
lowers elementwise matrix operations, matrix-scalar operations, row broadcasts,
`matvec_*`, `vecmat_*`, `matmul_*`, `outer_*`, and `row_softmax_*` to generated
WebGPU runtime kernels for both `f32` and `f64`. These lowerings preserve the
declared numeric width; `f64` kernels require device `SHADER_F64` support.

Example:

```flow
import std.cli { Args }
import std.vector { equals_f64 as vector_equals }
import std.matrix { equals_f64 as matrix_equals, matmul_f64, matvec_f64, row_sums_f64 }

program main(args: Args) -> exit_code: i64 {
    [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]] -> $a
    ($a, [1.0, 0.0, 1.0]) -> matvec_f64 -> $mv
    ($mv, [4.0, 10.0]) -> vector_equals -> $mv_ok

    ($a, [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]) -> matmul_f64 -> $mm
    ($mm, [[22.0, 28.0], [49.0, 64.0]]) -> matrix_equals -> $mm_ok

    $a -> row_sums_f64 -> $sums
    ($sums, [6.0, 15.0]) -> vector_equals -> $sum_ok

    ($mv_ok, $mm_ok, false) -> select -> $s1
    ($s1, $sum_ok, false) -> select -> $all_ok
    ($all_ok, 0, 1) -> select -> $exit_code
}
```
