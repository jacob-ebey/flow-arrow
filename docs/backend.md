# FlowArrow — Compiler Backend & Interop Decisions

This document records the architectural decisions for the FlowArrow
compiler implementation. It is the reference for all backend, ABI,
runtime, and tooling work.

Status: **living document**. Updated in place as implementation
progresses and decisions are revised.

---

## 1. Goals (recap)

The compiler must support:

1. **Native binaries** for common targets (x86_64, aarch64; Linux,
   macOS, Windows).
2. **WebAssembly binaries** for the web (`wasm32-unknown-unknown`)
   and for server-side WASM (`wasm32-wasi`).
3. **Library builds**: static archives, shared libraries, and WASM
   modules consumable from other languages.
4. **Binary application builds**: standalone executables.
5. **Interop** with Rust, Go, C, C++, JavaScript/TypeScript, and any
   other language that speaks the C ABI or WASM.

The language itself (see [`syntax.md`](./syntax.md), [`overview.md`](./overview.md))
is a pure dataflow DAG with no mutation, no hidden control flow, and
explicit parallelism. The backend must preserve and exploit that
structure.

---

## 2. Decisions

### 2.1 Implementation language: **Rust**

The compiler, runtime, and surrounding tooling are written in **Rust**.

**Rationale:**

| Factor                | Rust contribution                                              |
| --------------------- | -------------------------------------------------------------- |
| LLVM bindings         | `inkwell` (safe) + `llvm-sys` (raw) — best-maintained anywhere |
| WASM toolchain        | First-class `wasm32-unknown-unknown` and `wasm32-wasi`         |
| Shared types          | Compiler ↔ runtime share IR / ABI definitions, no FFI between them |
| `no_std` runtime      | Clean support for the WASM and freestanding runtime builds     |
| C ABI emission        | `extern "C"` + `#[repr(C)]` match our interop story directly   |
| Parser ecosystem      | `logos`, hand-written recursive descent, `ariadne` / `miette` diagnostics |
| Graph-heavy code      | Borrow checker keeps IR / scheduler code sound                 |
| Distribution          | Single static binary; trivial cross-compilation                |

FlowArrow itself is intentionally not Turing-complete (no unbounded
recursion or loops) and cannot self-host. A separate host language is
required; Rust is the choice.

**Alternatives considered:**

- **C++** — direct LLVM C++ API access; weaker tooling and no borrow
  checker. No FlowArrow-specific advantage. Rejected.
- **OCaml** — strong compiler heritage; lagging LLVM bindings, smaller
  ecosystem, weaker WASM tooling. Rejected.
- **Zig** — promising but pre-1.0; revisit later. Rejected for now.
- **Go** — GC, weak LLVM story, poor WASM library output. Rejected.
- **Self-hosting in FlowArrow** — impossible by design. Rejected.

### 2.2 Primary backend: **LLVM**

The primary code generator is **LLVM**.

**Rationale:**

| Requirement                       | LLVM contribution                                |
| --------------------------------- | ------------------------------------------------ |
| Native binaries (multi-arch)      | Production-grade codegen for x86_64, aarch64, …  |
| WASM for web and WASI             | First-class `wasm32-unknown-unknown` / `wasm32-wasi` |
| Library + application outputs     | Object files + `lld` covers all link modes       |
| Interop                           | Stable C ABI per target                          |
| Optimizations                     | Mature optimizer, PGO, LTO                       |
| Debug info                        | DWARF on native, source maps on WASM             |
| Cross-compilation                 | Built-in via target triples                      |

FlowArrow's IR already exposes all legal parallelism explicitly, so we
do not need an optimizer that *discovers* parallelism — but we do need
strong scalar/loop/SIMD codegen, which LLVM provides.

**LLVM bindings:** `inkwell` (safe wrapper) where it suffices, falling
back to `llvm-sys` for anything `inkwell` does not expose. See §2.1
for the implementation language decision.

### 2.3 Interop contract: **C ABI**

The cross-language contract is the **platform C ABI** plus a generated
C header. Every other language ecosystem speaks C; nothing else has
this property.

**Implications:**

- Public FlowArrow functions are emitted with C linkage and the
  platform calling convention.
- Each library build emits a corresponding C header (`<name>.h`)
  describing all exported symbols, types, and ownership rules.
- Rust consumers use `bindgen` against the header.
- Go consumers use `cgo` against the header.
- C / C++ / Zig / Swift consumers `#include` the header.
- JS / web consumers load the `.wasm` artifact via the generated JS
  bindings (§2.5).

We will **not** invent a FlowArrow-specific FFI. The C ABI is the
contract; everything else is a binding generated on top of it.

**Ownership model** (must be defined before v0.1):

- Single-assignment semantics map cleanly to arena allocation; arenas
  are scoped to a top-level `program` invocation.
- Pointers crossing the FFI boundary are either:
  - **Borrowed**: caller retains ownership; lifetime documented.
  - **Transferred**: caller must free via a paired `flowarrow_free_*`
    function exported from the same library.
- No hidden global allocator state.

