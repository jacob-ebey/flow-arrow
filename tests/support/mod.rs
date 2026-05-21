#![allow(dead_code)]

use flowarrow::{BuildOutput, build_file};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn build_source(name: &str, source: &str) -> BuildOutput {
    let path = source_path(name);
    fs::write(&path, source).expect("write source");
    build_file(&path, None).expect("build")
}

pub fn run_source(name: &str, source: &str, stdin: &[u8]) -> Output {
    let build = build_source(name, source);
    let mut child = Command::new(&build.executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("run")
}

pub fn source_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "flowarrow-stdlib-{name}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("temp dir");
    root.join("main.flow")
}
