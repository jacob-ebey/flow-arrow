use crate::{ast::Module, codegen, parser, typecheck};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

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
    pub gpu: bool,
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
            gpu: false,
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
            gpu: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BuildOptimization {
    O0,
    O1,
    O2,
    #[default]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrateType {
    #[default]
    Bin,
    Cdylib,
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

struct NativeGpuRuntime {
    link_dir: PathBuf,
    link_name: &'static str,
}

struct GpuRuntimeCacheLock {
    path: PathBuf,
}

impl Drop for GpuRuntimeCacheLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

pub fn build_file(path: &Path, emit_llvm: Option<&Path>) -> Result<BuildOutput, String> {
    let options = BuildOptions {
        emit_llvm: emit_llvm.map(PathBuf::from),
        ..BuildOptions::default()
    };
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
    if options.gpu
        && !matches!(
            options.target,
            BuildTarget::Typescript | BuildTarget::Javascript | BuildTarget::Native(_)
        )
    {
        return Err(
            "`--gpu` is supported for native, TypeScript, and JavaScript builds".to_string(),
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
            gpu: options.gpu,
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
    if options.gpu {
        build_wasm_gpu_runtime(&build_dir)?;
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
            gpu: options.gpu,
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
    if options.gpu {
        build_wasm_gpu_runtime(&build_dir)?;
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
    let llvm = lowered.emit_direct_llvm_with_gpu(options.gpu)?;
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
    let gpu_runtime = if options.gpu {
        Some(build_native_gpu_runtime(&plan)?)
    } else {
        None
    };

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
    link_native_executable(
        &plan,
        &runtime_c,
        &foreign_c_objects,
        gpu_runtime.as_ref(),
        options,
    )?;
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
    let emitted = lowered.emit_native_cdylib_c_with_gpu(options.gpu)?;
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
    let gpu_runtime = if options.gpu {
        Some(build_native_gpu_runtime(&plan)?)
    } else {
        None
    };

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
    link_native_cdylib_c(
        &plan,
        &c_path,
        &foreign_c_objects,
        gpu_runtime.as_ref(),
        options,
    )?;
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
    gpu_runtime: Option<&NativeGpuRuntime>,
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
    add_native_gpu_link_flags(&mut clang, gpu_runtime);
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
    gpu_runtime: Option<&NativeGpuRuntime>,
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
    add_native_gpu_link_flags(&mut clang, gpu_runtime);
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

fn add_native_gpu_link_flags(clang: &mut Command, gpu_runtime: Option<&NativeGpuRuntime>) {
    let Some(runtime) = gpu_runtime else {
        return;
    };
    clang
        .arg("-L")
        .arg(&runtime.link_dir)
        .arg(format!("-l{}", runtime.link_name));
    if cfg!(target_os = "macos") {
        clang.arg("-Wl,-rpath,@executable_path");
        clang.arg("-Wl,-rpath,@loader_path");
    } else if cfg!(target_os = "linux") {
        clang.arg("-Wl,-rpath,$ORIGIN");
    }
}

fn build_native_gpu_runtime(plan: &BuildPlan) -> Result<NativeGpuRuntime, String> {
    let cache_dir = gpu_runtime_cache_dir()?;
    let cache_key = gpu_runtime_cache_key("native", &host_target(), native_gpu_runtime_manifest());
    let cache_entry = cache_dir.join(cache_key);
    let built = cache_entry.join(dynamic_library_name("flowarrow_gpu_runtime"));
    if should_rebuild_gpu_runtime() || !built.exists() {
        let _lock = lock_gpu_runtime_cache(&cache_entry)?;
        if should_rebuild_gpu_runtime() || !built.exists() {
            build_cached_native_gpu_runtime(&cache_dir, &cache_entry)?;
        }
    }
    let bundled = plan
        .build_dir
        .join(dynamic_library_name("flowarrow_gpu_runtime"));
    fs::copy(&built, &bundled).map_err(|error| {
        format!(
            "failed to copy native GPU runtime from `{}` to `{}`: {error}",
            built.display(),
            bundled.display()
        )
    })?;
    Ok(NativeGpuRuntime {
        link_dir: plan.build_dir.clone(),
        link_name: "flowarrow_gpu_runtime",
    })
}

fn build_wasm_gpu_runtime(build_dir: &Path) -> Result<(), String> {
    let cache_dir = gpu_runtime_cache_dir()?;
    let cache_key = gpu_runtime_cache_key(
        "webgpu",
        "wasm32-unknown-unknown",
        wasm_gpu_runtime_manifest(),
    );
    let cache_entry = cache_dir.join(cache_key);
    let cached_wasm = cache_entry.join("flowarrow_gpu_runtime_bg.wasm");
    let cached_mjs = cache_entry.join("flowarrow_gpu_runtime.mjs");
    if should_rebuild_gpu_runtime() || !cached_wasm.exists() || !cached_mjs.exists() {
        let _lock = lock_gpu_runtime_cache(&cache_entry)?;
        if should_rebuild_gpu_runtime() || !cached_wasm.exists() || !cached_mjs.exists() {
            build_cached_wasm_gpu_runtime(&cache_dir, &cache_entry)?;
        }
    }
    copy_wasm_gpu_runtime_from_cache(&cache_entry, build_dir)
}

fn build_cached_native_gpu_runtime(cache_dir: &Path, cache_entry: &Path) -> Result<(), String> {
    let staging = staging_cache_dir(cache_dir, cache_entry);
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|error| format!("failed to clear `{}`: {error}", staging.display()))?;
    }
    let runtime_dir = staging.join("build");
    let src_dir = runtime_dir.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", src_dir.display()))?;
    let manifest = runtime_dir.join("Cargo.toml");
    let source = src_dir.join("lib.rs");
    fs::write(&manifest, native_gpu_runtime_manifest())
        .map_err(|error| format!("failed to write `{}`: {error}", manifest.display()))?;
    fs::write(&source, gpu_runtime_source())
        .map_err(|error| format!("failed to write `{}`: {error}", source.display()))?;

    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .map_err(|error| format!("failed to invoke cargo for native GPU runtime: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "native GPU runtime build failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let built = runtime_dir
        .join("target")
        .join("release")
        .join(dynamic_library_name("flowarrow_gpu_runtime"));
    let cached = staging.join(dynamic_library_name("flowarrow_gpu_runtime"));
    fs::copy(&built, &cached).map_err(|error| {
        format!(
            "failed to copy native GPU runtime from `{}` to `{}`: {error}",
            built.display(),
            cached.display()
        )
    })?;
    if cfg!(target_os = "macos") {
        let install_name = format!("@rpath/{}", dynamic_library_name("flowarrow_gpu_runtime"));
        let output = Command::new("install_name_tool")
            .arg("-id")
            .arg(&install_name)
            .arg(&cached)
            .output()
            .map_err(|error| {
                format!(
                    "failed to invoke install_name_tool for native GPU runtime `{}`: {error}",
                    cached.display()
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "failed to set native GPU runtime install name:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    let _ = fs::remove_dir_all(&runtime_dir);
    publish_gpu_runtime_cache(&staging, cache_entry)
}

fn build_cached_wasm_gpu_runtime(cache_dir: &Path, cache_entry: &Path) -> Result<(), String> {
    let staging = staging_cache_dir(cache_dir, cache_entry);
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|error| format!("failed to clear `{}`: {error}", staging.display()))?;
    }
    let runtime_dir = staging.join("build");
    let src_dir = runtime_dir.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", src_dir.display()))?;
    let manifest = runtime_dir.join("Cargo.toml");
    let source = src_dir.join("lib.rs");
    fs::write(&manifest, wasm_gpu_runtime_manifest())
        .map_err(|error| format!("failed to write `{}`: {error}", manifest.display()))?;
    fs::write(&source, gpu_runtime_source())
        .map_err(|error| format!("failed to write `{}`: {error}", source.display()))?;

    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .map_err(|error| format!("failed to invoke cargo for WebGPU WASM runtime: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "WebGPU WASM runtime build failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let wasm = runtime_dir
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("flowarrow_gpu_runtime.wasm");
    let output = run_wasm_bindgen(cache_dir, &staging, &wasm)?;
    if !output.status.success() {
        return Err(format!(
            "wasm-bindgen failed for WebGPU runtime:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let js = staging.join("flowarrow_gpu_runtime.js");
    let mjs = staging.join("flowarrow_gpu_runtime.mjs");
    if mjs.exists() {
        fs::remove_file(&mjs)
            .map_err(|error| format!("failed to replace `{}`: {error}", mjs.display()))?;
    }
    fs::rename(&js, &mjs).map_err(|error| {
        format!(
            "failed to rename WebGPU runtime glue `{}` to `{}`: {error}",
            js.display(),
            mjs.display()
        )
    })?;
    let _ = fs::remove_dir_all(&runtime_dir);
    publish_gpu_runtime_cache(&staging, cache_entry)
}

fn copy_wasm_gpu_runtime_from_cache(cache_entry: &Path, build_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(build_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", build_dir.display()))?;
    let stale_js = build_dir.join("flowarrow_gpu_runtime.js");
    if stale_js.exists() {
        fs::remove_file(&stale_js).map_err(|error| {
            format!(
                "failed to remove stale WebGPU runtime glue `{}`: {error}",
                stale_js.display()
            )
        })?;
    }
    for file in [
        "flowarrow_gpu_runtime.mjs",
        "flowarrow_gpu_runtime_bg.wasm",
        "flowarrow_gpu_runtime.d.ts",
        "flowarrow_gpu_runtime_bg.wasm.d.ts",
    ] {
        let source = cache_entry.join(file);
        let destination = build_dir.join(file);
        fs::copy(&source, &destination).map_err(|error| {
            format!(
                "failed to copy WebGPU runtime artifact from `{}` to `{}`: {error}",
                source.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn gpu_runtime_cache_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("FLOWARROW_GPU_RUNTIME_CACHE") {
        let path = PathBuf::from(path);
        fs::create_dir_all(&path)
            .map_err(|error| format!("failed to create `{}`: {error}", path.display()))?;
        return Ok(path);
    }

    let root = if cfg!(target_os = "macos") {
        home_dir()
            .map(|home| home.join("Library").join("Caches"))
            .ok_or_else(|| {
                "failed to locate home directory for FlowArrow GPU runtime cache".to_string()
            })?
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join("AppData").join("Local")))
            .ok_or_else(|| {
                "failed to locate local app data directory for FlowArrow GPU runtime cache"
                    .to_string()
            })?
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join(".cache")))
            .ok_or_else(|| {
                "failed to locate cache directory for FlowArrow GPU runtime cache".to_string()
            })?
    };
    let path = root.join("flowarrow").join("gpu-runtime");
    fs::create_dir_all(&path)
        .map_err(|error| format!("failed to create `{}`: {error}", path.display()))?;
    Ok(path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn gpu_runtime_cache_key(kind: &str, target: &str, manifest: &str) -> String {
    let hash = fnv_hash(
        [
            b"flowarrow-gpu-runtime-cache-v2".as_slice(),
            env!("CARGO_PKG_VERSION").as_bytes(),
            kind.as_bytes(),
            target.as_bytes(),
            manifest.as_bytes(),
            gpu_runtime_source().as_bytes(),
        ]
        .into_iter(),
    );
    format!("{kind}-{target}-{hash:016x}")
}

fn fnv_hash<'a>(parts: impl Iterator<Item = &'a [u8]>) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for part in parts {
        for byte in part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn should_rebuild_gpu_runtime() -> bool {
    std::env::var_os("FLOWARROW_REBUILD_GPU_RUNTIME")
        .map(|value| {
            let value = value.to_string_lossy();
            !value.is_empty() && value != "0" && value != "false" && value != "FALSE"
        })
        .unwrap_or(false)
}

fn lock_gpu_runtime_cache(cache_entry: &Path) -> Result<GpuRuntimeCacheLock, String> {
    let lock = cache_entry.with_extension("lock");
    let started = Instant::now();
    loop {
        match fs::create_dir(&lock) {
            Ok(()) => return Ok(GpuRuntimeCacheLock { path: lock }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if started.elapsed() > Duration::from_secs(300) {
                    return Err(format!(
                        "timed out waiting for FlowArrow GPU runtime cache lock `{}`",
                        lock.display()
                    ));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(format!(
                    "failed to create FlowArrow GPU runtime cache lock `{}`: {error}",
                    lock.display()
                ));
            }
        }
    }
}

fn staging_cache_dir(cache_dir: &Path, cache_entry: &Path) -> PathBuf {
    let name = cache_entry
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "runtime".into());
    cache_dir.join(format!("{name}.staging.{}", std::process::id()))
}

fn publish_gpu_runtime_cache(staging: &Path, cache_entry: &Path) -> Result<(), String> {
    if cache_entry.exists() {
        fs::remove_dir_all(cache_entry)
            .map_err(|error| format!("failed to replace `{}`: {error}", cache_entry.display()))?;
    }
    fs::rename(staging, cache_entry).map_err(|error| {
        format!(
            "failed to publish FlowArrow GPU runtime cache `{}` to `{}`: {error}",
            staging.display(),
            cache_entry.display()
        )
    })
}

fn run_wasm_bindgen(
    cache_dir: &Path,
    build_dir: &Path,
    wasm: &Path,
) -> Result<std::process::Output, String> {
    let args = [
        "--target".to_string(),
        "web".to_string(),
        "--out-dir".to_string(),
        build_dir.to_string_lossy().into_owned(),
        "--out-name".to_string(),
        "flowarrow_gpu_runtime".to_string(),
        wasm.to_string_lossy().into_owned(),
    ];
    match Command::new("wasm-bindgen").args(&args).output() {
        Ok(output) => Ok(output),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            run_cached_wasm_bindgen_cli(cache_dir, &args)
        }
        Err(error) => Err(format!(
            "failed to invoke wasm-bindgen for WebGPU runtime: {error}"
        )),
    }
}

fn run_cached_wasm_bindgen_cli(
    runtime_dir: &Path,
    args: &[String],
) -> Result<std::process::Output, String> {
    let install_dir = runtime_dir.join("wasm-bindgen-cli");
    let bin = install_dir
        .join("bin")
        .join(format!("wasm-bindgen{}", std::env::consts::EXE_SUFFIX));
    if !bin.exists() {
        let output = Command::new("cargo")
            .arg("install")
            .arg("wasm-bindgen-cli")
            .arg("--version")
            .arg("0.2.122")
            .arg("--locked")
            .arg("--root")
            .arg(&install_dir)
            .output()
            .map_err(|error| format!("failed to install wasm-bindgen CLI: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "failed to install wasm-bindgen CLI for WebGPU runtime:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    Command::new(&bin).args(args).output().map_err(|error| {
        format!(
            "failed to invoke cached wasm-bindgen CLI `{}`: {error}",
            bin.display()
        )
    })
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

fn native_gpu_runtime_manifest() -> &'static str {
    "[package]\n\
name = \"flowarrow_gpu_runtime\"\n\
version = \"0.1.0\"\n\
edition = \"2024\"\n\
\n\
[lib]\n\
crate-type = [\"cdylib\"]\n\
\n\
[dependencies]\n\
wgpu = \"29.0.3\"\n"
}

fn wasm_gpu_runtime_manifest() -> &'static str {
    "[package]\n\
name = \"flowarrow_gpu_runtime\"\n\
version = \"0.1.0\"\n\
edition = \"2024\"\n\
\n\
[lib]\n\
crate-type = [\"cdylib\"]\n\
\n\
[dependencies]\n\
wasm-bindgen = \"0.2.122\"\n\
wasm-bindgen-futures = \"0.4.72\"\n\
wgpu = \"29.0.3\"\n"
}

fn gpu_runtime_source() -> &'static str {
    r##"#[cfg(not(target_arch = "wasm32"))]
use std::ffi::CStr;
use std::future::Future;
#[cfg(not(target_arch = "wasm32"))]
use std::os::raw::c_char;
use std::slice;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::task::{Context, Poll, Wake, Waker};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use wgpu::util::DeviceExt;

struct GpuRuntime {
    device: wgpu::Device,
    queue: wgpu::Queue,
    features: wgpu::Features,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FaGpuSliceF64 {
    count: usize,
    items: *const f64,
}

struct GpuMatrixInput {
    rows: usize,
    cols: usize,
    values: Vec<f64>,
}

#[cfg(not(target_arch = "wasm32"))]
static GPU: OnceLock<Mutex<GpuRuntime>> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub extern "C" fn fa_gpu_require_device() {
    let _ = runtime();
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_require_device() -> Result<(), JsValue> {
    GpuRuntime::new()
        .await
        .map(|_| ())
        .map_err(|error| JsValue::from_str(&error))
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_map_i32(
    wgsl: *const c_char,
    input: *const i32,
    output: *mut i32,
    count: usize,
) {
    let wgsl = read_wgsl(wgsl);
    if count > 0 && (input.is_null() || output.is_null()) {
        fail("FlowArrow GPU target received null sequence storage");
    }
    let input = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(input, count) }
    };
    let output = if count == 0 {
        &mut []
    } else {
        unsafe { slice::from_raw_parts_mut(output, count) }
    };
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    let mapped = runtime.map_i32(&wgsl, input);
    output.copy_from_slice(&mapped);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_map_i32(wgsl: String, input: Vec<i32>) -> Result<Vec<i32>, JsValue> {
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    Ok(runtime.map_i32(&wgsl, &input).await)
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_map_f32(
    wgsl: *const c_char,
    input: *const f32,
    output: *mut f32,
    count: usize,
) {
    let wgsl = read_wgsl(wgsl);
    if count > 0 && (input.is_null() || output.is_null()) {
        fail("FlowArrow GPU target received null sequence storage");
    }
    let input = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(input, count) }
    };
    let output = if count == 0 {
        &mut []
    } else {
        unsafe { slice::from_raw_parts_mut(output, count) }
    };
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    runtime.map_f32(&wgsl, input, output);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_map_f32(wgsl: String, input: Vec<f32>) -> Result<Vec<f32>, JsValue> {
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    Ok(runtime.map_f32(&wgsl, &input).await)
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_map_f64(
    wgsl: *const c_char,
    input: *const f64,
    output: *mut f64,
    count: usize,
) {
    let wgsl = read_wgsl(wgsl);
    if count > 0 && (input.is_null() || output.is_null()) {
        fail("FlowArrow GPU target received null sequence storage");
    }
    let input = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(input, count) }
    };
    let output = if count == 0 {
        &mut []
    } else {
        unsafe { slice::from_raw_parts_mut(output, count) }
    };
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    runtime.map_f64(&wgsl, input, output);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_map_f64(wgsl: String, input: Vec<f64>) -> Result<Vec<f64>, JsValue> {
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    runtime.map_f64(&wgsl, &input).await
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_reduce_i32(
    op: u32,
    input: *const i32,
    count: usize,
    identity: i32,
) -> i32 {
    if count > 0 && input.is_null() {
        fail("FlowArrow GPU target received null reduce input");
    }
    let input = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(input, count) }
    };
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    runtime.reduce_i32(op, input, identity)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_reduce_i32(
    op: u32,
    input: Vec<i32>,
    identity: i32,
) -> Result<i32, JsValue> {
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    Ok(runtime.reduce_i32(op, &input, identity).await)
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_reduce_f32(
    op: u32,
    input: *const f32,
    count: usize,
    identity: f32,
) -> f32 {
    if count > 0 && input.is_null() {
        fail("FlowArrow GPU target received null reduce input");
    }
    let input = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(input, count) }
    };
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    runtime.reduce_f32(op, input, identity)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_reduce_f32(
    op: u32,
    input: Vec<f32>,
    identity: f32,
) -> Result<f32, JsValue> {
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    Ok(runtime.reduce_f32(op, &input, identity).await)
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_reduce_f64(
    op: u32,
    input: *const f64,
    count: usize,
    identity: f64,
) -> f64 {
    if count > 0 && input.is_null() {
        fail("FlowArrow GPU target received null reduce input");
    }
    let input = if count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(input, count) }
    };
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    runtime.reduce_f64(op, input, identity)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_reduce_f64(
    op: u32,
    input: Vec<f64>,
    identity: f64,
) -> Result<f64, JsValue> {
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    runtime.reduce_f64(op, &input, identity).await
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_repeat_vector_accum_f64(
    wgsl: *const c_char,
    left: *const f64,
    left_count: usize,
    right: *const f64,
    right_count: usize,
    score: f64,
    iterations: i64,
) -> f64 {
    if iterations <= 0 {
        return score;
    }
    let wgsl = read_wgsl(wgsl);
    if left_count != right_count {
        fail("FlowArrow GPU vector accumulator requires equal vector lengths");
    }
    if left_count > 0 && (left.is_null() || right.is_null()) {
        fail("FlowArrow GPU vector accumulator received null vector storage");
    }
    let left = if left_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(left, left_count) }
    };
    let right = if right_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(right, right_count) }
    };
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    let delta = runtime.reduce_program_f64(&wgsl, &[left.to_vec(), right.to_vec()], &[], left_count);
    score + iterations as f64 * delta
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_repeat_vector_accum_f64(
    wgsl: String,
    left: Vec<f64>,
    right: Vec<f64>,
    score: f64,
    iterations: i64,
) -> Result<f64, JsValue> {
    if iterations <= 0 {
        return Ok(score);
    }
    if left.len() != right.len() {
        return Err(JsValue::from_str(
            "FlowArrow GPU vector accumulator requires equal vector lengths",
        ));
    }
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    let work_items = left.len();
    let delta = runtime
        .reduce_program_f64(&wgsl, &[left, right], &[], work_items)
        .await?;
    Ok(score + iterations as f64 * delta)
}

#[cfg(not(target_arch = "wasm32"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fa_gpu_repeat_matrix_accum_f64(
    wgsl: *const c_char,
    left_rows: *const FaGpuSliceF64,
    left_count: usize,
    right_rows: *const FaGpuSliceF64,
    right_count: usize,
    vector: *const f64,
    vector_count: usize,
    score: f64,
    iterations: i64,
) -> f64 {
    if iterations <= 0 {
        return score;
    }
    let wgsl = read_wgsl(wgsl);
    if left_count > 0 && left_rows.is_null() {
        fail("FlowArrow GPU matrix accumulator received null left matrix rows");
    }
    if right_count > 0 && right_rows.is_null() {
        fail("FlowArrow GPU matrix accumulator received null right matrix rows");
    }
    if vector_count > 0 && vector.is_null() {
        fail("FlowArrow GPU matrix accumulator received null vector storage");
    }
    let left_rows = if left_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(left_rows, left_count) }
    };
    let right_rows = if right_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(right_rows, right_count) }
    };
    let vector = if vector_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(vector, vector_count) }
    };
    let (left, left_shape) = flatten_matrix(left_rows, "left");
    let (right, right_shape) = flatten_matrix(right_rows, "right");
    if left_shape.1 != right_shape.0 {
        fail("FlowArrow GPU matrix accumulator requires left columns to equal right rows");
    }
    if vector_count != left_shape.1 {
        fail("FlowArrow GPU matrix accumulator requires vector length to equal left columns");
    }
    let work_items = left_shape.0.checked_mul(left_shape.1).unwrap_or_else(|| {
        fail("FlowArrow GPU matrix accumulator work size overflowed");
    });
    let mut runtime = runtime().lock().unwrap_or_else(|_| {
        fail("FlowArrow GPU target runtime lock was poisoned");
    });
    let delta = runtime.reduce_program_f64(
        &wgsl,
        &[vector.to_vec()],
        &[
            GpuMatrixInput {
                rows: left_shape.0,
                cols: left_shape.1,
                values: left,
            },
            GpuMatrixInput {
                rows: right_shape.0,
                cols: right_shape.1,
                values: right,
            },
        ],
        work_items,
    );
    score + iterations as f64 * delta
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn fa_gpu_repeat_matrix_accum_f64(
    wgsl: String,
    left_values: Vec<f64>,
    left_rows: u32,
    left_cols: u32,
    right_values: Vec<f64>,
    right_rows: u32,
    right_cols: u32,
    vector: Vec<f64>,
    score: f64,
    iterations: i64,
) -> Result<f64, JsValue> {
    if iterations <= 0 {
        return Ok(score);
    }
    let left_shape = (left_rows as usize, left_cols as usize);
    let right_shape = (right_rows as usize, right_cols as usize);
    if left_values.len() != left_shape.0.saturating_mul(left_shape.1) {
        return Err(JsValue::from_str(
            "FlowArrow GPU matrix accumulator received malformed left matrix storage",
        ));
    }
    if right_values.len() != right_shape.0.saturating_mul(right_shape.1) {
        return Err(JsValue::from_str(
            "FlowArrow GPU matrix accumulator received malformed right matrix storage",
        ));
    }
    if left_shape.1 != right_shape.0 {
        return Err(JsValue::from_str(
            "FlowArrow GPU matrix accumulator requires left columns to equal right rows",
        ));
    }
    if vector.len() != left_shape.1 {
        return Err(JsValue::from_str(
            "FlowArrow GPU matrix accumulator requires vector length to equal left columns",
        ));
    }
    let work_items = left_shape
        .0
        .checked_mul(left_shape.1)
        .ok_or_else(|| JsValue::from_str("FlowArrow GPU matrix accumulator work size overflowed"))?;
    let mut runtime = GpuRuntime::new()
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    let delta = runtime
        .reduce_program_f64(
            &wgsl,
            &[vector],
            &[
                GpuMatrixInput {
                    rows: left_shape.0,
                    cols: left_shape.1,
                    values: left_values,
                },
                GpuMatrixInput {
                    rows: right_shape.0,
                    cols: right_shape.1,
                    values: right_values,
                },
            ],
            work_items,
        )
        .await?;
    Ok(score + iterations as f64 * delta)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_wgsl(wgsl: *const c_char) -> String {
    if wgsl.is_null() {
        fail("FlowArrow GPU target received a null WGSL kernel");
    }
    unsafe { CStr::from_ptr(wgsl) }
        .to_str()
        .unwrap_or_else(|_| fail("FlowArrow GPU target received non-UTF-8 WGSL"))
        .to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn runtime() -> &'static Mutex<GpuRuntime> {
    GPU.get_or_init(|| {
        let runtime = block_on(GpuRuntime::new()).unwrap_or_else(|error| {
            fail(&format!("FlowArrow GPU target requires a native GPU device: {error}"));
        });
        Mutex::new(runtime)
    })
}

#[cfg(not(target_arch = "wasm32"))]
impl GpuRuntime {
    async fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        let features = adapter.features() & wgpu::Features::SHADER_F64;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: features,
                ..wgpu::DeviceDescriptor::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            device,
            queue,
            features,
        })
    }

    fn map_i32(&mut self, wgsl: &str, input: &[i32]) -> Vec<i32> {
        let bytes = self.dispatch_map(wgsl, i32_as_bytes(input), input.len());
        bytes_as_i32(&bytes).to_vec()
    }

    fn map_f32(&mut self, wgsl: &str, input: &[f32], output: &mut [f32]) {
        if input.len() != output.len() {
            fail("FlowArrow GPU map received mismatched input and output lengths");
        }
        if input.is_empty() {
            return;
        }

        let mapped = self.dispatch_map(wgsl, f32_as_bytes(input), input.len());
        output.copy_from_slice(bytes_as_f32(&mapped));
    }

    fn map_f64(&mut self, wgsl: &str, input: &[f64], output: &mut [f64]) {
        if input.len() != output.len() {
            fail("FlowArrow GPU map received mismatched input and output lengths");
        }
        if input.is_empty() {
            return;
        }
        self.require_f64_support();
        let mapped = self.dispatch_map(wgsl, f64_as_bytes(input), input.len());
        output.copy_from_slice(bytes_as_f64(&mapped));
    }

    fn dispatch_map(&mut self, wgsl: &str, input_bytes: &[u8], len: usize) -> Vec<u8> {
        if len == 0 {
            return Vec::new();
        }
        let input_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.input"),
            contents: input_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let byte_len = input_bytes.len() as wgpu::BufferAddress;
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flowarrow.gpu.output"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params_buffer = self.map_params_buffer(len);

        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowarrow.gpu.kernel"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flowarrow.gpu.pipeline"),
            layout: None,
            module: &shader,
            entry_point: None,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flowarrow.gpu.bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flowarrow.gpu.encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flowarrow.gpu.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(checked_workgroups(len, "map input length"), 1, 1);
        }
        let readback = self.readback_buffer(byte_len);
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &readback, 0, byte_len);
        self.queue.submit(Some(encoder.finish()));
        self.read_buffer(&readback)
    }

    fn reduce_i32(&mut self, op: u32, input: &[i32], identity: i32) -> i32 {
        let bytes = self.dispatch_reduce("i32", op, i32_as_bytes(input), input.len(), identity);
        bytes_as_i32(&bytes)[0]
    }

    fn reduce_f32(&mut self, op: u32, input: &[f32], identity: f32) -> f32 {
        let bytes = self.dispatch_reduce("f32", op, f32_as_bytes(input), input.len(), identity);
        bytes_as_f32(&bytes)[0]
    }

    fn reduce_f64(&mut self, op: u32, input: &[f64], identity: f64) -> f64 {
        self.require_f64_support();
        let bytes = self.dispatch_reduce("f64", op, f64_as_bytes(input), input.len(), identity);
        bytes_as_f64(&bytes)[0]
    }

    fn reduce_program_f64(
        &mut self,
        wgsl: &str,
        slices: &[Vec<f64>],
        matrices: &[GpuMatrixInput],
        work_items: usize,
    ) -> f64 {
        self.require_f64_support();
        if work_items == 0 {
            return 0.0;
        }
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flowarrow.gpu.program.output"),
            size: (work_items * 8).max(8) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params_buffer = self.program_params_buffer(slices, matrices, work_items);
        let mut storage_buffers = Vec::with_capacity(slices.len() + matrices.len());
        for slice in slices {
            storage_buffers.push(self.storage_f64_buffer("flowarrow.gpu.program.slice", slice));
        }
        for matrix in matrices {
            storage_buffers
                .push(self.storage_f64_buffer("flowarrow.gpu.program.matrix", &matrix.values));
        }

        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowarrow.gpu.program.kernel"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flowarrow.gpu.program.pipeline"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let mut entries = Vec::with_capacity(2 + storage_buffers.len());
        entries.push(wgpu::BindGroupEntry {
            binding: 0,
            resource: output_buffer.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 1,
            resource: params_buffer.as_entire_binding(),
        });
        for (index, buffer) in storage_buffers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: (2 + index) as u32,
                resource: buffer.as_entire_binding(),
            });
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flowarrow.gpu.program.bind_group"),
            layout: &bind_group_layout,
            entries: &entries,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flowarrow.gpu.program.encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flowarrow.gpu.program.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(checked_workgroups(work_items, "GPU program work item count"), 1, 1);
        }
        self.queue.submit(Some(encoder.finish()));
        let bytes = self.dispatch_reduce_buffer("f64", 0, output_buffer, work_items, 0.0, false);
        bytes_as_f64(&bytes)[0]
    }

    fn dispatch_reduce<T: GpuParam>(
        &mut self,
        scalar: &str,
        op: u32,
        input_bytes: &[u8],
        len: usize,
        identity: T,
    ) -> Vec<u8> {
        if len == 0 {
            return identity.to_bytes();
        }
        let input_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.reduce.input"),
            contents: input_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        self.dispatch_reduce_buffer(scalar, op, input_buffer, len, identity, true)
    }

    fn dispatch_reduce_buffer<T: GpuParam>(
        &mut self,
        scalar: &str,
        op: u32,
        input_buffer: wgpu::Buffer,
        len: usize,
        identity: T,
        include_initial_identity: bool,
    ) -> Vec<u8> {
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowarrow.gpu.reduce.kernel"),
            source: wgpu::ShaderSource::Wgsl(reduce_wgsl(scalar).into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flowarrow.gpu.reduce.pipeline"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let mut current_len = len;
        let mut current_buffer = input_buffer;
        let mut keep_alive = Vec::new();
        let mut include_identity = include_initial_identity;
        while current_len > 1 || include_identity {
            let virtual_len = current_len + usize::from(include_identity);
            let output_len = virtual_len.div_ceil(2);
            let byte_len = (output_len * T::BYTE_LEN).max(T::BYTE_LEN) as wgpu::BufferAddress;
            let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("flowarrow.gpu.reduce.output"),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let params_buffer = self.reduce_params_buffer(op, current_len, identity, include_identity);
            let bind_group_layout = pipeline.get_bind_group_layout(0);
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("flowarrow.gpu.reduce.bind_group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: current_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params_buffer.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("flowarrow.gpu.reduce.encoder"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("flowarrow.gpu.reduce.pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(checked_workgroups(output_len, "reduce output length"), 1, 1);
            }
            self.queue.submit(Some(encoder.finish()));
            keep_alive.push(current_buffer);
            keep_alive.push(params_buffer);
            current_buffer = output_buffer;
            current_len = output_len;
            include_identity = false;
        }
        let readback = self.readback_buffer(T::BYTE_LEN as wgpu::BufferAddress);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flowarrow.gpu.reduce.readback.encoder"),
            });
        encoder.copy_buffer_to_buffer(&current_buffer, 0, &readback, 0, T::BYTE_LEN as u64);
        self.queue.submit(Some(encoder.finish()));
        self.read_buffer(&readback)
    }

    fn map_params_buffer(&self, len: usize) -> wgpu::Buffer {
        let mut params = [0u8; 16];
        params[0..4].copy_from_slice(&checked_usize_u32(len, "GPU map input length").to_le_bytes());
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.map.params"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn reduce_params_buffer<T: GpuParam>(
        &self,
        op: u32,
        len: usize,
        identity: T,
        include_identity: bool,
    ) -> wgpu::Buffer {
        let mut params = vec![0u8; if T::BYTE_LEN == 8 { 24 } else { 16 }];
        params[0..4].copy_from_slice(&checked_usize_u32(len, "GPU reduce input length").to_le_bytes());
        params[4..8].copy_from_slice(&op.to_le_bytes());
        identity.write_bytes(&mut params[8..8 + T::BYTE_LEN]);
        let include_offset = 8 + T::BYTE_LEN;
        params[include_offset..include_offset + 4]
            .copy_from_slice(&(u32::from(include_identity)).to_le_bytes());
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.reduce.params"),
            contents: params.as_slice(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn program_params_buffer(
        &self,
        slices: &[Vec<f64>],
        matrices: &[GpuMatrixInput],
        work_items: usize,
    ) -> wgpu::Buffer {
        if slices.len() > 4 || matrices.len() > 4 {
            fail("FlowArrow GPU generated program exceeded runtime descriptor capacity");
        }
        let mut words = [0u32; 14];
        words[0] = checked_usize_u32(work_items, "GPU program work item count");
        for (index, slice) in slices.iter().enumerate() {
            words[2 + index] = checked_usize_u32(slice.len(), "GPU slice length");
        }
        for (index, matrix) in matrices.iter().enumerate() {
            words[6 + index * 2] = checked_usize_u32(matrix.rows, "GPU matrix row count");
            words[7 + index * 2] = checked_usize_u32(matrix.cols, "GPU matrix column count");
        }
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.program.params"),
            contents: u32_as_bytes(&words),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn storage_f64_buffer(&self, label: &'static str, values: &[f64]) -> wgpu::Buffer {
        if values.is_empty() {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &[0u8; 8],
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        } else {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: f64_as_bytes(values),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        }
    }

    fn require_f64_support(&self) {
        if !self.features.contains(wgpu::Features::SHADER_F64) {
            fail("FlowArrow GPU f64 requires a device with wgpu SHADER_F64 support");
        }
    }

    fn readback_buffer(&self, byte_len: wgpu::BufferAddress) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flowarrow.gpu.readback"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    fn read_buffer(&mut self, readback: &wgpu::Buffer) -> Vec<u8> {
        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap_or_else(|error| {
                fail(&format!("FlowArrow GPU target failed while waiting for device: {error}"));
            });
        rx.recv()
            .unwrap_or_else(|_| fail("FlowArrow GPU target readback channel closed"))
            .unwrap_or_else(|error| {
                fail(&format!("FlowArrow GPU target failed to map readback: {error}"));
            });
        let mapped = slice.get_mapped_range();
        let out = mapped.to_vec();
        drop(mapped);
        readback.unmap();
        out
    }
}

#[cfg(target_arch = "wasm32")]
impl GpuRuntime {
    async fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        let features = adapter.features() & wgpu::Features::SHADER_F64;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: features,
                ..wgpu::DeviceDescriptor::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            device,
            queue,
            features,
        })
    }

    async fn map_i32(&mut self, wgsl: &str, input: &[i32]) -> Vec<i32> {
        let bytes = self.dispatch_map(wgsl, i32_as_bytes(input), input.len()).await;
        bytes_as_i32(&bytes).to_vec()
    }

    async fn map_f32(&mut self, wgsl: &str, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        let mapped = self.dispatch_map(wgsl, f32_as_bytes(input), input.len()).await;
        bytes_as_f32(&mapped).to_vec()
    }

    async fn map_f64(&mut self, wgsl: &str, input: &[f64]) -> Result<Vec<f64>, JsValue> {
        if input.is_empty() {
            return Ok(Vec::new());
        }
        self.require_f64_support()?;
        let mapped = self.dispatch_map(wgsl, f64_as_bytes(input), input.len()).await;
        Ok(bytes_as_f64(&mapped).to_vec())
    }

    async fn dispatch_map(&mut self, wgsl: &str, input_bytes: &[u8], len: usize) -> Vec<u8> {
        if len == 0 {
            return Vec::new();
        }
        let input_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.input"),
            contents: input_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let byte_len = input_bytes.len() as wgpu::BufferAddress;
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flowarrow.gpu.output"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params_buffer = self.map_params_buffer(len);

        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowarrow.gpu.kernel"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flowarrow.gpu.pipeline"),
            layout: None,
            module: &shader,
            entry_point: None,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flowarrow.gpu.bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flowarrow.gpu.encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flowarrow.gpu.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(checked_workgroups(len, "map input length"), 1, 1);
        }
        let readback = self.readback_buffer(byte_len);
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &readback, 0, byte_len);
        self.queue.submit(Some(encoder.finish()));
        self.read_buffer(&readback).await
    }

    async fn reduce_i32(&mut self, op: u32, input: &[i32], identity: i32) -> i32 {
        let bytes = self
            .dispatch_reduce("i32", op, i32_as_bytes(input), input.len(), identity)
            .await;
        bytes_as_i32(&bytes)[0]
    }

    async fn reduce_f32(&mut self, op: u32, input: &[f32], identity: f32) -> f32 {
        let bytes = self
            .dispatch_reduce("f32", op, f32_as_bytes(input), input.len(), identity)
            .await;
        bytes_as_f32(&bytes)[0]
    }

    async fn reduce_f64(&mut self, op: u32, input: &[f64], identity: f64) -> Result<f64, JsValue> {
        self.require_f64_support()?;
        let bytes = self
            .dispatch_reduce("f64", op, f64_as_bytes(input), input.len(), identity)
            .await;
        Ok(bytes_as_f64(&bytes)[0])
    }

    async fn reduce_program_f64(
        &mut self,
        wgsl: &str,
        slices: &[Vec<f64>],
        matrices: &[GpuMatrixInput],
        work_items: usize,
    ) -> Result<f64, JsValue> {
        self.require_f64_support()?;
        if work_items == 0 {
            return Ok(0.0);
        }
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flowarrow.gpu.program.output"),
            size: (work_items * 8).max(8) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let params_buffer = self.program_params_buffer(slices, matrices, work_items);
        let mut storage_buffers = Vec::with_capacity(slices.len() + matrices.len());
        for slice in slices {
            storage_buffers.push(self.storage_f64_buffer("flowarrow.gpu.program.slice", slice));
        }
        for matrix in matrices {
            storage_buffers
                .push(self.storage_f64_buffer("flowarrow.gpu.program.matrix", &matrix.values));
        }

        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowarrow.gpu.program.kernel"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flowarrow.gpu.program.pipeline"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let mut entries = Vec::with_capacity(2 + storage_buffers.len());
        entries.push(wgpu::BindGroupEntry {
            binding: 0,
            resource: output_buffer.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 1,
            resource: params_buffer.as_entire_binding(),
        });
        for (index, buffer) in storage_buffers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: (2 + index) as u32,
                resource: buffer.as_entire_binding(),
            });
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flowarrow.gpu.program.bind_group"),
            layout: &bind_group_layout,
            entries: &entries,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flowarrow.gpu.program.encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flowarrow.gpu.program.pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(checked_workgroups(work_items, "GPU program work item count"), 1, 1);
        }
        self.queue.submit(Some(encoder.finish()));
        let bytes = self
            .dispatch_reduce_buffer("f64", 0, output_buffer, work_items, 0.0, false)
            .await;
        Ok(bytes_as_f64(&bytes)[0])
    }

    async fn dispatch_reduce<T: GpuParam>(
        &mut self,
        scalar: &str,
        op: u32,
        input_bytes: &[u8],
        len: usize,
        identity: T,
    ) -> Vec<u8> {
        if len == 0 {
            return identity.to_bytes();
        }
        let input_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.reduce.input"),
            contents: input_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        self.dispatch_reduce_buffer(scalar, op, input_buffer, len, identity, true)
            .await
    }

    async fn dispatch_reduce_buffer<T: GpuParam>(
        &mut self,
        scalar: &str,
        op: u32,
        input_buffer: wgpu::Buffer,
        len: usize,
        identity: T,
        include_initial_identity: bool,
    ) -> Vec<u8> {
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flowarrow.gpu.reduce.kernel"),
            source: wgpu::ShaderSource::Wgsl(reduce_wgsl(scalar).into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flowarrow.gpu.reduce.pipeline"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let mut current_len = len;
        let mut current_buffer = input_buffer;
        let mut keep_alive = Vec::new();
        let mut include_identity = include_initial_identity;
        while current_len > 1 || include_identity {
            let virtual_len = current_len + usize::from(include_identity);
            let output_len = virtual_len.div_ceil(2);
            let byte_len = (output_len * T::BYTE_LEN).max(T::BYTE_LEN) as wgpu::BufferAddress;
            let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("flowarrow.gpu.reduce.output"),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let params_buffer =
                self.reduce_params_buffer(op, current_len, identity, include_identity);
            let bind_group_layout = pipeline.get_bind_group_layout(0);
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("flowarrow.gpu.reduce.bind_group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: current_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params_buffer.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("flowarrow.gpu.reduce.encoder"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("flowarrow.gpu.reduce.pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(checked_workgroups(output_len, "reduce output length"), 1, 1);
            }
            self.queue.submit(Some(encoder.finish()));
            keep_alive.push(current_buffer);
            keep_alive.push(params_buffer);
            current_buffer = output_buffer;
            current_len = output_len;
            include_identity = false;
        }
        let readback = self.readback_buffer(T::BYTE_LEN as wgpu::BufferAddress);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flowarrow.gpu.reduce.readback.encoder"),
            });
        encoder.copy_buffer_to_buffer(&current_buffer, 0, &readback, 0, T::BYTE_LEN as u64);
        self.queue.submit(Some(encoder.finish()));
        self.read_buffer(&readback).await
    }

    fn map_params_buffer(&self, len: usize) -> wgpu::Buffer {
        let mut params = [0u8; 16];
        params[0..4].copy_from_slice(&checked_usize_u32(len, "GPU map input length").to_le_bytes());
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.map.params"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn reduce_params_buffer<T: GpuParam>(
        &self,
        op: u32,
        len: usize,
        identity: T,
        include_identity: bool,
    ) -> wgpu::Buffer {
        let mut params = vec![0u8; if T::BYTE_LEN == 8 { 24 } else { 16 }];
        params[0..4].copy_from_slice(&checked_usize_u32(len, "GPU reduce input length").to_le_bytes());
        params[4..8].copy_from_slice(&op.to_le_bytes());
        identity.write_bytes(&mut params[8..8 + T::BYTE_LEN]);
        let include_offset = 8 + T::BYTE_LEN;
        params[include_offset..include_offset + 4]
            .copy_from_slice(&(u32::from(include_identity)).to_le_bytes());
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.reduce.params"),
            contents: params.as_slice(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn program_params_buffer(
        &self,
        slices: &[Vec<f64>],
        matrices: &[GpuMatrixInput],
        work_items: usize,
    ) -> wgpu::Buffer {
        if slices.len() > 4 || matrices.len() > 4 {
            fail("FlowArrow GPU generated program exceeded runtime descriptor capacity");
        }
        let mut words = [0u32; 14];
        words[0] = checked_usize_u32(work_items, "GPU program work item count");
        for (index, slice) in slices.iter().enumerate() {
            words[2 + index] = checked_usize_u32(slice.len(), "GPU slice length");
        }
        for (index, matrix) in matrices.iter().enumerate() {
            words[6 + index * 2] = checked_usize_u32(matrix.rows, "GPU matrix row count");
            words[7 + index * 2] = checked_usize_u32(matrix.cols, "GPU matrix column count");
        }
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flowarrow.gpu.program.params"),
            contents: u32_as_bytes(&words),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn storage_f64_buffer(&self, label: &'static str, values: &[f64]) -> wgpu::Buffer {
        if values.is_empty() {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &[0u8; 8],
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        } else {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: f64_as_bytes(values),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        }
    }

    fn require_f64_support(&self) -> Result<(), JsValue> {
        if !self.features.contains(wgpu::Features::SHADER_F64) {
            return Err(JsValue::from_str(
                "FlowArrow GPU f64 requires a device with wgpu SHADER_F64 support",
            ));
        }
        Ok(())
    }

    fn readback_buffer(&self, byte_len: wgpu::BufferAddress) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flowarrow.gpu.readback"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    async fn read_buffer(&mut self, readback: &wgpu::Buffer) -> Vec<u8> {
        let slice = readback.slice(..);
        map_buffer_async(slice).await.unwrap_or_else(|error| {
            fail(&format!("FlowArrow GPU target failed to map readback: {error}"));
        });
        let mapped = slice.get_mapped_range();
        let out = mapped.to_vec();
        drop(mapped);
        readback.unmap();
        out
    }
}

#[cfg(target_arch = "wasm32")]
struct MapReadFuture {
    state: Rc<RefCell<MapReadState>>,
}

#[cfg(target_arch = "wasm32")]
struct MapReadState {
    result: Option<Result<(), wgpu::BufferAsyncError>>,
    waker: Option<Waker>,
}

#[cfg(target_arch = "wasm32")]
fn map_buffer_async(slice: wgpu::BufferSlice<'_>) -> MapReadFuture {
    let state = Rc::new(RefCell::new(MapReadState {
        result: None,
        waker: None,
    }));
    let callback_state = Rc::clone(&state);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let mut state = callback_state.borrow_mut();
        state.result = Some(result);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    });
    MapReadFuture { state }
}

