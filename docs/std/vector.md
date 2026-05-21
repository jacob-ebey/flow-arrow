# `std.vector`

`std.vector` is backed by bundled FlowArrow source instead of native
runtime code. Importing it expands its FlowArrow nodes into the program
under compiler-internal names, so the LLVM/C backend compiles the same
graph structure it would compile for user-defined nodes.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `sum` | `Seq[Real]` | `Real` | Reduces vector items with `std.math.add` |
| `mean` | `Seq[Real]` | `Real` | Divides `sum(values)` by `std.seq.length(values)` |
| `neg` | `Seq[Real]` | `Seq[Real]` | Elementwise numeric negation |
| `abs` | `Seq[Real]` | `Seq[Real]` | Elementwise absolute value |
| `add` | `(Seq[Real],Seq[Real])` | `Seq[Real]` | Pairwise addition |
| `sub` | `(Seq[Real],Seq[Real])` | `Seq[Real]` | Pairwise subtraction |
| `mul` | `(Seq[Real],Seq[Real])` | `Seq[Real]` | Pairwise multiplication |
| `div` | `(Seq[Real],Seq[Real])` | `Seq[Real]` | Pairwise division |
| `add_scalar` | `(Seq[Real],Real)` | `Seq[Real]` | Adds a scalar to each item |
| `sub_scalar` | `(Seq[Real],Real)` | `Seq[Real]` | Subtracts a scalar from each item |
| `scalar_sub` | `(Real,Seq[Real])` | `Seq[Real]` | Subtracts each item from a scalar |
| `mul_scalar` | `(Seq[Real],Real)` | `Seq[Real]` | Multiplies each item by a scalar |
| `scalar_mul` | `(Real,Seq[Real])` | `Seq[Real]` | Multiplies each item by a scalar |
| `div_scalar` | `(Seq[Real],Real)` | `Seq[Real]` | Divides each item by a scalar |
| `scalar_div` | `(Real,Seq[Real])` | `Seq[Real]` | Divides a scalar by each item |
| `equals` | `(Seq[Real],Seq[Real])` | `Bool` | Pairwise equality followed by `std.predicates.all` |
| `dot` | `(Seq[Real],Seq[Real])` | `Real` | Pairwise multiplication followed by `sum` |
| `squared_norm` | `Seq[Real]` | `Real` | Sum of squared values |
| `l1_norm` | `Seq[Real]` | `Real` | Sum of absolute values |
| `norm` | `Seq[Real]` | `Real` | Square root of `squared_norm` |
| `normalize` | `Seq[Real]` | `Seq[Real]` | Divides values by their Euclidean norm |
| `cosine_similarity` | `(Seq[Real],Seq[Real])` | `Real` | Dot product divided by both norms |
| `squared_distance` | `(Seq[Real],Seq[Real])` | `Real` | Sum of squared pairwise differences |
| `distance` | `(Seq[Real],Seq[Real])` | `Real` | Square root of `squared_distance` |

The current module is intentionally `Real`-specific. Shape-specific
types such as `Vec[N, Real]` are syntax-level design targets, but they
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

program main(args: Args) -> exit_code: Int {
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
