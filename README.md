# FlowArrow Developer Guide

FlowArrow is a compiler and runtime for a static dataflow language. This
repository contains the parser, typechecker, C runtime/codegen backend, standard
library registry, examples, tests, docs, and VS Code syntax extension.

## Prerequisites

Required for normal development:

- Rust toolchain with edition 2024 support
- `clang`
- `pkg-config`

Recommended:

- `ripgrep` (`rg`) for repository search
- VS Code, if working on the bundled editor extension

Optional native dependencies:

- `libjpeg` development headers and libraries for `std.cv` JPEG support
- `libpng` development headers and libraries for `std.cv` PNG support
- H2O development headers and libraries for `std.http`
  - FlowArrow looks for `libh2o-evloop` first, then `libh2o`, through
    `pkg-config`.
  - FlowArrow compiles the H2O runtime against H2O's evloop backend and adds
    `pkg-config` flags for `openssl` and `libuv`, because H2O headers include
    those dependency headers on common installations.
  - Without H2O installed, non-HTTP builds and tests still work. Building a
    program that imports `std.http` fails with a clear dependency diagnostic.
- SQLite 3 development headers and libraries for `std.sqlite`
  - FlowArrow looks for `sqlite3` through `pkg-config`.
  - Without SQLite development files installed, non-SQLite builds and tests
    still work. Building a program that imports `std.sqlite` fails with a clear
    dependency diagnostic.

On macOS with Homebrew, a typical setup is:

```sh
brew install rust llvm pkg-config ripgrep jpeg-turbo libpng h2o sqlite
```

If Homebrew installs `clang` or libraries outside the default search path, make
sure `clang` and `pkg-config` can see them from your shell.

On Debian/Ubuntu, the equivalent baseline is:

```sh
sudo apt-get update
sudo apt-get install -y clang pkg-config ripgrep libjpeg-dev libpng-dev libsqlite3-dev
```

H2O package names vary by distribution. If no packaged `libh2o` development
package is available, install H2O separately and expose its `.pc` file through
`PKG_CONFIG_PATH`.

## Common Commands

Build and test the Rust crate:

```sh
cargo check
cargo test
```

Run a FlowArrow program:

```sh
cargo run -- run examples/add-numbers-from-args/main.flow 1 2 3
```

Typecheck without building native code:

```sh
cargo run -- typecheck examples/http-server/main.flow
```

Build an example:

```sh
cargo run -- build examples/add-numbers-from-stdin/main.flow
cargo run -- build examples/add-numbers-from-stdin/main.flow --target native
```

Print a Mermaid execution graph:

```sh
cargo run -- graph examples/add-numbers-from-stdin/main.flow
```

Print a compact graph that collapses intermediate bindings into edge labels:

```sh
cargo run -- graph --compact examples/add-numbers-from-stdin/main.flow
```

Format a FlowArrow source file:

```sh
cargo run -- fmt examples/add-numbers-from-stdin/main.flow --check
cargo run -- fmt examples/add-numbers-from-stdin/main.flow
```

Run benchmarks:

```sh
cargo bench
```

## Build Artifacts

`flowarrow build` defaults to the host native target. `--target native`,
`--target host`, and the host target triple select the same native backend.
`wasm32-unknown-unknown` supports library-style WASM builds for exportable
scalar nodes, `typescript` emits generated TypeScript source, and
`javascript` emits generated JavaScript plus TypeScript declarations from the
TypeScript backend with OXC.

Native outputs are written under the source file's local build directory:

```text
examples/<name>/build/<target>/
```

The generated LLVM modules are cached in:

```text
examples/<name>/build/<target>/.cache/
```

These files are generated artifacts. Do not edit them by hand.

## Static Demo

The static demo in `static/index.html` loads `static/flowarrow.wasm` and compiles
FlowArrow examples to JavaScript, TypeScript, and LLVM IR preview text in the
browser. Rebuild the WASM
compiler artifact before serving or publishing `static/`:

```sh
source ~/.zshrc
rtk rustup target add wasm32-unknown-unknown
rtk cargo build --lib --target wasm32-unknown-unknown --release
rtk cp target/wasm32-unknown-unknown/release/flowarrow.wasm static/flowarrow.wasm
```

If `cargo` or `rustc` resolves to a non-rustup toolchain, such as Homebrew
Rust on macOS, the build can still report that `std` for
`wasm32-unknown-unknown` is missing after `rustup target add` succeeds. Sourcing
the shell config first ensures rustup's shims are ahead of that toolchain on
`PATH`.

## Optional Dependency Checks

Check CV-related libraries:

```sh
pkg-config --cflags --libs libjpeg
pkg-config --cflags --libs libpng
```

Check H2O for `std.http`:

```sh
pkg-config --cflags --libs libh2o-evloop || pkg-config --cflags --libs libh2o
pkg-config --cflags --libs openssl
pkg-config --cflags --libs libuv
```

Check SQLite for `std.sqlite`:

```sh
pkg-config --cflags --libs sqlite3
```

The HTTP example typechecks everywhere:

```sh
cargo run -- typecheck examples/http-server/main.flow
```

It only builds and runs when H2O is available through `pkg-config`:

```sh
cargo run -- build examples/http-server/main.flow
examples/http-server/build/<host-target>/main
```

Use the matching directory under `examples/http-server/build/`. The server
listens on `0.0.0.0:8080` and can be checked with
`curl http://127.0.0.1:8080/health` or a browser.

The SQLite example builds and runs when SQLite is available through
`pkg-config`:

```sh
cargo run -- build examples/sqlite-todos/main.flow
examples/sqlite-todos/build/<host-target>/main
```

## Repository Map

- `src/` - compiler, formatter, Mermaid graph emitter, stdlib registry, and C
  runtime fragments
- `tests/` - integration tests for stdlib behavior and local imports
- `examples/` - FlowArrow programs used as design and backend pressure tests
- `docs/` - language, backend, formatting, faults, and stdlib documentation
- `benches/` - benchmark notes and benchmark targets
- `editors/vscode/` - VS Code syntax extension

## VS Code Extension

The extension is local to `editors/vscode`.

Run it in extension-development mode:

```sh
cd editors/vscode
code --extensionDevelopmentPath="$(pwd)"
```

Package it locally if `vsce` is available:

```sh
cd editors/vscode
npx @vscode/vsce package
code --install-extension flowarrow-vscode-0.1.0.vsix
```

Update the TextMate grammar when adding or removing language keywords,
combinators, declaration forms, or literal syntax.

## Development Notes

- Keep `docs/syntax.md` in sync with parser changes.
- Keep `docs/overview.md` in sync with language semantics.
- Keep `docs/std/` and `src/stdlib.rs` in sync when adding stdlib modules or
  exports.
- Update `editors/vscode/syntaxes/flowarrow.tmLanguage.json` for syntax changes.
- Prefer focused tests for parser/typechecker/codegen behavior, then run the
  full `cargo test` suite before handing off changes.
