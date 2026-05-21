mod support;

use std::fs;
use std::process::Command;

#[test]
fn std_fs_metadata_helpers_run() {
    let source = r#"
        import std.bytes { concat_bytes }
        import std.cli { Args, argv }
        import std.fs { exists, file_size, is_dir, is_file }
        import std.int { format_int }
        import std.io { write_stdout }
        import std.seq { head, tail }

        program main(args: Args) -> exit_code: Faultable[Int] {
            $args -> argv -> $paths
            $paths -> head -> $file_path
            $paths -> tail -> head -> $dir_path
            $file_path -> exists -> $exists
            $file_path -> is_file -> $is_file
            $dir_path -> is_dir -> $is_dir
            $file_path -> file_size -> format_int -> $size
            ($exists, "exists", "missing") -> select -> $exists_text
            ($is_file, "file", "not-file") -> select -> $file_text
            ($is_dir, "dir", "not-dir") -> select -> $dir_text
            [$exists_text, ":", $file_text, ":", $dir_text, ":", $size, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("fs-metadata", source);
    let file_path = support::source_path("fs-metadata-input").with_file_name("input.bin");
    let dir_path = file_path.parent().expect("parent").to_path_buf();
    fs::write(&file_path, b"abcdef").expect("write input");

    let output = Command::new(&build.executable)
        .arg(&file_path)
        .arg(&dir_path)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        "exists:file:dir:6\n"
    );
}

#[test]
fn std_fs_directory_and_batch_read_helpers_support_grep_shape() {
    let source = r#"
        import std.bytes { concat_bytes, contains, split_lines }
        import std.cli { Args, argv }
        import std.fault { expect }
        import std.fs { basename, dirname, list_dir, read_files, walk_files }
        import std.io { write_stdout }
        import std.seq { broadcast_right, flatten, head }
        import std.tuple { first, second }

        node grep_file(input: ((Bytes, Bytes), Bytes)) -> out: Seq[Bytes] {
            $input -> first -> first -> basename -> $name
            $input -> second -> $needle
            $input -> first -> second -> split_lines -> $lines
            ($lines, $needle) -> broadcast_right -> filter line_contains -> $matches
            ($matches, $name) -> broadcast_right -> map format_match -> $out
        }

        node line_contains(input: (Bytes, Bytes)) -> keep: Bool {
            $input -> contains -> $keep
        }

        node format_match(input: ((Bytes, Bytes), Bytes)) -> out: Bytes {
            $input -> second -> $name
            $input -> first -> first -> $line
            [$name, ":", $line, "\n"] -> concat_bytes -> $out
        }

        program main(args: Args) -> exit_code: Int {
            $args -> argv -> head -> $root
            $root -> dirname -> $dir
            $dir -> list_dir -> expect -> $entries
            $entries -> head -> expect -> $first_entry
            $dir -> walk_files -> expect -> read_files -> expect -> $files
            ($files, "needle") -> broadcast_right -> map grep_file -> flatten -> $lines
            $lines -> concat_bytes -> $matches
            ["entry:", $first_entry, "\n", $matches] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("fs-grep-shape", source);
    let root = support::source_path("fs-grep-root").with_file_name("grep-root");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).expect("create nested");
    fs::write(root.join("a.txt"), b"needle one\nskip\n").expect("write a");
    fs::write(nested.join("b.txt"), b"skip\nneedle two\n").expect("write b");

    let output = Command::new(&build.executable)
        .arg(root.join("a.txt"))
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("entry:a.txt\n"), "{stdout}");
    assert!(stdout.contains("a.txt:needle one\n"), "{stdout}");
    assert!(stdout.contains("b.txt:needle two\n"), "{stdout}");
}
