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
the same vectors for a fixed number of iterations. It builds that
program once, then samples:

- native Rust code that performs the equivalent scalar loops in-process
- the compiled FlowArrow executable

The compiled executable is run as a process, so the FlowArrow number
includes process startup. Increase `--iterations` or vector length when
you want startup overhead to matter less.

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
