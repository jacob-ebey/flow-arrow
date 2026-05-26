mod support;

#[test]
fn std_math_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.fault { expect }
        import std.math { add, sub, mul, div, rem, neg, abs, sqrt, eq, lt, gt, le, ge, min, max }

        program main(args: Args) -> exit_code: Int {
            (2, 3) -> add -> $five_a
            ($five_a, 5) -> eq -> $add_ok
            (8, 3) -> sub -> $five_b
            ($five_b, 5) -> eq -> $sub_ok
            (3, 4) -> mul -> $twelve
            ($twelve, 12) -> eq -> $mul_ok
            (10, 3) -> div -> expect -> $three
            ($three, 3) -> eq -> $div_ok
            (10, 3) -> rem -> expect -> $one
            ($one, 1) -> eq -> $rem_ok
            5 -> neg -> $minus_five
            ($minus_five, -5) -> eq -> $neg_ok
            -8 -> abs -> $eight
            ($eight, 8) -> eq -> $abs_ok
            81 -> sqrt -> expect -> $nine
            ($nine, 9.0) -> eq -> $sqrt_ok
            (2, 3) -> lt -> $lt_ok
            (3, 2) -> gt -> $gt_ok
            (3, 3) -> le -> $le_ok
            (3, 3) -> ge -> $ge_ok
            (7, 4) -> min -> $four
            ($four, 4) -> eq -> $min_ok
            (7, 4) -> max -> $seven
            ($seven, 7) -> eq -> $max_ok

            ($add_ok, $sub_ok, false) -> select -> $s1
            ($s1, $mul_ok, false) -> select -> $s2
            ($s2, $div_ok, false) -> select -> $s3
            ($s3, $rem_ok, false) -> select -> $s4
            ($s4, $neg_ok, false) -> select -> $s5
            ($s5, $abs_ok, false) -> select -> $s6
            ($s6, $sqrt_ok, false) -> select -> $s7
            ($s7, $lt_ok, false) -> select -> $s8
            ($s8, $gt_ok, false) -> select -> $s9
            ($s9, $le_ok, false) -> select -> $s10
            ($s10, $ge_ok, false) -> select -> $s11
            ($s11, $min_ok, false) -> select -> $s12
            ($s12, $max_ok, false) -> select -> $all_ok
            ($all_ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("math", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn std_math_real_functions_and_usage_faults_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { cos, eq, exp, sin }

        program main(args: Args) -> exit_code: Int {
            0 -> sin -> $sin_zero
            ($sin_zero, 0.0) -> eq -> $sin_ok
            0 -> cos -> $cos_zero
            ($cos_zero, 1.0) -> eq -> $cos_ok
            0 -> exp -> $exp_zero
            ($exp_zero, 1.0) -> eq -> $exp_ok
            ($sin_ok, $cos_ok, false) -> select -> $s1
            ($s1, $exp_ok, false) -> select -> $all_ok
            ($all_ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("math-real-functions", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn std_math_invalid_numeric_inputs_are_reported() {
    for (name, source, expected) in [
        (
            "math-div-zero",
            r#"
                import std.cli { Args }
                import std.math { div }

                program main(args: Args) -> exit_code: Faultable[Int] {
                    (1, 0) -> div -> $exit_code
                }
            "#,
            "div: division by zero",
        ),
        (
            "math-rem-zero",
            r#"
                import std.cli { Args }
                import std.math { rem }

                program main(args: Args) -> exit_code: Faultable[Int] {
                    (1, 0) -> rem -> $exit_code
                }
            "#,
            "rem: remainder by zero",
        ),
        (
            "math-add-overflow",
            r#"
                import std.cli { Args }
                import std.fault { expect }
                import std.int { parse_int }
                import std.math { add }

                program main(args: Args) -> exit_code: Int {
                    "9223372036854775807" -> parse_int -> expect -> $max
                    ($max, 1) -> add -> $exit_code
                }
            "#,
            "add: integer overflow",
        ),
        (
            "math-neg-overflow",
            r#"
                import std.cli { Args }
                import std.fault { expect }
                import std.int { parse_int }
                import std.math { neg }

                program main(args: Args) -> exit_code: Int {
                    "-9223372036854775808" -> parse_int -> expect -> neg -> $exit_code
                }
            "#,
            "neg: integer overflow",
        ),
        (
            "math-abs-overflow",
            r#"
                import std.cli { Args }
                import std.fault { expect }
                import std.int { parse_int }
                import std.math { abs }

                program main(args: Args) -> exit_code: Int {
                    "-9223372036854775808" -> parse_int -> expect -> abs -> $exit_code
                }
            "#,
            "abs: integer overflow",
        ),
        (
            "math-sqrt-negative",
            r#"
                import std.cli { Args }
                import std.math { sqrt }
                import std.real { format_real }
                import std.io { write_stdout }

                program main(args: Args) -> exit_code: Faultable[Int] {
                    -1.0 -> sqrt -> format_real -> write_stdout -> $exit_code
                }
            "#,
            "sqrt: negative input",
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

#[test]
fn std_math_invalid_inputs_are_recoverable_with_fault_map() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.fault { format_faults, has_faults }
        import std.int { format_int }
        import std.io { write_stderr, write_stdout }
        import std.math { div, sqrt }
        import std.seq { length }

        program main(args: Args) -> exit_code: Int {
            [(1, 0), (6, 2), (8, 0)] -> fault map div { ok -> $quotients, fault -> $div_faults }
            [-1.0, 4.0] -> fault map sqrt { ok -> $roots, fault -> $sqrt_faults }

            $quotients -> length -> format_int -> $quotient_count
            $roots -> length -> format_int -> $root_count
            ["ok:", $quotient_count, ":", $root_count, "\n"] -> concat_bytes -> write_stdout -> $stdout_status

            $div_faults -> format_faults -> $div_messages
            $sqrt_faults -> format_faults -> $sqrt_messages
            [$div_messages, "\n", $sqrt_messages, "\n"] -> concat_bytes -> write_stderr -> $stderr_status

            $div_faults -> has_faults -> $has_div_faults
            $sqrt_faults -> has_faults -> $has_sqrt_faults
            ($has_div_faults, $has_sqrt_faults, false) -> select -> $captured
            ($captured, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("math-recoverable-fault-map", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "ok:1:1\n");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(
        stderr.contains("div: division by zero"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("sqrt: negative input"),
        "stderr was: {stderr}"
    );
}
