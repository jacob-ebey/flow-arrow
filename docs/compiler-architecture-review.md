# FlowArrow Compiler Architecture Review

This is a compiler-architecture critique focused on maintainability and long
term health. It is intentionally biased toward structural risks: places where
the compiler can keep working today but becomes harder to extend, diagnose, and
trust as the language and target set grow.

## Executive Summary

FlowArrow has a coherent surface pipeline:

```text
source -> lexer/parser -> AST -> module expansion -> typecheck -> monomorphize -> codegen/build
```

The implementation does not preserve that clean shape internally. The largest
maintenance problem is that there is no durable typed, resolved intermediate
representation between parsing and backend emission. The parser produces a
mostly untyped syntactic AST where names and types are strings. Module
resolution rewrites those strings. Type checking parses and validates them into
a private `Type`, then discards the result. Code generation reparses the same
strings into a second private `Ty`, rebuilds symbol tables, reimplements
assignability and builtin semantics, and then emits multiple targets.

That design is manageable while the language is small. It becomes expensive once
FlowArrow has local imports, source-backed stdlib modules, static node
parameters, fault propagation, structs, streams, C/JS foreign nodes, TypeScript,
LLVM preview, native LLVM object emission, WASM, C runtime fragments, and LSP
diagnostics. The current architecture makes every language feature a cross-file
coordination exercise.

The main recommendation is to introduce a canonical resolved and typed IR, then
make every backend consume that IR. This should happen incrementally:

1. Extract the shared type system and type parser from `typecheck.rs` and
   `codegen/mod.rs`.
2. Change type checking from `Result<(), String>` to `Result<TypedModule,
   CompilerError>`.
3. Move module resolution and monomorphization into a single compile session.
4. Split codegen into backend-specific lowerings that consume `TypedModule`
   instead of reparsing declarations.
5. Replace message-derived diagnostics with span-carrying structured errors.

This review was followed by a first repair pass that completed the highest
leverage low-risk cleanup: `src/types.rs` is now the single owner of the
semantic type enum, signatures, type parser, type substitution, assignability,
and sequence-literal type joining. Type checking and codegen both consume that
module. Native build paths now lower a module once and derive LLVM, runtime C,
foreign C sources, foreign dependencies, and WASM artifacts from the same
`LoweredModule`.

A second repair pass introduced explicit resolved and typed compiler contracts.
`module_resolver` now returns `ResolvedModule`, carrying the backend-compatible
lowered AST plus `ModuleId`/`SymbolId` tables. `typecheck` now exposes
`TypedModule`, `TypedCallable`, typed ports, typed chains, structured typed
endpoints, structured typed stages, and typed stage input/output facts. Existing
`check_*` entry points are wrappers over the typed path. `LoweredModule` owns a
`TypedModule`, and backend setup consumes typed callable/foreign signatures
instead of reparsing those signatures from syntax when a typed module is
available.

## What Is Working

- The project has a clear language model and a compact AST that is easy to read.
- The codebase already separates parsing, formatting, module expansion, type
  checking, monomorphization, build orchestration, and several backends at the
  file level.
- Source-backed stdlib modules are a good idea: they keep higher-level library
  code in FlowArrow instead of hard-coding every abstraction in Rust.
- Integration coverage is broad. The stdlib tests exercise real compile/run
  paths through `tests/support/mod.rs`, which catches many backend regressions.
- The build cache hashes generated artifacts and foreign C dependencies, which
  is the right direction for reproducible local builds.

## P0: No Canonical Typed IR

Status after repair passes: partially addressed. `TypedModule` now exists and
contains typed callables, ports, chains, structured typed endpoints, structured
typed stages, stage input/output types, and symbol references where a stage
names a callable. The remaining problem is that the backend-compatible AST is
still carried inside the typed module, and most emitters still lower from that
AST while consulting typed facts.

### Evidence

