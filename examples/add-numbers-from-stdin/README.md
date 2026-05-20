# add-numbers-from-stdin

```text
$ echo "1\n2\n3.5\n" | flowarrow run main.flow
6.5
```

## Why this example matters

It exercises three things at once:

1. **Boundary-only effects.** `program main(input: Bytes) -> output: Bytes`
   is the *entire* interface to the outside world. The graph itself
   is pure; the runtime is responsible for wiring `input` to stdin
   and `output` to stdout.

2. **Dynamic-size sequences.** The number of input lines is not
   known at compile time. `split_lines` produces a `Seq[Bytes]`
   whose length is a runtime value, `filter not_empty` produces
   another such sequence, and `map parse_real` produces `Seq[Real]`
   of the same dynamic length. The *graph shape* remains static —
   only the width of the parallel region varies.

3. **Parallel reduce.** `reduce add(identity: 0.0)` is compiled as a
   balanced parallel-sum tree; `add` is associative, so the
   reduction is legal.

## What it does *not* require

- No loops.
- No mutation.
- No recursion.
- No conditional control flow.
- No statement ordering. The lines in `main.flow` could appear in
  any order and the program would be identical.
