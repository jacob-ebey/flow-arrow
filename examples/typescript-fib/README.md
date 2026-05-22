# typescript-fib

Builds TypeScript source from FlowArrow and calls the exported `fib` node from
Node.js.

```sh
flowarrow build --target typescript --crate-type cdylib fib.flow
tsc --noEmit --target ES2022 --module NodeNext --moduleResolution NodeNext --allowImportingTsExtensions run.ts build/typescript/fib.ts
node run.ts
```