### 2.4 Output modes

The CLI follows Rust's convention because users already know it:

```text
flowarrow build --target <triple> --crate-type <kind>
```

Current implementation status: `flowarrow build` defaults to the host
native target and accepts `--target native`, `--target host`, or the host
target triple for that backend. `wasm32-unknown-unknown` supports an
initial `--crate-type cdylib` reactor-module path for pure scalar node
exports. `typescript` emits TypeScript source under `build/typescript/`.
`wasm32-wasi` is parsed but not implemented yet.

Build optimization defaults to `-O3`. Users can select a clang-style
optimization level with `-O0`, `-O1`, `-O2`, `-O3`, `-Os`, or `-Oz`.
Native builds pass that optimization flag to the clang invocations used
for runtime LLVM emission and final linking. WASM `cdylib` builds map
the optimization level to LLVM target-machine optimization, with `-O3`
using LLVM's aggressive optimization level.

Additional native compiler and linker flags can be passed with repeated
`--compiler-flag <flag>` / `--cflag <flag>` and
`--linker-flag <flag>` / `--ldflag <flag>`. A `--` delimiter also
collects the remaining build arguments as compiler flags. WASM `cdylib`
builds use the direct LLVM backend rather than a clang frontend, so they
accept optimization flags and `--linker-flag` values for `wasm-ld` but
reject arbitrary compiler flags.

Supported `--crate-type` values:

| Kind         | Native artifact         | WASM artifact            |
| ------------ | ----------------------- | ------------------------ |
| `bin`        | executable              | `.wasm` command module   |
| `staticlib`  | `.a` / `.lib`           | n/a                      |
| `cdylib`     | `.so` / `.dylib` / `.dll` | `.wasm` reactor module |

Supported `--target` values (initial set):

```text
x86_64-unknown-linux-gnu
x86_64-apple-darwin
aarch64-apple-darwin
aarch64-unknown-linux-gnu
x86_64-pc-windows-msvc
wasm32-unknown-unknown
wasm32-wasi
typescript
```

The `typescript` target emits standalone `.ts` source instead of invoking
a native compiler or linker. It supports the core language lowering and
core stdlib nodes for CLI arguments, text/bytes, integer/real conversion,
math, predicates, faults, tuples, and sequences. Native-backed modules
such as `std.cv`, `std.http`, and `std.sqlite` are intentionally rejected
by this backend until they have target-specific runtime support.

### 2.5 WebAssembly story

- The `wasm32-unknown-unknown` target produces a freestanding module
  suitable for browsers or JavaScript runtimes. The implemented first
  slice is `--crate-type cdylib` with ABI-compatible top-level nodes
  exported as core WASM functions. Exported node inputs and outputs are
  currently limited to scalar `Int` and `Real` values, with `Int`
  represented as WASM `i64`.
- The `wasm32-wasi` target produces a module runnable under wasmtime
  / wasmer / wasi-compatible hosts. This target is planned but not yet
  implemented.
- For browser use, the compiler emits a companion JavaScript/TypeScript
  bindings file that:
  - imports the `.wasm`,
  - exposes each exported FlowArrow function as a typed JS function,
  - handles linear-memory string and buffer marshalling.
- The bindings convention is compatible with `wasm-bindgen`'s ABI so
  existing tooling (`wasm-pack`, bundler plugins) works unchanged.
  We are **not** taking a dependency on `wasm-bindgen` itself; we
  match its ABI and emit our own bindings.

### 2.6 Runtime library

A small support library (`libflowarrow_rt`) is required for:

- Work-stealing scheduler for the dataflow DAG.
- Arena allocators (single-assignment ⇒ arenas are the natural fit).
- Reduction / scan tree primitives.
- Map / grid dispatch primitives.
- Boundary I/O — invoked only at `program` entry/exit.

Properties:

- Implemented in **Rust**, `#![no_std]` where possible.
- Dual-compiled to:
  - native static + dynamic libraries
  - `wasm32` modules
- No FlowArrow program ever calls runtime functions directly; the
  compiler emits the calls.
- The runtime exposes a stable C ABI internally so the compiler can
  link against any version that matches its expected interface
  version.

### 2.7 IR pipeline

```text
FlowArrow source
   │
   ▼
Lexer / Parser  ──►  AST
   │
   ▼
DAG IR (FlowArrow's own IR; SSA-shaped, matches language semantics)
   │
   ├──► Static analysis
   │       - acyclicity
   │       - single-assignment
   │       - type & shape checking
   │       - associativity checks for reduce / scan
   │
   ├──► Scheduling & cost analysis
   │       - critical path
   │       - parallel partitioning
   │       - lowering choices (sequential / SIMD / parallel / GPU)
   │
   ▼
LLVM IR lowering
   │
   ├──► native target → object files → lld → exe / .a / .so / .dylib / .dll
   └──► wasm32 target → .wasm → (browser bindings | wasi host)
```

The FlowArrow DAG IR is preserved as a first-class artifact (emittable
via `--emit=flowarrow-ir`) for debugging and tooling.