#[cfg(target_arch = "wasm32")]
impl Future for MapReadFuture {
    type Output = Result<(), wgpu::BufferAsyncError>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let mut state = self.state.borrow_mut();
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

trait GpuParam: Copy {
    const BYTE_LEN: usize;

    fn write_bytes(self, output: &mut [u8]);

    fn to_bytes(self) -> Vec<u8> {
        let mut output = vec![0u8; Self::BYTE_LEN];
        self.write_bytes(&mut output);
        output
    }
}

impl GpuParam for i32 {
    const BYTE_LEN: usize = 4;

    fn write_bytes(self, output: &mut [u8]) {
        output.copy_from_slice(&self.to_le_bytes());
    }
}

impl GpuParam for f32 {
    const BYTE_LEN: usize = 4;

    fn write_bytes(self, output: &mut [u8]) {
        output.copy_from_slice(&self.to_le_bytes());
    }
}

impl GpuParam for f64 {
    const BYTE_LEN: usize = 8;

    fn write_bytes(self, output: &mut [u8]) {
        output.copy_from_slice(&self.to_le_bytes());
    }
}

fn checked_usize_u32(value: usize, label: &str) -> u32 {
    u32::try_from(value).unwrap_or_else(|_| {
        fail(&format!("FlowArrow GPU {label} exceeds 32-bit runtime limits"));
    })
}

fn checked_workgroups(items: usize, label: &str) -> u32 {
    checked_usize_u32(items.div_ceil(64), label)
}

fn i32_as_bytes(values: &[i32]) -> &[u8] {
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<u8>(),
            std::mem::size_of_val(values),
        )
    }
}

