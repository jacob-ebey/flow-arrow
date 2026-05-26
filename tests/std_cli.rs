mod support;

use std::process::Command;

#[test]
fn std_cli_args_type_runs() {
    let source = r#"
        import std.cli { Args }

        program main(args: Args) -> exit_code: i64 {
            0 -> $exit_code
        }
    "#;

    let output = support::run_source("cli-args", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn std_cli_argv_runs() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args, argv }
        import std.io { write_stdout }

        program main(args: Args) -> exit_code: i64 {
            $args -> argv -> $raw_args
            $raw_args -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("cli-argv", source);
    let output = Command::new(&build.executable)
        .args(["one", "two", "three"])
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "onetwothree"
    );
}

#[test]
fn std_cli_flag_helpers_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args, flag_present, flag_value }
        import std.io { write_stdout }

        program main(args: Args) -> exit_code: Faultable[i64] {
            ($args, "--verbose") -> flag_present -> $verbose
            ($args, "--name") -> flag_value -> $name
            ($verbose, "verbose", "quiet") -> select -> $mode
            [$mode, ":", $name, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("cli-flags", source);
    let output = Command::new(&build.executable)
        .args(["--name=ada", "--verbose"])
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "verbose:ada\n"
    );
}

#[test]
fn std_cli_flag_value_accepts_separate_value_and_missing_faults() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args, flag_present, flag_value }
        import std.fault { expect }
        import std.io { write_stdout }

        program main(args: Args) -> exit_code: i64 {
            ($args, "--verbose") -> flag_present -> $verbose
            ($args, "--missing") -> flag_present -> $missing
            ($args, "--name") -> flag_value -> expect -> $name
            ($verbose, "verbose", "quiet") -> select -> $mode
            ($missing, "bad", "absent") -> select -> $missing_text
            [$mode, ":", $missing_text, ":", $name, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("cli-flags-separate", source);
    let output = Command::new(&build.executable)
        .args(["--verbose", "--name", "ada"])
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "verbose:absent:ada\n"
    );

    let missing = Command::new(&build.executable)
        .args(["--verbose"])
        .output()
        .expect("run missing");
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("flag_value"), "stderr was: {stderr}");
}
