mod support;

use flowarrow::build_file;
use std::fs;
use std::process::Command;

#[test]
fn std_vector_source_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq }
        import std.vector {
            sum,
            mean,
            neg as vector_neg,
            abs as vector_abs,
            add as vector_add,
            sub as vector_sub,
            mul as vector_mul,
            div as vector_div,
            equals as vector_equals,
            dot,
            squared_norm,
            l1_norm,
            norm,
            squared_distance,
            distance,
        }

        program main(args: Args) -> exit_code: Int {
            [1.0, 2.0, 3.5] -> sum -> $total
            ($total, 6.5) -> eq -> $sum_ok

            [2.0, 4.0, 6.0] -> mean -> $mean
            ($mean, 4.0) -> eq -> $mean_ok

            [1.0, -2.0, 3.0] -> vector_neg -> $negated
            ($negated, [-1.0, 2.0, -3.0]) -> vector_equals -> $neg_ok

            [-1.0, 2.0, -3.0] -> vector_abs -> $absolute
            ($absolute, [1.0, 2.0, 3.0]) -> vector_equals -> $abs_ok

            ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]) -> vector_add -> $added
            ($added, [5.0, 7.0, 9.0]) -> vector_equals -> $add_ok

            ([9.0, 8.0, 7.0], [1.0, 2.0, 3.0]) -> vector_sub -> $subbed
            ($subbed, [8.0, 6.0, 4.0]) -> vector_equals -> $sub_ok

            ([2.0, 3.0, 4.0], [5.0, 6.0, 7.0]) -> vector_mul -> $multiplied
            ($multiplied, [10.0, 18.0, 28.0]) -> vector_equals -> $mul_ok

            ([8.0, 9.0, 12.0], [2.0, 3.0, 4.0]) -> vector_div -> $divided
            ($divided, [4.0, 3.0, 3.0]) -> vector_equals -> $div_ok

            ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]) -> dot -> $dot_total
            ($dot_total, 32.0) -> eq -> $dot_ok

            [2.0, 3.0, 6.0] -> squared_norm -> $norm_squared
            ($norm_squared, 49.0) -> eq -> $norm_ok

            [-2.0, 3.0, -6.0] -> l1_norm -> $l1
            ($l1, 11.0) -> eq -> $l1_ok

            [3.0, 4.0] -> norm -> $norm
            ($norm, 5.0) -> eq -> $norm_sqrt_ok

            ([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]) -> squared_distance -> $distance_squared
            ($distance_squared, 25.0) -> eq -> $distance_ok

            ([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]) -> distance -> $distance
            ($distance, 5.0) -> eq -> $distance_sqrt_ok

            ($sum_ok, $mean_ok, false) -> select -> $s1
            ($s1, $neg_ok, false) -> select -> $s2
            ($s2, $abs_ok, false) -> select -> $s3
            ($s3, $add_ok, false) -> select -> $s4
            ($s4, $sub_ok, false) -> select -> $s5
            ($s5, $mul_ok, false) -> select -> $s6
            ($s6, $div_ok, false) -> select -> $s7
            ($s7, $dot_ok, false) -> select -> $s8
            ($s8, $norm_ok, false) -> select -> $s9
            ($s9, $l1_ok, false) -> select -> $s10
            ($s10, $norm_sqrt_ok, false) -> select -> $s11
            ($s11, $distance_ok, false) -> select -> $s12
            ($s12, $distance_sqrt_ok, false) -> select -> $all_ok
            ($all_ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let build = support::build_source("vector-source", source);
    let output = Command::new(&build.executable).output().expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let runtime_c = fs::read_to_string(build.build_dir.join(".cache/runtime.c")).expect("runtime");
    assert!(runtime_c.contains("flow_node___flow_std_vector_sum"));
    assert!(runtime_c.contains("flow_node___flow_std_vector_dot"));
    assert!(runtime_c.contains("for (size_t"));
    assert!(!runtime_c.contains("FaValue"));
}

#[test]
fn source_backed_stdlib_alias_imports_are_rewritten() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq }
        import std.vector as vector

        program main(args: Args) -> exit_code: Int {
            [2.0, 3.0, 4.0] -> vector.sum -> $total
            ($total, 9.0) -> eq -> $ok
            ($ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("vector-alias", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn source_backed_stdlib_reports_unknown_exports() {
    let source = r#"
        import std.cli { Args }
        import std.vector { missing }

        program main(args: Args) -> exit_code: Int {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("vector-missing-export");
    fs::write(&path, source).expect("write source");
    let error = build_file(&path, None).expect_err("build should fail");
    assert!(error.contains("module `std.vector` does not export `missing`"));
}

#[test]
fn source_backed_stdlib_helpers_are_private() {
    let source = r#"
        import std.cli { Args }
        import std.vector { _dot_pair }

        program main(args: Args) -> exit_code: Int {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("vector-private-helper");
    fs::write(&path, source).expect("write source");
    let error = build_file(&path, None).expect_err("build should fail");
    assert!(error.contains("module `std.vector` does not export `_dot_pair`"));
}
