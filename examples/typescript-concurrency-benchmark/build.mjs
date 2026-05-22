import { spawnSync } from "node:child_process";
import { copyFile, mkdir, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const rootUrl = new URL("./", import.meta.url);
const root = fileURLToPath(rootUrl);
const flowarrow = process.env.FLOWARROW_BIN
  ? { command: process.env.FLOWARROW_BIN, prefix: [] }
  : {
      command: "cargo",
      prefix: ["run", "--quiet", "--manifest-path", "../../Cargo.toml", "--"],
    };

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
}

async function buildVariant(name, flags) {
  const tsDir = new URL(`build/benchmark/${name}/`, rootUrl);
  const jsDir = new URL(`build/benchmark/${name}-js/`, rootUrl);
  const generated = new URL("build/typescript/bench.ts", rootUrl);
  const target = new URL("bench.ts", tsDir);

  await rm(tsDir, { recursive: true, force: true });
  await rm(jsDir, { recursive: true, force: true });
  await mkdir(tsDir, { recursive: true });

  run(flowarrow.command, [
    ...flowarrow.prefix,
    "build",
    "--target",
    "typescript",
    "--crate-type",
    "cdylib",
    ...flags,
    "bench.flow",
  ]);
  await copyFile(generated, target);

  run("tsc", [
    "--target",
    "ES2022",
    "--module",
    "ESNext",
    "--moduleResolution",
    "bundler",
    "--outDir",
    fileURLToPath(jsDir),
    fileURLToPath(target),
  ]);
}

await buildVariant("sequential", []);
await buildVariant("workers", ["--workers"]);

console.log("Built sequential and worker-enabled TypeScript variants under build/benchmark/.");
