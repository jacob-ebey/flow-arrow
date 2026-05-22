use crate::{ast::Module, codegen, parser, typecheck};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;

const NATIVE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    pub target: BuildTarget,
    pub emit_llvm: Option<PathBuf>,
}

impl BuildOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_target(target: BuildTarget) -> Self {
        Self {
            target,
            emit_llvm: None,
        }
    }

    pub fn emit_llvm(mut self, path: impl Into<PathBuf>) -> Self {
        self.emit_llvm = Some(path.into());
        self
    }
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            target: BuildTarget::native_host(),
            emit_llvm: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildTarget {
    Native(NativeTarget),
    Wasm(WasmTarget),
}

impl BuildTarget {
    pub fn native_host() -> Self {
        Self::Native(NativeTarget::host())
    }

    pub fn triple(&self) -> &str {
        match self {
            Self::Native(target) => target.triple(),
            Self::Wasm(target) => target.triple(),
        }
    }

    pub fn is_wasm(&self) -> bool {
        matches!(self, Self::Wasm(_))
    }

    pub fn supported_targets() -> Vec<&'static str> {
        let mut targets = Vec::from(NATIVE_TARGETS);
        targets.extend(WasmTarget::SUPPORTED.iter().map(|target| target.triple()));
        targets
    }
}

impl Default for BuildTarget {
    fn default() -> Self {
        Self::native_host()
    }
}

