mod support;

use flowarrow::{build_file, typecheck_file};
use std::process::Command;

#[test]
fn std_cli_args_type_runs() {
    let source = r#"
        import std.cli { Args }

        program main(args: Args) -> exit_code: Int {
            0 -> $exit_code
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
fn std_cli_argv_runs() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args, argv }
        import std.io { write_stdout }

        program main(args: Args) -> exit_code: Int {
            $args -> argv -> $raw_args
            $raw_args -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("cli-argv", source);
    let output = Command::new(&build.executable)
        .args(["one", "two", "three"])
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "onetwothree"
    );
}

#[test]
fn std_cli_unsupported_flag_nodes_are_rejected() {
    for (name, input) in [
        ("flag_present", r#"($args, "--verbose")"#),
        ("flag_value", r#"($args, "--name")"#),
    ] {
        let source = format!(
            r#"
                import std.cli {{ Args, {name} }}

                program main(args: Args) -> exit_code: Int {{
                    {input} -> {name} -> $unused
                    0 -> $exit_code
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
