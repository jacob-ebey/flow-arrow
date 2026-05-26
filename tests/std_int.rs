mod support;

#[test]
fn std_int_nodes_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.int { parse_int, format_int }
        import std.io { write_stdout }
        import std.math { add }

        program main(args: Args) -> exit_code: Faultable[i64] {
            "41" -> parse_int -> $parsed
            ($parsed, 1) -> add -> $value
            $value -> format_int -> $text
            [$text, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("int", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "42\n");
}

#[test]
fn std_int_parse_edges_and_faults_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.int { parse_int, format_int }
        import std.io { write_stdout }
        import std.math { add }

        program main(args: Args) -> exit_code: Faultable[i64] {
            " -42 " -> parse_int -> $negative
            "0" -> parse_int -> $zero
            ($negative, $zero) -> add -> format_int -> $sum
            [$sum, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("int-edges", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "-42\n");

    let fault_source = r#"
        import std.cli { Args }
        import std.fault { expect }
        import std.int { parse_int }

        program main(args: Args) -> exit_code: i64 {
            "12x" -> parse_int -> expect -> $bad
            0 -> $exit_code
        }
    "#;
    let output = support::run_source("int-parse-fault", fault_source, b"");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("expected i64"), "stderr was: {stderr}");
}
