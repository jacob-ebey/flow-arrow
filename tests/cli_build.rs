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
    assert!(stderr.contains("WASM backend is not implemented yet"));
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
