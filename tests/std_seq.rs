mod support;

#[test]
fn std_seq_expanded_helpers_run() {
    let source = r#"
        import std.bytes { concat_bytes, join_bytes }
        import std.cli { Args }
        import std.io { write_stdout }
        import std.seq { drop, fill, is_empty, reverse, take }

        program main(args: Args) -> exit_code: i64 {
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

#[test]
fn std_seq_index_update_and_concat_helpers_run() {
    let source = r#"
        import std.bytes { concat_bytes, join_bytes }
        import std.cli { Args }
        import std.fault { expect }
        import std.int { format_int }
        import std.io { write_stdout }
        import std.seq { append, at, concat, get, get_or, last, set, slice }

        program main(args: Args) -> exit_code: i64 {
            ["a", "b", "c"] -> $items
            ($items, 1) -> get -> $got
            ($items, 20, "fallback") -> get_or -> $fallback
            ($items, 2) -> at -> expect -> $at
            $items -> last -> expect -> $last
            ($items, 1, "B") -> set -> $set
            ($set, "d") -> append -> $appended
            ($appended, 1, 3) -> slice -> $middle
            ($middle, ["x", "y"]) -> concat -> $joined_items
            ($joined_items, "") -> join_bytes -> $joined
            [$got, ":", $fallback, ":", $at, ":", $last, ":", $joined, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("seq-index-update", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "b:fallback:c:c:Bcxy\n"
    );
}

#[test]
fn std_seq_fault_and_usage_paths_are_reported() {
    for (name, source, expected) in [
        (
            "seq-head-empty-fault",
            r#"
                import std.fault { expect }
                import std.cli { Args }
                import std.seq { drop, head }

                program main(args: Args) -> exit_code: i64 {
                    (["x"], 1) -> drop -> head -> expect -> $bad
                    0 -> $exit_code
                }
            "#,
            "head: empty sequence",
        ),
        (
            "seq-get-range-fault",
            r#"
                import std.cli { Args }
                import std.seq { get }

                program main(args: Args) -> exit_code: i64 {
                    (["x"], 1) -> get -> $bad
                    0 -> $exit_code
                }
            "#,
            "get: index out of range",
        ),
        (
            "seq-take-negative-fault",
            r#"
                import std.cli { Args }
                import std.seq { take }

                program main(args: Args) -> exit_code: i64 {
                    (["x"], -1) -> take -> $bad
                    0 -> $exit_code
                }
            "#,
            "take: count must be non-negative",
        ),
        (
            "seq-zip-length-fault",
            r#"
                import std.cli { Args }
                import std.seq { zip }

                program main(args: Args) -> exit_code: i64 {
                    (["a"], ["b", "c"]) -> zip -> $bad
                    0 -> $exit_code
                }
            "#,
            "zip: sequences must have the same length",
        ),
    ] {
        let output = support::run_source(name, source, b"");
        assert!(!output.status.success(), "{name} unexpectedly succeeded");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected),
            "{name}: expected {expected:?}, stderr was: {stderr}"
        );
    }
}
