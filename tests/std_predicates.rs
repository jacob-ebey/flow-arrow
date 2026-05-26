mod support;

#[test]
fn std_predicate_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.predicates { not_empty, is_empty, and, or, xor, not, all, any }

        program main(args: Args) -> exit_code: i64 {
            "x" -> not_empty -> $not_empty_ok
            "" -> is_empty -> $is_empty_ok
            (true, true) -> and -> $and_ok
            (false, true) -> or -> $or_ok
            (true, false) -> xor -> $xor_ok
            false -> not -> $not_ok
            [true, true] -> all -> $all_ok
            [false, true] -> any -> $any_ok

            ($not_empty_ok, $is_empty_ok, false) -> select -> $s1
            ($s1, $and_ok, false) -> select -> $s2
            ($s2, $or_ok, false) -> select -> $s3
            ($s3, $xor_ok, false) -> select -> $s4
            ($s4, $not_ok, false) -> select -> $s5
            ($s5, $all_ok, false) -> select -> $s6
            ($s6, $any_ok, false) -> select -> $all_passed
            ($all_passed, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("predicates", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
