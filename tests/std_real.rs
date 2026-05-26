mod support;

#[test]
fn std_real_nodes_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.io { write_stdout }
        import std.math { add_f64 as add }
        import std.real { parse_real, format_real, from_int }

        program main(args: Args) -> exit_code: Faultable[i64] {
            "2.5" -> parse_real -> $parsed
            ($parsed, 0.5) -> add -> $value
            0 -> from_int -> $zero
            ($value, $zero) -> add -> $same_value
            $same_value -> format_real -> $text
            [$text, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("real", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "3\n");
}

#[test]
fn std_real_parse_edges_and_faults_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.io { write_stdout }
        import std.real { parse_real, format_real }

        program main(args: Args) -> exit_code: Faultable[i64] {
            " 2 " -> parse_real -> format_real -> $whole
            "-3.25" -> parse_real -> format_real -> $negative
            [$whole, ":", $negative, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("real-edges", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "2:-3.25\n");

    let fault_source = r#"
        import std.cli { Args }
        import std.fault { expect }
        import std.real { parse_real }

        program main(args: Args) -> exit_code: i64 {
            "bad" -> parse_real -> expect -> $bad
            0 -> $exit_code
        }
    "#;
    let output = support::run_source("real-parse-fault", fault_source, b"");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("expected f64"), "stderr was: {stderr}");
}
