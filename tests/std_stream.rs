mod support;

use std::fs;
use std::process::Command;

#[test]
fn std_stream_copy_file_runs_without_bytes_materialization() {
    let source = r#"
        import std.cli { Args, argv }
        import std.seq { head, tail }
        import std.stream { open_file, copy_to_file }

        program main(args: Args) -> exit_code: Faultable[Int] {
            $args -> argv -> $paths
            $paths -> head -> $input_path
            $paths -> tail -> head -> $output_path
            $input_path -> open_file -> $stream
            ($stream, $output_path) -> copy_to_file -> $exit_code
        }
    "#;

    let build = support::build_source("stream-copy", source);
    let input_path = support::source_path("stream-copy-input").with_file_name("input.bin");
    let output_path = input_path.with_file_name("output.bin");
    let contents = b"small test contents stand in for a large checkpoint";
    fs::write(&input_path, contents).expect("write input");

    let output = Command::new(&build.executable)
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&output_path).expect("read output"), contents);
}

#[test]
fn std_stream_read_at_reads_a_slice_without_consuming_the_stream() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args, argv }
        import std.io { write_stdout }
        import std.seq { head }
        import std.stream { open_file, read_at }

        program main(args: Args) -> exit_code: Faultable[Int] {
            $args -> argv -> head -> $input_path
            $input_path -> open_file -> $stream
            ($stream, 6, 6) -> read_at -> $middle
            ($stream, 0, 5) -> read_at -> $front
            [$front, " ", $middle, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("stream-read-at", source);
    let input_path = support::source_path("stream-read-at-input").with_file_name("input.txt");
    fs::write(&input_path, b"hello stream world").expect("write input");

    let output = Command::new(&build.executable)
        .arg(&input_path)
        .output()
        .expect("run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"hello stream\n");
}