### 2.8 Tooling

| Concern         | Choice                                        |
| --------------- | --------------------------------------------- |
| Linker          | `lld` (cross-platform, ships with LLVM)       |
| Archive format  | platform-standard (`ar`, COFF, Mach-O)        |
| Debug info      | DWARF on native; WASM DWARF + source maps     |
| Sanitizers      | ASan / UBSan / TSan inherited from LLVM       |
| PGO / LTO       | LLVM ThinLTO + instrumentation-based PGO      |
| Test harness    | First-party `flowarrow test`                  |
| Package format  | TBD; out of scope for this document           |

### 2.9 Crate layout

A single Cargo workspace, all Rust:

```text
flowarrow-cli       `flowarrow build|run|test|fmt` entry point
flowarrow-parser    lexer (logos) + recursive-descent parser, diagnostics
flowarrow-ir        DAG IR types, analyses, scheduling, cost model
flowarrow-codegen   LLVM IR lowering via `inkwell`
flowarrow-rt        runtime: scheduler, arenas, reductions, scans
                    (`no_std` where possible; dual-built native + wasm32)
flowarrow-bindgen   emits C headers and JS/TS bindings for library builds
```

IR types are shared between crates as ordinary Rust types — no
serialization boundary inside the compiler.

---

## 3. Alternatives considered

Recorded so the reasoning is not lost.

### Cranelift

- **Pros:** Much faster compile times; native + WASM support;
  excellent Rust ecosystem fit; simpler than LLVM.
- **Cons:** Weaker optimizer; fewer target platforms; less mature
  debug info.
- **Decision:** Not the primary backend. **Reserved as a future
  `--backend=cranelift` mode for fast dev builds**, mirroring what
  Zig and rustc are doing. The IR pipeline (§2.7) is backend-agnostic
  precisely so this is a drop-in addition later.

### MLIR

- **Pros:** Designed for dataflow / multi-level lowering. FlowArrow's
  `grid`, `map`, `reduce`, `scan` map naturally onto `linalg`,
  `affine`, and `scf` dialects. Excellent path to GPU / SIMD /
  distributed targets.
- **Cons:** Heavier dependency; smaller community; WASM still bottoms
  out at LLVM anyway; adds complexity before we need it.
- **Decision:** **Revisit at v1.0+** when GPU / SIMD / distributed
  lowering becomes a priority. Our DAG IR is shaped so that a future
  MLIR lowering is feasible.

### Transpile to C

- **Pros:** Cheapest to bootstrap; portable; trivially interops.
- **Cons:** Poor debug experience; slow compile; awkward WASM via
  Emscripten; surrenders the explicit parallel scheduling story.
- **Decision:** Rejected as a destination. May be used as a debug
  `--emit=c` artifact, never the production path.

### Transpile to Rust

- **Pros:** Free WASM; excellent Rust interop; reuses cargo.
- **Cons:** Couples to Rust's compile times and ABI quirks; makes
  interop with C / Go *worse* (extra hop); ties our release cadence
  to rustc.
- **Decision:** Rejected.

### GraalVM / JVM / .NET

- **Cons:** Heavyweight runtimes; poor WASM library story; poor C
  interop; wrong shape for a pure parallel dataflow language.
- **Decision:** Rejected.

### Go backend

- **Cons:** GC; no stable C ABI; weak WASM library story.
- **Decision:** Rejected.

---

## 4. Open questions

These do not block initial implementation but must be answered before
v0.1 is released.

1. **Concrete memory model across the FFI boundary** — borrowed vs.
   transferred conventions; whether arenas are exposed to callers.
2. **Fault surface** — FlowArrow is pure and cannot throw exceptions,
   but invalid data and `program` boundaries may fault. How are faults
   expressed in the C ABI? (Likely: out-parameter status code; never via
   longjmp, exceptions, or host-language unwinding.)
3. **ABI versioning** — how the compiler and runtime negotiate
   compatible versions. Symbol versioning vs. a single
   `flowarrow_abi_version` constant.
4. **Generated bindings packaging** — do we ship a separate
   `flowarrow-bindgen` tool, or fold it into `flowarrow build`?
5. **Component Model** — adopt the WASM Component Model for the
   WASM target now, or stick with core modules and revisit?

---

## 5. Summary

- **Implementation language:** Rust, single Cargo workspace.
- **Backend:** LLVM, via `inkwell`.
- **Interop:** C ABI + generated headers; WASM bindings layered on
  top.
- **Outputs:** `bin`, `staticlib`, `cdylib` for native; `bin`,
  `cdylib` for WASM.
- **Runtime:** small Rust crate, `no_std` where possible, dual-built.
- **IR:** FlowArrow DAG IR is the stable internal interface; LLVM IR
  is one of potentially several lowerings (Cranelift, MLIR may
  follow).
- **Rejected for now:** Cranelift (deferred), MLIR (deferred),
  C/Rust transpilation, JVM/.NET, Go.

This is the reference. All implementation work should be consistent
with the decisions above; update this document in place as decisions
evolve.
