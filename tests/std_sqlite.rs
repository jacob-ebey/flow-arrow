mod support;

use flowarrow::{build_file, typecheck_file};
use std::fs;
use std::process::Command;

const SQLITE_SOURCE: &str = r#"
    import std.bytes { concat_bytes }
    import std.cli { Args }
    import std.fault { expect }
    import std.int { format_int }
    import std.io { write_stdout }
    import std.seq { slice }
    import std.sqlite as sqlite
    import std.stream as stream
    import std.tuple { first, second }

    program main(args: Args) -> exit_code: Faultable[Int] {
        () -> sqlite.open_memory -> $conn0
        () -> sqlite.null -> $null_value
        [$null_value] -> $one_param
        ($one_param, 0, 0) -> slice -> $no_params
        ($conn0, "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL)", $no_params) -> sqlite.exec -> first -> $conn1
        "alpha" -> sqlite.text -> $title1
        "beta" -> sqlite.text -> $title2
        ($conn1, "INSERT INTO todos (title) VALUES (?)", [$title1]) -> sqlite.exec -> first -> $conn2
        ($conn2, "INSERT INTO todos (title) VALUES (?)", [$title2]) -> sqlite.exec -> first -> $conn3
        ($conn3, "SELECT id, title FROM todos ORDER BY id", $no_params) -> sqlite.query -> $query
        $query -> first -> $conn4
        $query -> second -> map todo_line -> stream.to_seq -> concat_bytes -> $output
        $output -> write_stdout -> $written
        $conn4 -> sqlite.close -> $exit_code
    }

    node todo_line(row: sqlite.Row) -> line: Bytes {
        ($row, "id") -> sqlite.value_named -> sqlite.as_int -> expect -> format_int -> $id
        ($row, "title") -> sqlite.value_named -> sqlite.as_text -> expect -> $title
        ["todo ", $id, ": ", $title, "\n"] -> concat_bytes -> $line
    }
"#;

