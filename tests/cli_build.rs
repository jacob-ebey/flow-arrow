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
