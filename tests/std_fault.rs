mod support;

#[test]
fn std_fault_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.fault { has_faults, format_faults }
        import std.io { write_stderr }
        import std.real { parse_real }

        program main(args: Args) -> exit_code: Int {
            ["1", "bad"] -> fault map parse_real { ok -> numbers, fault -> faults }
            faults -> has_faults -> invalid
            faults -> format_faults -> message
            message -> write_stderr -> stderr_status
            (invalid, stderr_status, 1) -> select -> exit_code
        }
    "#;

    let output = support::run_source("fault", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("utf8"),
        "line 2: expected Real, got \"bad\"\n"
    );
}
