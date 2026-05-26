# typescript-concurrency-benchmark

Builds the same FlowArrow library twice with the TypeScript backend:

- `sequential`: normal TypeScript output.
- `workers`: TypeScript output with `--workers` enabled.

The benchmark compiles both generated `.ts` files to `.mjs` JavaScript, imports them,
and calls the exported
`score_batch(width: i64) -> summary: JobSummary`. The struct return value keeps
the four aggregate fields named in the generated TypeScript object shape.

```sh
npm run bench
```

You can tune the benchmark size from the terminal:

```sh
WIDTH=500000 WARMUPS=3 ITERATIONS=10 node run.mjs
```

The worker-enabled build uses Node `worker_threads` under Node.js and browser
Blob module workers when Web Worker globals are available. Both worker paths use
`SharedArrayBuffer` views for the batch data. The benchmark calls the generated
`__flowarrow_setup_workers()` hook before warmups and
`__flowarrow_teardown_workers()` afterward so worker startup cost does not
dominate each sample and the process exits cleanly.
