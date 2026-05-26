mod support;

#[test]
fn std_fault_nodes_run() {
    let source = r#"
        import std.cli { Args }
        import std.fault { has_faults, format_faults }
        import std.io { write_stderr }
        import std.real { parse_real }

        program main(args: Args) -> exit_code: i64 {
            ["1", "bad"] -> fault map parse_real { ok -> $numbers, fault -> $faults }
            $faults -> has_faults -> $invalid
            $faults -> format_faults -> $message
            $message -> write_stderr -> $stderr_status
            ($invalid, $stderr_status, 1) -> select -> $exit_code
        }
    "#;

    let output = support::run_source("fault", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "");
    assert_eq!(
        String::from_utf8(output.stderr).expect("utf8"),
        "line 2: expected f64, got \"bad\"\n"
    );
}

#[test]
fn std_fault_collect_accepts_faultable_sequence() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.fault { collect }
        import std.int { format_int }
        import std.io { write_stdout }
        import std.real { parse_real }
        import std.seq { head, length }

        node parse_items(items: Seq[Bytes]) -> out: Faultable[Seq[f64]] {
            $items -> map parse_real -> collect -> $out
        }

        program main(args: Args) -> exit_code: Faultable[i64] {
            [["1", "2"], ["3"]] -> map parse_items -> collect -> head -> length -> format_int -> $count
            ["ok:", $count, "\n"] -> concat_bytes -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("fault-collect-faultable-seq", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "ok:2\n");
}

#[test]
fn plain_values_flow_into_faultable_outputs() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.io { write_stdout }
        import std.predicates { is_empty }
        import std.seq { tail }

        node empty_lines(input: Bytes) -> out: Seq[Bytes] {
            [""] -> tail -> $out
        }

        node maybe_lines(input: Bytes) -> out: Faultable[Seq[Bytes]] {
            $input
            -> match {
                input_empty() -> empty_lines
                _ -> one_line
            }
            -> $out
        }

        node input_empty(input: Bytes) -> out: Bool {
            $input -> is_empty -> $out
        }

        node one_line(input: Bytes) -> out: Faultable[Seq[Bytes]] {
            [$input] -> $out
        }

        program main(args: Args) -> exit_code: Faultable[i64] {
            "ok" -> maybe_lines -> concat_bytes -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("fault-implicit-ok", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "ok");
}

#[test]
fn empty_sequence_select_and_discard_destructure_typecheck_and_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.int { format_int }
        import std.io { write_stdout }
        import std.seq { length }

        node config(include_path: Bool) -> out: Faultable[(Bytes, Seq[Bytes])] {
            ($include_path, ["root/.gitignore"], []) -> select -> $paths
            ("root", $paths) -> $out
        }

        program main(args: Args) -> exit_code: Faultable[i64] {
            false -> config -> ($, $paths)
            $paths -> length -> format_int -> write_stdout -> $exit_code
        }
    "#;

    let output = support::run_source("fault-empty-select-discard", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "0");
}
