mod support;

#[test]
fn std_int_nodes_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.int { parse_int, format_int }
        import std.io { write_stdout }
        import std.math { add }

        program main(args: Args) -> exit_code: Faultable[Int] {
            "41" -> parse_int -> parsed
            (parsed, 1) -> add -> value
            value -> format_int -> text
            [text, "\n"] -> concat_bytes -> output
            output -> write_stdout -> exit_code
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
