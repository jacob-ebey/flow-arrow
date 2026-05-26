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

        program main(args: Args) -> exit_code: i64 {
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

    let runtime_llvm =
        fs::read_to_string(build.build_dir.join(".cache/runtime.ll")).expect("runtime llvm");
    let main_llvm = fs::read_to_string(build.build_dir.join(".cache/main.ll")).expect("main llvm");
    assert!(runtime_llvm.contains("define"));
    assert!(main_llvm.contains("@flow_unboxed_main"));
    assert!(!build.build_dir.join(".cache/runtime.c").exists());
}

#[test]
fn std_vector_f32_source_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq }
        import std.real { from_int_f32 }
        import std.vector {
            add_f32 as vector_add_f32,
            equals_f32 as vector_equals_f32,
            dot_f32,
            squared_norm_f32,
            squared_distance_f32,
        }

        program main(args: Args) -> exit_code: i64 {
            1  -> from_int_f32 -> $one
            2  -> from_int_f32 -> $two
            3  -> from_int_f32 -> $three
            4  -> from_int_f32 -> $four
            5  -> from_int_f32 -> $five
            6  -> from_int_f32 -> $six
            7  -> from_int_f32 -> $seven
            9  -> from_int_f32 -> $nine
            25 -> from_int_f32 -> $twenty_five
            32 -> from_int_f32 -> $thirty_two
            49 -> from_int_f32 -> $forty_nine

            ([$one, $two, $three], [$four, $five, $six]) -> vector_add_f32 -> $added
            ($added, [$five, $seven, $nine]) -> vector_equals_f32 -> $add_ok

            ([$one, $two, $three], [$four, $five, $six]) -> dot_f32 -> $dot_total
            ($dot_total, $thirty_two) -> eq -> $dot_ok

            [$two, $three, $six] -> squared_norm_f32 -> $norm_squared
            ($norm_squared, $forty_nine) -> eq -> $norm_ok

            ([$one, $two, $three], [$four, $six, $three]) -> squared_distance_f32 -> $distance_squared
            ($distance_squared, $twenty_five) -> eq -> $distance_ok

            ($add_ok, $dot_ok, false) -> select -> $s1
            ($s1, $norm_ok, false) -> select -> $s2
            ($s2, $distance_ok, false) -> select -> $all_ok
            ($all_ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("vector-f32-source", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn source_backed_stdlib_alias_imports_are_rewritten() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq }
        import std.vector as vector

        program main(args: Args) -> exit_code: i64 {
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
fn std_matrix_source_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq }
        import std.vector { equals as vector_equals }
        import std.matrix {
            rows,
            cols,
            flatten,
            transpose,
            neg as matrix_neg,
            abs as matrix_abs,
            add as matrix_add,
            sub as matrix_sub,
            mul as matrix_mul,
            div as matrix_div,
            add_scalar,
            scalar_sub,
            mul_scalar,
            add_row,
            equals as matrix_equals,
            sum as matrix_sum,
            mean as matrix_mean,
            row_sums,
            column_sums,
            row_means,
            column_means,
            squared_norm,
            l1_norm,
            squared_distance,
            matvec,
            vecmat,
            matmul,
            outer,
            gram,
        }

        program main(args: Args) -> exit_code: i64 {
            [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]] -> $a
            [[6.0, 5.0, 4.0], [3.0, 2.0, 1.0]] -> $b

            $a -> rows -> $row_count
            ($row_count, 2) -> eq -> $rows_ok

            $a -> cols -> $col_count
            ($col_count, 3) -> eq -> $cols_ok

            $a -> flatten -> $flat
            ($flat, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]) -> vector_equals -> $flat_ok

            $a -> transpose -> $transposed
            ($transposed, [[1.0, 4.0], [2.0, 5.0], [3.0, 6.0]]) -> matrix_equals -> $transpose_ok

            $a -> matrix_neg -> matrix_abs -> $absolute
            ($absolute, $a) -> matrix_equals -> $abs_ok

            ($a, $b) -> matrix_add -> $added
            ($added, [[7.0, 7.0, 7.0], [7.0, 7.0, 7.0]]) -> matrix_equals -> $add_ok

            ($a, $b) -> matrix_sub -> $subbed
            ($subbed, [[-5.0, -3.0, -1.0], [1.0, 3.0, 5.0]]) -> matrix_equals -> $sub_ok

            ($a, $b) -> matrix_mul -> $multiplied
            ($multiplied, [[6.0, 10.0, 12.0], [12.0, 10.0, 6.0]]) -> matrix_equals -> $mul_ok

            ($a, [[1.0, 2.0, 3.0], [2.0, 5.0, 3.0]]) -> matrix_div -> $divided
            ($divided, [[1.0, 1.0, 1.0], [2.0, 1.0, 2.0]]) -> matrix_equals -> $div_ok

            ($a, 10.0) -> add_scalar -> $scalar_added
            ($scalar_added, [[11.0, 12.0, 13.0], [14.0, 15.0, 16.0]]) -> matrix_equals -> $add_scalar_ok

            (10.0, $a) -> scalar_sub -> $scalar_subbed
            ($scalar_subbed, [[9.0, 8.0, 7.0], [6.0, 5.0, 4.0]]) -> matrix_equals -> $scalar_sub_ok

            ($a, 2.0) -> mul_scalar -> $scaled
            ($scaled, [[2.0, 4.0, 6.0], [8.0, 10.0, 12.0]]) -> matrix_equals -> $mul_scalar_ok

            ($a, [10.0, 20.0, 30.0]) -> add_row -> $row_added
            ($row_added, [[11.0, 22.0, 33.0], [14.0, 25.0, 36.0]]) -> matrix_equals -> $add_row_ok

            $a -> matrix_sum -> $total
            ($total, 21.0) -> eq -> $sum_ok

            $a -> matrix_mean -> $average
            ($average, 3.5) -> eq -> $mean_ok

            $a -> row_sums -> $rs
            ($rs, [6.0, 15.0]) -> vector_equals -> $row_sums_ok

            $a -> column_sums -> $cs
            ($cs, [5.0, 7.0, 9.0]) -> vector_equals -> $column_sums_ok

            $a -> row_means -> $rm
            ($rm, [2.0, 5.0]) -> vector_equals -> $row_means_ok

            $a -> column_means -> $cm
            ($cm, [2.5, 3.5, 4.5]) -> vector_equals -> $column_means_ok

            $a -> squared_norm -> $sn
            ($sn, 91.0) -> eq -> $squared_norm_ok

            $a -> l1_norm -> $l1
            ($l1, 21.0) -> eq -> $l1_ok

            ($a, $a) -> squared_distance -> $sd
            ($sd, 0.0) -> eq -> $distance_ok

            ($a, [1.0, 0.0, 1.0]) -> matvec -> $mv
            ($mv, [4.0, 10.0]) -> vector_equals -> $matvec_ok

            ([1.0, 1.0], $a) -> vecmat -> $vm
            ($vm, [5.0, 7.0, 9.0]) -> vector_equals -> $vecmat_ok

            ($a, [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]) -> matmul -> $mm
            ($mm, [[22.0, 28.0], [49.0, 64.0]]) -> matrix_equals -> $matmul_ok

            ([1.0, 2.0], [3.0, 4.0, 5.0]) -> outer -> $outer_product
            ($outer_product, [[3.0, 4.0, 5.0], [6.0, 8.0, 10.0]]) -> matrix_equals -> $outer_ok

            $a -> gram -> $gram_matrix
            ($gram_matrix, [[14.0, 32.0], [32.0, 77.0]]) -> matrix_equals -> $gram_ok

            ($rows_ok, $cols_ok, false) -> select -> $s1
            ($s1, $flat_ok, false) -> select -> $s2
            ($s2, $transpose_ok, false) -> select -> $s3
            ($s3, $abs_ok, false) -> select -> $s4
            ($s4, $add_ok, false) -> select -> $s5
            ($s5, $sub_ok, false) -> select -> $s6
            ($s6, $mul_ok, false) -> select -> $s7
            ($s7, $div_ok, false) -> select -> $s8
            ($s8, $add_scalar_ok, false) -> select -> $s9
            ($s9, $scalar_sub_ok, false) -> select -> $s10
            ($s10, $mul_scalar_ok, false) -> select -> $s11
            ($s11, $add_row_ok, false) -> select -> $s12
            ($s12, $sum_ok, false) -> select -> $s13
            ($s13, $mean_ok, false) -> select -> $s14
            ($s14, $row_sums_ok, false) -> select -> $s15
            ($s15, $column_sums_ok, false) -> select -> $s16
            ($s16, $row_means_ok, false) -> select -> $s17
            ($s17, $column_means_ok, false) -> select -> $s18
            ($s18, $squared_norm_ok, false) -> select -> $s19
            ($s19, $l1_ok, false) -> select -> $s20
            ($s20, $distance_ok, false) -> select -> $s21
            ($s21, $matvec_ok, false) -> select -> $s22
            ($s22, $vecmat_ok, false) -> select -> $s23
            ($s23, $matmul_ok, false) -> select -> $s24
            ($s24, $outer_ok, false) -> select -> $s25
            ($s25, $gram_ok, false) -> select -> $all_ok
            ($all_ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let build = support::build_source("matrix-source", source);
    let output = Command::new(&build.executable).output().expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let runtime_llvm =
        fs::read_to_string(build.build_dir.join(".cache/runtime.ll")).expect("runtime llvm");
    let main_llvm = fs::read_to_string(build.build_dir.join(".cache/main.ll")).expect("main llvm");
    assert!(runtime_llvm.contains("define"));
    assert!(main_llvm.contains("@flow_unboxed_main"));
    assert!(!build.build_dir.join(".cache/runtime.c").exists());
}

#[test]
fn source_backed_stdlib_reports_unknown_exports() {
    let source = r#"
        import std.cli { Args }
        import std.vector { missing }

        program main(args: Args) -> exit_code: i64 {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("vector-missing-export");
    fs::write(&path, source).expect("write source");
    let error = build_file(&path, None).expect_err("build should fail");
    assert!(error.contains("module `std.vector` does not export `missing`"));
}

#[test]
fn source_backed_matrix_helpers_are_private() {
    let source = r#"
        import std.cli { Args }
        import std.matrix { row_matmul }

        program main(args: Args) -> exit_code: i64 {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("matrix-private-helper");
    fs::write(&path, source).expect("write source");
    let error = build_file(&path, None).expect_err("build should fail");
    assert!(error.contains("module `std.matrix` does not export `row_matmul`"));
}

#[test]
fn source_backed_stdlib_helpers_are_private() {
    let source = r#"
        import std.cli { Args }
        import std.vector { dot_pair }

        program main(args: Args) -> exit_code: i64 {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("vector-private-helper");
    fs::write(&path, source).expect("write source");
    let error = build_file(&path, None).expect_err("build should fail");
    assert!(error.contains("module `std.vector` does not export `dot_pair`"));
}
