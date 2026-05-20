# `std.vector`

`std.vector` is backed by bundled FlowArrow source instead of native
runtime code. Importing it expands its FlowArrow nodes into the program
under compiler-internal names, so the LLVM/C backend compiles the same
graph structure it would compile for user-defined nodes.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `sum` | `Seq[Real]` | `Real` | Reduces vector items with `std.math.add` |
| `dot` | `(Seq[Real],Seq[Real])` | `Real` | Zips two vectors, multiplies each pair, then sums |

Example:

```flow
import std.cli { Args }
import std.math { eq }
import std.vector { dot, sum }

program main(args: Args) -> exit_code: Int {
    [1.0, 2.0, 3.0] -> sum -> $total
    ([1.0, 2.0], [3.0, 4.0]) -> dot -> $dot_total
    ($dot_total, 11.0) -> eq -> $ok
    ($ok, 0, 1) -> select -> $exit_code
}
```
