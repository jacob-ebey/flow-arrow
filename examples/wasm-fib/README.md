# wasm-fib

Builds a `wasm32-unknown-unknown` reactor module from FlowArrow and calls the
exported `fib` node from Node.js.

```sh
flowarrow build --target wasm32-unknown-unknown --crate-type cdylib fib.flow
node run.mjs
```
