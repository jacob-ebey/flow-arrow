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
            import std.math { add }

            node plus_one(value: Int) -> out: Int {
                ($value, 1) -> add -> $out
            }
        "#,
    )
    .expect("write helper");
    fs::write(
        &main_path,
        r#"
            import "./helper.flow" { plus_one }
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                0 -> plus_one -> $exit_code
            }
        "#,
    )
    .expect("write main");

    let build = build_file(&main_path, None).expect("build");
    let output = Command::new(&build.executable).output().expect("run");
    assert_eq!(output.status.code(), Some(1));
}
