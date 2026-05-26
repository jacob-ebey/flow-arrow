mod support;

use flowarrow::build_file;
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn std_cv_jpeg_pipeline_decodes_grayscales_and_encodes() {
    let path = support::source_path("cv-jpeg-generate");
    let root = path.parent().expect("source root");
    let input_path = root.join("input.jpg");
    let output_path = root.join("output.jpg");
    let input_path_text = input_path.to_string_lossy();
    let output_path_text = output_path.to_string_lossy();
    let generator = format!(
        r#"
        import std.cli {{ Args }}
        import std.cv {{ save_jpeg }}

        program main(args: Args) -> exit_code: Faultable[i64] {{
            ((2, 1), [[(1.0, (0.0, 0.0)), (0.0, (1.0, 0.0))]]) -> $image
            ("{input_path_text}", $image) -> save_jpeg -> $exit_code
        }}
    "#
    );

    fs::write(&path, generator).expect("write source");
    let build = build_file(&path, None).expect("build generator");
    let output = Command::new(&build.executable)
        .output()
        .expect("run generator");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let app = build_file(Path::new("examples/grayscale-image/main.flow"), None).expect("build app");
    let output = Command::new(&app.executable)
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run app");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let validator_path = root.join("validate.flow");
    let validator = format!(
        r#"
        import std.cli {{ Args }}
        import std.cv {{ Pixel, load_jpeg, pixels }}
        import std.math {{ abs_f64 as abs, le_f64 as le, sub_f64 as sub }}
        import std.predicates {{ all, and }}
        import std.tuple {{ first, second }}

        program main(args: Args) -> exit_code: Faultable[i64] {{
            "{output_path_text}" -> load_jpeg -> pixels -> map near_gray -> all -> $ok
            ($ok, 0, 1) -> select -> $exit_code
        }}

        node near_gray(pixel: Pixel) -> ok: Bool {{
            $pixel -> first -> $red
            $pixel -> second -> $green_blue
            $green_blue -> first -> $green
            $green_blue -> second -> $blue
            ($red, $green) -> sub -> abs -> $rg_delta
            ($green, $blue) -> sub -> abs -> $gb_delta
            ($rg_delta, 0.012) -> le -> $rg_ok
            ($gb_delta, 0.012) -> le -> $gb_ok
            ($rg_ok, $gb_ok) -> and -> $ok
        }}
    "#
    );
    fs::write(&validator_path, validator).expect("write validator");
    let validator = build_file(&validator_path, None).expect("build validator");
    let output = Command::new(&validator.executable)
        .output()
        .expect("run validator");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let runtime_llvm =
        fs::read_to_string(app.build_dir.join(".cache/runtime.ll")).expect("runtime llvm");
    assert!(runtime_llvm.contains("fa_cv_decode_jpeg"));
    assert!(runtime_llvm.contains("fa_cv_encode_jpeg"));
    assert!(!app.build_dir.join(".cache/runtime.c").exists());
}

#[test]
fn std_cv_rejects_invalid_jpeg() {
    let source = r#"
        import std.bytes { codes_to_bytes }
        import std.cli { Args }
        import std.cv { decode_jpeg }
        import std.tuple { second }

        program main(args: Args) -> exit_code: Faultable[i64] {
            [110, 111, 116, 45, 106, 112, 101, 103] -> codes_to_bytes -> $not_jpeg
            $not_jpeg -> decode_jpeg -> $image
            ($image, 0) -> second -> $exit_code
        }
    "#;

    let build = support::build_source("cv-invalid-jpeg", source);
    let output = Command::new(&build.executable).output().expect("run");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("decode_jpeg"));
}

