use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod ast;
mod codegen;
mod lexer;
mod parser;
mod runtime;

pub fn run_file(path: &Path) -> Result<u8, String> {
    let build = build_file(path, None)?;
    let status = Command::new(&build.executable)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to run `{}`: {error}", build.executable.display()))?;
    Ok(status.code().unwrap_or(1).try_into().unwrap_or(1))
}

pub fn build_file(path: &Path, emit_llvm: Option<&Path>) -> Result<BuildOutput, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let module = parser::parse(&source)?;
    let llvm = codegen::emit_module(&module)?;

    if let Some(out) = emit_llvm {
        fs::write(out, &llvm)
            .map_err(|error| format!("failed to write `{}`: {error}", out.display()))?;
    }

    let build_dir = cached_build_dir(path, &source);
    fs::create_dir_all(&build_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", build_dir.display()))?;
    let llvm_path = build_dir.join("main.ll");
    let runtime_path = build_dir.join("runtime.c");
    let executable = build_dir.join("app");
    fs::write(&llvm_path, llvm)
        .map_err(|error| format!("failed to write `{}`: {error}", llvm_path.display()))?;
    fs::write(&runtime_path, runtime::C_SOURCE)
        .map_err(|error| format!("failed to write `{}`: {error}", runtime_path.display()))?;

    if executable.exists() {
        return Ok(BuildOutput {
            build_dir,
            executable,
        });
    }

    let output = Command::new("clang")
        .arg("-O0")
        .arg(&llvm_path)
        .arg(&runtime_path)
        .arg("-o")
        .arg(&executable)
        .output()
        .map_err(|error| {
            "failed to invoke clang for LLVM backend: ".to_string() + &error.to_string()
        })?;
    if !output.status.success() {
        return Err(format!(
            "LLVM backend failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(BuildOutput {
        build_dir,
        executable,
    })
}

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub build_dir: PathBuf,
    pub executable: PathBuf,
}

fn cached_build_dir(path: &Path, source: &str) -> PathBuf {
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    root.join(".flowarrow")
        .join("build")
        .join(format!("{:016x}", build_hash(source)))
}

fn build_hash(source: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in env!("CARGO_PKG_VERSION")
        .as_bytes()
        .iter()
        .chain(source.as_bytes())
        .chain(runtime::C_SOURCE.as_bytes())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn parses_99_bottles_declarations() {
        let source = include_str!("../examples/99-bottles/main.flow");
        let module = parser::parse(source).expect("parse");
        let names = module
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                ast::Decl::Node(callable) | ast::Decl::Program(callable) => {
                    Some(callable.name.as_str())
                }
                ast::Decl::Import => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["main", "verse_for", "final_verse_node"]);
    }

    #[test]
    fn emits_llvm_for_map_reduce() {
        let source = include_str!("../examples/99-bottles/main.flow");
        let module = parser::parse(source).expect("parse");
        let llvm = codegen::emit_module(&module).expect("llvm");
        assert!(llvm.contains("define ptr @flow_node_verse_for"));
        assert!(llvm.contains("call ptr @fa_map"));
        assert!(llvm.contains("call ptr @fa_reduce"));
        assert!(llvm.contains("define i32 @main"));
    }

    #[test]
    fn llvm_backend_runs_99_bottles() {
        let build = build_file(Path::new("examples/99-bottles/main.flow"), None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf8");
        assert!(stdout.starts_with(
            "99 bottles of beer on the wall, 99 bottles of beer.\n\
             Take one down and pass it around, 98 bottles of beer on the wall.\n\n\
             98 bottles of beer on the wall, 98 bottles of beer.\n"
        ));
        assert!(stdout.contains(
            "1 bottle of beer on the wall, 1 bottle of beer.\n\
             Take one down and pass it around, 0 bottles of beer on the wall.\n\n"
        ));
        assert!(stdout.ends_with(
            "No more bottles of beer on the wall, no more bottles of beer.\n\
             Go to the store and buy some more, 99 bottles of beer on the wall.\n"
        ));
    }

    #[test]
    fn llvm_backend_runs_add_numbers_from_stdin() {
        let source = include_str!("../examples/add-numbers-from-stdin/main.flow");
        let module = parser::parse(source).expect("parse");
        let llvm = codegen::emit_module(&module).expect("llvm");
        assert!(llvm.contains("call ptr @fa_filter"));
        assert!(llvm.contains("call ptr @fa_map"));
        assert!(llvm.contains("call ptr @fa_reduce"));

        let build = build_file(Path::new("examples/add-numbers-from-stdin/main.flow"), None)
            .expect("build");
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
            .write_all(b"1\n2\n3.5\n")
            .expect("write stdin");
        let output = child.wait_with_output().expect("run");
        assert!(
            output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "6.5\n");
    }
}