- The AST stores declared types as raw strings: `TypeAlias.ty`, `NodeParam.input`,
  `NodeParam.output`, and `Port.ty` are all `String` in `src/ast.rs`.
- Type checking defines a private semantic `Type` and `Signature` in
  `src/typecheck.rs`, uses them for validation, then returns only `Result<()>`.
- Codegen defines a second private `Ty` and `Signature` in `src/codegen/mod.rs`
  and rebuilds type information from the original AST.
- Public entry points call type checking and then call codegen, but codegen does
  not receive the checked semantic result.

### Why This Hurts

The compiler has two sources of truth for the type system. A change to
assignability, `Faultable`, `Stream`, `OneOf`, structs, or stdlib signatures must
be mirrored in type checking and codegen. When the two disagree, FlowArrow can
accept a program that a backend cannot emit, or reject a program in one API path
but fail later in another.

The shape also blocks better diagnostics. After type checking decides that a
chain stage has a type, that information is not attached to the AST for LSP,
graph output, optimization, or backend emission. Every consumer has to rediscover
or approximate semantic information.

### Recommended Change

Create a canonical semantic layer, for example:

```rust
pub struct TypedModule {
    pub sources: SourceMap,
    pub declarations: Vec<TypedDecl>,
    pub symbols: SymbolTable,
    pub types: TypeInterner,
}

pub struct TypedCallable {
    pub id: SymbolId,
    pub kind: CallableKind,
    pub effect: Effect,
    pub signature: Signature,
    pub node_params: Vec<NodeParamSignature>,
    pub chains: Vec<TypedChain>,
}

pub struct TypedStage {
    pub span: SourceSpan,
    pub kind: StageKind,
    pub input: TypeId,
    pub output: TypeId,
}
```

Backends should consume `TypedModule`. The syntactic AST should remain useful for
formatting and early parse diagnostics, but not as the backend contract.

## P0: Type Parsing And Type Semantics Are Duplicated

Status after repair passes: addressed for the semantic type layer. `src/types.rs`
now owns `Type`, `Signature`, type parsing, primitive/stdlib type symbols,
substitution, assignability, empty-sequence detection, and sequence item type
joining. Typecheck and codegen both use that module.

### Evidence

- `src/typecheck.rs` has its own `TypeParser`, `match_types`,
  `assignable_type`, `substitute`, stdlib signature expansion, and primitive
  type mapping.
- `src/codegen/mod.rs` has a separate `Ty`, separate `TypeParser`,
  `builtin_output_type`, `sequence_item_type`, output assignability helpers,
  C type naming, LLVM type lowering, and ABI export checks.
- The two parsers already differ: the codegen parser skips whitespace while the
  typechecker parser is stricter and encodes stdlib special cases separately.

### Why This Hurts

This is semantic drift waiting to happen. The codegen type layer is not just a
layout layer; it decides output types, validates builtin shapes, and returns user
visible errors. That means backend code can become a second typechecker.

### Recommended Change

Move the type system into a shared module, for example `src/types.rs`:

- `Type`, `TypeId`, `Signature`, `TypeParser`.
- Substitution, unification, assignability, `contains_faultable`,
  `strip_faultable`, `sequence_item_type`.
- Structured primitive and runtime type registry.

Then make codegen lowering map `TypeId` to backend layout types instead of
owning a second semantic type enum.

## P0: Compile Pipeline Work Is Repeated And Inconsistent

Status after repair passes: partially addressed. Build/codegen paths now create
one `LoweredModule` and derive backend artifacts from that object. Public API
entry points still parse/typecheck before invoking backend wrappers, so a future
`CompileSession` should absorb those wrappers.

### Evidence

`build_file_with_options` parses and typechecks once, then each target path calls
codegen helpers that expand imports and monomorphize again. Native binary builds
call four separate codegen queries: direct LLVM, runtime support C, foreign C
source paths, and foreign C dependency paths. Each query re-expands sources,
re-monomorphizes, and rebuilds `TypedCodegen`.

### Why This Hurts

