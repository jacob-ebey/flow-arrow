# higher-order-nodes

Demonstrates static node parameters:

```flow
node twice<step: node(Int) -> Int>(x: Int) -> y: Int {
    $x -> step -> step -> $y
}

40 -> twice<increment> -> $answer
```

`twice<increment>` is resolved before typecheck/codegen. The generated graph is
equivalent to calling `increment` twice; no node value flows at runtime.

```sh
flowarrow run main.flow
```

Expected output:

```text
42
```

`lib.flow` is the same pattern as an `extern node` for library-style targets:

```sh
flowarrow build --target typescript --crate-type cdylib lib.flow
flowarrow build --target wasm32-unknown-unknown --crate-type cdylib lib.flow
```
