mod support;

use flowarrow::build_file;
use std::fs;
use std::process::Command;

#[test]
fn local_flow_imports_typecheck_and_run() {
    let main_path = support::source_path("local-import-main");
    let dir = main_path.parent().expect("parent");
    let helper_path = dir.join("helper.flow");
    fs::write(
        &helper_path,
        r#"
            import std.fault { expect }
            import std.math { add_i64 as add }

            extern node plus_one(value: i64) -> out: i64 {
                ($value, 1) -> add -> expect -> $out
            }
        "#,
    )
    .expect("write helper");
    fs::write(
        &main_path,
        r#"
            import "./helper.flow" { plus_one }
            import std.cli { Args }

            program main(args: Args) -> exit_code: i64 {
                0 -> plus_one -> $exit_code
            }
        "#,
    )
    .expect("write main");

    let build = build_file(&main_path, None).expect("build");
    let output = Command::new(&build.executable).output().expect("run");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn local_flow_import_rejects_non_extern_node() {
    let main_path = support::source_path("local-import-private-node");
    let dir = main_path.parent().expect("parent");
    let helper_path = dir.join("helper.flow");
    fs::write(
        &helper_path,
        r#"
            node hidden(value: i64) -> out: i64 {
                $value -> $out
            }
        "#,
    )
    .expect("write helper");
    fs::write(
        &main_path,
        r#"
            import "./helper.flow" { hidden }
            import std.cli { Args }

            program main(args: Args) -> exit_code: i64 {
                0 -> hidden -> $exit_code
            }
        "#,
    )
    .expect("write main");

    let error = build_file(&main_path, None).expect_err("non-extern node should not import");
    assert!(error.contains("local module `./helper.flow` does not export `hidden`"));
}
