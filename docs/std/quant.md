# `std.quant`

`std.quant` provides source-backed types and helpers for quantized inference
pipelines. The first target is Q4_K/Q4_K_M-style weight storage: typed
metadata, packed byte buffers, and nibble helpers that can be consumed by
future dequantization and fused matrix-vector kernels.

| Export | Type |
| --- | --- |
| `TokenId` | `i64` |
| `Position` | `i64` |
| `F32Vector` | `Seq[f32]` |
| `F32Matrix` | `Seq[Seq[f32]]` |
| `Logits` | `Seq[f32]` |
| `Activation` | `Seq[f32]` |
| `Q4KBlock` | `(f32,f32,Bytes,Bytes)` |
| `Q4KMatrix` | `(i64,i64,Seq[Q4KBlock])` |
| `Q4KMWeightMatrix` | `Q4KMatrix` |
| `KVCache` | `(Seq[Seq[f32]],Seq[Seq[f32]])` |

`Q4KBlock` stores `(delta, min_delta, scales, quants)`. The byte buffers keep
the packed quantized representation opaque until a backend-specific optimized
kernel consumes it. `Q4KMatrix` and `Q4KMWeightMatrix` store
`(rows, cols, blocks)`.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `q4_k_block_size` | `()` | `i64` | Number of logical values in one Q4_K block |
| `q4_k_subblock_size` | `()` | `i64` | Logical values in one Q4_K sub-block |
| `q4_k_subblocks` | `()` | `i64` | Number of sub-blocks in one Q4_K block |
| `q4_k_scale_bytes` | `()` | `i64` | Packed scale/min metadata bytes per block |
| `q4_k_quant_bytes` | `()` | `i64` | Packed quantized payload bytes per block |
| `q4_k_block` | `(f32,f32,Bytes,Bytes)` | `Q4KBlock` | Build a block value |
| `q4_k_block_delta` | `Q4KBlock` | `f32` | Extract block scale delta |
| `q4_k_block_min_delta` | `Q4KBlock` | `f32` | Extract block min delta |
| `q4_k_block_scales` | `Q4KBlock` | `Bytes` | Extract packed scale/min bytes |
| `q4_k_block_quants` | `Q4KBlock` | `Bytes` | Extract packed 4-bit weight bytes |
| `q4_k_matrix` | `(i64,i64,Seq[Q4KBlock])` | `Q4KMatrix` | Build a matrix |
| `q4_k_matrix_rows`, `q4_k_m_weight_rows` | `Q4KMatrix` | `i64` | Extract row count |
| `q4_k_matrix_cols`, `q4_k_m_weight_cols` | `Q4KMatrix` | `i64` | Extract column count |
| `q4_k_matrix_blocks`, `q4_k_m_weight_blocks` | `Q4KMatrix` | `Seq[Q4KBlock]` | Extract blocks |
| `q4_k_m_weight_matrix` | `(i64,i64,Seq[Q4KBlock])` | `Q4KMWeightMatrix` | Build a Q4_K_M weight matrix |
| `low_nibble` | `i64` | `i64` | Extract low packed 4-bit value |
| `high_nibble` | `i64` | `i64` | Extract high packed 4-bit value |

Example:

```flow
import std.cli { Args }
import std.quant {
    Q4KMWeightMatrix,
    q4_k_block,
    q4_k_m_weight_matrix,
    q4_k_m_weight_rows,
}

node keep(matrix: Q4KMWeightMatrix) -> out: Q4KMWeightMatrix {
    $matrix -> $out
}

program main(args: Args) -> exit_code: i64 {
    1.0f32 -> $delta
    0.0f32 -> $min_delta
    ($delta, $min_delta, "abcdefghijkl", "quantized payload") -> q4_k_block -> $block
    (1, 256, [$block]) -> q4_k_m_weight_matrix -> keep -> $weights
    $weights -> q4_k_m_weight_rows -> $exit_code
}
```
