# GPU accumulator benchmark

This example mirrors the `gpu_accumulator` benchmark as a normal FlowArrow
program. It builds two deterministic vectors, repeats a pure vector scoring
kernel many times, and prints the final scalar score.

Run it on the GPU:

```sh
cargo run -- run --gpu examples/gpu-accumulator-benchmark/main.flow
```

Run it on the CPU:

```sh
cargo run -- run examples/gpu-accumulator-benchmark/main.flow
```

The workload is intentionally shaped for the native GPU backend: immutable
vectors stay at the compute boundary, the repeated kernel is pure, and only the
final scalar crosses back to host I/O for printing.
