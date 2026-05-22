import { readFile } from "node:fs/promises";

const wasmPath = new URL("./build/wasm32-unknown-unknown/fib.wasm", import.meta.url);
const bytes = await readFile(wasmPath);
const { instance } = await WebAssembly.instantiate(bytes, {});

console.log(instance.exports.fib(10n).toString());
