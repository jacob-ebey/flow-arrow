mod support;

use flowarrow::build_file;
use std::fs;
use std::process::Command;

#[test]
fn std_vector_source_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.math { eq }
        import std.vector { sum, dot }

        program main(args: Args) -> exit_code: Int {
            [1.0, 2.0, 3.5] -> sum -> $total
            ($total, 6.5) -> eq -> $sum_ok

            ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]) -> dot -> $dot_total
            ($dot_total, 32.0) -> eq -> $dot_ok

            ($sum_ok, $dot_ok, false) -> select -> $all_ok
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
    assert!(runtime_c.contains("fa_map("));
    assert!(runtime_c.contains("fa_reduce("));
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
