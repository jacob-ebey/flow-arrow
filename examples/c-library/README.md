# c-library

Builds a FlowArrow module as a native shared library and calls the exported
nodes from a C application.

```sh
flowarrow build --crate-type cdylib stats.flow
clang app.c -I build/<host-target> build/<host-target>/libstats.<so|dylib> -Wl,-rpath,build/<host-target> -o build/<host-target>/app
build/<host-target>/app
```

Expected output:

```text
jobs: 16
total score: 1632
peak score: 272
total weight: 288
peak weight: 33
```

The build writes `stats.h` beside the shared library. Top-level `extern node`
declarations become C ABI functions with generated typedefs for FlowArrow
runtime values such as bytes, tuples, sequences, structs, and faultables.
Private `node` helpers stay internal to the generated library.
