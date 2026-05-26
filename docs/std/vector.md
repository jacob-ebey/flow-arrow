# `std.vector`

`std.vector` is backed by bundled FlowArrow source instead of native
runtime code. Importing it expands its FlowArrow nodes into the program
under compiler-internal names, so the LLVM backend compiles the same
graph structure it would compile for user-defined nodes.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `sum` | `Seq[f64]` | `f64` | Reduces vector items with `std.math.add` |
| `mean` | `Seq[f64]` | `f64` | Divides `sum(values)` by `std.seq.length(values)` |
| `neg` | `Seq[f64]` | `Seq[f64]` | Elementwise numeric negation |
| `abs` | `Seq[f64]` | `Seq[f64]` | Elementwise absolute value |
| `add` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise addition |
| `sub` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise subtraction |
| `mul` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise multiplication |
| `div` | `(Seq[f64],Seq[f64])` | `Seq[f64]` | Pairwise division |
| `add_scalar` | `(Seq[f64],f64)` | `Seq[f64]` | Adds a scalar to each item |
| `sub_scalar` | `(Seq[f64],f64)` | `Seq[f64]` | Subtracts a scalar from each item |
| `scalar_sub` | `(f64,Seq[f64])` | `Seq[f64]` | Subtracts each item from a scalar |
| `mul_scalar` | `(Seq[f64],f64)` | `Seq[f64]` | Multiplies each item by a scalar |
| `scalar_mul` | `(f64,Seq[f64])` | `Seq[f64]` | Multiplies each item by a scalar |
| `div_scalar` | `(Seq[f64],f64)` | `Seq[f64]` | Divides each item by a scalar |
| `scalar_div` | `(f64,Seq[f64])` | `Seq[f64]` | Divides a scalar by each item |
| `equals` | `(Seq[f64],Seq[f64])` | `Bool` | Pairwise equality followed by `std.predicates.all` |
| `dot` | `(Seq[f64],Seq[f64])` | `f64` | Pairwise multiplication followed by `sum` |
| `squared_norm` | `Seq[f64]` | `f64` | Sum of squared values |
| `l1_norm` | `Seq[f64]` | `f64` | Sum of absolute values |
| `norm` | `Seq[f64]` | `f64` | Square root of `squared_norm` |
| `normalize` | `Seq[f64]` | `Seq[f64]` | Divides values by their Euclidean norm |
| `cosine_similarity` | `(Seq[f64],Seq[f64])` | `f64` | Dot product divided by both norms |
| `squared_distance` | `(Seq[f64],Seq[f64])` | `f64` | Sum of squared pairwise differences |
| `distance` | `(Seq[f64],Seq[f64])` | `f64` | Square root of `squared_distance` |

The current module is intentionally `f64`-specific. Shape-specific
types such as `Vec[N, f64]` are syntax-level design targets, but they
are not represented by the current checker yet. Binary vector
operations use `std.seq.zip`, so mismatched lengths propagate the same
runtime fault as `zip`. Scalar operations use `std.seq.broadcast_left`
or `std.seq.broadcast_right`. `mean`, `normalize`, `cosine_similarity`,
and division nodes inherit `std.math.div` division-by-zero behavior.
`norm` and `distance` use `std.math.sqrt`.

Example:

```flow
import std.cli { Args }
import std.math { eq }
import std.vector { add as vector_add, distance, dot, equals, norm, sum }

program main(args: Args) -> exit_code: i64 {
    [1.0, 2.0, 3.0] -> sum -> $total
    ([1.0, 2.0], [3.0, 4.0]) -> vector_add -> $added
    ([1.0, 2.0], [3.0, 4.0]) -> dot -> $dot_total
    [3.0, 4.0] -> norm -> $magnitude
    ([1.0, 2.0], [4.0, 6.0]) -> distance -> $gap
    ($added, [4.0, 6.0]) -> equals -> $add_ok
    ($dot_total, 11.0) -> eq -> $ok
    ($add_ok, $ok, false) -> select -> $all_ok
    ($all_ok, 0, 1) -> select -> $exit_code
}
```
