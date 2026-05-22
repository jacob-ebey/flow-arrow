import { performance } from "node:perf_hooks";

const width = BigInt(process.env.WIDTH ?? "200000");
const warmups = Number(process.env.WARMUPS ?? "2");
const iterations = Number(process.env.ITERATIONS ?? "8");

const sequential = await import("./build/benchmark/sequential-js/bench.mjs");
const workers = await import("./build/benchmark/workers-js/bench.mjs");

function formatTuple(value) {
  return `[${value.map((item) => item.toString()).join(", ")}]`;
}

async function measure(label, fn) {
  for (let index = 0; index < warmups; index++) {
    await fn(width);
  }

  const samples = [];
  let result = null;
  for (let index = 0; index < iterations; index++) {
    const start = performance.now();
    result = await fn(width);
    samples.push(performance.now() - start);
  }

  samples.sort((left, right) => left - right);
  const total = samples.reduce((sum, value) => sum + value, 0);
  const mean = total / samples.length;
  const median = samples[Math.floor(samples.length / 2)];
  const min = samples[0];
  return { label, result, mean, median, min };
}

const sequentialResult = await measure("sequential", sequential.score_batch);
await workers.__flowarrow_setup_workers();
let workersResult;
try {
  workersResult = await measure("workers", workers.score_batch);
} finally {
  await workers.__flowarrow_teardown_workers();
}

if (formatTuple(sequentialResult.result) !== formatTuple(workersResult.result)) {
  throw new Error(
    `result mismatch: sequential=${formatTuple(sequentialResult.result)} workers=${formatTuple(workersResult.result)}`,
  );
}

for (const result of [sequentialResult, workersResult]) {
  console.log(
    `${result.label.padEnd(10)} min=${result.min.toFixed(2)}ms median=${result.median.toFixed(2)}ms mean=${result.mean.toFixed(2)}ms result=${formatTuple(result.result)}`,
  );
}

console.log(`speedup    ${(sequentialResult.mean / workersResult.mean).toFixed(2)}x by mean`);
console.log("Note: the worker build prewarms pooled workers before sampling and tears them down afterward.");
