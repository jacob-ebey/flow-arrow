mod support;

use flowarrow::{build_file, typecheck_file};

#[test]
fn std_cli_args_type_runs() {
    let source = r#"
        import std.cli { Args }

        program main(args: Args) -> exit_code: Int {
            0 -> exit_code
        }
    "#;

    let output = support::run_source("cli-args", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn std_cli_unsupported_nodes_are_rejected() {
    for (name, input) in [
        ("argv", "args"),
        ("flag_present", r#"(args, "--verbose")"#),
        ("flag_value", r#"(args, "--name")"#),
    ] {
        let source = format!(
            r#"
                import std.cli {{ Args, {name} }}

                program main(args: Args) -> exit_code: Int {{
                    {input} -> {name} -> unused
                    0 -> exit_code
                }}
            "#
        );
        let path = support::source_path(&format!("cli-{name}"));
        std::fs::write(&path, source).expect("write source");
        let error = typecheck_file(&path).expect_err("typecheck should fail");
        assert!(
            error.contains("is declared in the stdlib but is not implemented"),
            "{name}: {error}"
        );
        assert!(build_file(&path, None).is_err(), "{name} should not build");
    }
}
