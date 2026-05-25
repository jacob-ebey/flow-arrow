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
    pub crate_type: CrateType,
    pub optimization: BuildOptimization,
    pub compiler_flags: Vec<String>,
    pub linker_flags: Vec<String>,
    pub emit_llvm: Option<PathBuf>,
    pub worker_concurrency: bool,
}

impl BuildOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_target(target: BuildTarget) -> Self {
        Self {
            target,
            crate_type: CrateType::Bin,
            optimization: BuildOptimization::default(),
            compiler_flags: Vec::new(),
            linker_flags: Vec::new(),
            emit_llvm: None,
            worker_concurrency: false,
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
            crate_type: CrateType::Bin,
            optimization: BuildOptimization::default(),
            compiler_flags: Vec::new(),
            linker_flags: Vec::new(),
            emit_llvm: None,
            worker_concurrency: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildOptimization {
    O0,
    O1,
    O2,
    O3,
    Os,
    Oz,
}

impl BuildOptimization {
    pub fn from_clang_flag(flag: &str) -> Option<Self> {
        match flag {
            "-O0" => Some(Self::O0),
            "-O1" | "-Og" => Some(Self::O1),
            "-O2" => Some(Self::O2),
            "-O3" => Some(Self::O3),
            "-Os" => Some(Self::Os),
            "-Oz" => Some(Self::Oz),
            _ => None,
        }
    }

    fn clang_flag(self) -> &'static str {
        match self {
            Self::O0 => "-O0",
            Self::O1 => "-O1",
            Self::O2 => "-O2",
            Self::O3 => "-O3",
            Self::Os => "-Os",
            Self::Oz => "-Oz",
        }
    }

    fn llvm_level(self) -> inkwell::OptimizationLevel {
        match self {
            Self::O0 => inkwell::OptimizationLevel::None,
            Self::O1 => inkwell::OptimizationLevel::Less,
            Self::O2 | Self::Os | Self::Oz => inkwell::OptimizationLevel::Default,
            Self::O3 => inkwell::OptimizationLevel::Aggressive,
        }
    }
}

impl Default for BuildOptimization {
    fn default() -> Self {
        Self::O3
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateType {
    Bin,
    Cdylib,
}

impl Default for CrateType {
    fn default() -> Self {
        Self::Bin
    }
}

impl FromStr for CrateType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "bin" => Ok(Self::Bin),
            "cdylib" => Ok(Self::Cdylib),
            other => Err(format!(
                "unsupported crate type `{other}`; supported crate types are `bin`, `cdylib`"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildTarget {
    Native(NativeTarget),
    Wasm(WasmTarget),
    Typescript,
    Javascript,
}

impl BuildTarget {
    pub fn native_host() -> Self {
        Self::Native(NativeTarget::host())
    }

    pub fn triple(&self) -> &str {
        match self {
            Self::Native(target) => target.triple(),
            Self::Wasm(target) => target.triple(),
            Self::Typescript => "typescript",
            Self::Javascript => "javascript",
        }
    }

    pub fn is_wasm(&self) -> bool {
        matches!(self, Self::Wasm(_))
    }

    pub fn supported_targets() -> Vec<&'static str> {
        let mut targets = Vec::from(NATIVE_TARGETS);
        targets.extend(WasmTarget::SUPPORTED.iter().map(|target| target.triple()));
        targets.push("typescript");
        targets.push("javascript");
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
            "typescript" | "ts" => Ok(Self::Typescript),
            "javascript" | "js" => Ok(Self::Javascript),
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
    object_path: PathBuf,
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
    match options.crate_type {
        CrateType::Bin => typecheck::check_module_with_base(&module, base_dir)?,
        CrateType::Cdylib => typecheck::check_library_module_with_base(&module, base_dir)?,
    }
    if options.worker_concurrency
        && !matches!(
            options.target,
            BuildTarget::Typescript | BuildTarget::Javascript
        )
    {
        return Err(
            "`--workers` is only supported for TypeScript and JavaScript builds".to_string(),
        );
    }

    match &options.target {
        BuildTarget::Native(target) => build_native(path, base_dir, &module, target, options),
        BuildTarget::Wasm(target) => build_wasm(path, base_dir, &module, *target, options),
        BuildTarget::Typescript => build_typescript(path, base_dir, &module, options),
        BuildTarget::Javascript => build_javascript(path, base_dir, &module, options),
    }
}

fn build_typescript(
    path: &Path,
    base_dir: &Path,
    module: &Module,
    options: &BuildOptions,
) -> Result<BuildOutput, String> {
    if !options.compiler_flags.is_empty() || !options.linker_flags.is_empty() {
        return Err(
            "TypeScript builds emit source directly and do not accept compiler or linker flags"
                .to_string(),
        );
    }
    if options.emit_llvm.is_some() {
        return Err("TypeScript builds do not support `--emit-llvm`".to_string());
    }

    let target = BuildTarget::Typescript;
    let build_dir = build_dir(path, &target);
    fs::create_dir_all(&build_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", build_dir.display()))?;
    let executable_name = executable_name(path)?;
    let artifacts = codegen::emit_typescript_artifacts_with_base_and_options(
        module,
        base_dir,
        codegen::TypeScriptBackendOptions {
            worker_concurrency: options.worker_concurrency,
            worker_module_specifier: options
                .worker_concurrency
                .then(|| format!("./{executable_name}.worker.mjs")),
        },
    )?;
    let artifact = build_dir.join(format!("{executable_name}.ts"));
    fs::write(&artifact, artifacts.source)
        .map_err(|error| format!("failed to write `{}`: {error}", artifact.display()))?;
    for file in artifacts.files {
        let path = build_dir.join(file.path);
        fs::write(&path, file.source)
            .map_err(|error| format!("failed to write `{}`: {error}", path.display()))?;
    }
    Ok(BuildOutput {
        build_dir,
        executable: artifact,
    })
}

fn build_javascript(
    path: &Path,
    base_dir: &Path,
    module: &Module,
    options: &BuildOptions,
) -> Result<BuildOutput, String> {
    if !options.compiler_flags.is_empty() || !options.linker_flags.is_empty() {
        return Err(
            "JavaScript builds emit source directly and do not accept compiler or linker flags"
                .to_string(),
        );
    }
    if options.emit_llvm.is_some() {
        return Err("JavaScript builds do not support `--emit-llvm`".to_string());
    }

    let target = BuildTarget::Javascript;
    let build_dir = build_dir(path, &target);
    fs::create_dir_all(&build_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", build_dir.display()))?;
    let executable_name = executable_name(path)?;
    let artifacts = codegen::emit_javascript_artifacts_with_base_and_options(
        module,
        base_dir,
        codegen::TypeScriptBackendOptions {
            worker_concurrency: options.worker_concurrency,
            worker_module_specifier: options
                .worker_concurrency
                .then(|| format!("./{executable_name}.worker.mjs")),
        },
    )?;
    let artifact = build_dir.join(format!("{executable_name}.mjs"));
    let declarations = build_dir.join(format!("{executable_name}.d.ts"));
    fs::write(&artifact, artifacts.javascript)
        .map_err(|error| format!("failed to write `{}`: {error}", artifact.display()))?;
    fs::write(&declarations, artifacts.declarations)
        .map_err(|error| format!("failed to write `{}`: {error}", declarations.display()))?;
    for file in artifacts.files {
        let path = build_dir.join(file.path);
        fs::write(&path, file.source)
            .map_err(|error| format!("failed to write `{}`: {error}", path.display()))?;
    }
    Ok(BuildOutput {
        build_dir,
        executable: artifact,
    })
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

    match options.crate_type {
        CrateType::Bin => build_native_bin(path, base_dir, module, target, options),
        CrateType::Cdylib => build_native_cdylib(path, base_dir, module, target, options),
    }
}

fn build_native_bin(
    path: &Path,
    base_dir: &Path,
    module: &Module,
    target: &NativeTarget,
    options: &BuildOptions,
) -> Result<BuildOutput, String> {
    let lowered = codegen::lower_module_with_base(module, base_dir)?;
    let llvm = lowered.emit_direct_llvm()?;
    let runtime_c = lowered.emit_runtime_support_c()?;
    let foreign_c_sources = lowered.foreign_c_source_paths(base_dir)?;
    let foreign_c_dependencies = lowered.foreign_c_dependency_paths(base_dir)?;
    let foreign_c_hash = foreign_c_hash_input(&foreign_c_dependencies)?;
    let plan = BuildPlan::new(
        path,
        &BuildTarget::Native(target.clone()),
        options,
        &llvm,
        &runtime_c,
        &foreign_c_hash,
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
    emit_native_runtime_llvm(&runtime_c, &plan.runtime_llvm_path, options)?;
    let foreign_c_objects =
        compile_foreign_c_sources(&foreign_c_sources, &plan.cache_dir, options, false)?;
    copy_emitted_llvm(&plan.llvm_path, options.emit_llvm.as_deref())?;
    remove_stale_runtime_c(&plan.stale_runtime_c_path)?;
    link_native_executable(&plan, &runtime_c, &foreign_c_objects, options)?;
    fs::write(&plan.hash_path, plan.build_hash)
        .map_err(|error| format!("failed to write `{}`: {error}", plan.hash_path.display()))?;

    Ok(BuildOutput {
        build_dir: plan.build_dir,
        executable: plan.executable,
    })
}

fn build_native_cdylib(
    path: &Path,
    base_dir: &Path,
    module: &Module,
    target: &NativeTarget,
    options: &BuildOptions,
) -> Result<BuildOutput, String> {
    let lowered = codegen::lower_module_with_base(module, base_dir)?;
    let emitted = lowered.emit_native_cdylib_c()?;
    if emitted.exports.is_empty() {
        return Err(
            "native cdylib build requires at least one top-level `extern node` export".to_string(),
        );
    }
    let foreign_c_sources = lowered.foreign_c_source_paths(base_dir)?;
    let foreign_c_dependencies = lowered.foreign_c_dependency_paths(base_dir)?;
    let foreign_c_hash = foreign_c_hash_input(&foreign_c_dependencies)?;
    let plan = BuildPlan::new(
        path,
        &BuildTarget::Native(target.clone()),
        options,
        &emitted.source,
        "",
        &foreign_c_hash,
    )?;
    fs::create_dir_all(&plan.cache_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", plan.cache_dir.display()))?;

    let executable_name = executable_name(path)?;
    let header_path = plan.build_dir.join(format!("{executable_name}.h"));
    fs::write(&header_path, emitted.header)
        .map_err(|error| format!("failed to write `{}`: {error}", header_path.display()))?;

    let c_path = plan.cache_dir.join("library.c");
    if plan.is_native_cdylib_fresh(&c_path) {
        return Ok(BuildOutput {
            build_dir: plan.build_dir,
            executable: plan.executable,
        });
    }

    fs::write(&c_path, emitted.source)
        .map_err(|error| format!("failed to write `{}`: {error}", c_path.display()))?;
    let foreign_c_objects =
        compile_foreign_c_sources(&foreign_c_sources, &plan.cache_dir, options, true)?;
    if let Some(out) = options.emit_llvm.as_deref() {
        fs::copy(&c_path, out).map_err(|error| {
            format!(
                "failed to copy emitted C from `{}` to `{}`: {error}",
                c_path.display(),
                out.display()
            )
        })?;
    }
    remove_stale_runtime_c(&plan.stale_runtime_c_path)?;
    link_native_cdylib_c(&plan, &c_path, &foreign_c_objects, options)?;
    fs::write(&plan.hash_path, plan.build_hash)
        .map_err(|error| format!("failed to write `{}`: {error}", plan.hash_path.display()))?;

    Ok(BuildOutput {
        build_dir: plan.build_dir,
        executable: plan.executable,
    })
}

fn build_wasm(
    path: &Path,
    base_dir: &Path,
    module: &Module,
    target: WasmTarget,
    options: &BuildOptions,
) -> Result<BuildOutput, String> {
    if target != WasmTarget::UnknownUnknown {
        return Err(format!(
            "build target `{}` is recognized, but only `wasm32-unknown-unknown` is implemented for WASM builds",
            target.triple()
        ));
    }
    if options.crate_type != CrateType::Cdylib {
        return Err(
            "WASM builds currently require `--crate-type cdylib` for an exportable reactor module"
                .to_string(),
        );
    }
    if !options.compiler_flags.is_empty() {
        return Err(
            "WASM cdylib builds use the direct LLVM backend; use `-O*` for optimization and `--linker-flag` for wasm-ld flags"
                .to_string(),
        );
    }

    let lowered = codegen::lower_module_with_base(module, base_dir)?;
    let emitted =
        lowered.emit_wasm_cdylib_llvm(target.triple(), options.optimization.llvm_level())?;
    if emitted.exports.is_empty() {
        return Err(
            "WASM cdylib build requires at least one top-level `extern node` export".to_string(),
        );
    }
    let export_hash = emitted.exports.join(",");
    let plan = BuildPlan::new(
        path,
        &BuildTarget::Wasm(target),
        options,
        &emitted.llvm,
        &export_hash,
        "",
    )?;
    fs::create_dir_all(&plan.cache_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", plan.cache_dir.display()))?;

    if plan.is_wasm_fresh() {
        copy_emitted_llvm(&plan.llvm_path, options.emit_llvm.as_deref())?;
        return Ok(BuildOutput {
            build_dir: plan.build_dir,
            executable: plan.executable,
        });
    }

    fs::write(&plan.llvm_path, emitted.llvm)
        .map_err(|error| format!("failed to write `{}`: {error}", plan.llvm_path.display()))?;
    fs::write(&plan.object_path, emitted.object)
        .map_err(|error| format!("failed to write `{}`: {error}", plan.object_path.display()))?;
    copy_emitted_llvm(&plan.llvm_path, options.emit_llvm.as_deref())?;
    link_wasm_cdylib(&plan, target, &emitted.exports, options)?;
    fs::write(&plan.hash_path, plan.build_hash)
        .map_err(|error| format!("failed to write `{}`: {error}", plan.hash_path.display()))?;

    Ok(BuildOutput {
        build_dir: plan.build_dir,
        executable: plan.executable,
    })
}

impl BuildPlan {
    fn new(
        path: &Path,
        target: &BuildTarget,
        options: &BuildOptions,
        llvm: &str,
        runtime_c: &str,
        foreign_c_hash: &str,
    ) -> Result<Self, String> {
        let build_dir = build_dir(path, target);
        let cache_dir = build_dir.join(".cache");
        let executable_name = executable_name(path)?;
        let executable = match target {
            BuildTarget::Native(_) => match options.crate_type {
                CrateType::Bin => {
                    build_dir.join(format!("{executable_name}{}", std::env::consts::EXE_SUFFIX))
                }
                CrateType::Cdylib => build_dir.join(dynamic_library_name(&executable_name)),
            },
            BuildTarget::Wasm(_) => build_dir.join(format!("{executable_name}.wasm")),
            BuildTarget::Typescript => build_dir.join(format!("{executable_name}.ts")),
            BuildTarget::Javascript => build_dir.join(format!("{executable_name}.mjs")),
        };
        Ok(Self {
            build_dir,
            cache_dir: cache_dir.clone(),
            executable,
            llvm_path: cache_dir.join("main.ll"),
            object_path: cache_dir.join("main.o"),
            runtime_llvm_path: cache_dir.join("runtime.ll"),
            stale_runtime_c_path: cache_dir.join("runtime.c"),
            hash_path: cache_dir.join("build.hash"),
            build_hash: format!(
                "{:016x}",
                build_hash(target, options, llvm, runtime_c, foreign_c_hash)
            ),
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

    fn is_wasm_fresh(&self) -> bool {
        self.executable.exists()
            && self.llvm_path.exists()
            && self.object_path.exists()
            && fs::read_to_string(&self.hash_path)
                .map(|cached_hash| cached_hash == self.build_hash)
                .unwrap_or(false)
    }

    fn is_native_cdylib_fresh(&self, c_path: &Path) -> bool {
        self.executable.exists()
            && c_path.exists()
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

fn link_native_executable(
    plan: &BuildPlan,
    runtime_c: &str,
    foreign_c_objects: &[PathBuf],
    options: &BuildOptions,
) -> Result<(), String> {
    let mut clang = Command::new("clang");
    clang
        .arg(options.optimization.clang_flag())
        .arg("-pthread")
        .args(&options.compiler_flags)
        .arg(&plan.llvm_path)
        .arg(&plan.runtime_llvm_path);
    clang.args(foreign_c_objects);
    add_native_compiler_flags(&mut clang, runtime_c)?;
    clang.args(&options.linker_flags);
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

fn link_native_cdylib_c(
    plan: &BuildPlan,
    c_path: &Path,
    foreign_c_objects: &[PathBuf],
    options: &BuildOptions,
) -> Result<(), String> {
    let runtime_c = fs::read_to_string(c_path)
        .map_err(|error| format!("failed to read `{}`: {error}", c_path.display()))?;
    let mut clang = Command::new("clang");
    clang
        .arg(options.optimization.clang_flag())
        .arg("-pthread")
        .arg("-fPIC");
    if cfg!(target_os = "macos") {
        clang.arg("-dynamiclib");
    } else {
        clang.arg("-shared");
    }
    clang.args(&options.compiler_flags).arg(c_path);
    clang.args(foreign_c_objects);
    add_native_compiler_flags(&mut clang, &runtime_c)?;
    clang.args(&options.linker_flags);
    let output = clang
        .arg("-o")
        .arg(&plan.executable)
        .output()
        .map_err(|error| {
            "failed to invoke clang for native cdylib backend: ".to_string() + &error.to_string()
        })?;
    if !output.status.success() {
        return Err(format!(
            "native cdylib backend failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn compile_foreign_c_sources(
    sources: &[PathBuf],
    cache_dir: &Path,
    options: &BuildOptions,
    pic: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut objects = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let object = cache_dir.join(format!("foreign-{index}.o"));
        let mut clang = Command::new("clang");
        clang
            .arg(options.optimization.clang_flag())
            .arg("-pthread")
            .args(&options.compiler_flags);
        if pic {
            clang.arg("-fPIC");
        }
        if let Some(parent) = source.parent() {
            clang.arg("-I").arg(parent);
        }
        let output = clang
            .arg("-c")
            .arg(source)
            .arg("-o")
            .arg(&object)
            .output()
            .map_err(|error| {
                format!(
                    "failed to invoke clang for foreign C source `{}`: {error}",
                    source.display()
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "foreign C compilation failed for `{}`:\n{}{}",
                source.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        objects.push(object);
    }
    Ok(objects)
}

fn link_wasm_cdylib(
    plan: &BuildPlan,
    _target: WasmTarget,
    exports: &[String],
    options: &BuildOptions,
) -> Result<(), String> {
    let linker = wasm_linker()?;
    let mut command = Command::new(&linker.path);
    if linker.needs_flavor {
        command.args(["-flavor", "wasm"]);
    }
    command.arg("--no-entry").arg(&plan.object_path);
    for export in exports {
        command.arg(format!("--export={export}"));
    }
    command.args(&options.linker_flags);
    let output = command
        .arg("-o")
        .arg(&plan.executable)
        .output()
        .map_err(|error| {
            format!(
                "failed to invoke `{}` for WASM backend: {error}",
                linker.path.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "WASM backend failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

struct WasmLinker {
    path: PathBuf,
    needs_flavor: bool,
}

fn wasm_linker() -> Result<WasmLinker, String> {
    if let Some(path) = find_on_path("wasm-ld") {
        return Ok(WasmLinker {
            path,
            needs_flavor: false,
        });
    }
    let sysroot = rustc_sysroot()?;
    let wasm_ld = sysroot
        .join("lib")
        .join("rustlib")
        .join(host_target())
        .join("bin")
        .join("gcc-ld")
        .join("wasm-ld");
    if wasm_ld.exists() {
        return Ok(WasmLinker {
            path: wasm_ld,
            needs_flavor: false,
        });
    }
    if let Some(path) = find_on_path("rust-lld") {
        return Ok(WasmLinker {
            path,
            needs_flavor: true,
        });
    }
    let rust_lld = sysroot
        .join("lib")
        .join("rustlib")
        .join(host_target())
        .join("bin")
        .join("rust-lld");
    if rust_lld.exists() {
        return Ok(WasmLinker {
            path: rust_lld,
            needs_flavor: true,
        });
    }
    for candidate in rustup_wasm_linker_candidates() {
        if candidate.path.exists() {
            return Ok(candidate);
        }
    }
    Err("WASM backend requires `wasm-ld` or `rust-lld` from a Rust toolchain".to_string())
}

fn rustc_sysroot() -> Result<PathBuf, String> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map_err(|error| format!("failed to invoke rustc to locate WASM linker: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "failed to locate Rust sysroot for WASM linker:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(program))
            .find(|path| path.exists())
    })
}

fn rustup_wasm_linker_candidates() -> Vec<WasmLinker> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let toolchains = PathBuf::from(home).join(".rustup").join("toolchains");
    let Ok(entries) = fs::read_dir(toolchains) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let root = entry.path();
        let bin = root
            .join("lib")
            .join("rustlib")
            .join(host_target())
            .join("bin");
        candidates.push(WasmLinker {
            path: bin.join("gcc-ld").join("wasm-ld"),
            needs_flavor: false,
        });
        candidates.push(WasmLinker {
            path: bin.join("rust-lld"),
            needs_flavor: true,
        });
    }
    candidates
}

fn emit_native_runtime_llvm(
    runtime_c: &str,
    runtime_llvm_path: &Path,
    options: &BuildOptions,
) -> Result<(), String> {
    let mut clang = Command::new("clang");
    clang
        .arg(options.optimization.clang_flag())
        .arg("-pthread")
        .args(&options.compiler_flags)
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

fn dynamic_library_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{name}.dylib")
    } else {
        format!("lib{name}.so")
    }
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

fn foreign_c_hash_input(sources: &[PathBuf]) -> Result<String, String> {
    let mut input = String::new();
    for source in sources {
        input.push_str(&source.to_string_lossy());
        input.push('\0');
        let bytes = fs::read(source).map_err(|error| {
            format!(
                "failed to read foreign C source `{}`: {error}",
                source.display()
            )
        })?;
        input.push_str(&format!("{:x}", bytes.len()));
        input.push('\0');
        input.push_str(&String::from_utf8_lossy(&bytes));
        input.push('\0');
    }
    Ok(input)
}

fn build_hash(
    target: &BuildTarget,
    options: &BuildOptions,
    source: &str,
    runtime_c: &str,
    foreign_c: &str,
) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    let options_hash = format!(
        "{:?}:{:?}:{:?}:{:?}",
        options.crate_type, options.optimization, options.compiler_flags, options.linker_flags
    );
    for byte in env!("CARGO_PKG_VERSION")
        .as_bytes()
        .iter()
        .chain(b":llvm-runtime-ir-v2:")
        .chain(target.triple().as_bytes())
        .chain(b":")
        .chain(options_hash.as_bytes())
        .chain(b":")
        .chain(source.as_bytes())
        .chain(runtime_c.as_bytes())
        .chain(foreign_c.as_bytes())
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
        assert_eq!(
            BuildTarget::from_str("typescript"),
            Ok(BuildTarget::Typescript)
        );
        assert_eq!(BuildTarget::from_str("ts"), Ok(BuildTarget::Typescript));
        assert_eq!(
            BuildTarget::from_str("javascript"),
            Ok(BuildTarget::Javascript)
        );
        assert_eq!(BuildTarget::from_str("js"), Ok(BuildTarget::Javascript));
    }

    #[test]
    fn parses_supported_crate_types() {
        assert_eq!(CrateType::from_str("bin"), Ok(CrateType::Bin));
        assert_eq!(CrateType::from_str("cdylib"), Ok(CrateType::Cdylib));
    }

    #[test]
    fn parses_clang_style_optimization_flags() {
        assert_eq!(
            BuildOptimization::from_clang_flag("-O0"),
            Some(BuildOptimization::O0)
        );
        assert_eq!(
            BuildOptimization::from_clang_flag("-O2"),
            Some(BuildOptimization::O2)
        );
        assert_eq!(
            BuildOptimization::from_clang_flag("-O3"),
            Some(BuildOptimization::O3)
        );
        assert_eq!(
            BuildOptimization::from_clang_flag("-Oz"),
            Some(BuildOptimization::Oz)
        );
        assert_eq!(BuildOptimization::from_clang_flag("-g"), None);
    }

    #[test]
    fn wasm_build_target_requires_cdylib() {
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

        assert!(error.contains("WASM builds currently require `--crate-type cdylib`"));
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
