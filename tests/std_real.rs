mod support;

#[test]
fn std_real_nodes_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.io { write_stdout }
        import std.math { add }
        import std.real { parse_real, format_real }

        program main(args: Args) -> exit_code: Faultable[Int] {
            "2.5" -> parse_real -> $parsed
            ($parsed, 0.5) -> add -> $value
            $value -> format_real -> $text
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
