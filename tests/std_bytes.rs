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
