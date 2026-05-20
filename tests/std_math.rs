mod support;

#[test]
fn std_math_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { add, sub, mul, div, rem, eq, lt, gt, le, ge, max }

        program main(args: Args) -> exit_code: Int {
            (2, 3) -> add -> $five_a
            ($five_a, 5) -> eq -> $add_ok
            (8, 3) -> sub -> $five_b
            ($five_b, 5) -> eq -> $sub_ok
            (3, 4) -> mul -> $twelve
            ($twelve, 12) -> eq -> $mul_ok
            (10, 3) -> div -> $three
            ($three, 3) -> eq -> $div_ok
            (10, 3) -> rem -> $one
            ($one, 1) -> eq -> $rem_ok
            (2, 3) -> lt -> $lt_ok
            (3, 2) -> gt -> $gt_ok
            (3, 3) -> le -> $le_ok
            (3, 3) -> ge -> $ge_ok
            (7, 4) -> max -> $seven
            ($seven, 7) -> eq -> $max_ok

            ($add_ok, $sub_ok, false) -> select -> $s1
            ($s1, $mul_ok, false) -> select -> $s2
            ($s2, $div_ok, false) -> select -> $s3
            ($s3, $rem_ok, false) -> select -> $s4
            ($s4, $lt_ok, false) -> select -> $s5
            ($s5, $gt_ok, false) -> select -> $s6
            ($s6, $le_ok, false) -> select -> $s7
            ($s7, $ge_ok, false) -> select -> $s8
            ($s8, $max_ok, false) -> select -> $all_ok
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