fn f32_as_bytes(values: &[f32]) -> &[u8] {
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<u8>(),
            std::mem::size_of_val(values),
        )
    }
}

fn f64_as_bytes(values: &[f64]) -> &[u8] {
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<u8>(),
            std::mem::size_of_val(values),
        )
    }
}

fn u32_as_bytes(values: &[u32]) -> &[u8] {
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<u8>(),
            std::mem::size_of_val(values),
        )
    }
}

fn bytes_as_i32(values: &[u8]) -> &[i32] {
    if values.len() % std::mem::size_of::<i32>() != 0 {
        fail("FlowArrow GPU target read back misaligned i32 data");
    }
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<i32>(),
            values.len() / std::mem::size_of::<i32>(),
        )
    }
}

fn bytes_as_f32(values: &[u8]) -> &[f32] {
    if values.len() % std::mem::size_of::<f32>() != 0 {
        fail("FlowArrow GPU target read back misaligned f32 data");
    }
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<f32>(),
            values.len() / std::mem::size_of::<f32>(),
        )
    }
}

fn bytes_as_f64(values: &[u8]) -> &[f64] {
    if values.len() % std::mem::size_of::<f64>() != 0 {
        fail("FlowArrow GPU target read back misaligned f64 data");
    }
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<f64>(),
            values.len() / std::mem::size_of::<f64>(),
        )
    }
}

