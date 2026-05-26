mod support;

#[test]
fn std_math_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.fault { expect }
        import std.math {
            add_i64 as add,
            sub_i64 as sub,
            mul_i64 as mul,
            div_i64 as div,
            rem_i64 as rem,
            neg_i64 as neg,
            abs_i64 as abs,
            sqrt_f64 as sqrt,
            eq_i64 as eq,
            eq_f64,
            lt_i64 as lt,
            gt_i64 as gt,
            le_i64 as le,
            ge_i64 as ge,
            min_i64 as min,
            max_i64 as max,
        }

        program main(args: Args) -> exit_code: i64 {
            (2, 3) -> add -> expect -> $five_a
            ($five_a, 5) -> eq -> $add_ok
            (8, 3) -> sub -> expect -> $five_b
            ($five_b, 5) -> eq -> $sub_ok
            (3, 4) -> mul -> expect -> $twelve
            ($twelve, 12) -> eq -> $mul_ok
            (10, 3) -> div -> expect -> $three
            ($three, 3) -> eq -> $div_ok
            (10, 3) -> rem -> expect -> $one
            ($one, 1) -> eq -> $rem_ok
            5 -> neg -> expect -> $minus_five
            ($minus_five, -5) -> eq -> $neg_ok
            -8 -> abs -> expect -> $eight
            ($eight, 8) -> eq -> $abs_ok
            81.0 -> sqrt -> expect -> $nine
            ($nine, 9.0) -> eq_f64 -> $sqrt_ok
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
fn std_math_rejects_removed_generic_numeric_exports() {
    let source = r#"
        import std.cli { Args }
        import std.math { add }

        program main(args: Args) -> exit_code: i64 {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("math-removed-generic-add");
    std::fs::write(&path, source).expect("write source");
    let error = flowarrow::typecheck_file(&path).expect_err("typecheck should fail");
    assert!(error.contains("module `std.math` does not export `add`"));
}

#[test]
fn std_math_real_functions_and_usage_faults_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { cos_f64 as cos, eq_f64 as eq, exp_f64 as exp, sin_f64 as sin }

        program main(args: Args) -> exit_code: i64 {
            0.0 -> sin -> $sin_zero
            ($sin_zero, 0.0) -> eq -> $sin_ok
            0.0 -> cos -> $cos_zero
            ($cos_zero, 1.0) -> eq -> $cos_ok
            0.0 -> exp -> $exp_zero
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
                import std.math { div_i64 as div }

                program main(args: Args) -> exit_code: Faultable[i64] {
                    (1, 0) -> div -> $exit_code
                }
            "#,
            "div: division by zero",
        ),
        (
            "math-rem-zero",
            r#"
                import std.cli { Args }
                import std.math { rem_i64 as rem }

                program main(args: Args) -> exit_code: Faultable[i64] {
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
                import std.math { add_i64 as add }

                program main(args: Args) -> exit_code: i64 {
                    "9223372036854775807" -> parse_int -> expect -> $max
                    ($max, 1) -> add -> expect -> $exit_code
                }
            "#,
            "add: integer overflow",
        ),
        (
            "math-reduce-add-overflow",
            r#"
                import std.cli { Args }
                import std.fault { expect }
                import std.int { parse_int }
                import std.math { add_i64 as add }

                program main(args: Args) -> exit_code: Faultable[i64] {
                    "9223372036854775807" -> parse_int -> expect -> $max
                    [$max, 1] -> reduce add(identity: 0) -> $exit_code
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
                import std.math { neg_i64 as neg }

                program main(args: Args) -> exit_code: i64 {
                    "-9223372036854775808" -> parse_int -> expect -> neg -> expect -> $exit_code
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
                import std.math { abs_i64 as abs }

                program main(args: Args) -> exit_code: i64 {
                    "-9223372036854775808" -> parse_int -> expect -> abs -> expect -> $exit_code
                }
            "#,
            "abs: integer overflow",
        ),
        (
            "math-sqrt-negative",
            r#"
                import std.cli { Args }
                import std.math { sqrt_f64 as sqrt }
                import std.real { format_real }
                import std.io { write_stdout }

                program main(args: Args) -> exit_code: Faultable[i64] {
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
        import std.fault { expect, format_faults, has_faults }
        import std.int { format_int, parse_int }
        import std.io { write_stderr, write_stdout }
        import std.math { abs_i64 as abs, add_i64 as add, div_i64 as div, sqrt_f64 as sqrt }
        import std.seq { length }

        program main(args: Args) -> exit_code: i64 {
            "9223372036854775807" -> parse_int -> expect -> $max
            "-9223372036854775808" -> parse_int -> expect -> $min
            [(1, 0), (6, 2), (8, 0)] -> fault map div { ok -> $quotients, fault -> $div_faults }
            [-1.0, 4.0] -> fault map sqrt { ok -> $roots, fault -> $sqrt_faults }
            [($max, 1), (5, 7)] -> fault map add { ok -> $sums, fault -> $add_faults }
            [$min, -7] -> fault map abs { ok -> $magnitudes, fault -> $abs_faults }

            $quotients -> length -> format_int -> $quotient_count
            $roots -> length -> format_int -> $root_count
            $sums -> length -> format_int -> $sum_count
            $magnitudes -> length -> format_int -> $magnitude_count
            ["ok:", $quotient_count, ":", $root_count, ":", $sum_count, ":", $magnitude_count, "\n"] -> concat_bytes -> write_stdout -> $stdout_status

            $div_faults -> format_faults -> $div_messages
            $sqrt_faults -> format_faults -> $sqrt_messages
            $add_faults -> format_faults -> $add_messages
            $abs_faults -> format_faults -> $abs_messages
            [$div_messages, "\n", $sqrt_messages, "\n", $add_messages, "\n", $abs_messages, "\n"] -> concat_bytes -> write_stderr -> $stderr_status

            $div_faults -> has_faults -> $has_div_faults
            $sqrt_faults -> has_faults -> $has_sqrt_faults
            $add_faults -> has_faults -> $has_add_faults
            $abs_faults -> has_faults -> $has_abs_faults
            ($has_div_faults, $has_sqrt_faults, false) -> select -> $captured0
            ($captured0, $has_add_faults, false) -> select -> $captured1
            ($captured1, $has_abs_faults, false) -> select -> $captured
            ($captured, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("math-recoverable-fault-map", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "ok:1:1:1:1\n"
    );
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(
        stderr.contains("div: division by zero"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("sqrt: negative input"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("add: integer overflow"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("abs: integer overflow"),
        "stderr was: {stderr}"
    );
}