fn sqlite_available() -> bool {
    Command::new("pkg-config")
        .args(["--cflags", "--libs", "sqlite3"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_sqlite_runtime_tests() -> bool {
    std::env::var("FLOWARROW_RUN_SQLITE_TESTS").ok().as_deref() == Some("1")
}

#[test]
fn std_sqlite_typechecks_streamed_rows() {
    let path = support::source_path("sqlite-typecheck");
    fs::write(&path, SQLITE_SOURCE).expect("write source");
    typecheck_file(&path).expect("typecheck");
}

#[test]
fn std_sqlite_build_reports_missing_pkg_config_or_builds() {
    let path = support::source_path("sqlite-build-diagnostic");
    fs::write(&path, SQLITE_SOURCE).expect("write source");
    typecheck_file(&path).expect("typecheck");
    match build_file(&path, None) {
        Ok(build) => {
            let runtime_c =
                fs::read_to_string(build.build_dir.join(".cache/runtime.c")).expect("read runtime");
            assert!(runtime_c.contains("#include <sqlite3.h>"));
            assert!(runtime_c.contains("fa_sqlite_query"));
            assert!(!runtime_c.contains("#include \"sqlite.h\""));
        }
        Err(error) => {
            assert!(error.contains("std.sqlite"), "unexpected error: {error}");
            assert!(error.contains("sqlite3"), "unexpected error: {error}");
            assert!(error.contains("pkg-config"), "unexpected error: {error}");
        }
    }
}

#[test]
fn std_sqlite_rejects_effectful_node_under_map() {
    let source = r#"
        import std.cli { Args }
        import std.sqlite { Connection, close }

        node bad(conn: Connection) -> out: Seq[Faultable[Int]] {
            [$conn] -> map close -> $out
        }

        program main(args: Args) -> exit_code: Int {
            0 -> $exit_code
        }
    "#;
    let path = support::source_path("sqlite-effectful-map");
    fs::write(&path, source).expect("write source");
    let error = typecheck_file(&path).expect_err("typecheck should fail");
    assert!(error.contains("cannot be used as a map/filter function"));
}

#[test]
fn non_sqlite_build_does_not_emit_sqlite_runtime() {
    let build = support::build_source(
        "sqlite-not-imported",
        r#"
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                0 -> $exit_code
            }
        "#,
    );
    let runtime_c = fs::read_to_string(build.build_dir.join(".cache/runtime.c")).expect("runtime");
    assert!(!runtime_c.contains("sqlite3.h"));
    assert!(!runtime_c.contains("fa_sqlite_"));
}

#[test]
fn std_sqlite_runtime_stream_query_runs() {
    if !run_sqlite_runtime_tests() || !sqlite_available() {
        return;
    }
    let output = support::run_source("sqlite-runtime-stream", SQLITE_SOURCE, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"todo 1: alpha\ntodo 2: beta\n");
}

#[test]
fn std_sqlite_runtime_query_all_values_and_transactions_run() {
    if !run_sqlite_runtime_tests() || !sqlite_available() {
        return;
    }
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args }
        import std.fault { expect }
        import std.int { format_int }
        import std.io { write_stdout }
        import std.real { format_real }
        import std.seq { head, length, slice }
        import std.sqlite as sqlite
        import std.tuple { first, second }

        program main(args: Args) -> exit_code: Faultable[Int] {
            () -> sqlite.open_memory -> $conn0
            () -> sqlite.null -> $null_value
            [$null_value] -> $one_param
            ($one_param, 0, 0) -> slice -> $no_params
            ($conn0, "CREATE TABLE sample (id INTEGER, score REAL, name TEXT, payload BLOB, optional TEXT)", $no_params) -> sqlite.exec -> first -> $conn1
            $conn1 -> sqlite.begin -> $conn2
            99 -> sqlite.int -> $rolled_id
            ($conn2, "INSERT INTO sample (id, score, name, payload, optional) VALUES (?, 1.0, 'rolled', x'00', NULL)", [$rolled_id]) -> sqlite.exec -> first -> $conn3
            $conn3 -> sqlite.rollback -> $conn4
            $conn4 -> sqlite.begin_immediate -> $conn5
            7 -> sqlite.int -> $id
            2.5 -> sqlite.real -> $score
            "seven" -> sqlite.text -> $name
            "blob-bytes" -> sqlite.blob -> $payload
            () -> sqlite.null -> $optional
            ($conn5, "INSERT INTO sample (id, score, name, payload, optional) VALUES (?, ?, ?, ?, ?)", [$id, $score, $name, $payload, $optional]) -> sqlite.exec -> first -> $conn6
            $conn6 -> sqlite.commit -> $conn7
            ($conn7, "SELECT id, score, name, payload, optional FROM sample ORDER BY id", $no_params) -> sqlite.query_all -> $result
            $result -> first -> $conn8
            $result -> second -> $rows
            $rows -> length -> format_int -> $count
            $rows -> head -> expect -> row_summary -> $summary
            ["rows=", $count, "\n", $summary] -> concat_bytes -> $output
            $output -> write_stdout -> $written
            $conn8 -> sqlite.close -> $exit_code
        }

        node row_summary(row: sqlite.Row) -> out: Faultable[Bytes] {
            ($row, "id") -> sqlite.value_named -> sqlite.as_int -> format_int -> $id
            ($row, "score") -> sqlite.value_named -> sqlite.as_real -> format_real -> $score
            ($row, "name") -> sqlite.value_named -> sqlite.as_text -> $name
            ($row, "payload") -> sqlite.value_named -> sqlite.as_blob -> $payload
            ($row, "optional") -> sqlite.value_named -> sqlite.kind -> $optional_kind
            ["id=", $id, " score=", $score, " name=", $name, " payload=", $payload, " optional=", $optional_kind, "\n"] -> concat_bytes -> $out
        }
    "#;
    let output = support::run_source("sqlite-runtime-query-all", source, b"");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "rows=1\nid=7 score=2.5 name=seven payload=blob-bytes optional=null\n"
    );
}

#[test]
fn std_sqlite_runtime_faults_on_bad_param_count() {
    if !run_sqlite_runtime_tests() || !sqlite_available() {
        return;
    }
    let source = r#"
        import std.cli { Args }
        import std.seq { slice }
        import std.sqlite as sqlite
        import std.tuple { first }

        program main(args: Args) -> exit_code: Faultable[Int] {
            () -> sqlite.open_memory -> $conn0
            () -> sqlite.null -> $null_value
            [$null_value] -> $one_param
            ($one_param, 0, 0) -> slice -> $no_params
            ($conn0, "CREATE TABLE t (id INTEGER)", $no_params) -> sqlite.exec -> first -> $conn1
            ($conn1, "INSERT INTO t (id) VALUES (?)", $no_params) -> sqlite.exec -> first -> $conn2
            $conn2 -> sqlite.close -> $exit_code
        }
    "#;
    let build = support::build_source("sqlite-runtime-param-fault", source);
    let output = Command::new(&build.executable).output().expect("run");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("parameter count"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
