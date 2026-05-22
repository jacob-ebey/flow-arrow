# ts-interop

TS/JS language interop example. It imports native Node APIs from `node:os`,
uses `console.log` as a global host boundary, and keeps those host calls
visible in the FlowArrow graph.

The example covers the first target interop surface:

- ESM imports from native Node modules, using `node:os`.
- Global host objects, using `console.log`.
- Explicit purity/effect markers on foreign nodes.
- Struct-shaped FlowArrow values assembled from foreign calls.

Target source shape:

```flow
foreign js module "node:os" {
    pure node platform() -> value: Bytes = platform
}

foreign js global "console" {
    io node log(message: Bytes) -> done: Unit = log
}
```

Expected TypeScript/JavaScript lowering shape:

```ts
import * as __fa_node_os from "node:os";

function platform(): string {
  return __fa_node_os.platform();
}

function available_parallelism(): bigint {
  return BigInt(__fa_node_os.availableParallelism());
}

function log(message: string): undefined {
  console.log(message);
  return undefined;
}
```

The complete application in `main.flow` reads several values from `node:os`,
collects them into `RuntimeInfo`, renders a message, and writes it with
`console.log`. The `program main(args: Args) -> exit_code: Int` shape stays
compatible with the existing executable entrypoint contract.

Build as an ESM application:

```sh
flowarrow build --target javascript main.flow
node build/javascript/main.mjs
```

Expected terminal output shape:

```text
FlowArrow TS interop
platform: darwin
arch: arm64
available parallelism: 12
home: /Users/example
```
