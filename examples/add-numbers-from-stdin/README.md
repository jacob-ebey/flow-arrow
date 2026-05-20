# add-numbers-from-stdin

```text
$ echo "1\n2\n3.5\n" | flowarrow run main.flow
6.5
```

## Why this example matters

It exercises three things at once:

1. **Boundary-only effects.** `program main(args: Args) -> exit_code: Int`
   is the process entry shape. Stdin and stdout are explicit
   `std.io` boundary nodes (`read_stdin`, `write_stdout`) inside the
   program body, and the returned value is the process exit code.

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
