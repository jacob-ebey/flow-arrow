mod support;

#[test]
fn intrinsic_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.fault { expect }
        import std.math { add, eq }

        program main(args: Args) -> exit_code: i64 {
            (0, 5, 2) -> range_step -> $values
            $values -> reduce add(identity: 0) -> expect -> $sum
            ($sum, 6) -> eq -> $ok
            ($ok, 0, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("intrinsic", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
