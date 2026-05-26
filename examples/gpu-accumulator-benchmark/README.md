# GPU accumulator benchmark

This example mirrors the `gpu_accumulator` benchmark as a normal FlowArrow
program. It builds two deterministic vectors, repeats a pure vector scoring
kernel many times, and prints the final scalar score.

Run with GPU support enabled:

```sh
cargo run -- run --gpu examples/gpu-accumulator-benchmark/main.flow
```

Run it on the CPU:

```sh
cargo run -- run examples/gpu-accumulator-benchmark/main.flow
```

The workload is intentionally shaped like a GPU-resident repeated accumulator.
It uses `f32` vectors and a generated WGSL `f32` repeat accumulator, so the GPU
path stays in the source numeric domain without lowering from or to another
float width. The scoring kernel uses the `std.vector` f32 exports for dot
product, squared distance, and squared norm. The optimized GPU path computes
the repeated accumulator as one GPU reduction plus one `f32` multiply by the
iteration count, instead of dispatching the scoring kernel once per iteration.

## Browser validation

Build the JavaScript artifact with GPU support enabled:

```sh
cargo run -- build --target javascript --crate-type cdylib --gpu examples/gpu-accumulator-benchmark/lib.flow
```

Serve this directory and open `index.html`:

```sh
cd examples/gpu-accumulator-benchmark
npx serve
```

The page imports `build/javascript/lib.mjs`, calls the exported
`run_gpu_accumulator` node, and compares the score with the expected value.