fn flatten_matrix(rows: &[FaGpuSliceF64], label: &str) -> (Vec<f64>, (usize, usize)) {
    let cols = rows.first().map(|row| row.count).unwrap_or(0);
    let mut values = Vec::with_capacity(rows.len().saturating_mul(cols));
    for row in rows {
        if row.count != cols {
            fail(&format!("FlowArrow GPU {label} matrix must be rectangular"));
        }
        if row.count > 0 && row.items.is_null() {
            fail(&format!("FlowArrow GPU {label} matrix contains null row storage"));
        }
        let row_values = if row.count == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(row.items, row.count) }
        };
        values.extend_from_slice(row_values);
    }
    (values, (rows.len(), cols))
}

fn reduce_wgsl(scalar: &str) -> String {
    REDUCE_WGSL.replace("__SCALAR__", scalar)
}

const REDUCE_WGSL: &str = r#"struct FaGpuReduceParams { len: u32, op: u32, identity: __SCALAR__, include_identity: u32 };
@group(0) @binding(0) var<storage, read> fa_input: array<__SCALAR__>;
@group(0) @binding(1) var<storage, read_write> fa_output: array<__SCALAR__>;
@group(0) @binding(2) var<uniform> fa_params: FaGpuReduceParams;

fn fa_apply(left: __SCALAR__, right: __SCALAR__) -> __SCALAR__ {
  if (fa_params.op == 0u) { return left + right; }
  if (fa_params.op == 1u) { return min(left, right); }
  return max(left, right);
}

