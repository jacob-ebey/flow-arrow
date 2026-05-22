# typescript-fib

Builds JavaScript and TypeScript declarations from FlowArrow and calls the
exported `fib` node from Node.js.

```sh
flowarrow build --target typescript --crate-type cdylib fib.flow
flowarrow build --target javascript --crate-type cdylib fib.flow
tsc --noEmit --target ES2022 --module NodeNext --moduleResolution NodeNext run.ts build/typescript/fib.d.ts
node run.ts
```