#[test]
fn std_cv_generic_load_detects_png_bmp_ppm_and_pgm() {
    let path = support::source_path("cv-multiformat-generate");
    let root = path.parent().expect("source root");
    let png_path = root.join("input.png");
    let bmp_path = root.join("input.bmp");
    let ppm_path = root.join("input.ppm");
    let pgm_path = root.join("input.pgm");
    let png_path_text = png_path.to_string_lossy();
    let bmp_path_text = bmp_path.to_string_lossy();
    let ppm_path_text = ppm_path.to_string_lossy();
    let pgm_path_text = pgm_path.to_string_lossy();
    let generator = format!(
        r#"
        import std.cli {{ Args }}
        import std.cv {{ save_bmp, save_pgm, save_png, save_ppm }}

        program main(args: Args) -> exit_code: Faultable[i64] {{
            ((2, 1), [[(1.0, (0.0, 0.0)), (0.0, (1.0, 0.0))]]) -> $image
            ("{png_path_text}", $image) -> save_png -> $png_status
            ("{bmp_path_text}", $image) -> save_bmp -> $bmp_status
            ("{ppm_path_text}", $image) -> save_ppm -> $ppm_status
            ("{pgm_path_text}", $image) -> save_pgm -> $exit_code
        }}
    "#
    );

    fs::write(&path, generator).expect("write source");
    let build = build_file(&path, None).expect("build generator");
    let output = Command::new(&build.executable)
        .output()
        .expect("run generator");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        &fs::read(&png_path).expect("png")[..8],
        b"\x89PNG\r\n\x1a\n"
    );
    assert_eq!(&fs::read(&bmp_path).expect("bmp")[..2], b"BM");
    assert!(
        fs::read(&ppm_path)
            .expect("ppm")
            .starts_with(b"P6\n2 1\n255\n")
    );
    assert!(
        fs::read(&pgm_path)
            .expect("pgm")
            .starts_with(b"P5\n2 1\n255\n")
    );

    for (name, image_path, expected_red, expected_green, expected_blue) in [
        ("png", &png_path, "1.0", "0.0", "0.0"),
        ("bmp", &bmp_path, "1.0", "0.0", "0.0"),
        ("ppm", &ppm_path, "1.0", "0.0", "0.0"),
        (
            "pgm",
            &pgm_path,
            "0.2980392156862745",
            "0.2980392156862745",
            "0.2980392156862745",
        ),
    ] {
        let image_path_text = image_path.to_string_lossy();
        let validator = format!(
            r#"
            import std.cli {{ Args }}
            import std.cv {{ height, load, pixels, width }}
            import std.math {{ abs_f64 as abs, eq_i64 as eq, le_f64 as le, sub_f64 as sub }}
            import std.predicates {{ and }}
            import std.seq {{ head }}
            import std.tuple {{ first, second }}

            program main(args: Args) -> exit_code: Faultable[i64] {{
                "{image_path_text}" -> load -> $image
                $image -> width -> $width
                $image -> height -> $height
                ($width, 2) -> eq -> $width_ok
                ($height, 1) -> eq -> $height_ok
                $image -> pixels -> head -> $pixel
                $pixel -> first -> $red
                $pixel -> second -> first -> $green
                $pixel -> second -> second -> $blue
                ($red, {expected_red}) -> sub -> abs -> $red_delta
                ($green, {expected_green}) -> sub -> abs -> $green_delta
                ($blue, {expected_blue}) -> sub -> abs -> $blue_delta
                ($red_delta, 0.004) -> le -> $red_ok
                ($green_delta, 0.004) -> le -> $green_ok
                ($blue_delta, 0.004) -> le -> $blue_ok
                ($width_ok, $height_ok) -> and -> $shape_ok
                ($shape_ok, $red_ok) -> and -> $s1
                ($s1, $green_ok) -> and -> $s2
                ($s2, $blue_ok) -> and -> $ok
                ($ok, 0, 1) -> select -> $exit_code
            }}
        "#
        );
        let validator_path = root.join(format!("validate-{name}.flow"));
        fs::write(&validator_path, validator).expect("write validator");
        let validator = build_file(&validator_path, None).expect("build validator");
        let output = Command::new(&validator.executable)
            .output()
            .expect("run validator");
        assert!(
            output.status.success(),
            "{name} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn std_cv_generic_decode_rejects_unknown_format() {
    let source = r#"
        import std.bytes { codes_to_bytes }
        import std.cli { Args }
        import std.cv { decode }
        import std.tuple { second }

        program main(args: Args) -> exit_code: Faultable[i64] {
            [110, 111, 116, 45, 105, 109, 97, 103, 101] -> codes_to_bytes -> $not_image
            $not_image -> decode -> $image
            ($image, 0) -> second -> $exit_code
        }
    "#;

    let build = support::build_source("cv-invalid-generic", source);
    let output = Command::new(&build.executable).output().expect("run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported image format"),
        "stderr: {stderr}"
    );
}

#[test]
fn std_cv_exposes_channel_matrices_for_matrix_pipelines() {
    let source = r#"
        import std.cli { Args }
        import std.cv { luma_matrix, red_matrix }
        import std.matrix { add_scalar, equals as matrix_equals }
        import std.predicates { and }

        program main(args: Args) -> exit_code: i64 {
            ((2, 1), [[(1.0, (0.0, 0.0)), (0.0, (1.0, 0.0))]]) -> $image
            $image -> red_matrix -> $red
            ($red, [[1.0, 0.0]]) -> matrix_equals -> $red_ok
            ($red, 1.0) -> add_scalar -> $red_plus_one
            ($red_plus_one, [[2.0, 1.0]]) -> matrix_equals -> $matrix_pipeline_ok
            $image -> luma_matrix -> $luma
            ($luma, [[0.299, 0.587]]) -> matrix_equals -> $luma_ok
            ($red_ok, $matrix_pipeline_ok) -> and -> $s1
            ($s1, $luma_ok) -> and -> $ok
            ($ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("cv-channel-matrices", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