fn fa_value_at(index: u32) -> __SCALAR__ {
  if (fa_params.include_identity != 0u && index == 0u) { return fa_params.identity; }
  let source_index = index - select(0u, 1u, fa_params.include_identity != 0u);
  return fa_input[source_index];
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) fa_gid: vec3<u32>) {
  let fa_i = fa_gid.x;
  let virtual_len = fa_params.len + select(0u, 1u, fa_params.include_identity != 0u);
  let out_len = (virtual_len + 1u) / 2u;
  if (fa_i >= out_len) { return; }
  let left_index = fa_i * 2u;
  let right_index = left_index + 1u;
  let left = fa_value_at(left_index);
  let right = select(fa_params.identity, fa_value_at(right_index), right_index < virtual_len);
  fa_output[fa_i] = fa_apply(left, right);
}
"#;

#[cfg(not(target_arch = "wasm32"))]
fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWaker(thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::abort();
}
"##
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
        "{:?}:{:?}:{:?}:{:?}:{:?}",
        options.crate_type,
        options.optimization,
        options.compiler_flags,
        options.linker_flags,
        options.gpu
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
        .chain(if options.gpu {
            gpu_runtime_source().as_bytes()
        } else {
            b""
        })
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

                program main(args: Args) -> exit_code: i64 {
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
