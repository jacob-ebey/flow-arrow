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
It uses `f64` vectors, and FlowArrow does not lower `f64` source data into
`f32` GPU buffers. The GPU path uses WGSL `f64` storage and arithmetic, so it
requires a native/WebGPU device with shader-f64 support and fails explicitly
when that feature is unavailable. Current browser WebGPU and macOS/Metal wgpu
adapters may not expose shader-f64 even on recent Apple Silicon hardware.

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
