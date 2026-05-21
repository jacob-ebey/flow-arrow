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

        program main(args: Args) -> exit_code: Faultable[Int] {{
            ((2, 1), [(255, (0, 0)), (0, (255, 0))]) -> $image
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
        import std.cv {{ load_jpeg }}
        import std.math {{ abs, le, sub }}
        import std.predicates {{ all, and }}
        import std.tuple {{ first, second }}

        program main(args: Args) -> exit_code: Faultable[Int] {{
            "{output_path_text}" -> load_jpeg -> second -> map near_gray -> all -> $ok
            ($ok, 0, 1) -> select -> $exit_code
        }}

        node near_gray(pixel: (Int,(Int,Int))) -> ok: Bool {{
            $pixel -> first -> $red
            $pixel -> second -> $green_blue
            $green_blue -> first -> $green
            $green_blue -> second -> $blue
            ($red, $green) -> sub -> abs -> $rg_delta
            ($green, $blue) -> sub -> abs -> $gb_delta
            ($rg_delta, 3) -> le -> $rg_ok
            ($gb_delta, 3) -> le -> $gb_ok
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

    let runtime_c = fs::read_to_string(app.build_dir.join(".cache/runtime.c")).expect("runtime");
    assert!(runtime_c.contains("fa_cv_decode_jpeg"));
    assert!(runtime_c.contains("fa_cv_encode_jpeg"));
    assert!(runtime_c.contains("flow_node___flow_std_cv_grayscale"));
}

#[test]
fn std_cv_rejects_invalid_jpeg() {
    let source = r#"
        import std.bytes { codes_to_bytes }
        import std.cli { Args }
        import std.cv { decode_jpeg }
        import std.tuple { second }

        program main(args: Args) -> exit_code: Faultable[Int] {
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