impl FromStr for BuildTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "native" | "host" => Ok(Self::native_host()),
            "wasm32-unknown-unknown" => Ok(Self::Wasm(WasmTarget::UnknownUnknown)),
            "wasm32-wasi" => Ok(Self::Wasm(WasmTarget::Wasi)),
            target if NATIVE_TARGETS.contains(&target) => Ok(Self::Native(NativeTarget {
                triple: target.to_string(),
            })),
            other => Err(format!(
                "unsupported build target `{other}`; supported targets are `native`, `host`, {}",
                BuildTarget::supported_targets()
                    .into_iter()
                    .map(|target| format!("`{target}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTarget {
    triple: String,
}

impl NativeTarget {
    pub fn host() -> Self {
        Self {
            triple: host_target(),
        }
    }

    pub fn triple(&self) -> &str {
        &self.triple
    }

    fn is_host(&self) -> bool {
        self.triple == host_target()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmTarget {
    UnknownUnknown,
    Wasi,
}

impl WasmTarget {
    const SUPPORTED: [Self; 2] = [Self::UnknownUnknown, Self::Wasi];

    pub fn triple(self) -> &'static str {
        match self {
            Self::UnknownUnknown => "wasm32-unknown-unknown",
            Self::Wasi => "wasm32-wasi",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub build_dir: PathBuf,
    pub executable: PathBuf,
}

struct BuildPlan {
    build_dir: PathBuf,
    cache_dir: PathBuf,
    executable: PathBuf,
    llvm_path: PathBuf,
    runtime_llvm_path: PathBuf,
    stale_runtime_c_path: PathBuf,
    hash_path: PathBuf,
    build_hash: String,
}

pub fn build_file(path: &Path, emit_llvm: Option<&Path>) -> Result<BuildOutput, String> {
    let mut options = BuildOptions::default();
    options.emit_llvm = emit_llvm.map(PathBuf::from);
    build_file_with_options(path, &options)
}

pub fn build_file_with_options(path: &Path, options: &BuildOptions) -> Result<BuildOutput, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let module = parser::parse(&source)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    typecheck::check_module_with_base(&module, base_dir)?;

    match &options.target {
        BuildTarget::Native(target) => build_native(path, base_dir, &module, target, options),
        BuildTarget::Wasm(target) => Err(format!(
            "build target `{}` is recognized, but the WASM backend is not implemented yet",
            target.triple()
        )),
    }
}

fn build_native(
    path: &Path,
    base_dir: &Path,
    module: &Module,
    target: &NativeTarget,
    options: &BuildOptions,
) -> Result<BuildOutput, String> {
    if !target.is_host() {
        return Err(format!(
            "native target `{}` is recognized, but cross-compilation is not implemented yet; use `native` or `{}`",
            target.triple(),
            host_target()
        ));
    }

    let llvm = codegen::emit_direct_llvm_with_base(module, base_dir)?;
    let runtime_c = codegen::emit_runtime_support_c_with_base(module, base_dir)?;
    let plan = BuildPlan::new(
        path,
        &BuildTarget::Native(target.clone()),
        &llvm,
        &runtime_c,
    )?;
    fs::create_dir_all(&plan.cache_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", plan.cache_dir.display()))?;

    if plan.is_fresh() {
        copy_emitted_llvm(&plan.llvm_path, options.emit_llvm.as_deref())?;
        return Ok(BuildOutput {
            build_dir: plan.build_dir,
            executable: plan.executable,
        });
    }

    fs::write(&plan.llvm_path, llvm)
        .map_err(|error| format!("failed to write `{}`: {error}", plan.llvm_path.display()))?;
    emit_native_runtime_llvm(&runtime_c, &plan.runtime_llvm_path)?;
    copy_emitted_llvm(&plan.llvm_path, options.emit_llvm.as_deref())?;
    remove_stale_runtime_c(&plan.stale_runtime_c_path)?;
    link_native_executable(&plan, &runtime_c)?;
    fs::write(&plan.hash_path, plan.build_hash)
        .map_err(|error| format!("failed to write `{}`: {error}", plan.hash_path.display()))?;

    Ok(BuildOutput {
        build_dir: plan.build_dir,
        executable: plan.executable,
    })
}

impl BuildPlan {
    fn new(path: &Path, target: &BuildTarget, llvm: &str, runtime_c: &str) -> Result<Self, String> {
        let build_dir = build_dir(path, target);
        let cache_dir = build_dir.join(".cache");
        let executable_name = executable_name(path)?;
        let executable =
            build_dir.join(format!("{executable_name}{}", std::env::consts::EXE_SUFFIX));
        Ok(Self {
            build_dir,
            cache_dir: cache_dir.clone(),
            executable,
            llvm_path: cache_dir.join("main.ll"),
            runtime_llvm_path: cache_dir.join("runtime.ll"),
            stale_runtime_c_path: cache_dir.join("runtime.c"),
            hash_path: cache_dir.join("build.hash"),
            build_hash: format!("{:016x}", build_hash(target, llvm, runtime_c)),
        })
    }

    fn is_fresh(&self) -> bool {
        self.executable.exists()
            && self.runtime_llvm_path.exists()
            && self.llvm_path.exists()
            && fs::read_to_string(&self.hash_path)
                .map(|cached_hash| cached_hash == self.build_hash)
                .unwrap_or(false)
    }
}

fn copy_emitted_llvm(llvm_path: &Path, emit_llvm: Option<&Path>) -> Result<(), String> {
    if let Some(out) = emit_llvm {
        fs::copy(llvm_path, out).map_err(|error| {
            format!(
                "failed to copy emitted LLVM from `{}` to `{}`: {error}",
                llvm_path.display(),
                out.display()
            )
        })?;
    }
    Ok(())
}

fn remove_stale_runtime_c(stale_runtime_c_path: &Path) -> Result<(), String> {
    if stale_runtime_c_path.exists() {
        fs::remove_file(stale_runtime_c_path).map_err(|error| {
            format!(
                "failed to remove stale generated C artifact `{}`: {error}",
                stale_runtime_c_path.display()
            )
        })?;
    }
    Ok(())
}

fn link_native_executable(plan: &BuildPlan, runtime_c: &str) -> Result<(), String> {
    let mut clang = Command::new("clang");
    clang
        .arg("-O3")
        .arg("-pthread")
        .arg(&plan.llvm_path)
        .arg(&plan.runtime_llvm_path);
    add_native_compiler_flags(&mut clang, runtime_c)?;
    let output = clang
        .arg("-o")
        .arg(&plan.executable)
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
    Ok(())
}

fn emit_native_runtime_llvm(runtime_c: &str, runtime_llvm_path: &Path) -> Result<(), String> {
    let mut clang = Command::new("clang");
    clang
        .arg("-O3")
        .arg("-pthread")
        .arg("-x")
        .arg("c")
        .arg("-S")
        .arg("-emit-llvm")
        .arg("-o")
        .arg(runtime_llvm_path)
        .arg("-");
    add_native_compiler_flags(&mut clang, runtime_c)?;

    let mut child = clang
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to invoke clang for LLVM runtime emission: {error}"))?;
    let runtime_c = runtime_c
        .replace("static inline ", "inline ")
        .replace("static ", "");
    child
        .stdin
        .take()
        .ok_or_else(|| "failed to open clang stdin".to_string())?
        .write_all(runtime_c.as_bytes())
        .map_err(|error| format!("failed to write generated runtime to clang stdin: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to wait for clang runtime emission: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "LLVM runtime emission failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn add_native_compiler_flags(clang: &mut Command, runtime_c: &str) -> Result<(), String> {
    if runtime_c.contains("jpeglib.h") || runtime_c.contains("png.h") {
        for flag in cv_compiler_flags(runtime_c)? {
            clang.arg(flag);
        }
    }
    if runtime_c.contains("h2o.h") {
        for flag in http_compiler_flags()? {
            clang.arg(flag);
        }
    }
    if runtime_c.contains("sqlite3.h") {
        for flag in sqlite_compiler_flags()? {
            clang.arg(flag);
        }
    }
    Ok(())
}

fn build_dir(path: &Path, target: &BuildTarget) -> PathBuf {
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    root.join("build").join(target.triple())
}

fn executable_name(path: &Path) -> Result<String, String> {
    let name = path
        .file_stem()
        .ok_or_else(|| format!("`{}` has no file basename", path.display()))?
        .to_string_lossy();
    if name.is_empty() {
        return Err(format!("`{}` has no file basename", path.display()));
    }
    Ok(name.into_owned())
}

pub(crate) fn host_target() -> String {
    let arch = std::env::consts::ARCH;
    let os = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "windows" => "pc-windows-msvc",
        "linux" => "unknown-linux-gnu",
        other => return format!("{arch}-unknown-{other}"),
    };
    format!("{arch}-{os}")
}

fn build_hash(target: &BuildTarget, source: &str, runtime_c: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in env!("CARGO_PKG_VERSION")
        .as_bytes()
        .iter()
        .chain(b":llvm-runtime-ir-v2:")
        .chain(target.triple().as_bytes())
        .chain(b":")
        .chain(source.as_bytes())
        .chain(runtime_c.as_bytes())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn cv_compiler_flags(runtime_c: &str) -> Result<Vec<String>, String> {
    let mut flags = Vec::new();
    if runtime_c.contains("jpeglib.h") {
        flags.extend(pkg_config_flags("libjpeg", "JPEG")?);
    }
    if runtime_c.contains("png.h") {
        flags.extend(pkg_config_flags("libpng", "PNG")?);
    }
    Ok(dedup_flags(flags))
}

fn http_compiler_flags() -> Result<Vec<String>, String> {
    let mut flags = pkg_config_flags_any(&["libh2o-evloop", "libh2o"], "std.http", "HTTP/H2O")?;
    if cfg!(target_os = "macos") {
        flags.push("-DH2O_USE_KQUEUE=1".to_string());
    } else if cfg!(target_os = "linux") {
        flags.push("-DH2O_USE_EPOLL=1".to_string());
    } else {
        flags.push("-DH2O_USE_SELECT=1".to_string());
    }
    flags.extend(pkg_config_flags_for("std.http", "openssl", "OpenSSL")?);
    flags.extend(pkg_config_flags_for("std.http", "libuv", "libuv")?);
    Ok(dedup_flags(flags))
}

fn sqlite_compiler_flags() -> Result<Vec<String>, String> {
    Ok(dedup_flags(pkg_config_flags_for(
        "std.sqlite",
        "sqlite3",
        "SQLite",
    )?))
}

fn pkg_config_flags_any(
    packages: &[&str],
    feature: &str,
    label: &str,
) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for package in packages {
        match pkg_config_flags_for(feature, package, label) {
            Ok(flags) => return Ok(flags),
            Err(error) => errors.push(error),
        }
    }
    Err(format!(
        "std.http {label} support requires H2O development headers and libraries; tried pkg-config packages `{}`:\n{}",
        packages.join("`, `"),
        errors.join("\n")
    ))
}

fn pkg_config_flags(package: &str, label: &str) -> Result<Vec<String>, String> {
    pkg_config_flags_for("std.cv", package, label)
}

fn pkg_config_flags_for(feature: &str, package: &str, label: &str) -> Result<Vec<String>, String> {
    let output = Command::new("pkg-config")
        .args(["--libs", "--cflags", package])
        .output()
        .map_err(|error| format!("failed to invoke pkg-config for {package}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{feature} {label} support requires development headers and libraries; pkg-config {package} failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let mut flags = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if cfg!(target_os = "macos") {
        let rpaths = flags
            .iter()
            .filter_map(|flag| flag.strip_prefix("-L"))
            .map(|path| format!("-Wl,-rpath,{path}"))
            .collect::<Vec<_>>();
        flags.extend(rpaths);
    }
    Ok(flags)
}

fn dedup_flags(flags: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for flag in flags {
        if !deduped.contains(&flag) {
            deduped.push(flag);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_target_is_native_host() {
        let target = BuildTarget::default();
        assert_eq!(target.triple(), host_target());
        assert!(!target.is_wasm());
    }

    #[test]
    fn parses_supported_build_targets() {
        assert_eq!(
            BuildTarget::from_str("native"),
            Ok(BuildTarget::native_host())
        );
        assert_eq!(
            BuildTarget::from_str("host"),
            Ok(BuildTarget::native_host())
        );
        assert_eq!(
            BuildTarget::from_str("wasm32-unknown-unknown"),
            Ok(BuildTarget::Wasm(WasmTarget::UnknownUnknown))
        );
        assert_eq!(
            BuildTarget::from_str("wasm32-wasi"),
            Ok(BuildTarget::Wasm(WasmTarget::Wasi))
        );
    }

    #[test]
    fn wasm_build_target_reports_backend_gap() {
        let root = unique_temp_root("wasm-target");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    0 -> $exit_code
                }
            "#,
        )
        .expect("write source");
        let options = BuildOptions::with_target(BuildTarget::Wasm(WasmTarget::UnknownUnknown));

        let error = build_file_with_options(&path, &options).expect_err("wasm build should fail");

        assert!(error.contains("build target `wasm32-unknown-unknown` is recognized"));
        assert!(error.contains("WASM backend is not implemented yet"));
    }

    fn unique_temp_root(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "flowarrow-{prefix}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp dir");
        root
    }
}
