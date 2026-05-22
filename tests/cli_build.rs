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

    let generated = fs::read_to_string("examples/typescript-fib/build/javascript/fib.js")
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
        const source = fs.readFileSync("examples/typescript-fib/build/javascript/fib.js", "utf8");
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
            "examples/add-numbers-from-args/build/javascript/main.js",
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