Repeated phase execution creates needless CPU work, but the bigger problem is
consistency. If expansion, monomorphization, or typed codegen construction ever
becomes stateful, configurable, or diagnostic-rich, every public API path must be
kept in sync. It also makes caching harder: the build cache sees emitted text,
not a stable lowered compilation result.

### Recommended Change

Introduce a compile session:

```rust
pub struct CompileSession {
    source_map: SourceMap,
    options: CompileOptions,
}

impl CompileSession {
    pub fn parse_file(&mut self, path: &Path) -> Result<ParsedModule, CompilerError>;
    pub fn resolve(&mut self, parsed: ParsedModule) -> Result<ResolvedModule, CompilerError>;
    pub fn typecheck(&mut self, resolved: ResolvedModule) -> Result<TypedModule, CompilerError>;
    pub fn monomorphize(&mut self, typed: TypedModule) -> Result<TypedModule, CompilerError>;
}
```

Build orchestration should request a single lowered module and then ask target
backends for artifacts from that same object.

## P1: Module Resolution Rewrites Syntax Instead Of Producing Symbols

Status after repair passes: partially addressed. The resolver now returns a
`ResolvedModule` with `ModuleId` and `SymbolId` tables, and typed stages can
carry the resolved symbol for callable references. The resolver still preserves
the existing backend-compatible name rewriting; replacing rewritten names with
structural symbol references remains the next step.

### Evidence

`module_resolver.rs` expands source-backed stdlib and local modules by cloning
declarations, rewriting names, and textually rewriting type strings. It mints
internal names from source paths or module names and rewrites endpoint names,
stage names, match arms, struct names, and foreign node names.

### Why This Hurts

Name mangling is doing the job of a resolver. It makes collisions and display
names harder to reason about, and it loses the relationship between a source
name and the symbol it resolved to. Diagnostics can only point back to best
effort token matches in the root source. Cross-file diagnostics, rename support,
go-to-definition, and good error messages all want symbol IDs and source spans,
not rewritten strings.

Textual type rewriting is especially fragile. It must know enough type syntax to
avoid false replacements, but it is not using the type parser. A real type AST
would let module resolution rewrite names structurally.

### Recommended Change

Replace rewrite-by-mangling with:

- `ModuleId` for root, stdlib source modules, and local files.
- `SymbolId` for each type, node, program, foreign node, and import alias.
- Resolved references in endpoints and stages.
- Backend-only symbol mangling as a final lowering step.

Generated internal names can remain, but they should be a backend concern, not
the semantic identity of a node.

## P1: Codegen Is A Backend Framework, Optimizer, ABI Layer, And Runtime Emitter In One File

### Evidence

`src/codegen/mod.rs` is over 12,000 lines. It contains public emit entry points,
the private codegen type system, typed codegen state, C runtime emission,
parallel helper generation, fusion detection and emission, direct LLVM lowering,
LLVM type registry, C type registry, ABI header generation, builtin output type
logic, symbol sanitization, and tests. TypeScript and LLVM-text emitters depend
on internal pieces from this module.

### Why This Hurts

Large files are not automatically bad. This one is problematic because unrelated
compiler responsibilities are coupled through one private state object:
`TypedCodegen`. It is difficult to change a backend without understanding global
codegen invariants. It is also hard to test optimizations, ABI lowering, runtime
selection, and builtin typing independently.

### Recommended Change

Split codegen around explicit artifacts and backend contracts:

```text
src/backend/
  mod.rs              Backend trait, Artifact model
  c_runtime.rs        C source generation from typed IR
  llvm_direct.rs      Direct LLVM object/IR lowering
  llvm_preview.rs     Text preview emitter, if still needed
  typescript.rs       TypeScript/JavaScript source backend
  abi.rs              Native C/WASM export ABI lowering
  layout.rs           Type layout registry per backend
  fusion.rs           Typed optimization patterns
```

