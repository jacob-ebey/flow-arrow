use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

const UNFORMATTED: &str = r#"program   main( args:Args)->exit_code:i64{0->$exit_code}"#;
const FORMATTED: &str = r#"program main(args: Args) -> exit_code: i64 {
    0 -> $exit_code
}
"#;

fn flowarrow() -> &'static str {
    env!("CARGO_BIN_EXE_flowarrow")
}

#[test]
fn fmt_stdin_formats_source_to_stdout() {
    let mut child = Command::new(flowarrow())
        .args(["fmt", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn flowarrow fmt");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(UNFORMATTED.as_bytes())
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for flowarrow fmt");
    assert!(
        output.status.success(),
        "fmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), FORMATTED);
}

#[test]
fn fmt_path_still_formats_file_in_place() {
    let path = temp_flow_path("flowarrow-fmt-path");
    fs::write(&path, UNFORMATTED).expect("write source");

    let output = Command::new(flowarrow())
        .args(["fmt", path.to_str().expect("utf8 path")])
        .output()
        .expect("run flowarrow fmt");

    let formatted = fs::read_to_string(&path).expect("read formatted source");
    fs::remove_file(&path).expect("remove temp source");

    assert!(
        output.status.success(),
        "fmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(formatted, FORMATTED);
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
