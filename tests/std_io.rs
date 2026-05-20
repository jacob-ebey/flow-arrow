mod support;

#[test]
fn std_io_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.io { read_stdin, write_stdout, write_stderr }
        import std.math { add }

        program main(args: Args) -> exit_code: Int {
            () -> read_stdin -> $input
            $input -> write_stdout -> $stdout_status
            "err\n" -> write_stderr -> $stderr_status
            ($stdout_status, $stderr_status) -> add -> $exit_code
        }
    "#;

    let output = support::run_source("io", source, b"out\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "out\n");
    assert_eq!(String::from_utf8(output.stderr).expect("utf8"), "err\n");
}
