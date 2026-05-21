mod support;

#[test]
fn std_seq_expanded_helpers_run() {
    let source = r#"
        import std.bytes { concat_bytes, join_bytes }
        import std.cli { Args }
        import std.io { write_stdout }
        import std.seq { drop, fill, is_empty, reverse, take }

        program main(args: Args) -> exit_code: Int {
            ["a", "b", "c", "d"] -> reverse -> $reversed
            ($reversed, 2) -> take -> $first_two
            ($reversed, 1) -> drop -> $without_first
            ("x", 3) -> fill -> $filled
            (["z"], 1) -> drop -> is_empty -> $empty
            ($empty, "empty", "not-empty") -> select -> $empty_text
            ($first_two, "") -> join_bytes -> $a
            ($without_first, "") -> join_bytes -> $b
            ($filled, "") -> join_bytes -> $c
            [$a, ":", $b, ":", $c, ":", $empty_text, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("seq-expanded", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "dc:cba:xxx:empty\n"
    );
}
