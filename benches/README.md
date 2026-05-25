# Benchmarks

Benchmarks compare compiled FlowArrow CPU and GPU applications with native
Rust equivalents. They are custom `cargo bench` targets with no external
benchmarking dependency.

The Rust baselines are built as generated dependency-free Cargo projects.
Plain `cargo bench` runs the CPU comparison. GPU samples are opt-in with
`--gpu` or `FLOWARROW_BENCH_GPU=1`; they require a native GPU adapter at
runtime and fail instead of falling back to CPU execution. GPU runs default to
one sample because process startup plus GPU device setup is expensive at these
benchmark sizes; raise it with `--gpu-samples N` once the runtime cost is
acceptable.

## Vector

```sh
cargo bench --bench vector
```

The vector benchmark generates a FlowArrow program that uses
`std.vector` to run dot product, squared distance, and squared norm over
the same vectors for a fixed number of iterations. It also generates and
compiles a Rust release executable with equivalent vector loops for the
native baseline. It always builds the Rust executable and FlowArrow CPU
executable. With `--gpu`, it also builds a FlowArrow GPU executable. It then
samples:

- the compiled Rust executable
- the compiled FlowArrow CPU executable
- the compiled FlowArrow GPU executable, when enabled

All samples are process executions, so process startup and GPU runtime/device
setup are included. Increase `--iterations` or vector length when you want
startup overhead to matter less.

Options:

```sh
cargo bench --bench vector -- --gpu --gpu-samples 3 --len 1024 --iterations 250 --samples 20
```

Environment equivalents:

```sh
FLOWARROW_BENCH_VECTOR_LEN=1024 \
FLOWARROW_BENCH_ITERATIONS=250 \
FLOWARROW_BENCH_SAMPLES=20 \
FLOWARROW_BENCH_GPU=1 \
FLOWARROW_BENCH_GPU_SAMPLES=3 \
cargo bench --bench vector
```

## Matrix

```sh
cargo bench --bench matrix
```

The matrix benchmark generates a FlowArrow program that uses
`std.matrix` for matrix multiplication, matrix-vector multiplication,
and row reductions over fixed matrices for a fixed number of iterations.
It also generates and compiles a Rust release executable that uses
equivalent matrix and vector loops for the native baseline. Like the vector
benchmark, pass `--gpu` to include the FlowArrow GPU executable and GPU/CPU
ratios.

Options:

```sh
cargo bench --bench matrix -- --gpu --gpu-samples 3 --rows 32 --inner 32 --cols 32 --iterations 100 --samples 20
```

Environment equivalents:

```sh
FLOWARROW_BENCH_MATRIX_ROWS=32 \
FLOWARROW_BENCH_MATRIX_INNER=32 \
FLOWARROW_BENCH_MATRIX_COLS=32 \
FLOWARROW_BENCH_ITERATIONS=100 \
FLOWARROW_BENCH_SAMPLES=20 \
FLOWARROW_BENCH_GPU=1 \
FLOWARROW_BENCH_GPU_SAMPLES=3 \
cargo bench --bench matrix
```

## GPU Accumulator

```sh
cargo bench --bench gpu_accumulator -- --gpu
```

The GPU accumulator benchmark is intentionally shaped to favor the GPU backend.
It repeats a pure vector scoring kernel many times over immutable vectors. The
CPU executable performs the full vector work for every repeat iteration; the
GPU executable lowers the repeat accumulator to one generated WGSL reduction
program and applies the repeat count to the scalar score.

Options:

```sh
cargo bench --bench gpu_accumulator -- --gpu --len 4096 --iterations 20000 --samples 3 --gpu-samples 3
```

Environment equivalents:

```sh
FLOWARROW_BENCH_GPU_ACCUMULATOR_LEN=4096 \
FLOWARROW_BENCH_ITERATIONS=20000 \
FLOWARROW_BENCH_GPU=1 \
FLOWARROW_BENCH_GPU_SAMPLES=3 \
cargo bench --bench gpu_accumulator
```
