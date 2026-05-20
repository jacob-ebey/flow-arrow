mod support;

#[test]
fn std_bytes_nodes_run() {
    let source = r#"
        import std.bytes { split_lines, join_bytes, concat_bytes }
        import std.cli { Args }
        import std.io { read_stdin, write_stdout }

        program main(args: Args) -> exit_code: Int {
            () -> read_stdin -> input
            input -> split_lines -> lines
            (lines, "|") -> join_bytes -> joined
            [joined, "\n", "done\n"] -> concat_bytes -> output
            output -> write_stdout -> exit_code
        }
    "#;

    let output = support::run_source("bytes", source, b"a\r\nb\nc");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "a|b|c\ndone\n"
    );
}
