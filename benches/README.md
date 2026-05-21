# Benchmarks

Benchmarks compare compiled FlowArrow applications with native Rust
equivalents. They are custom `cargo bench` targets with no external
benchmarking dependency.

## Vector

```sh
cargo bench --bench vector
```

The vector benchmark generates a FlowArrow program that uses
`std.vector` to run dot product, squared distance, and squared norm over
the same vectors for a fixed number of iterations. It also generates and
compiles an equivalent Rust program with `rustc -C opt-level=3`. It
builds both programs once, then samples:

- the compiled Rust executable
- the compiled FlowArrow executable

Both samples are process executions, so process startup is included on
both sides. Increase `--iterations` or vector length when you want
startup overhead to matter less.

Options:

```sh
cargo bench --bench vector -- --len 1024 --iterations 250 --samples 20
```

Environment equivalents:

```sh
FLOWARROW_BENCH_VECTOR_LEN=1024 \
FLOWARROW_BENCH_ITERATIONS=250 \
FLOWARROW_BENCH_SAMPLES=20 \
cargo bench --bench vector
```

## Matrix

```sh
cargo bench --bench matrix
```

The matrix benchmark generates a FlowArrow program that uses
`std.matrix` for matrix multiplication, matrix-vector multiplication,
and row reductions over fixed matrices for a fixed number of iterations.
It also generates and compiles an equivalent Rust program with
`rustc -C opt-level=3`.

Options:

```sh
cargo bench --bench matrix -- --rows 32 --inner 32 --cols 32 --iterations 100 --samples 20
```

Environment equivalents:

```sh
FLOWARROW_BENCH_MATRIX_ROWS=32 \
FLOWARROW_BENCH_MATRIX_INNER=32 \
FLOWARROW_BENCH_MATRIX_COLS=32 \
FLOWARROW_BENCH_ITERATIONS=100 \
FLOWARROW_BENCH_SAMPLES=20 \
cargo bench --bench matrix
```
