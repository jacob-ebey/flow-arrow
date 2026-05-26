# `std.matrix`

`std.matrix` is a source-backed matrix library built on `Seq[Seq[f64]]`.
Rows are the outer sequence and columns are positions inside each row.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `rows` | `Seq[Seq[f64]]` | `i64` | Outer sequence length |
| `cols` | `Seq[Seq[f64]]` | `i64` | First row length, or `0` for an empty matrix |
| `flatten` | `Seq[Seq[f64]]` | `Seq[f64]` | Row-major flattening |
| `transpose` | `Seq[Seq[f64]]` | `Seq[Seq[f64]]` | Row/column transpose |
| `neg`, `abs` | `Seq[Seq[f64]]` | `Seq[Seq[f64]]` | Elementwise unary operations |
| `add`, `sub`, `mul`, `div` | `(Seq[Seq[f64]],Seq[Seq[f64]])` | `Seq[Seq[f64]]` | Elementwise binary operations |
| `add_scalar`, `sub_scalar`, `mul_scalar`, `div_scalar` | `(Seq[Seq[f64]],f64)` | `Seq[Seq[f64]]` | Matrix-scalar operations |
| `scalar_sub`, `scalar_mul`, `scalar_div` | `(f64,Seq[Seq[f64]])` | `Seq[Seq[f64]]` | Scalar-matrix operations |
| `add_row`, `sub_row`, `mul_row`, `div_row` | `(Seq[Seq[f64]],Seq[f64])` | `Seq[Seq[f64]]` | Broadcast a row vector across rows |
| `equals` | `(Seq[Seq[f64]],Seq[Seq[f64]])` | `Bool` | Rowwise equality followed by `all` |
| `sum`, `mean` | `Seq[Seq[f64]]` | `f64` | Matrix-wide reductions |
| `row_sums`, `row_means`, `row_norms` | `Seq[Seq[f64]]` | `Seq[f64]` | Per-row reductions |
| `column_sums`, `column_means`, `column_norms` | `Seq[Seq[f64]]` | `Seq[f64]` | Per-column reductions via transpose |
| `squared_norm`, `l1_norm`, `norm`, `frobenius_norm` | `Seq[Seq[f64]]` | `f64` | Matrix norms |
| `normalize_rows` | `Seq[Seq[f64]]` | `Seq[Seq[f64]]` | Normalize each row by its Euclidean norm |
| `squared_distance`, `distance` | `(Seq[Seq[f64]],Seq[Seq[f64]])` | `f64` | Frobenius distance |
| `matvec` | `(Seq[Seq[f64]],Seq[f64])` | `Seq[f64]` | Matrix-vector multiplication |
| `vecmat` | `(Seq[f64],Seq[Seq[f64]])` | `Seq[f64]` | Vector-matrix multiplication |
| `matmul` | `(Seq[Seq[f64]],Seq[Seq[f64]])` | `Seq[Seq[f64]]` | Matrix multiplication |
| `outer` | `(Seq[f64],Seq[f64])` | `Seq[Seq[f64]]` | Outer product |
| `gram` | `Seq[Seq[f64]]` | `Seq[Seq[f64]]` | `matrix * transpose(matrix)` |

Binary row and matrix operations use `std.seq.zip`, so mismatched row
counts or row lengths propagate the same runtime fault as `zip`.
`transpose` checks that every row has the same length. Division and
normalization inherit `std.math.div_f64` division-by-zero behavior.

Example:

```flow
import std.cli { Args }
import std.vector { equals as vector_equals }
import std.matrix { matmul, matvec, row_sums, equals as matrix_equals }

program main(args: Args) -> exit_code: i64 {
    [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]] -> $a
    ($a, [1.0, 0.0, 1.0]) -> matvec -> $mv
    ($mv, [4.0, 10.0]) -> vector_equals -> $mv_ok

    ($a, [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]) -> matmul -> $mm
    ($mm, [[22.0, 28.0], [49.0, 64.0]]) -> matrix_equals -> $mm_ok

    $a -> row_sums -> $sums
    ($sums, [6.0, 15.0]) -> vector_equals -> $sum_ok

    ($mv_ok, $mm_ok, false) -> select -> $s1
    ($s1, $sum_ok, false) -> select -> $all_ok
    ($all_ok, 0, 1) -> select -> $exit_code
}
```
