mod support;

use std::process::Command;

#[test]
fn std_cli_args_type_runs() {
    let source = r#"
        import std.cli { Args }

        program main(args: Args) -> exit_code: Int {
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

        program main(args: Args) -> exit_code: Int {
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

        program main(args: Args) -> exit_code: Faultable[Int] {
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
