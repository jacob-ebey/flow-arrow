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

#[test]
fn std_fs_walk_files_supports_effectful_map_and_globs() {
    let source = r#"
        import std.bytes { concat_bytes, join_bytes }
        import std.cli { Args, argv }
        import std.fault { collect, expect }
        import std.fs { walk_files }
        import std.io { write_stdout }
        import std.seq { flatten }

        program main(args: Args) -> exit_code: Int {
            $args -> argv -> map walk_files -> collect -> expect -> flatten -> $paths
            ($paths, "\n") -> join_bytes -> $listing
            [$listing, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("fs-effect-map-globs", source);
    let root = support::source_path("fs-effect-map-globs-root").with_file_name("glob-root");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).expect("create nested");
    fs::write(root.join("a.txt"), b"a").expect("write a");
    fs::write(root.join("b.log"), b"b").expect("write b");
    fs::write(nested.join("c.txt"), b"c").expect("write c");

    let pattern = root.join("*.txt");
    let output = Command::new(&build.executable)
        .arg(pattern)
        .arg(&nested)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        format!(
            "{}\n{}\n",
            root.join("a.txt").to_string_lossy(),
            nested.join("c.txt").to_string_lossy()
        )
    );
}

#[test]
fn std_fs_grep_example_builds_and_runs_with_faultable_pipeline() {
    let source = include_str!("../examples/grep/main.flow");

    let build = support::build_source("fs-grep-example", source);
    let root = support::source_path("fs-grep-example-root").with_file_name("grep-example-root");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).expect("create nested");
    fs::write(root.join("a.txt"), b"needle one\nskip\n").expect("write a");
    fs::write(nested.join("b.txt"), b"skip\nneedle two\n").expect("write b");

    let output = Command::new(&build.executable)
        .arg("needle")
        .arg(root.join("*.txt"))
        .arg(&nested)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains(":1:needle one\n"), "{stdout}");
    assert!(stdout.contains(":2:needle two\n"), "{stdout}");
    assert!(stdout.contains("files walked: 2\n"), "{stdout}");
    assert!(stdout.contains("files scanned: 2\n"), "{stdout}");
}

#[test]
fn std_fs_path_helpers_and_sorted_directory_listing_run() {
    let source = r#"
        import std.bytes { concat_bytes, join_bytes }
        import std.cli { Args, argv }
        import std.fault { expect }
        import std.fs { basename, dirname, join_path, list_dir }
        import std.io { write_stdout }
        import std.seq { head }

        program main(args: Args) -> exit_code: Int {
            $args -> argv -> head -> expect -> $dir
            ($dir, "z.txt") -> join_path -> $path
            $path -> basename -> $base
            $path -> dirname -> $parent
            $dir -> list_dir -> expect -> $entries
            ($entries, ",") -> join_bytes -> $listing
            [$base, "\n", $parent, "\n", $listing, "\n"] -> concat_bytes -> $output
            $output -> write_stdout -> $exit_code
        }
    "#;

    let build = support::build_source("fs-path-listing", source);
    let dir = support::source_path("fs-path-listing-root").with_file_name("listing-root");
    fs::create_dir_all(&dir).expect("create dir");
    fs::write(dir.join("z.txt"), b"z").expect("write z");
    fs::write(dir.join("a.txt"), b"a").expect("write a");
    fs::create_dir(dir.join("subdir")).expect("create subdir");

    let output = Command::new(&build.executable)
        .arg(&dir)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8"),
        format!("z.txt\n{}\na.txt,subdir,z.txt\n", dir.to_string_lossy())
    );
}

#[test]
fn std_fs_fault_paths_are_reported() {
    let missing_path = support::source_path("fs-missing").with_file_name("missing.txt");
    for (name, source, expected) in [
        (
            "fs-file-size-missing",
            r#"
                import std.cli { Args, argv }
                import std.fs { file_size }
                import std.seq { head }

                program main(args: Args) -> exit_code: Faultable[Int] {
                    $args -> argv -> head -> $path
                    $path -> file_size -> $exit_code
                }
            "#,
            "file_size",
        ),
        (
            "fs-list-dir-missing",
            r#"
                import std.cli { Args, argv }
                import std.fault { expect }
                import std.fs { list_dir }
                import std.seq { head }

                program main(args: Args) -> exit_code: Int {
                    $args -> argv -> head -> expect -> $path
                    $path -> list_dir -> expect -> $entries
                    0 -> $exit_code
                }
            "#,
            "list_dir",
        ),
        (
            "fs-read-files-missing",
            r#"
                import std.cli { Args, argv }
                import std.fault { expect }
                import std.fs { read_files }

                program main(args: Args) -> exit_code: Int {
                    $args -> argv -> read_files -> expect -> $files
                    0 -> $exit_code
                }
            "#,
            "read_file",
        ),
    ] {
        let build = support::build_source(name, source);
        let output = Command::new(&build.executable)
            .arg(&missing_path)
            .output()
            .expect("run");
        assert!(!output.status.success(), "{name} unexpectedly succeeded");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected),
            "{name}: expected {expected:?}, stderr was: {stderr}"
        );
    }
}
