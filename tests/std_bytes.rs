mod support;

#[test]
fn std_bytes_nodes_run() {
    let source = r#"
        import std.bytes { split_lines, join_bytes, concat_bytes }
        import std.cli { Args }
        import std.io { read_stdin, write_stdout }

        program main(args: Args) -> exit_code: Int {
            () -> read_stdin -> $input
            $input -> split_lines -> $lines
            ($lines, "|") -> join_bytes -> $joined
            [$joined, "\n", "done\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
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

#[test]
fn std_bytes_trim_split_on_strip_round_trip() {
    let source = r#"
        import std.bytes { concat_bytes, join_bytes, split_on, strip_prefix, strip_suffix, trim }
        import std.cli { Args }
        import std.io { read_stdin, write_stdout }

        program main(args: Args) -> exit_code: Faultable[Int] {
            () -> read_stdin -> $input
            $input -> trim -> $framed
            ($framed, "[") -> strip_prefix -> $after_open
            ($after_open, "]") -> strip_suffix -> $inner
            ($inner, ",") -> split_on -> $raw_tokens
            $raw_tokens -> map trim -> $tokens
            ($tokens, "|") -> join_bytes -> $joined
            [$joined, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("bytes_new", source, b"  [ a , b , c ]\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "a|b|c\n");
}

#[test]
fn std_bytes_strip_prefix_fault_propagates() {
    let source = r#"
        import std.bytes { concat_bytes, strip_prefix }
        import std.cli { Args }
        import std.io { read_stdin, write_stdout }

        program main(args: Args) -> exit_code: Faultable[Int] {
            () -> read_stdin -> $input
            ($input, "[") -> strip_prefix -> $inner
            [$inner, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("bytes_strip_fault", source, b"oops");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("strip_prefix"), "stderr was: {stderr}");
}

#[test]
fn std_bytes_expanded_text_helpers_run() {
    let source = r#"
        import std.bytes {
            ascii_lower,
            ascii_upper,
            concat_bytes,
            contains,
            drop,
            ends_with,
            index_of,
            last_index_of,
            replace,
            repeat_bytes,
            slice,
            starts_with,
            take,
        }
        import std.cli { Args }
        import std.int { format_int }
        import std.io { write_stdout }

        program main(args: Args) -> exit_code: Int {
            "Hello Flow Flow" -> ascii_lower -> $lower
            $lower -> ascii_upper -> $upper
            ($lower, "hello") -> starts_with -> $starts
            ($lower, "flow") -> ends_with -> $ends
            ($lower, "flow") -> contains -> $has
            ($lower, "flow") -> index_of -> format_int -> $first
            ($lower, "flow") -> last_index_of -> format_int -> $last
            ($lower, 6, 10) -> slice -> $middle
            ($lower, 5) -> take -> $front
            ($lower, 6) -> drop -> $rest
            ($lower, "flow", "arrow") -> replace -> $replaced
            ("ha", 3) -> repeat_bytes -> $laugh
            ($starts, "S", "s") -> select -> $starts_text
            ($ends, "E", "e") -> select -> $ends_text
            ($has, "H", "h") -> select -> $has_text
            [$upper, "\n", $starts_text, $ends_text, $has_text, ":", $first, ":", $last, ":", $middle, ":", $front, ":", $rest, ":", $replaced, ":", $laugh, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("bytes-expanded", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "HELLO FLOW FLOW\nSEH:6:11:flow:hello:flow flow:hello arrow arrow:hahaha\n"
    );
}

#[test]
fn std_bytes_search_and_slice_edge_cases_run() {
    let source = r#"
        import std.bytes {
            concat_bytes,
            contains,
            drop,
            ends_with,
            index_of,
            join_bytes,
            last_index_of,
            repeat_bytes,
            slice,
            starts_with,
            take,
        }
        import std.cli { Args }
        import std.int { format_int }
        import std.io { write_stdout }

        program main(args: Args) -> exit_code: Int {
            ("abc", "") -> starts_with -> $starts_empty
            ("abc", "") -> ends_with -> $ends_empty
            ("abc", "") -> contains -> $contains_empty
            ("abc", "") -> index_of -> format_int -> $empty_first
            ("abc", "") -> last_index_of -> format_int -> $empty_last
            ("abc", "z") -> index_of -> format_int -> $missing_first
            ("abc", "z") -> last_index_of -> format_int -> $missing_last
            ("abc", 20) -> take -> $take_large
            ("abc", 20) -> drop -> $drop_large
            ("abc", 1, 1) -> slice -> $empty_slice
            ("x", 0) -> repeat_bytes -> $repeat_zero
            [$starts_empty, $ends_empty, $contains_empty] -> map bool_text -> $bools
            ($bools, "") -> join_bytes -> $bool_text
            [$bool_text, ":", $empty_first, ":", $empty_last, ":", $missing_first, ":", $missing_last, ":", $take_large, ":", $drop_large, ":", $empty_slice, ":", $repeat_zero, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }

        node bool_text(value: Bool) -> out: Bytes {
            ($value, "1", "0") -> select -> $out
        }
    "#;

    let output = support::run_source("bytes-edges", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "111:0:3:-1:-1:abc:::\n"
    );
}

#[test]
fn std_bytes_invalid_ranges_and_empty_needles_fault() {
    for (name, source, expected) in [
        (
            "bytes-slice-range-fault",
            r#"
                import std.bytes { slice }
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    ("abc", 2, 1) -> slice -> $bad
                    0 -> $exit_code
                }
            "#,
            "bytes.slice: index out of range",
        ),
        (
            "bytes-split-empty-fault",
            r#"
                import std.bytes { split_on }
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    ("abc", "") -> split_on -> $bad
                    0 -> $exit_code
                }
            "#,
            "split_on: delimiter cannot be empty",
        ),
        (
            "bytes-replace-empty-fault",
            r#"
                import std.bytes { replace }
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    ("abc", "", "x") -> replace -> $bad
                    0 -> $exit_code
                }
            "#,
            "replace: needle cannot be empty",
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