Each backend should receive `TypedModule` plus backend options and return
structured artifacts, not mutate shared codegen state.

## P1: Stdlib Metadata Is Not Declarative Enough

### Evidence

The central stdlib registry lists `StdSymbol` values with string signatures and
runtime support flags. Type checking then special-cases numeric signatures and
reduce signatures. Codegen has its own builtin output typing and large match
statements for backend lowering. The higher-order whitelist is another separate
hard-coded list.

### Why This Hurts

Adding a stdlib intrinsic requires edits in the registry, typechecker, one or
more backend match statements, docs, tests, and sometimes runtime fragments.
There is no single declaration that says:

- this symbol has these overloads,
- these backends implement it,
- it is pure or effectful,
- it can be used as a higher-order function,
- it has reduce semantics,
- it needs these runtime fragments or native dependencies.

### Recommended Change

Make stdlib symbols declarative and typed:

```rust
pub struct BuiltinSpec {
    pub id: BuiltinId,
    pub module: ModuleName,
    pub name: &'static str,
    pub kind: SymbolKind,
    pub signatures: &'static [SignatureSpec],
    pub reduce_signatures: &'static [SignatureSpec],
    pub effect: Effect,
    pub higher_order: HigherOrderPolicy,
    pub runtime: RuntimeRequirement,
    pub backends: BackendSupport,
}
```

Backends can still implement builtins with match statements, but those matches
should be keyed by `BuiltinId` and checked against the registry.

## P1: Diagnostics Are Message-Derived

### Evidence

The parser emits `SourceDiagnostic` with spans, but later phases return plain
`String`. `diagnostic.rs` tries to recover a span by extracting backtick-delimited
items from the message, lexing the source, and finding a token with a matching
name. The LSP path similarly falls back to range heuristics.

### Why This Hurts

Error strings are not a stable API. If a message is reworded, span recovery can
break. If an error references an imported symbol, generated internal name,
expanded stdlib node, tuple type, or repeated identifier, the heuristic can pick
the wrong location or no location. This directly limits LSP quality and CLI
diagnostics.

### Recommended Change

Use a structured error type everywhere after parsing:

```rust
pub struct CompilerError {
    pub code: ErrorCode,
    pub message: String,
    pub primary: Option<SourceSpanId>,
    pub labels: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
}
```

AST nodes should carry spans, or the resolved/typed IR should preserve spans for
references and declarations. String formatting should be the final rendering
step, not the error transport.

## P2: The Public API Exposes Phase Fragments Instead Of Compiler Intent

### Evidence

`lib.rs` exposes separate helpers for TypeScript, JavaScript artifacts, LLVM IR
preview, format, typecheck, graph, build, and run. Most compile helpers repeat:
parse, choose program vs library checking, map diagnostics, then call a backend.
The WASM API repeats the same pattern.

### Why This Hurts

The API surface encourages new features to add another parallel path. Options
such as library/program mode, worker concurrency, base directory, target, and
diagnostic formatting are split across layers.

### Recommended Change

Add a single high-level API:

```rust
pub fn compile(source: SourceInput, options: CompileRequest)
    -> Result<CompileOutput, CompilerError>;
```

Keep existing public functions as wrappers, but implement them through the common
request object. This gives CLI, library, WASM, LSP, and tests the same behavior.

## P2: Tests Are Broad But Too Backend-Coupled

### Evidence

There are many useful integration tests in `tests/*.rs`, and `src/lib.rs` holds
a large number of compiler tests that parse/typecheck/codegen/run examples. The
shared integration helper writes temporary `.flow` files, calls `build_file`,
and executes the produced binary.

### Why This Hurts

End-to-end tests are valuable, but they are expensive and often diagnose failures
late. A type system change may fail as a native build problem. A module resolver
change may surface as a C codegen substring mismatch. Because there is no typed
IR, there are few natural phase-level assertions.

### Recommended Change

After `TypedModule` exists, add tests at these boundaries:

