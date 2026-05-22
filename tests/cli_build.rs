use std::fs;
use std::process::Command;

fn flowarrow() -> &'static str {
    env!("CARGO_BIN_EXE_flowarrow")
}

#[test]
fn build_accepts_target_option_before_path() {
    let path = temp_flow_path("flowarrow-build-target");
    fs::write(
        &path,
        r#"
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                0 -> $exit_code
            }
        "#,
    )
    .expect("write source");

    let output = Command::new(flowarrow())
        .args([
            "build",
            "--target",
            "wasm32-wasi",
            path.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run flowarrow build");
    fs::remove_file(&path).expect("remove temp source");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("build target `wasm32-wasi` is recognized"));
    assert!(stderr.contains("only `wasm32-unknown-unknown` is implemented"));
}

#[test]
fn build_accepts_optimization_and_extra_compiler_flags() {
    let path = temp_flow_path("flowarrow-build-flags");
    fs::write(
        &path,
        r#"
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                0 -> $exit_code
            }
        "#,
    )
    .expect("write source");

    let output = Command::new(flowarrow())
        .args([
            "build",
            "-O2",
            "--compiler-flag",
            "-fno-builtin",
            path.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run flowarrow build");
    fs::remove_file(&path).expect("remove temp source");

    assert!(
        output.status.success(),
        "flowarrow build failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn build_wasm_fib_example_and_run_node_script() {
    let output = Command::new(flowarrow())
        .args([
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "--crate-type",
            "cdylib",
            "examples/wasm-fib/fib.flow",
        ])
        .output()
        .expect("run flowarrow build");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("WASM backend requires `wasm-ld`")
            || stderr.contains("failed to initialize target `wasm32-unknown-unknown`")
        {
            eprintln!("skipping WASM example test: {stderr}");
            return;
        }
    }
    assert!(
        output.status.success(),
        "flowarrow build failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping WASM example runtime test: node is not installed");
        return;
    }

    let output = Command::new("node")
        .arg("examples/wasm-fib/run.mjs")
        .output()
        .expect("run node example");
    assert!(
        output.status.success(),
        "node example failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "55");
}

#[test]
fn build_typescript_fib_example_and_typecheck_output() {
    let output = Command::new(flowarrow())
        .args([
            "build",
            "--target",
            "typescript",
            "--crate-type",
            "cdylib",
            "examples/typescript-fib/fib.flow",
        ])
        .output()
        .expect("run flowarrow build");
    assert!(
        output.status.success(),
        "flowarrow build failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let generated_ts = fs::read_to_string("examples/typescript-fib/build/typescript/fib.ts")
        .expect("read generated TypeScript");
    assert!(generated_ts.contains("export function fib(depth: bigint): bigint"));
    assert!(generated_ts.contains("function _fib_step(a: bigint, b: bigint)"));
    assert!(generated_ts.contains("let result: bigint = 0n"));
    assert!(generated_ts.contains("[result, t0] = _fib_step(result, t0)"));
    assert!(generated_ts.contains("return result;"));
    assert!(generated_ts.contains("return [b, a + b];"));
    assert!(!generated_ts.contains("arguments["));
    assert!(!generated_ts.contains("...t0"));
    assert!(!generated_ts.contains("return t0[0]"));
    assert!(!generated_ts.contains("input: {"));
    assert!(!generated_ts.contains(".f0"));
    assert!(generated_ts.contains("for (let"));
    assert!(!generated_ts.contains("faParseReal"));
    assert!(!generated_ts.contains("FaArgs"));
    assert!(!generated_ts.contains("FaFaultable"));

    if Command::new("tsc").arg("--version").output().is_ok() {
        let output = Command::new("tsc")
            .args([
                "--noEmit",
                "--target",
                "ES2022",
                "--module",
                "NodeNext",
                "--moduleResolution",
                "NodeNext",
                "examples/typescript-fib/build/typescript/fib.ts",
            ])
            .output()
            .expect("run tsc");
        assert!(
            output.status.success(),
            "generated TypeScript failed typecheck:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    } else {
        eprintln!("skipping TypeScript typecheck: tsc is not installed");
    }
}

#[test]
fn build_javascript_fib_example_and_run_node_script() {
    let output = Command::new(flowarrow())
        .args([
            "build",
            "--target",
            "javascript",
            "--crate-type",
            "cdylib",
            "examples/typescript-fib/fib.flow",
        ])
        .output()
        .expect("run flowarrow build");
    assert!(
        output.status.success(),
        "flowarrow build failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let generated = fs::read_to_string("examples/typescript-fib/build/javascript/fib.mjs")
        .expect("read generated JavaScript");
    let generated_dts = fs::read_to_string("examples/typescript-fib/build/javascript/fib.d.ts")
        .expect("read generated JavaScript declarations");
    assert!(generated.contains("export function fib(depth)"));
    assert!(generated.contains("function _fib_step(a, b)"));
    assert!(generated.contains("let result = 0n"));
    assert!(generated.contains("[result, t0] = _fib_step(result, t0)"));
    assert!(generated.contains("return result;"));
    assert!(generated.contains("return [b, a + b];"));
    assert!(!generated.contains("arguments["));
    assert!(!generated.contains("...t0"));
    assert!(!generated.contains("return t0[0]"));
    assert!(!generated.contains(".f0"));
    assert!(!generated.contains(": bigint"));
    assert!(!generated.contains("faParseReal"));
    assert!(generated_dts.contains("export declare function fib(depth: bigint): bigint"));
    assert!(!generated_dts.contains("FaArgs"));

    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping JavaScript example runtime test: node is not installed");
        return;
    }

    let script = r#"
        const fs = require("node:fs");
        const source = fs.readFileSync("examples/typescript-fib/build/javascript/fib.mjs", "utf8");
        import("data:text/javascript," + encodeURIComponent(source))
          .then((mod) => console.log(mod.fib(10n).toString()));
    "#;
    let output = Command::new("node")
        .args(["-e", script])
        .output()
        .expect("run node JavaScript example");
    assert!(
        output.status.success(),
        "node JavaScript example failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "55");
}

#[test]
fn build_javascript_program_with_core_stdlib_and_run_node() {
    let output = Command::new(flowarrow())
        .args([
            "build",
            "--target",
            "javascript",
            "examples/add-numbers-from-args/main.flow",
        ])
        .output()
        .expect("run flowarrow build");
    assert!(
        output.status.success(),
        "flowarrow build failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let generated_dts =
        fs::read_to_string("examples/add-numbers-from-args/build/javascript/main.d.ts")
            .expect("read generated JavaScript declarations");
    assert!(generated_dts.contains("type FaArgs"));
    assert!(!generated_dts.contains("export type FaArgs"));

    if Command::new("tsc").arg("--version").output().is_ok() {
        let output = Command::new("tsc")
            .args([
                "--noEmit",
                "--target",
                "ES2022",
                "--module",
                "ES2022",
                "examples/add-numbers-from-args/build/javascript/main.d.ts",
            ])
            .output()
            .expect("run tsc");
        assert!(
            output.status.success(),
            "generated program TypeScript failed typecheck:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    } else {
        eprintln!("skipping TypeScript program typecheck: tsc is not installed");
    }

    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping TypeScript program runtime test: node is not installed");
        return;
    }

    let output = Command::new("node")
        .args([
            "examples/add-numbers-from-args/build/javascript/main.mjs",
            "1.5",
            "2.5",
            "3",
        ])
        .output()
        .expect("run node TypeScript program");
    assert!(
        output.status.success(),
        "node TypeScript program failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "7");
}

#[test]
fn build_javascript_with_worker_concurrency_and_run_node_workers() {
    let output = Command::new(flowarrow())
        .args([
            "build",
            "--target",
            "javascript",
            "--crate-type",
            "cdylib",
            "--workers",
            "examples/concurrency/main.flow",
        ])
        .output()
        .expect("run flowarrow build");
    assert!(
        output.status.success(),
        "flowarrow build failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let generated = fs::read_to_string("examples/concurrency/build/javascript/main.mjs")
        .expect("read generated JavaScript");
    let declarations = fs::read_to_string("examples/concurrency/build/javascript/main.d.ts")
        .expect("read generated JavaScript declarations");
    let worker = fs::read_to_string("examples/concurrency/build/javascript/main.worker.mjs")
        .expect("read generated JavaScript worker");
    assert!(generated.contains("new runtime.Worker(runtime.workerUrl, { type: \"module\" })"));
    assert!(!generated.contains("new workerGlobals.Blob"));
    assert!(generated.contains("export async function __flowarrow_setup_workers"));
    assert!(generated.contains("export async function __flowarrow_teardown_workers"));
    assert!(generated.contains("const faScalarWorkerPools = new Map"));
    assert!(generated.contains("faUseSharedNumericSequences = true"));
    assert!(generated.contains("function faScalarInputBuffer"));
    assert!(generated.contains("node:worker_threads"));
    assert!(generated.contains("new runtime.Worker(new URL(runtime.workerUrl)"));
    assert!(generated.contains("execArgv: []"));
    assert!(generated.contains("new URL(\"./main.worker.mjs\", import.meta.url).href"));
    assert!(generated.contains("const __flowarrow_worker_mapper_ids"));
    assert!(worker.contains("const faScalarWorkerMappers = new Map"));
    assert!(worker.contains("const weight ="));
    assert!(!worker.contains("eval("));
    assert!(!generated.contains("eval("));
    assert!(!generated.contains("eval: true"));
    assert!(generated.contains("SharedArrayBuffer"));
    assert!(generated.contains("faParallelMapBigInt"));
    assert!(declarations.contains("export declare function main(args: FaArgs): Promise<bigint>"));
    assert!(
        declarations.contains("export declare function __flowarrow_setup_workers(): Promise<void>")
    );
    assert!(
        declarations
            .contains("export declare function __flowarrow_teardown_workers(): Promise<void>")
    );

    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping JavaScript worker test: node is not installed");
        return;
    }

    let script = r#"
        const { pathToFileURL } = require("node:url");
        import(pathToFileURL("examples/concurrency/build/javascript/main.mjs").href).then(async (mod) => {
          const realProcess = globalThis.process;
          const realWorkerThreads = realProcess.getBuiltinModule?.("node:worker_threads") ?? require("node:worker_threads");
          let workerStarts = 0;
          class CountingWorker extends realWorkerThreads.Worker {
            constructor(...args) {
              workerStarts += 1;
              super(...args);
            }
          }
          let stdout = "";
          globalThis.process = {
            argv: [],
            versions: realProcess.versions,
            getBuiltinModule: (name) => name === "node:worker_threads"
              ? { ...realWorkerThreads, Worker: CountingWorker }
              : realProcess.getBuiltinModule?.(name),
            stdout: { write: (bytes) => { stdout += bytes; } },
          };
          await mod.__flowarrow_setup_workers();
          const startsAfterSetup = workerStarts;
          const code = await mod.main({ argv: [] });
          const startsAfterFirstCall = workerStarts;
          const secondCode = await mod.main({ argv: [] });
          const startsAfterSecondCall = workerStarts;
          await mod.__flowarrow_teardown_workers();
          console.log(String(code));
          console.log(String(secondCode));
          console.log(stdout);
          console.log(`starts after setup: ${startsAfterSetup}`);
          console.log(`starts after first call: ${startsAfterFirstCall}`);
          console.log(`starts after second call: ${startsAfterSecondCall}`);
          console.log(`worker starts: ${workerStarts}`);
        });
    "#;
    let output = Command::new("node")
        .args(["--input-type=commonjs", "-e", script])
        .output()
        .expect("run node JavaScript worker example");
    assert!(
        output.status.success(),
        "node JavaScript worker example failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "0\njobs: 16\ntotal score: 1632\npeak score: 272\ntotal weight: 288\npeak weight: 33\n"
    ));
    assert!(stdout.contains("0\n0\njobs: 16"));
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("worker starts: ") && !line.ends_with(": 0")),
        "expected generated JavaScript to start node worker_threads, got:\n{stdout}"
    );
    let starts_after_setup = stdout
        .lines()
        .find_map(|line| line.strip_prefix("starts after setup: "))
        .and_then(|value| value.parse::<usize>().ok())
        .expect("read starts after setup");
    let starts_after_first_call = stdout
        .lines()
        .find_map(|line| line.strip_prefix("starts after first call: "))
        .and_then(|value| value.parse::<usize>().ok())
        .expect("read starts after first call");
    let starts_after_second_call = stdout
        .lines()
        .find_map(|line| line.strip_prefix("starts after second call: "))
        .and_then(|value| value.parse::<usize>().ok())
        .expect("read starts after second call");
    assert!(starts_after_setup > 0);
    assert_eq!(starts_after_first_call, starts_after_setup);
    assert_eq!(starts_after_second_call, starts_after_setup);
}

#[test]
fn typescript_concurrency_benchmark_example_builds_and_runs() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping TypeScript concurrency benchmark example: node is not installed");
        return;
    }
    if Command::new("tsc").arg("--version").output().is_err() {
        eprintln!("skipping TypeScript concurrency benchmark example: tsc is not installed");
        return;
    }

    let build = Command::new("node")
        .arg("examples/typescript-concurrency-benchmark/build.mjs")
        .env("FLOWARROW_BIN", flowarrow())
        .output()
        .expect("build TypeScript concurrency benchmark example");
    assert!(
        build.status.success(),
        "benchmark build failed:\n{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let sequential_ts = fs::read_to_string(
        "examples/typescript-concurrency-benchmark/build/benchmark/sequential/bench.mts",
    )
    .expect("read sequential TypeScript benchmark build");
    let workers_ts = fs::read_to_string(
        "examples/typescript-concurrency-benchmark/build/benchmark/workers/bench.mts",
    )
    .expect("read worker TypeScript benchmark build");
    assert!(sequential_ts.contains("export function score_batch"));
    assert!(!sequential_ts.contains("new Worker"));
    assert!(workers_ts.contains("export async function score_batch"));
    assert!(workers_ts.contains("export async function __flowarrow_setup_workers"));
    assert!(workers_ts.contains("export async function __flowarrow_teardown_workers"));
    assert!(workers_ts.contains("new runtime.Worker(runtime.workerUrl, { type: \"module\" })"));
    assert!(workers_ts.contains("faUseSharedNumericSequences = true"));
    assert!(workers_ts.contains("function faScalarInputBuffer"));
    assert!(workers_ts.contains("node:worker_threads"));
    assert!(workers_ts.contains("new runtime.Worker(new URL(runtime.workerUrl)"));
    assert!(workers_ts.contains("execArgv: []"));
    assert!(workers_ts.contains("new URL(\"./bench.worker.mjs\", import.meta.url).href"));
    assert!(workers_ts.contains("const __flowarrow_worker_mapper_ids"));
    assert!(!workers_ts.contains("eval("));
    assert!(!workers_ts.contains("eval: true"));
    assert!(workers_ts.contains("faScalarWorkerPools"));
    assert!(workers_ts.contains("SharedArrayBuffer"));

    let run = Command::new("node")
        .arg("examples/typescript-concurrency-benchmark/run.mjs")
        .env("WIDTH", "10000")
        .env("WARMUPS", "1")
        .env("ITERATIONS", "2")
        .output()
        .expect("run TypeScript concurrency benchmark example");
    assert!(
        run.status.success(),
        "benchmark run failed:\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("sequential"));
    assert!(stdout.contains("workers"));
    assert!(stdout.contains("result=[333333330000, 99990000, 99999999, 19999]"));
}

fn temp_flow_path(prefix: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "{prefix}-{}-{}.flow",
        std::process::id(),
        unique_id()
    ));
    path
}

fn unique_id() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos()
}
