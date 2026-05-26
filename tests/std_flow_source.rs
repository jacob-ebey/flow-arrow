mod support;

use flowarrow::build_file;
use std::fs;
use std::process::Command;

#[test]
fn std_vector_source_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq_f64 as eq }
        import std.vector {
            sum_f64,
            mean_f64,
            neg_f64 as vector_neg,
            abs_f64 as vector_abs,
            add_f64 as vector_add,
            sub_f64 as vector_sub,
            mul_f64 as vector_mul,
            div_f64 as vector_div,
            equals_f64 as vector_equals,
            dot_f64,
            squared_norm_f64,
            l1_norm_f64,
            norm_f64,
            min_f64,
            max_f64,
            exp_f64,
            relu_f64,
            sigmoid_f64,
            silu_f64,
            softmax_f64,
            squared_distance_f64,
            distance_f64,
        }

        program main(args: Args) -> exit_code: i64 {
            [1.0, 2.0, 3.5] -> sum_f64 -> $total
            ($total, 6.5) -> eq -> $sum_ok

            [2.0, 4.0, 6.0] -> mean_f64 -> $mean_f64
            ($mean_f64, 4.0) -> eq -> $mean_ok

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

            ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]) -> dot_f64 -> $dot_total
            ($dot_total, 32.0) -> eq -> $dot_ok

            [2.0, 3.0, 6.0] -> squared_norm_f64 -> $norm_squared
            ($norm_squared, 49.0) -> eq -> $norm_ok

            [-2.0, 3.0, -6.0] -> l1_norm_f64 -> $l1
            ($l1, 11.0) -> eq -> $l1_ok

            [3.0, 4.0] -> norm_f64 -> $norm_f64
            ($norm_f64, 5.0) -> eq -> $norm_sqrt_ok

            [3.0, -1.0, 2.0] -> min_f64 -> $minimum
            ($minimum, -1.0) -> eq -> $min_ok

            [3.0, -1.0, 2.0] -> max_f64 -> $maximum
            ($maximum, 3.0) -> eq -> $max_ok

            [0.0] -> exp_f64 -> $exp_values
            ($exp_values, [1.0]) -> vector_equals -> $exp_ok

            [-1.0, 0.0, 2.0] -> relu_f64 -> $relu_values
            ($relu_values, [0.0, 0.0, 2.0]) -> vector_equals -> $relu_ok

            [0.0] -> sigmoid_f64 -> $sigmoid_values
            ($sigmoid_values, [0.5]) -> vector_equals -> $sigmoid_ok

            [0.0] -> silu_f64 -> $silu_values
            ($silu_values, [0.0]) -> vector_equals -> $silu_ok

            [0.0, 0.0] -> softmax_f64 -> $softmax_values
            ($softmax_values, [0.5, 0.5]) -> vector_equals -> $softmax_ok

            ([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]) -> squared_distance_f64 -> $distance_squared
            ($distance_squared, 25.0) -> eq -> $distance_ok

            ([1.0, 2.0, 3.0], [4.0, 6.0, 3.0]) -> distance_f64 -> $distance_f64
            ($distance_f64, 5.0) -> eq -> $distance_sqrt_ok

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
            ($s11, $min_ok, false) -> select -> $s12
            ($s12, $max_ok, false) -> select -> $s13
            ($s13, $exp_ok, false) -> select -> $s14
            ($s14, $relu_ok, false) -> select -> $s15
            ($s15, $sigmoid_ok, false) -> select -> $s16
            ($s16, $silu_ok, false) -> select -> $s17
            ($s17, $softmax_ok, false) -> select -> $s18
            ($s18, $distance_ok, false) -> select -> $s19
            ($s19, $distance_sqrt_ok, false) -> select -> $all_ok
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
fn f32_literals_run_without_conversion_nodes() {
    let source = r#"
        import std.cli { Args }
        import std.io { write_stdout }
        import std.math { add_f32 }
        import std.real { format_real_f32 }

        program main(args: Args) -> exit_code: i64 {
            (1.0f32, 2.5f32) -> add_f32 -> format_real_f32 -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("f32-literals", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3.5");
}

#[test]
fn source_backed_stdlib_does_not_use_conversion_nodes() {
    let source_dir = std::path::Path::new("src/stdlib/source");
    for entry in fs::read_dir(source_dir).expect("source stdlib dir") {
        let path = entry.expect("source stdlib entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("flow") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("source stdlib file");
        assert!(
            !source.contains("from_"),
            "{} must not use from_* conversion nodes internally",
            path.display()
        );
    }
}

#[test]
fn std_vector_f32_source_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.fault { expect }
        import std.math { div_f32, eq_f32 as eq }
        import std.vector {
            add_f32 as vector_add_f32,
            equals_f32 as vector_equals_f32,
            dot_f32,
            mean_square_f32,
            rms_f32,
            rms_norm_f32,
            relu_f32,
            swiglu_f32,
            softmax_f32,
            squared_norm_f32,
            squared_distance_f32,
        }

        program main(args: Args) -> exit_code: i64 {
            1.0f32  -> $one
            2.0f32  -> $two
            3.0f32  -> $three
            4.0f32  -> $four
            5.0f32  -> $five
            6.0f32  -> $six
            7.0f32  -> $seven
            9.0f32  -> $nine
            25.0f32 -> $twenty_five
            32.0f32 -> $thirty_two
            49.0f32 -> $forty_nine
            ($one, $two) -> div_f32 -> expect -> $half
            0.0f32  -> $zero
            -1.0f32 -> $minus_one

            ([$one, $two, $three], [$four, $five, $six]) -> vector_add_f32 -> $added
            ($added, [$five, $seven, $nine]) -> vector_equals_f32 -> $add_ok

            [$minus_one, $zero, $two] -> relu_f32 -> $relu_values
            ($relu_values, [$zero, $zero, $two]) -> vector_equals_f32 -> $relu_ok

            [$zero, $zero] -> softmax_f32 -> $softmax_values
            ($softmax_values, [$half, $half]) -> vector_equals_f32 -> $softmax_ok

            ([$one, $two, $three], [$four, $five, $six]) -> dot_f32 -> $dot_total
            ($dot_total, $thirty_two) -> eq -> $dot_ok

            [$two, $three, $six] -> squared_norm_f32 -> $norm_squared
            ($norm_squared, $forty_nine) -> eq -> $norm_ok

            [$two, $two] -> mean_square_f32 -> $mean_square
            ($mean_square, $four) -> eq -> $mean_square_ok

            ([$two, $two], $zero) -> rms_f32 -> $rms
            ($rms, $two) -> eq -> $rms_ok

            ([$two, $two], [$one, $one], $zero) -> rms_norm_f32 -> $rms_normalized
            ($rms_normalized, [$one, $one]) -> vector_equals_f32 -> $rms_norm_ok

            ([$zero, $zero], [$four, $five]) -> swiglu_f32 -> $swiglu_values
            ($swiglu_values, [$zero, $zero]) -> vector_equals_f32 -> $swiglu_ok

            ([$one, $two, $three], [$four, $six, $three]) -> squared_distance_f32 -> $distance_squared
            ($distance_squared, $twenty_five) -> eq -> $distance_ok

            ($add_ok, $dot_ok, false) -> select -> $s1
            ($s1, $norm_ok, false) -> select -> $s2
            ($s2, $distance_ok, false) -> select -> $s3
            ($s3, $relu_ok, false) -> select -> $s4
            ($s4, $softmax_ok, false) -> select -> $s5
            ($s5, $mean_square_ok, false) -> select -> $s6
            ($s6, $rms_ok, false) -> select -> $s7
            ($s7, $rms_norm_ok, false) -> select -> $s8
            ($s8, $swiglu_ok, false) -> select -> $all_ok
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
fn std_matrix_f32_source_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.fault { expect }
        import std.math { div_f32, eq_f32 as eq }
        import std.vector { equals_f32 as vector_equals_f32 }
        import std.matrix {
            equals_f32 as matrix_equals_f32,
            add_f32 as matrix_add_f32,
            row_mean_squares_f32,
            row_rms_f32,
            row_rms_norm_f32,
            row_softmax_f32,
            row_swiglu_f32,
            matvec_f32,
            matmul_f32,
        }

        program main(args: Args) -> exit_code: i64 {
            0.0f32  -> $zero
            1.0f32  -> $one
            2.0f32  -> $two
            3.0f32  -> $three
            4.0f32  -> $four
            5.0f32  -> $five
            6.0f32  -> $six
            7.0f32  -> $seven
            8.0f32  -> $eight
            10.0f32 -> $ten
            12.0f32 -> $twelve
            22.0f32 -> $twenty_two
            25.0f32 -> $twenty_five
            28.0f32 -> $twenty_eight
            49.0f32 -> $forty_nine
            64.0f32 -> $sixty_four
            ($one, $two) -> div_f32 -> expect -> $half

            [[$one, $two, $three], [$four, $five, $six]] -> $a

            ($a, $a) -> matrix_add_f32 -> $added
            ($added, [[$two, $four, $six], [$eight, $ten, $twelve]]) -> matrix_equals_f32 -> $add_ok

            [[$zero, $zero], [$zero, $zero]] -> row_softmax_f32 -> $softmax_rows
            ($softmax_rows, [[$half, $half], [$half, $half]]) -> matrix_equals_f32 -> $row_softmax_ok

            [[$two, $two], [$five, $five]] -> row_mean_squares_f32 -> $row_mean_squares
            ($row_mean_squares, [$four, $twenty_five]) -> vector_equals_f32 -> $row_mean_squares_ok

            ([[$two, $two], [$four, $four]], $zero) -> row_rms_f32 -> $row_rms
            ($row_rms, [$two, $four]) -> vector_equals_f32 -> $row_rms_ok

            ([[$two, $two], [$four, $four]], [$one, $one], $zero) -> row_rms_norm_f32 -> $row_rms_norm
            ($row_rms_norm, [[$one, $one], [$one, $one]]) -> matrix_equals_f32 -> $row_rms_norm_ok

            ([[$zero, $zero]], [[$four, $five]]) -> row_swiglu_f32 -> $row_swiglu
            ($row_swiglu, [[$zero, $zero]]) -> matrix_equals_f32 -> $row_swiglu_ok

            ($a, [$one, $zero, $one]) -> matvec_f32 -> $mv
            ($mv, [$four, $ten]) -> vector_equals_f32 -> $matvec_ok

            ($a, [[$one, $two], [$three, $four], [$five, $six]]) -> matmul_f32 -> $mm
            ($mm, [[$twenty_two, $twenty_eight], [$forty_nine, $sixty_four]]) -> matrix_equals_f32 -> $matmul_ok

            ($add_ok, $row_softmax_ok, false) -> select -> $s1
            ($s1, $matvec_ok, false) -> select -> $s2
            ($s2, $matmul_ok, false) -> select -> $s3
            ($s3, $row_mean_squares_ok, false) -> select -> $s4
            ($s4, $row_rms_ok, false) -> select -> $s5
            ($s5, $row_rms_norm_ok, false) -> select -> $s6
            ($s6, $row_swiglu_ok, false) -> select -> $all_ok
            ($all_ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("matrix-f32-source", source, b"");
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
        import std.math { eq_f64 as eq }
        import std.vector as vector

        program main(args: Args) -> exit_code: i64 {
            [2.0, 3.0, 4.0] -> vector.sum_f64 -> $total
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
        import std.math { eq_i64, eq_f64 as eq }
        import std.vector { equals_f64 as vector_equals }
        import std.matrix {
            rows_f64,
            cols_f64,
            flatten_f64,
            transpose_f64,
            neg_f64 as matrix_neg,
            abs_f64 as matrix_abs,
            add_f64 as matrix_add,
            sub_f64 as matrix_sub,
            mul_f64 as matrix_mul,
            div_f64 as matrix_div,
            add_scalar_f64,
            scalar_sub_f64,
            mul_scalar_f64,
            add_row_f64,
            equals_f64 as matrix_equals,
            sum_f64 as matrix_sum,
            mean_f64 as matrix_mean,
            row_sums_f64,
            column_sums_f64,
            row_means_f64,
            column_means_f64,
            squared_norm_f64,
            l1_norm_f64,
            squared_distance_f64,
            row_softmax_f64,
            matvec_f64,
            vecmat_f64,
            matmul_f64,
            outer_f64,
            gram_f64,
        }

        program main(args: Args) -> exit_code: i64 {
            [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]] -> $a
            [[6.0, 5.0, 4.0], [3.0, 2.0, 1.0]] -> $b

            $a -> rows_f64 -> $row_count
            ($row_count, 2) -> eq_i64 -> $rows_ok

            $a -> cols_f64 -> $col_count
            ($col_count, 3) -> eq_i64 -> $cols_ok

            $a -> flatten_f64 -> $flat
            ($flat, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]) -> vector_equals -> $flat_ok

            $a -> transpose_f64 -> $transposed
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

            ($a, 10.0) -> add_scalar_f64 -> $scalar_added
            ($scalar_added, [[11.0, 12.0, 13.0], [14.0, 15.0, 16.0]]) -> matrix_equals -> $add_scalar_ok

            (10.0, $a) -> scalar_sub_f64 -> $scalar_subbed
            ($scalar_subbed, [[9.0, 8.0, 7.0], [6.0, 5.0, 4.0]]) -> matrix_equals -> $scalar_sub_ok

            ($a, 2.0) -> mul_scalar_f64 -> $scaled
            ($scaled, [[2.0, 4.0, 6.0], [8.0, 10.0, 12.0]]) -> matrix_equals -> $mul_scalar_ok

            ($a, [10.0, 20.0, 30.0]) -> add_row_f64 -> $row_added
            ($row_added, [[11.0, 22.0, 33.0], [14.0, 25.0, 36.0]]) -> matrix_equals -> $add_row_ok

            $a -> matrix_sum -> $total
            ($total, 21.0) -> eq -> $sum_ok

            $a -> matrix_mean -> $average
            ($average, 3.5) -> eq -> $mean_ok

            $a -> row_sums_f64 -> $rs
            ($rs, [6.0, 15.0]) -> vector_equals -> $row_sums_ok

            $a -> column_sums_f64 -> $cs
            ($cs, [5.0, 7.0, 9.0]) -> vector_equals -> $column_sums_ok

            $a -> row_means_f64 -> $rm
            ($rm, [2.0, 5.0]) -> vector_equals -> $row_means_ok

            $a -> column_means_f64 -> $cm
            ($cm, [2.5, 3.5, 4.5]) -> vector_equals -> $column_means_ok

            $a -> squared_norm_f64 -> $sn
            ($sn, 91.0) -> eq -> $squared_norm_ok

            $a -> l1_norm_f64 -> $l1
            ($l1, 21.0) -> eq -> $l1_ok

            ($a, $a) -> squared_distance_f64 -> $sd
            ($sd, 0.0) -> eq -> $distance_ok

            [[0.0, 0.0], [0.0, 0.0]] -> row_softmax_f64 -> $softmax_rows
            ($softmax_rows, [[0.5, 0.5], [0.5, 0.5]]) -> matrix_equals -> $row_softmax_ok

            ($a, [1.0, 0.0, 1.0]) -> matvec_f64 -> $mv
            ($mv, [4.0, 10.0]) -> vector_equals -> $matvec_ok

            ([1.0, 1.0], $a) -> vecmat_f64 -> $vm
            ($vm, [5.0, 7.0, 9.0]) -> vector_equals -> $vecmat_ok

            ($a, [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]) -> matmul_f64 -> $mm
            ($mm, [[22.0, 28.0], [49.0, 64.0]]) -> matrix_equals -> $matmul_ok

            ([1.0, 2.0], [3.0, 4.0, 5.0]) -> outer_f64 -> $outer_product
            ($outer_product, [[3.0, 4.0, 5.0], [6.0, 8.0, 10.0]]) -> matrix_equals -> $outer_ok

            $a -> gram_f64 -> $gram_matrix
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
            ($s21, $row_softmax_ok, false) -> select -> $s22
            ($s22, $matvec_ok, false) -> select -> $s23
            ($s23, $vecmat_ok, false) -> select -> $s24
            ($s24, $matmul_ok, false) -> select -> $s25
            ($s25, $outer_ok, false) -> select -> $s26
            ($s26, $gram_ok, false) -> select -> $all_ok
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
fn std_quant_core_types_and_q4_k_helpers_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq_i64 as eq }
        import std.quant {
            Q4KBlock,
            Q4KMWeightMatrix,
            q4_k_block,
            q4_k_block_delta,
            q4_k_block_min_delta,
            q4_k_block_quants,
            q4_k_block_scales,
            q4_k_block_size,
            q4_k_m_weight_blocks,
            q4_k_m_weight_cols,
            q4_k_m_weight_matrix,
            q4_k_m_weight_rows,
            q4_k_quant_bytes,
            q4_k_scale_bytes,
            q4_k_subblock_size,
            q4_k_subblocks,
            low_nibble,
            high_nibble,
        }

        node keep_block(block: Q4KBlock) -> out: Q4KBlock {
            $block -> $out
        }

        node keep_weight(matrix: Q4KMWeightMatrix) -> out: Q4KMWeightMatrix {
            $matrix -> $out
        }

        program main(args: Args) -> exit_code: i64 {
            1.0f32 -> $one
            2.0f32 -> $two
            ($one, $two, "abcdefghijkl", "quantized payload") -> q4_k_block -> keep_block -> $block
            $block -> q4_k_block_scales -> $scales
            $block -> q4_k_block_quants -> $quants
            $block -> q4_k_block_delta -> $delta
            $block -> q4_k_block_min_delta -> $min_delta

            (2, 256, [$block]) -> q4_k_m_weight_matrix -> keep_weight -> $weights
            $weights -> q4_k_m_weight_rows -> $rows
            $weights -> q4_k_m_weight_cols -> $cols
            $weights -> q4_k_m_weight_blocks -> $blocks

            () -> q4_k_block_size -> $block_size
            () -> q4_k_subblock_size -> $subblock_size
            () -> q4_k_subblocks -> $subblocks
            () -> q4_k_scale_bytes -> $scale_bytes
            () -> q4_k_quant_bytes -> $quant_bytes
            171 -> low_nibble -> $low
            171 -> high_nibble -> $high

            ($rows, 2) -> eq -> $rows_ok
            ($cols, 256) -> eq -> $cols_ok
            ($block_size, 256) -> eq -> $block_size_ok
            ($subblock_size, 32) -> eq -> $subblock_size_ok
            ($subblocks, 8) -> eq -> $subblocks_ok
            ($scale_bytes, 12) -> eq -> $scale_bytes_ok
            ($quant_bytes, 128) -> eq -> $quant_bytes_ok
            ($low, 11) -> eq -> $low_ok
            ($high, 10) -> eq -> $high_ok

            ($rows_ok, $cols_ok, false) -> select -> $s1
            ($s1, $block_size_ok, false) -> select -> $s2
            ($s2, $subblock_size_ok, false) -> select -> $s3
            ($s3, $subblocks_ok, false) -> select -> $s4
            ($s4, $scale_bytes_ok, false) -> select -> $s5
            ($s5, $quant_bytes_ok, false) -> select -> $s6
            ($s6, $low_ok, false) -> select -> $s7
            ($s7, $high_ok, false) -> select -> $all_ok
            ($all_ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("quant-core-types", source, b"");
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
fn std_vector_rejects_unsuffixed_numeric_exports() {
    let source = r#"
        import std.cli { Args }
        import std.vector { dot }

        program main(args: Args) -> exit_code: i64 {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("vector-unsuffixed-export");
    fs::write(&path, source).expect("write source");
    let error = build_file(&path, None).expect_err("build should fail");
    assert!(error.contains("module `std.vector` does not export `dot`"));
}

#[test]
fn std_matrix_rejects_unsuffixed_numeric_exports() {
    let source = r#"
        import std.cli { Args }
        import std.matrix { matmul }

        program main(args: Args) -> exit_code: i64 {
            0 -> $exit_code
        }
    "#;

    let path = support::source_path("matrix-unsuffixed-export");
    fs::write(&path, source).expect("write source");
    let error = build_file(&path, None).expect_err("build should fail");
    assert!(error.contains("module `std.matrix` does not export `matmul`"));
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