- Parser golden tests: source to AST with spans.
- Resolver golden tests: imports to symbol IDs and resolved references.
- Typechecker tests: typed chains, effects, signatures, fault propagation.
- Monomorphization tests: generic node instances and recursion errors.
- Backend conformance tests: same typed module through TypeScript, C runtime,
  direct LLVM, and WASM where supported.
- Error snapshot tests: stable error code, message, primary span, labels.

Keep the current compile/run tests as acceptance coverage.

## Suggested Target Architecture

```text
SourceMap
  |
  v
Lexer + Parser
  -> ParsedModule { AST + spans }
  |
  v
Resolver
  -> ResolvedModule { SymbolId refs, imports, module graph }
  |
  v
Typechecker
  -> TypedModule { TypeId, effects, signatures, typed stages }
  |
  v
Monomorphizer + Optimizer
  -> TypedModule
  |
  +--> TypeScriptBackend -> .ts/.mjs/.d.ts/worker files
  +--> CRuntimeBackend   -> generated C runtime support
  +--> DirectLlvmBackend -> LLVM IR/object
  +--> WasmBackend       -> wasm object/module
  +--> GraphBackend      -> Mermaid from typed graph
  +--> LspAnalysis       -> diagnostics, hover, completion, symbols
```

The key property is that backends no longer resolve names, parse types, or infer
types. They may reject unsupported target features, but semantic validation lives
before backend lowering.

## Incremental Refactor Plan

### Phase 1: Shared Types With No Behavior Change

- Add `src/types.rs`.
- Move `Type`, `Signature`, parser, display, substitution, assignability, and
  builtin primitive aliases out of `typecheck.rs`.
- Convert `codegen::Ty` to either re-export the same type or a thin backend
  layout wrapper around it.
- Keep public APIs unchanged.

This is the highest-leverage first move because it reduces semantic drift
without changing the whole compiler pipeline.

### Phase 2: Structured Errors And Spans

- Add spans to declarations, ports, endpoints, stages, imports, and type syntax.
- Replace `Result<_, String>` inside compiler phases with `Result<_,
  CompilerError>`.
- Keep wrapper functions that render `String` for the existing API.
- Update LSP to consume structured diagnostics directly.

### Phase 3: Resolved Module

- Introduce `SymbolId`, `ModuleId`, `ResolvedName`, and a resolver output type.
- Stop rewriting type text and stage node names in place.
- Keep backend symbol mangling as a late lowering step.
- Preserve source-backed stdlib expansion, but model it as modules in a graph.

### Phase 4: Typed Module

- Make typecheck return `TypedModule`.
- Store each chain source and stage input/output type.
- Store callable signatures, effects, and backend capability checks separately.
- Update Mermaid and LSP summary to use typed data instead of rerunning partial
  analysis.

### Phase 5: Backend Contracts

- Define `Backend` and `Artifact` abstractions.
- Split `codegen/mod.rs` by backend and responsibility.
- Make native build request all artifacts from one compiled module.
- Move fusion detection to a typed optimization pass.

## Non-Goals

- Do not rewrite the parser before typed IR exists. The parser is not the main
  limiting factor.
- Do not replace the C runtime strategy just to make the architecture cleaner.
  The generated C runtime can remain a valid backend artifact.
- Do not remove integration tests. Add phase-level tests beside them.
- Do not make every compiler type public. Keep the API stable, but use structured
  internal contracts.

## Highest Priority Fixes

1. Create a shared type system module and delete the duplicate `Type`/`Ty`
   semantic split.
2. Make type checking return a `TypedModule` that backends can consume.
3. Replace string-rewrite module expansion with resolved symbol references.
4. Add structured diagnostics with spans beyond parser errors.
5. Split `codegen/mod.rs` after the typed backend contract exists.

These steps should be done in that order. Splitting codegen first would move the
current coupling around without removing it. The architectural center of gravity
needs to shift from "backends rediscover semantics" to "backends lower checked
semantics."
