# typescript-fib

Builds JavaScript plus TypeScript declarations from FlowArrow and calls the
exported `fib` node from Node.js. The TypeScript target emits a `.ts` source
file when you want the generated TypeScript directly.

```sh
flowarrow build --target javascript --crate-type cdylib fib.flow
flowarrow build --target typescript --crate-type cdylib fib.flow
tsc --noEmit --target ES2022 --module NodeNext --moduleResolution NodeNext run.ts build/javascript/fib.d.ts
node run.ts
```
