@/Users/jacob/.codex/RTK.md

# FlowArrow Repo Guide

FlowArrow is a Rust compiler and runtime for a static dataflow language. The
main crate has no normal dependencies and exposes both a library API and the
`flowarrow` CLI.

## Command Rules

- Always prefix shell commands with `rtk` in this repo.
- Common checks:
  - `rtk cargo check`
  - `rtk cargo test`
  - `rtk cargo test <test_name>`
  - `rtk cargo run -- typecheck examples/add-numbers-from-stdin/main.flow`
  - `rtk cargo run -- run examples/add-numbers-from-args/main.flow 1 2 3`
  - `rtk cargo run -- fmt examples/add-numbers-from-stdin/main.flow --check`
  - `rtk cargo bench`
- Native builds use `clang` and optional `pkg-config` dependencies. `std.http`
  needs H2O/OpenSSL/libuv, `std.sqlite` needs SQLite, and `std.cv` image codecs
  need libjpeg/libpng when those paths are exercised.
- `flowarrow build` writes generated artifacts under
  `examples/<name>/build/<host-target>/`; do not edit generated build output by
  hand.

## Normal Change Workflow

Run commands in this order unless the change clearly calls for a narrower or
broader variant:

Formatting-only changes are an exception: they do not require repo exploration,
validation, rerunning tests, or extra commands/checks beyond the requested
formatting operation.

1. Make the code, docs, and test changes.
2. Run the narrowest useful focused tests first:
   - `rtk cargo test <test_name>`
   - `rtk cargo test --test std_<module>`
   - `rtk cargo run -- typecheck examples/<name>/main.flow`
   - `rtk cargo run -- run examples/<name>/main.flow`
3. Run a compiler sanity check after focused tests pass:
   - `rtk cargo check`
4. Run the full test suite before handoff when feasible:
   - `rtk cargo test`
5. Run formatting near the end, after edits have settled:
   - `rtk cargo fmt`
   - `rtk cargo run -- fmt <path.flow> --check` for changed FlowArrow sources,
     or `rtk cargo run -- fmt <path.flow>` when the source should be rewritten.
6. If formatting changed Rust or FlowArrow source, rerun the focused tests that
   cover the changed behavior.

## Fast File Map

- CLI commands and argument parsing: `src/main.rs`.
- Public crate entry points, build pipeline, clang/pkg-config integration:
  `src/lib.rs`.
- Syntax data model: `src/ast.rs`.
- Lexing: `src/lexer.rs`.
- Parsing and grammar behavior: `src/parser.rs`.
- Type checking, type parser, effects, higher-order combinator validation:
  `src/typecheck.rs`.
- Local and source-backed stdlib import expansion: `src/module_resolver.rs`.
- C runtime/codegen backend, type registry, fusions, builtin lowering:
  `src/codegen.rs`.
- FlowArrow formatter: `src/fmt.rs`.
- Mermaid graph output: `src/mermaid.rs`.
- Stdlib registry and shared embedded runtime/source modules: `src/stdlib.rs`.
- Per-module stdlib metadata: `src/stdlib/*.rs`.
- C runtime fragments and headers: `src/stdlib/*.c`, `src/stdlib/*.h`.
- Source-backed stdlib modules: `src/stdlib/source/*.flow`.
- Integration tests: `tests/*.rs`; shared test helpers: `tests/support/mod.rs`.
- End-user examples: `examples/*/main.flow`.
- Language and implementation docs: `docs/`.
- VS Code syntax extension: `editors/vscode/`.
- Benchmarks: `benches/`.

## Change Routing

- New or changed language syntax usually touches `src/ast.rs`, `src/lexer.rs`,
  `src/parser.rs`, `src/typecheck.rs`, `src/codegen.rs`, `src/fmt.rs`,
  `src/mermaid.rs`, `docs/syntax.md`, `docs/overview.md`, tests in
  `src/lib.rs`, and possibly the VS Code grammar.
- Type system or semantic changes start in `src/typecheck.rs`; mirror type
  parsing/lowering in `src/codegen.rs` when runtime output changes.
- Backend/runtime changes are mostly in `src/codegen.rs` plus the relevant
  `src/stdlib/*.c` or `.h` fragment. Keep `docs/backend.md` current for
  architectural decisions.
- CLI behavior lives in `src/main.rs`; public callable functions live in
  `src/lib.rs`.
- Formatter changes live in `src/fmt.rs` and should include formatter unit tests
  there. Run checked-in source formatting checks when behavior changes.
- Mermaid graph changes live in `src/mermaid.rs`; related coverage is in
  `src/lib.rs` tests.
- Local import or stdlib source expansion behavior belongs in
  `src/module_resolver.rs`; integration coverage belongs in
  `tests/local_imports.rs` or source-backed stdlib tests.
- Adding a stdlib intrinsic requires updating the per-module file in
  `src/stdlib/`, the `SYMBOLS` list in `src/stdlib.rs`, typecheck/codegen
  support when not direct, docs under `docs/std/`, and focused tests under
  `tests/std_<module>.rs`.
- Adding source-backed stdlib nodes requires `src/stdlib/source/<module>.flow`,
  exports in `src/stdlib.rs`, docs under `docs/std/`, and tests in
  `tests/std_flow_source.rs` or a module-specific integration test.
- Syntax keyword changes should update `src/lexer.rs`, parser/formatter tests,
  `docs/syntax.md`, and
  `editors/vscode/syntaxes/flowarrow.tmLanguage.json`.

## Testing Patterns

- Unit-heavy parser/typechecker/codegen coverage is currently in `src/lib.rs`;
  codegen-specific unit tests also live in `src/codegen.rs`.
- Integration tests compile and run temporary `.flow` programs through
  `tests/support/mod.rs`.
- Use `support::run_source(name, source, stdin)` when a test needs stdout,
  stderr, or an exit status.
- Use `support::build_source(name, source)` when only successful compilation is
  needed.
- Use `typecheck_file` for tests that should avoid native dependencies or
  runtime execution.
- Optional-native tests may typecheck everywhere but only build/run when the
  required system libraries are installed; preserve clear dependency diagnostics.

## Docs To Keep In Sync

- Parser or grammar changes: `docs/syntax.md`.
- Semantic model changes: `docs/overview.md`.
- Backend, ABI, runtime, build output, or interop changes: `docs/backend.md`.
- Formatting behavior changes: `docs/formatting.md`.
- Fault behavior changes: `docs/faults.md`.
- Stdlib additions or signature changes: `docs/std/README.md` and the matching
  `docs/std/<module>.md`.
- User-visible example behavior: matching `examples/*/README.md`.

## Local Development Notes

- The canonical CLI subcommands are `run`, `build`, `typecheck`, `fmt`, and
  `graph`.
- `program main(args: Args) -> exit_code: Int` is the normal executable entry
  point; `Faultable[Int]` is also accepted by the typechecker.
- `src/lib.rs::build_file` parses, typechecks, emits LLVM shim text, emits C
  runtime code, caches generated files, then invokes `clang`.
- `src/codegen.rs::emit_module` currently emits a thin LLVM entry shim that
  calls the generated C runtime entry point.
- Stdlib imports from `std.vector`, `std.matrix`, and `std.cv` can expand
  embedded FlowArrow source before typecheck/codegen.
- Local string imports are resolved relative to the importing source file by
  `module_resolver`.
