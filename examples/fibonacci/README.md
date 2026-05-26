# fibonacci

Reads a Fibonacci depth from stdin and prints the last Fibonacci number
at that depth.

```text
$ echo "12" | flowarrow run main.flow
89
```

## Why this example matters

It shows a small input-driven sequence pipeline:

1. **Boundary input.** `read_stdin` reads the requested depth and
   `parse_int` converts it to a `Faultable[i64]`; this example leaves
   parse faults unhandled, so `main` is typed as `Faultable[i64]`.
2. **FlowArrow Fibonacci algorithm.** `fib_result` is a FlowArrow node
   that starts from `(1, 0)`, uses `repeat<$depth> fib_step` to advance
   the pair, and projects the final first element.
3. **Single result output.** The final `i64` is formatted, a newline is
   appended, and stdout receives one line.

## What it does *not* require

- No loops.
- No mutation.
- No recursion.
- No conditional control flow.
