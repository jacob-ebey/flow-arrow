use crate::ast::*;
use crate::module_resolver;
use crate::monomorphize;
use crate::stdlib::{self, Effect, RuntimeSupport};
use crate::typecheck::{
    self, CheckMode, TypedCallable, TypedChain, TypedEndpoint, TypedEndpointKind, TypedMatchArm,
    TypedMatchGuard, TypedMatchTarget, TypedModule, TypedStage, TypedStageKind,
};
use crate::types::{
    Signature, Type as Ty, contains_empty_seq, parse_type, sequence_item_type, stdlib_type_symbol,
    substitute_partial,
};
#[cfg(not(target_arch = "wasm32"))]
use inkwell::OptimizationLevel;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
mod direct_llvm;
mod gpu;
mod llvm_text;
mod oxc_postprocess;
mod runtime_c;
mod runtime_fusion;
mod typescript;

#[cfg(not(target_arch = "wasm32"))]
use direct_llvm::{DirectExportAbi, DirectLlvm, DirectLlvmOptions};

pub(crate) struct LoweredModule {
    typed: TypedModule,
}

impl LoweredModule {
    pub(crate) fn with_stdlib_sources(module: &Module) -> Result<Self, String> {
        let resolved = module_resolver::resolve_stdlib_sources(module)?;
        Self::from_resolved(resolved)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn with_base(module: &Module, base_dir: &Path) -> Result<Self, String> {
        let resolved = module_resolver::resolve_sources(module, Some(base_dir))?;
        Self::from_resolved(resolved)
    }

    fn from_resolved(resolved: module_resolver::ResolvedModule) -> Result<Self, String> {
        let module = monomorphize::expand_module(resolved.module())?;
        let resolved = module_resolver::ResolvedModule::synthetic(module);
        let typed = typecheck::typed_resolved_module(resolved, CheckMode::Library)?;
        Ok(Self { typed })
    }

    fn typed(&self) -> Result<TypedCodegen<'_>, String> {
        TypedCodegen::from_typed(&self.typed)
    }

    fn has_foreign_js(&self) -> bool {
        self.typed.module().declarations.iter().any(|decl| {
            matches!(
                decl,
                Decl::Foreign(ForeignBlock {
                    target: ForeignTarget::Js,
                    ..
                })
            )
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn has_foreign(&self) -> bool {
        self.typed
            .module()
            .declarations
            .iter()
            .any(|decl| matches!(decl, Decl::Foreign(_)))
    }

    fn reject_foreign_js(&self) -> Result<(), String> {
        if self.has_foreign_js() {
            return Err(
                "foreign js declarations are supported only by the TypeScript and JavaScript backends"
                    .to_string(),
            );
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn emit_direct_llvm_with_gpu(&self, gpu: bool) -> Result<String, String> {
        self.reject_foreign_js()?;
        Ok(DirectLlvm::emit_with_options(
            TypedCodegen::from_typed_with_gpu(&self.typed, gpu)?,
            DirectLlvmOptions {
                gpu,
                ..DirectLlvmOptions::default()
            },
        )?
        .llvm)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn emit_runtime_c(&self) -> Result<String, String> {
        self.reject_foreign_js()?;
        self.typed()?.emit()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn emit_runtime_support_c(&self) -> Result<String, String> {
        self.reject_foreign_js()?;
        self.typed()?.emit_runtime_support_c()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn emit_native_cdylib_c_with_gpu(
        &self,
        gpu: bool,
    ) -> Result<NativeCdylibOutput, String> {
        self.reject_foreign_js()?;
        TypedCodegen::from_typed_with_gpu(&self.typed, gpu)?.emit_native_cdylib_c()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn emit_wasm_cdylib_llvm(
        &self,
        target_triple: &str,
        optimization: OptimizationLevel,
    ) -> Result<WasmCdylibOutput, String> {
        if self.has_foreign() {
            return Err("foreign declarations are not supported by WASM builds yet".to_string());
        }
        let emitted = DirectLlvm::emit_with_options(
            self.typed()?,
            DirectLlvmOptions {
                target_triple: Some(target_triple.to_string()),
                emit_entrypoint: false,
                export_abi: Some(DirectExportAbi::Wasm),
                emit_object: true,
                optimization,
                gpu: false,
            },
        )?;
        Ok(WasmCdylibOutput {
            llvm: emitted.llvm,
            object: emitted
                .object
                .ok_or_else(|| "WASM object emission did not produce an object file".to_string())?,
            exports: emitted.symbol_exports,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn foreign_c_source_paths(&self, base_dir: &Path) -> Result<Vec<PathBuf>, String> {
        let codegen = self.typed()?;
        let mut paths = codegen
            .foreign_c
            .values()
            .filter_map(|binding| binding.source.as_deref())
            .map(|path| absolutize_codegen_path(base_dir, path))
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn foreign_c_dependency_paths(
        &self,
        base_dir: &Path,
    ) -> Result<Vec<PathBuf>, String> {
        let codegen = self.typed()?;
        let mut paths = Vec::new();
        for binding in codegen.foreign_c.values() {
            paths.push(absolutize_codegen_path(base_dir, &binding.header));
            if let Some(source) = &binding.source {
                paths.push(absolutize_codegen_path(base_dir, source));
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn emit_typescript_source(&self, options: TypeScriptBackendOptions) -> Result<String, String> {
        let source = if options.worker_concurrency || options.gpu {
            typescript::emit_module_with_options(
                self.typed()?,
                typescript::TypeScriptEmitOptions {
                    worker_concurrency: options.worker_concurrency,
                    worker_module_specifier: if options.worker_concurrency {
                        Some(
                            options
                                .worker_module_specifier
                                .unwrap_or_else(|| "./flowarrow.worker.mjs".to_string()),
                        )
                    } else {
                        None
                    },
                    gpu: options.gpu,
                },
            )?
        } else {
            typescript::emit_module(self.typed()?)?
        };
        oxc_postprocess::emit_typescript(&source)
    }

    fn emit_typescript_artifacts(
        &self,
        options: TypeScriptBackendOptions,
    ) -> Result<TypeScriptArtifacts, String> {
        if !options.worker_concurrency {
            return Ok(TypeScriptArtifacts {
                source: self.emit_typescript_source(options)?,
                files: Vec::new(),
            });
        }
        let worker_path =
            worker_module_path_from_specifier(options.worker_module_specifier.as_deref());
        let emitted = typescript::emit_module_artifacts_with_options(
            self.typed()?,
            typescript::TypeScriptEmitOptions {
                worker_concurrency: true,
                worker_module_specifier: Some(
                    options
                        .worker_module_specifier
                        .unwrap_or_else(|| "./flowarrow.worker.mjs".to_string()),
                ),
                gpu: options.gpu,
            },
        )?;
        Ok(TypeScriptArtifacts {
            source: oxc_postprocess::emit_typescript(&emitted.source)?,
            files: vec![GeneratedSourceFile {
                path: worker_path,
                source: typescript::scalar_worker_module_source(&emitted.worker_mappers),
            }],
        })
    }

    fn emit_javascript_artifacts(
        &self,
        options: TypeScriptBackendOptions,
    ) -> Result<JavaScriptArtifacts, String> {
        if !options.worker_concurrency {
            let source = if options.gpu {
                typescript::emit_module_with_options(
                    self.typed()?,
                    typescript::TypeScriptEmitOptions {
                        worker_concurrency: false,
                        worker_module_specifier: None,
                        gpu: true,
                    },
                )?
            } else {
                typescript::emit_module(self.typed()?)?
            };
            let artifacts = oxc_postprocess::emit_javascript_artifacts(&source)?;
            return Ok(JavaScriptArtifacts {
                declarations: artifacts.declarations,
                javascript: artifacts.javascript,
                files: Vec::new(),
            });
        }
        let worker_path =
            worker_module_path_from_specifier(options.worker_module_specifier.as_deref());
        let emitted = typescript::emit_module_artifacts_with_options(
            self.typed()?,
            typescript::TypeScriptEmitOptions {
                worker_concurrency: true,
                worker_module_specifier: Some(
                    options
                        .worker_module_specifier
                        .unwrap_or_else(|| "./flowarrow.worker.mjs".to_string()),
                ),
                gpu: options.gpu,
            },
        )?;
        let artifacts = oxc_postprocess::emit_javascript_artifacts(&emitted.source)?;
        Ok(JavaScriptArtifacts {
            declarations: artifacts.declarations,
            javascript: artifacts.javascript,
            files: vec![GeneratedSourceFile {
                path: worker_path,
                source: typescript::scalar_worker_module_source(&emitted.worker_mappers),
            }],
        })
    }

    fn emit_llvm_ir_preview(&self) -> Result<String, String> {
        self.reject_foreign_js()?;
        llvm_text::emit_module(self.typed()?)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn absolutize_codegen_path(base_dir: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn lower_module_with_base(
    module: &Module,
    base_dir: &Path,
) -> Result<LoweredModule, String> {
    LoweredModule::with_base(module, base_dir)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct WasmCdylibOutput {
    pub llvm: String,
    pub object: Vec<u8>,
    pub exports: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct NativeCdylibOutput {
    pub source: String,
    pub header: String,
    pub exports: Vec<String>,
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub fn emit_runtime_c(module: &Module) -> Result<String, String> {
    LoweredModule::with_stdlib_sources(module)?.emit_runtime_c()
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub fn emit_runtime_c_with_base(module: &Module, base_dir: &Path) -> Result<String, String> {
    LoweredModule::with_base(module, base_dir)?.emit_runtime_c()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeScriptBackendOptions {
    pub worker_concurrency: bool,
    pub worker_module_specifier: Option<String>,
    pub gpu: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSourceFile {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScriptArtifacts {
    pub source: String,
    pub files: Vec<GeneratedSourceFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaScriptArtifacts {
    pub declarations: String,
    pub javascript: String,
    pub files: Vec<GeneratedSourceFile>,
}

fn worker_module_path_from_specifier(specifier: Option<&str>) -> String {
    specifier
        .unwrap_or("./flowarrow.worker.mjs")
        .trim_start_matches("./")
        .to_string()
}

pub fn emit_typescript_with_options(
    module: &Module,
    options: TypeScriptBackendOptions,
) -> Result<String, String> {
    LoweredModule::with_stdlib_sources(module)?.emit_typescript_source(options)
}

#[allow(dead_code)]
pub fn emit_typescript_artifacts_with_options(
    module: &Module,
    options: TypeScriptBackendOptions,
) -> Result<TypeScriptArtifacts, String> {
    LoweredModule::with_stdlib_sources(module)?.emit_typescript_artifacts(options)
}

pub fn emit_javascript_artifacts_with_options(
    module: &Module,
    options: TypeScriptBackendOptions,
) -> Result<JavaScriptArtifacts, String> {
    LoweredModule::with_stdlib_sources(module)?.emit_javascript_artifacts(options)
}

pub fn emit_llvm_ir_preview(module: &Module) -> Result<String, String> {
    LoweredModule::with_stdlib_sources(module)?.emit_llvm_ir_preview()
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub fn emit_typescript_with_base_and_options(
    module: &Module,
    base_dir: &Path,
    options: TypeScriptBackendOptions,
) -> Result<String, String> {
    LoweredModule::with_base(module, base_dir)?.emit_typescript_source(options)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn emit_typescript_artifacts_with_base_and_options(
    module: &Module,
    base_dir: &Path,
    options: TypeScriptBackendOptions,
) -> Result<TypeScriptArtifacts, String> {
    LoweredModule::with_base(module, base_dir)?.emit_typescript_artifacts(options)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn emit_javascript_artifacts_with_base_and_options(
    module: &Module,
    base_dir: &Path,
    options: TypeScriptBackendOptions,
) -> Result<JavaScriptArtifacts, String> {
    LoweredModule::with_base(module, base_dir)?.emit_javascript_artifacts(options)
}

#[derive(Debug, Clone)]
struct Value {
    code: String,
    ty: Ty,
}

#[derive(Debug, Clone)]
struct ParallelChainInput {
    name: String,
    field: String,
    c_ty: String,
    value: Value,
}

#[derive(Debug, Clone)]
struct ParallelChainHelper {
    worker: String,
    ctx_ty: String,
    ctx: String,
    inputs: Vec<ParallelChainInput>,
    target: BindingTarget,
    output_ty: Ty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnaryOp {
    Neg,
    Abs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapOp {
    Square,
    Abs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fusion {
    Sum,
    NestedSum,
    Mean,
    MapUnary(UnaryOp),
    ZipMap(BinaryOp),
    ZipMapReduceAdd(BinaryOp),
    MapReduceAdd(MapOp),
    ZipAllEqual,
    ZipDifferenceSquareSum,
    Sqrt(Box<Fusion>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReductionTerm {
    PairMul,
    PairDiffSquare,
    LeftSquare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatrixReductionTerm {
    ProductSum,
    MatvecSum,
    RowSumTotal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GpuRepeatAccumulatorKind {
    VectorScore,
    MatrixScore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GpuRepeatAccumulator {
    kind: GpuRepeatAccumulatorKind,
    wgsl: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BroadcastSide {
    Left,
    Right,
}

struct TypedCodegen<'a> {
    module: &'a Module,
    typed: &'a TypedModule,
    temp: usize,
    parallel_helper: usize,
    stream_helper: usize,
    parallel_helpers: String,
    callables: HashMap<String, &'a TypedCallable>,
    foreign_js: HashSet<String>,
    foreign_c: HashMap<String, ForeignCBinding>,
    signatures: HashMap<String, Signature>,
    stdlib_names: HashMap<String, String>,
    aliases: HashMap<String, Ty>,
    types: TypeRegistry,
    gpu_enabled: bool,
    gpu_plan: gpu::GpuPlan,
}

#[derive(Debug, Clone)]
struct ForeignCBinding {
    symbol: String,
    header: String,
    source: Option<String>,
}

#[derive(Default)]
struct TypeRegistry {
    types: BTreeMap<String, Ty>,
    use_cv_header: bool,
}

impl TypeRegistry {
    fn c_type(&mut self, ty: &Ty) -> String {
        match ty {
            Ty::Unit => "FaUnit".to_string(),
            Ty::Int => "int64_t".to_string(),
            Ty::Real | Ty::OneOf(_) => "double".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Bytes => "FaBytes".to_string(),
            Ty::Args => "FaArgs".to_string(),
            Ty::HttpServerConfig => {
                self.types.insert(type_name(ty), ty.clone());
                "FaHttpServerConfig".to_string()
            }
            Ty::HttpListener => {
                self.types.insert(type_name(ty), ty.clone());
                "FaHttpListener".to_string()
            }
            Ty::HttpRequest => {
                self.types.insert(type_name(ty), ty.clone());
                "FaHttpRequest".to_string()
            }
            Ty::HttpResponse => {
                self.types.insert(type_name(ty), ty.clone());
                "FaHttpResponse".to_string()
            }
            Ty::SqliteConnection => {
                self.types.insert(type_name(ty), ty.clone());
                "FaSqliteConnection".to_string()
            }
            Ty::SqliteRow => {
                self.types.insert(type_name(ty), ty.clone());
                "FaSqliteRow".to_string()
            }
            Ty::SqliteValue => {
                self.types.insert(type_name(ty), ty.clone());
                "FaSqliteValue".to_string()
            }
            Ty::Stream(_) => "FaStream".to_string(),
            Ty::Fault => "FaFault".to_string(),
            Ty::Var(_) => "FaUnit".to_string(),
            Ty::Seq(item) => {
                self.c_type(item);
                let name = type_name(ty);
                if !is_predefined_type_name(&name) {
                    self.types.insert(name.clone(), ty.clone());
                }
                name
            }
            Ty::Tuple(items) => {
                for item in items {
                    self.c_type(item);
                }
                let name = type_name(ty);
                if !is_predefined_type_name(&name) {
                    self.types.insert(name.clone(), ty.clone());
                }
                name
            }
            Ty::Struct { fields, .. } => {
                for (_, item) in fields {
                    self.c_type(item);
                }
                let name = type_name(ty);
                if !is_predefined_type_name(&name) {
                    self.types.insert(name.clone(), ty.clone());
                }
                name
            }
            Ty::Faultable(inner) => {
                self.c_type(inner);
                let name = type_name(ty);
                if !is_predefined_type_name(&name) {
                    self.types.insert(name.clone(), ty.clone());
                }
                name
            }
            Ty::EmptySeq => "FaUnit".to_string(),
        }
    }

    fn seq_new_name(&mut self, ty: &Ty) -> Result<String, String> {
        let Ty::Seq(_) = ty else {
            return Err(format!("expected sequence type, found `{ty}`"));
        };
        Ok(format!("{}_new", self.c_type(ty)))
    }

    fn set_use_cv_header(&mut self, use_cv_header: bool) {
        self.use_cv_header = use_cv_header;
    }

    fn uses_cv_header(&self) -> bool {
        self.use_cv_header
    }

    fn emit_typedefs(&mut self) -> String {
        let mut out = String::new();
        let mut entries = self
            .types
            .iter()
            .map(|(name, ty)| (type_depth(ty), name.clone(), ty.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (_, name, ty) in entries {
            if self.use_cv_header && is_cv_type_name(&name) {
                continue;
            }
            if is_http_runtime_type_name(&name) || is_sqlite_runtime_type_name(&name) {
                continue;
            }
            match ty {
                Ty::Seq(item) => {
                    let item_ty = self.c_type(&item);
                    out.push_str(&format!(
                        "typedef struct {{ size_t count; {item_ty} *items; }} {name};\n"
                    ));
                }
                Ty::HttpServerConfig
                | Ty::HttpListener
                | Ty::HttpRequest
                | Ty::HttpResponse
                | Ty::SqliteConnection
                | Ty::SqliteRow
                | Ty::SqliteValue => {}
                Ty::Tuple(items) => {
                    out.push_str("typedef struct { ");
                    for (index, item) in items.iter().enumerate() {
                        let item_ty = self.c_type(item);
                        out.push_str(&format!("{item_ty} f{index}; "));
                    }
                    out.push_str(&format!("}} {name};\n"));
                }
                Ty::Struct { fields, .. } => {
                    out.push_str("typedef struct { ");
                    for (field, item) in fields {
                        let item_ty = self.c_type(&item);
                        out.push_str(&format!("{item_ty} {}; ", c_ident(&field)));
                    }
                    out.push_str(&format!("}} {name};\n"));
                }
                Ty::Faultable(inner) => {
                    let inner_ty = self.c_type(&inner);
                    out.push_str(&format!(
                        "typedef struct {{ bool is_fault; FaFault fault; {inner_ty} value; }} {name};\n"
                    ));
                }
                _ => {}
            }
        }
        out.push('\n');
        out
    }

    fn emit_abi_typedefs(&mut self) -> String {
        let mut out = String::new();
        let mut entries = self
            .types
            .iter()
            .map(|(name, ty)| (type_depth(ty), name.clone(), ty.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (_, name, ty) in entries {
            match ty {
                Ty::Seq(item) => {
                    let item_ty = self.c_type(&item);
                    out.push_str(&format!(
                        "typedef struct {{ size_t count; {item_ty} *items; }} {name};\n"
                    ));
                }
                Ty::Tuple(items) => {
                    out.push_str("typedef struct { ");
                    for (index, item) in items.iter().enumerate() {
                        let item_ty = self.c_type(item);
                        out.push_str(&format!("{item_ty} f{index}; "));
                    }
                    out.push_str(&format!("}} {name};\n"));
                }
                Ty::Struct { fields, .. } => {
                    out.push_str("typedef struct { ");
                    for (field, item) in fields {
                        let item_ty = self.c_type(&item);
                        out.push_str(&format!("{item_ty} {}; ", c_ident(&field)));
                    }
                    out.push_str(&format!("}} {name};\n"));
                }
                Ty::Faultable(inner) => {
                    let inner_ty = self.c_type(&inner);
                    out.push_str(&format!(
                        "typedef struct {{ bool is_fault; FaFault fault; {inner_ty} value; }} {name};\n"
                    ));
                }
                Ty::HttpServerConfig => {
                    out.push_str("typedef struct { FaBytes host; int64_t port; bool tls; FaBytes cert_path; FaBytes key_path; bool http2; bool http3; } FaHttpServerConfig;\n");
                }
                Ty::HttpListener => {
                    out.push_str("typedef struct { FaHttpServerConfig config; void *state; } FaHttpListener;\n");
                }
                Ty::HttpRequest => {
                    out.push_str("typedef struct { FaBytes method; FaBytes path; FaBytes body; void *h2o_req; } FaHttpRequest;\n");
                }
                Ty::HttpResponse => {
                    self.c_type(&Ty::HttpRequest);
                    self.c_type(&Ty::Seq(Box::new(Ty::Bytes)));
                    out.push_str("typedef struct { FaHttpRequest request; int64_t status; FaSeq_Bytes header_names; FaSeq_Bytes header_values; FaBytes body; FaBytes content_type; } FaHttpResponse;\n");
                }
                Ty::SqliteConnection => {
                    out.push_str(
                        "typedef struct FaSqliteConnectionState FaSqliteConnectionState;\n",
                    );
                    out.push_str(
                        "typedef struct { FaSqliteConnectionState *state; } FaSqliteConnection;\n",
                    );
                }
                Ty::SqliteValue => {
                    out.push_str("typedef struct { int kind; int64_t int_value; double real_value; FaBytes bytes_value; } FaSqliteValue;\n");
                }
                Ty::SqliteRow => {
                    self.c_type(&Ty::SqliteValue);
                    out.push_str("typedef struct { size_t count; FaBytes *names; FaSqliteValue *values; } FaSqliteRow;\n");
                }
                Ty::Unit
                | Ty::Int
                | Ty::Real
                | Ty::Bool
                | Ty::Bytes
                | Ty::Args
                | Ty::Stream(_)
                | Ty::Fault
                | Ty::OneOf(_)
                | Ty::Var(_)
                | Ty::EmptySeq => {}
            }
        }
        out.push('\n');
        out
    }

    fn emit_helpers(&mut self) -> String {
        let mut out = String::new();
        let mut entries = self
            .types
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, ty) in entries {
            if self.use_cv_header && is_cv_type_name(&name) {
                continue;
            }
            if is_http_runtime_type_name(&name) || is_sqlite_runtime_type_name(&name) {
                continue;
            }
            match ty {
                Ty::Seq(item) => {
                    let item_ty = self.c_type(&item);
                    out.push_str(&format!(
                        "static {name} {name}_new(size_t count) {{\n  {name} seq;\n  seq.count = count;\n  seq.items = ({item_ty} *)fa_calloc(count ? count : 1, sizeof({item_ty}));\n  return seq;\n}}\n\n"
                    ));
                }
                Ty::Faultable(inner) => {
                    let inner_ty = self.c_type(&inner);
                    out.push_str(&format!(
                        "static {name} {name}_ok({inner_ty} value) {{\n  {name} out;\n  out.is_fault = false;\n  out.value = value;\n  return out;\n}}\n\nstatic {name} {name}_fault(FaFault fault) {{\n  {name} out;\n  out.is_fault = true;\n  out.fault = fault;\n  return out;\n}}\n\n"
                    ));
                }
                _ => {}
            }
        }
        out
    }
}

fn emit_preamble(out: &mut String) {
    stdlib::emit_runtime_c(out);
}

fn builtin_output_type(name: &str, input: &Ty) -> Result<Ty, String> {
    if name == "expect" {
        return builtin_output_type_plain(name, input);
    }
    if let Ty::Faultable(inner) = input {
        let output = builtin_output_type_plain(name, inner)?;
        return Ok(match output {
            Ty::Faultable(_) => output,
            other => Ty::Faultable(Box::new(other)),
        });
    }
    if let Some(unwrapped) = unwrap_faultable_tuple(input) {
        let output = builtin_output_type_plain(name, &unwrapped)?;
        return Ok(match output {
            Ty::Faultable(_) => output,
            other => Ty::Faultable(Box::new(other)),
        });
    }
    builtin_output_type_plain(name, input)
}

fn builtin_output_type_plain(name: &str, input: &Ty) -> Result<Ty, String> {
    match name {
        "argv" => Ok(Ty::Seq(Box::new(Ty::Bytes))),
        "flag_present" => Ok(Ty::Bool),
        "flag_value" => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
        "read_stdin" => Ok(Ty::Bytes),
        "write_stdout" | "write_stderr" => Ok(Ty::Int),
        "read_file" => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
        "write_file" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "exists" | "is_file" | "is_dir" => Ok(Ty::Bool),
        "file_size" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "join_path" | "basename" | "dirname" => Ok(Ty::Bytes),
        "list_dir" | "walk_files" => Ok(Ty::Faultable(Box::new(Ty::Seq(Box::new(Ty::Bytes))))),
        "read_files" => Ok(Ty::Faultable(Box::new(Ty::Seq(Box::new(Ty::Tuple(vec![
            Ty::Bytes,
            Ty::Bytes,
        ])))))),
        "open_file" => Ok(Ty::Faultable(Box::new(Ty::Stream(Box::new(Ty::Bytes))))),
        "read_at" => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
        "size" | "copy_to_file" | "close" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "to_seq" => {
            let Ty::Stream(item) = input else {
                return Err("to_seq expected stream input".to_string());
            };
            Ok(Ty::Faultable(Box::new(Ty::Seq(item.clone()))))
        }
        "drain" => {
            let Ty::Stream(_) = input else {
                return Err("drain expected stream input".to_string());
            };
            Ok(Ty::Faultable(Box::new(Ty::Int)))
        }
        "default_config" => Ok(Ty::HttpServerConfig),
        "with_tcp_listener" | "with_tls" | "with_http2" | "with_http3" => Ok(Ty::HttpServerConfig),
        "listen" => Ok(Ty::Faultable(Box::new(Ty::HttpListener))),
        "requests" => Ok(Ty::Stream(Box::new(Ty::HttpRequest))),
        "serve" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "route" => Ok(Ty::Bool),
        "body" => Ok(Ty::Bytes),
        "response" | "with_status" | "with_header" | "text" | "json" | "not_found" => {
            Ok(Ty::HttpResponse)
        }
        "sqlite.open"
        | "sqlite.open_readonly"
        | "sqlite.open_memory"
        | "sqlite.busy_timeout"
        | "sqlite.foreign_keys"
        | "sqlite.begin"
        | "sqlite.begin_immediate"
        | "sqlite.commit"
        | "sqlite.rollback" => Ok(Ty::Faultable(Box::new(Ty::SqliteConnection))),
        "sqlite.close" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "sqlite.null" | "sqlite.int" | "sqlite.real" | "sqlite.text" | "sqlite.blob" => {
            Ok(Ty::SqliteValue)
        }
        "sqlite.exec" => Ok(Ty::Faultable(Box::new(Ty::Tuple(vec![
            Ty::SqliteConnection,
            Ty::Int,
        ])))),
        "sqlite.query" => Ok(Ty::Faultable(Box::new(Ty::Tuple(vec![
            Ty::SqliteConnection,
            Ty::Stream(Box::new(Ty::SqliteRow)),
        ])))),
        "sqlite.query_all" => Ok(Ty::Faultable(Box::new(Ty::Tuple(vec![
            Ty::SqliteConnection,
            Ty::Seq(Box::new(Ty::SqliteRow)),
        ])))),
        "sqlite.column_count" => Ok(Ty::Int),
        "sqlite.column_name" => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
        "sqlite.value_at" | "sqlite.value_named" => Ok(Ty::Faultable(Box::new(Ty::SqliteValue))),
        "sqlite.kind" => Ok(Ty::Bytes),
        "sqlite.is_null" => Ok(Ty::Bool),
        "sqlite.as_int" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "sqlite.as_real" => Ok(Ty::Faultable(Box::new(Ty::Real))),
        "sqlite.as_text" | "sqlite.as_blob" => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
        "split_lines" | "split_on" => Ok(Ty::Seq(Box::new(Ty::Bytes))),
        "trim" | "join_bytes" | "codes_to_bytes" | "format_faults" | "ascii_lower"
        | "ascii_upper" => Ok(Ty::Bytes),
        "contains" | "starts_with" | "ends_with" => Ok(Ty::Bool),
        "index_of" | "last_index_of" => Ok(Ty::Int),
        "concat_bytes" => match input {
            Ty::Seq(item) if matches!(item.as_ref(), Ty::Faultable(inner) if inner.as_ref() == &Ty::Bytes) => {
                Ok(Ty::Faultable(Box::new(Ty::Bytes)))
            }
            _ => Ok(Ty::Bytes),
        },
        "replace" => Ok(Ty::Bytes),
        "slice" if matches!(input, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int, Ty::Int])) => {
            Ok(Ty::Bytes)
        }
        "take" | "drop" if matches!(input, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) => {
            Ok(Ty::Bytes)
        }
        "repeat_bytes" => Ok(Ty::Bytes),
        "strip_prefix" | "strip_suffix" => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
        "decode" | "decode_bmp" | "decode_jpeg" | "decode_png" | "decode_pnm" => {
            Ok(Ty::Faultable(Box::new(cv_image_ty())))
        }
        "encode_bmp" | "encode_jpeg" | "encode_pgm" | "encode_png" | "encode_ppm" => {
            Ok(Ty::Faultable(Box::new(Ty::Bytes)))
        }
        "bytes_to_codes" | "range_step" => Ok(Ty::Seq(Box::new(Ty::Int))),
        "byte_length" | "length" | "inner_length" | "bit_and" | "bit_or" | "bit_xor"
        | "bit_shl" | "bit_shr" => Ok(Ty::Int),
        "parse_int" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "parse_real" => Ok(Ty::Faultable(Box::new(Ty::Real))),
        "from_int" => Ok(Ty::Real),
        "format_int" | "format_real" => match input {
            Ty::Faultable(_) => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
            _ => Ok(Ty::Bytes),
        },
        "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => numeric_binary_output(input),
        "neg" | "abs" => Ok(input.clone()),
        "sqrt" | "exp" | "sin" | "cos" => Ok(Ty::Real),
        "eq" | "lt" | "gt" | "le" | "ge" | "not_empty" | "is_empty" | "and" | "or" | "xor"
        | "not" | "all" | "any" | "has_faults" => Ok(Ty::Bool),
        "collect" => {
            let Ty::Seq(item) = input else {
                return Err("collect expected sequence input".to_string());
            };
            let Ty::Faultable(ok) = item.as_ref() else {
                return Err("collect expected Seq[Faultable[V]] input".to_string());
            };
            Ok(Ty::Faultable(Box::new(Ty::Seq(ok.clone()))))
        }
        "expect" => {
            if let Ty::Faultable(inner) = input {
                Ok(inner.as_ref().clone())
            } else {
                Ok(input.clone())
            }
        }
        "select" => {
            let Ty::Tuple(items) = input else {
                return Err("select expected tuple input".to_string());
            };
            items
                .get(1)
                .cloned()
                .ok_or_else(|| "select expected three inputs".to_string())
        }
        "zip" => {
            let Ty::Tuple(items) = input else {
                return Err("zip expected tuple input".to_string());
            };
            let [Ty::Seq(left), Ty::Seq(right)] = items.as_slice() else {
                return Err("zip expected two sequence inputs".to_string());
            };
            Ok(Ty::Seq(Box::new(Ty::Tuple(vec![
                left.as_ref().clone(),
                right.as_ref().clone(),
            ]))))
        }
        "broadcast_left" => {
            let Ty::Tuple(items) = input else {
                return Err("broadcast_left expected tuple input".to_string());
            };
            let [left, Ty::Seq(right)] = items.as_slice() else {
                return Err("broadcast_left expected (A,Seq[B]) input".to_string());
            };
            Ok(Ty::Seq(Box::new(Ty::Tuple(vec![
                left.clone(),
                right.as_ref().clone(),
            ]))))
        }
        "broadcast_right" => {
            let Ty::Tuple(items) = input else {
                return Err("broadcast_right expected tuple input".to_string());
            };
            let [Ty::Seq(left), right] = items.as_slice() else {
                return Err("broadcast_right expected (Seq[A],B) input".to_string());
            };
            Ok(Ty::Seq(Box::new(Ty::Tuple(vec![
                left.as_ref().clone(),
                right.clone(),
            ]))))
        }
        "transpose" => {
            let Ty::Seq(row) = input else {
                return Err("transpose expected sequence input".to_string());
            };
            if !matches!(row.as_ref(), Ty::Seq(_)) {
                return Err("transpose expected nested sequence input".to_string());
            }
            Ok(input.clone())
        }
        "flatten" => {
            let Ty::Seq(row) = input else {
                return Err("flatten expected sequence input".to_string());
            };
            let Ty::Seq(item) = row.as_ref() else {
                return Err("flatten expected nested sequence input".to_string());
            };
            Ok(Ty::Seq(item.clone()))
        }
        "first" => {
            let Ty::Tuple(items) = input else {
                return Err("first expected tuple input".to_string());
            };
            items
                .first()
                .cloned()
                .ok_or_else(|| "first expected non-empty tuple input".to_string())
        }
        "second" => {
            let Ty::Tuple(items) = input else {
                return Err("second expected tuple input".to_string());
            };
            items
                .get(1)
                .cloned()
                .ok_or_else(|| "second expected two inputs".to_string())
        }
        "swap" => {
            let Ty::Tuple(items) = input else {
                return Err("swap expected tuple input".to_string());
            };
            let [left, right] = items.as_slice() else {
                return Err("swap expected two inputs".to_string());
            };
            Ok(Ty::Tuple(vec![right.clone(), left.clone()]))
        }
        "group_by_id" => {
            let Ty::Tuple(items) = input else {
                return Err("group_by_id expected tuple input".to_string());
            };
            let [Ty::Seq(value), Ty::Seq(_)] = items.as_slice() else {
                return Err("group_by_id expected two sequence inputs".to_string());
            };
            Ok(Ty::Seq(Box::new(Ty::Seq(value.clone()))))
        }
        "shift_right" | "shift_left" | "append" | "set" | "concat" => {
            let Ty::Tuple(items) = input else {
                return Err(format!("{name} expected tuple input"));
            };
            items
                .first()
                .cloned()
                .ok_or_else(|| format!("{name} expected sequence input"))
        }
        "tail" | "reverse" => Ok(input.clone()),
        "take" | "drop" => {
            let Ty::Tuple(items) = input else {
                return Err(format!("{name} expected tuple input"));
            };
            let [seq @ Ty::Seq(_), Ty::Int] = items.as_slice() else {
                return Err(format!("{name} expected (Seq[V],Int) input"));
            };
            Ok(seq.clone())
        }
        "fill" => {
            let Ty::Tuple(items) = input else {
                return Err("fill expected tuple input".to_string());
            };
            let [item, Ty::Int] = items.as_slice() else {
                return Err("fill expected (V,Int) input".to_string());
            };
            Ok(Ty::Seq(Box::new(item.clone())))
        }
        "slice" => {
            let Ty::Tuple(items) = input else {
                return Err("slice expected tuple input".to_string());
            };
            let [seq @ Ty::Seq(_), Ty::Int, Ty::Int] = items.as_slice() else {
                return Err("slice expected (Seq[V],Int,Int) input".to_string());
            };
            Ok(seq.clone())
        }
        "head" | "last" => {
            let Ty::Seq(item) = input else {
                return Err(format!("{name} expected sequence input"));
            };
            Ok(Ty::Faultable(item.clone()))
        }
        "get" => {
            let Ty::Tuple(items) = input else {
                return Err("get expected tuple input".to_string());
            };
            let [Ty::Seq(item), Ty::Int] = items.as_slice() else {
                return Err("get expected (Seq[V],Int) input".to_string());
            };
            Ok(item.as_ref().clone())
        }
        "get_or" => {
            let Ty::Tuple(items) = input else {
                return Err("get_or expected tuple input".to_string());
            };
            let [Ty::Seq(item), Ty::Int, _] = items.as_slice() else {
                return Err("get_or expected (Seq[V],Int,V) input".to_string());
            };
            Ok(item.as_ref().clone())
        }
        "at" => {
            let Ty::Tuple(items) = input else {
                return Err("at expected tuple input".to_string());
            };
            let [Ty::Seq(item), Ty::Int] = items.as_slice() else {
                return Err("at expected (Seq[V],Int) input".to_string());
            };
            Ok(Ty::Faultable(item.clone()))
        }
        other => Err(format!("unsupported builtin `{other}`")),
    }
}

fn cv_image_ty() -> Ty {
    Ty::Tuple(vec![
        Ty::Tuple(vec![Ty::Int, Ty::Int]),
        Ty::Seq(Box::new(cv_pixel_seq_ty())),
    ])
}

fn cv_pixel_seq_ty() -> Ty {
    Ty::Seq(Box::new(Ty::Tuple(vec![
        Ty::Real,
        Ty::Tuple(vec![Ty::Real, Ty::Real]),
    ])))
}

fn match_input_types(
    expected: &Ty,
    actual: &Ty,
    vars: &mut HashMap<String, Ty>,
) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    if let Ty::Faultable(actual) = actual {
        return match_input_types(expected, actual, vars);
    }
    if let Some(actual) = unwrap_faultable_tuple(actual) {
        return match_input_types(expected, &actual, vars);
    }
    match (expected, actual) {
        (Ty::Seq(_), Ty::EmptySeq) => Ok(()),
        (Ty::Var(name), actual) => {
            if matches!(actual, Ty::EmptySeq) {
                return Ok(());
            }
            if let Some(bound) = vars.get(name) {
                if bound == actual {
                    Ok(())
                } else {
                    Err(format!(
                        "type variable `{name}` was `{bound}` then `{actual}`"
                    ))
                }
            } else {
                vars.insert(name.clone(), actual.clone());
                Ok(())
            }
        }
        (Ty::Faultable(expected), Ty::Faultable(actual)) => {
            match_input_types(expected, actual, vars)
        }
        (Ty::Seq(expected), Ty::Seq(actual)) => match_input_types(expected, actual, vars),
        (Ty::Stream(expected), Ty::Stream(actual)) => match_input_types(expected, actual, vars),
        (Ty::OneOf(expected), actual) => {
            for expected in expected {
                let mut candidate_vars = vars.clone();
                if match_input_types(expected, actual, &mut candidate_vars).is_ok() {
                    *vars = candidate_vars;
                    return Ok(());
                }
            }
            Err(format!(
                "expected one of `{}`, found `{actual}`",
                Ty::OneOf(expected.clone())
            ))
        }
        (Ty::Tuple(expected), Ty::Tuple(actual)) if expected.len() == actual.len() => {
            for (expected, actual) in expected.iter().zip(actual) {
                match_input_types(expected, actual, vars)?;
            }
            Ok(())
        }
        (
            Ty::Struct {
                name: expected_name,
                fields: expected,
            },
            Ty::Struct {
                name: actual_name,
                fields: actual,
            },
        ) if expected_name == actual_name && expected.len() == actual.len() => {
            for ((expected_field, expected_ty), (actual_field, actual_ty)) in
                expected.iter().zip(actual)
            {
                if expected_field != actual_field {
                    return Err(format!(
                        "expected struct field `{expected_field}`, found `{actual_field}`"
                    ));
                }
                match_input_types(expected_ty, actual_ty, vars)?;
            }
            Ok(())
        }
        _ => Err(format!("expected `{expected}`, found `{actual}`")),
    }
}

fn assignable_output_ty(expected: &Ty, actual: &Ty) -> bool {
    if expected == actual {
        return true;
    }
    match (expected, actual) {
        (Ty::Faultable(expected), actual) => {
            expected.as_ref() == actual
                || unwrap_faultable_tuple(actual)
                    .as_ref()
                    .is_some_and(|actual| expected.as_ref() == actual)
        }
        (Ty::Seq(_), Ty::EmptySeq) => true,
        (Ty::Seq(expected), Ty::Seq(actual)) if matches!(actual.as_ref(), Ty::EmptySeq) => {
            assignable_output_ty(expected, actual)
        }
        (Ty::Seq(expected), Ty::Seq(actual)) => assignable_output_ty(expected, actual),
        (Ty::Stream(expected), Ty::Stream(actual)) => assignable_output_ty(expected, actual),
        (Ty::Tuple(expected), Ty::Tuple(actual)) if expected.len() == actual.len() => expected
            .iter()
            .zip(actual.iter())
            .all(|(expected, actual)| assignable_output_ty(expected, actual)),
        (
            Ty::Struct {
                name: expected_name,
                fields: expected,
            },
            Ty::Struct {
                name: actual_name,
                fields: actual,
            },
        ) if expected_name == actual_name && expected.len() == actual.len() => expected
            .iter()
            .zip(actual.iter())
            .all(|((expected_field, expected), (actual_field, actual))| {
                expected_field == actual_field && assignable_output_ty(expected, actual)
            }),
        _ => false,
    }
}

fn contains_type_var(input: &Ty) -> bool {
    match input {
        Ty::Var(_) => true,
        Ty::Faultable(item) | Ty::Seq(item) | Ty::Stream(item) => contains_type_var(item),
        Ty::Tuple(items) | Ty::OneOf(items) => items.iter().any(contains_type_var),
        Ty::Struct { fields, .. } => fields.iter().any(|(_, ty)| contains_type_var(ty)),
        _ => false,
    }
}

fn input_context_ty(expected: &Ty, actual: &Ty) -> Ty {
    match (expected, actual) {
        (_, Ty::EmptySeq) => expected.clone(),
        (expected, Ty::Faultable(actual)) => {
            Ty::Faultable(Box::new(input_context_ty(expected, actual)))
        }
        (Ty::Seq(expected), Ty::Seq(actual)) => {
            Ty::Seq(Box::new(input_context_ty(expected, actual)))
        }
        (Ty::Tuple(expected), Ty::Tuple(actual)) if expected.len() == actual.len() => Ty::Tuple(
            expected
                .iter()
                .zip(actual.iter())
                .map(|(expected, actual)| input_context_ty(expected, actual))
                .collect(),
        ),
        (
            Ty::Struct {
                name,
                fields: expected,
            },
            Ty::Struct { fields: actual, .. },
        ) if expected.len() == actual.len() => Ty::Struct {
            name: name.clone(),
            fields: expected
                .iter()
                .zip(actual.iter())
                .map(|((field, expected), (_, actual))| {
                    (field.clone(), input_context_ty(expected, actual))
                })
                .collect(),
        },
        _ => actual.clone(),
    }
}

fn unwrap_faultable_tuple(input: &Ty) -> Option<Ty> {
    let Ty::Tuple(items) = input else {
        return None;
    };
    let mut saw_faultable = false;
    let unwrapped = items
        .iter()
        .map(|item| match item {
            Ty::Faultable(inner) => {
                saw_faultable = true;
                inner.as_ref().clone()
            }
            Ty::Tuple(_) => {
                if let Some(unwrapped) = unwrap_faultable_tuple(item) {
                    saw_faultable = true;
                    unwrapped
                } else {
                    item.clone()
                }
            }
            other => other.clone(),
        })
        .collect::<Vec<_>>();
    saw_faultable.then_some(Ty::Tuple(unwrapped))
}

fn faultable_projection_ty(ty: &Ty) -> Ty {
    match ty {
        Ty::Faultable(_) => ty.clone(),
        other => Ty::Faultable(Box::new(other.clone())),
    }
}

fn contains_faultable_ty(input: &Ty) -> bool {
    match input {
        Ty::Faultable(_) => true,
        Ty::Seq(item) | Ty::Stream(item) => contains_faultable_ty(item),
        Ty::Tuple(items) => items.iter().any(contains_faultable_ty),
        Ty::Struct { fields, .. } => fields.iter().any(|(_, ty)| contains_faultable_ty(ty)),
        Ty::OneOf(items) => items.iter().any(contains_faultable_ty),
        _ => false,
    }
}

fn contains_tuple_faultable_ty(input: &Ty) -> bool {
    match input {
        Ty::Faultable(_) => true,
        Ty::Tuple(items) => items.iter().any(contains_tuple_faultable_ty),
        _ => false,
    }
}

fn emit_fault_checks_for_value(out: &mut String, target: &str, input: &str, input_ty: &Ty) {
    match input_ty {
        Ty::Faultable(_) => {
            out.push_str(&format!("  if (!{target}.is_fault && {input}.is_fault) {{ {target}.is_fault = true; {target}.fault = {input}.fault; }}\n"));
        }
        Ty::Tuple(items) => {
            for (index, item) in items.iter().enumerate() {
                emit_fault_checks_for_value(out, target, &format!("{input}.f{index}"), item);
            }
        }
        _ => {}
    }
}

fn emit_unwrap_faultable_value(
    out: &mut String,
    target: &str,
    input: &str,
    input_ty: &Ty,
    indent: &str,
) {
    match input_ty {
        Ty::Faultable(_) => {
            out.push_str(&format!("{indent}{target} = {input}.value;\n"));
        }
        Ty::Tuple(items) => {
            for (index, item) in items.iter().enumerate() {
                emit_unwrap_faultable_value(
                    out,
                    &format!("{target}.f{index}"),
                    &format!("{input}.f{index}"),
                    item,
                    indent,
                );
            }
        }
        _ => {
            out.push_str(&format!("{indent}{target} = {input};\n"));
        }
    }
}

fn is_predefined_type_name(name: &str) -> bool {
    if stdlib::is_runtime_header_type_name(name) {
        return true;
    }
    matches!(
        name,
        "FaSeq_Bytes"
            | "FaTuple_Bytes_Bytes"
            | "FaSeq_Tuple_Bytes_Bytes"
            | "FaSeq_Int"
            | "FaSeq_Fault"
            | "FaFaultable_Int"
            | "FaFaultable_Real"
            | "FaFaultable_Bytes"
            | "FaFaultable_Seq_Bytes"
            | "FaFaultable_Seq_Tuple_Bytes_Bytes"
            | "FaFaultable_Stream_Bytes"
            | "FaSeq_Real"
            | "FaFaultable_Seq_Real"
    )
}

fn is_cv_type_name(name: &str) -> bool {
    matches!(
        name,
        "FaTuple_Real_Real"
            | "FaTuple_Real_Tuple_Real_Real"
            | "FaSeq_Tuple_Real_Tuple_Real_Real"
            | "FaSeq_Seq_Tuple_Real_Tuple_Real_Real"
            | "FaTuple_Int_Int"
            | "FaTuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real"
            | "FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real"
    )
}

fn is_http_runtime_type_name(name: &str) -> bool {
    stdlib::is_runtime_header_type_name(name)
}

fn is_sqlite_runtime_type_name(name: &str) -> bool {
    stdlib::is_runtime_header_type_name(name)
}

fn numeric_binary_output(input: &Ty) -> Result<Ty, String> {
    let Ty::Tuple(items) = input else {
        return Err("numeric binary op expected tuple input".to_string());
    };
    let [left, right] = items.as_slice() else {
        return Err("numeric binary op expected two inputs".to_string());
    };
    if left == &Ty::Int && right == &Ty::Int {
        Ok(Ty::Int)
    } else {
        Ok(Ty::Real)
    }
}

fn add_expr(left: &str, right: &str, ty: &Ty) -> String {
    if ty == &Ty::Int {
        format!("({left} + {right})")
    } else {
        format!("((double){left} + (double){right})")
    }
}

fn numeric_binary_expr(name: &str, input: &str, output_ty: &Ty) -> String {
    let left = format!("{input}.f0");
    let right = format!("{input}.f1");
    let cast_left = if output_ty == &Ty::Int {
        left.clone()
    } else {
        format!("(double){left}")
    };
    let cast_right = if output_ty == &Ty::Int {
        right.clone()
    } else {
        format!("(double){right}")
    };
    match name {
        "add" => format!("({cast_left} + {cast_right})"),
        "sub" => format!("({cast_left} - {cast_right})"),
        "mul" => format!("({cast_left} * {cast_right})"),
        "div" => format!("({cast_left} / {cast_right})"),
        "rem" => {
            if output_ty == &Ty::Int {
                format!("({left} % {right})")
            } else {
                format!("fmod({cast_left}, {cast_right})")
            }
        }
        "min" => format!("({cast_left} < {cast_right} ? {cast_left} : {cast_right})"),
        "max" => format!("({cast_left} > {cast_right} ? {cast_left} : {cast_right})"),
        _ => unreachable!(),
    }
}

fn numeric_unary_expr(name: &str, input: &str, output_ty: &Ty) -> String {
    match name {
        "neg" => format!("(-({input}))"),
        "abs" if output_ty == &Ty::Int => format!("(({input}) < 0 ? -({input}) : ({input}))"),
        "abs" => format!("fabs({input})"),
        "sqrt" => format!("sqrt((double){input})"),
        "exp" => format!("exp((double){input})"),
        "sin" => format!("sin((double){input})"),
        "cos" => format!("cos((double){input})"),
        _ => unreachable!(),
    }
}

fn min_max_expr(op: &str, left: &str, right: &str, ty: &Ty) -> String {
    let cast_left = if ty == &Ty::Int {
        left.to_string()
    } else {
        format!("(double){left}")
    };
    let cast_right = if ty == &Ty::Int {
        right.to_string()
    } else {
        format!("(double){right}")
    };
    match op {
        "min" => format!("({cast_left} < {cast_right} ? {cast_left} : {cast_right})"),
        "max" => format!("({cast_left} > {cast_right} ? {cast_left} : {cast_right})"),
        _ => unreachable!(),
    }
}

fn binary_op_expr(op: BinaryOp, left: &str, right: &str) -> String {
    match op {
        BinaryOp::Add => format!("((double){left} + (double){right})"),
        BinaryOp::Sub => format!("((double){left} - (double){right})"),
        BinaryOp::Mul => format!("((double){left} * (double){right})"),
        BinaryOp::Div => format!("((double){left} / (double){right})"),
    }
}

fn compare_expr(name: &str, input: &str) -> String {
    let op = match name {
        "eq" => "==",
        "lt" => "<",
        "gt" => ">",
        "le" => "<=",
        "ge" => ">=",
        _ => unreachable!(),
    };
    format!("((double){input}.f0 {op} (double){input}.f1)")
}

fn stages_binding_output<'a>(chain: &'a TypedChain, output: &str) -> Option<&'a [TypedStage]> {
    let (last, stages) = chain.stages.split_last()?;
    match &last.kind {
        TypedStageKind::Bind {
            target: BindingTarget::Discard,
        } => None,
        TypedStageKind::Bind {
            target: BindingTarget::Variable(name),
        } if name == output => Some(stages),
        _ => None,
    }
}

fn final_variable(chain: &TypedChain) -> Option<&str> {
    match &chain.stages.last()?.kind {
        TypedStageKind::Bind {
            target: BindingTarget::Discard,
        } => None,
        TypedStageKind::Bind {
            target: BindingTarget::Variable(name),
        } => Some(name),
        _ => None,
    }
}

fn fuse_single_use_chains(callable: &TypedCallable) -> Vec<TypedChain> {
    let mut chains = callable.chains.clone();
    loop {
        let mut uses = HashMap::new();
        for chain in &chains {
            count_endpoint_vars(&chain.source, &mut uses);
            for stage in &chain.stages {
                match &stage.kind {
                    TypedStageKind::Reduce { identity, .. }
                    | TypedStageKind::Scan { identity, .. } => {
                        count_endpoint_vars(identity, &mut uses);
                    }
                    TypedStageKind::Repeat { count, .. } => count_endpoint_vars(count, &mut uses),
                    TypedStageKind::Match { arms } => {
                        for arm in arms {
                            if let TypedMatchGuard::Call { args, .. } = &arm.guard {
                                for arg in args {
                                    count_endpoint_vars(arg, &mut uses);
                                }
                            }
                            if let TypedMatchTarget::Value(endpoint) = &arm.target {
                                count_endpoint_vars(endpoint, &mut uses);
                            }
                        }
                    }
                    TypedStageKind::Call { .. }
                    | TypedStageKind::Bind { .. }
                    | TypedStageKind::Field { .. }
                    | TypedStageKind::Map { .. }
                    | TypedStageKind::Filter { .. }
                    | TypedStageKind::FaultMap { .. } => {}
                }
            }
        }

        let mut changed = false;
        for producer_index in 0..chains.len() {
            let Some(binding) = final_variable(&chains[producer_index]).map(ToString::to_string)
            else {
                continue;
            };
            if callable.outputs.iter().any(|output| output.name == binding) {
                continue;
            }
            if uses.get(&binding).copied().unwrap_or(0) != 1 {
                continue;
            }
            let Some(consumer_index) = chains.iter().position(
                |chain| matches!(&chain.source.kind, TypedEndpointKind::Variable(name) if name == &binding),
            ) else {
                continue;
            };
            if producer_index == consumer_index {
                continue;
            }

            let mut stages = chains[producer_index].stages.clone();
            stages.pop();
            stages.extend(chains[consumer_index].stages.clone());
            chains[consumer_index] = TypedChain {
                source: chains[producer_index].source.clone(),
                stages,
            };
            chains.remove(producer_index);
            changed = true;
            break;
        }
        if !changed {
            break;
        }
    }
    chains
}

fn count_endpoint_vars(endpoint: &TypedEndpoint, uses: &mut HashMap<String, usize>) {
    match &endpoint.kind {
        TypedEndpointKind::Variable(name) => {
            *uses.entry(name.clone()).or_insert(0) += 1;
        }
        TypedEndpointKind::Tuple(items) | TypedEndpointKind::Seq(items) => {
            for item in items {
                count_endpoint_vars(item, uses);
            }
        }
        TypedEndpointKind::Struct { fields, .. } => {
            for (_, item) in fields {
                count_endpoint_vars(item, uses);
            }
        }
        TypedEndpointKind::Eval { source, stages } => {
            count_endpoint_vars(source, uses);
            for stage in stages {
                match &stage.kind {
                    TypedStageKind::Repeat { count, .. }
                    | TypedStageKind::Reduce {
                        identity: count, ..
                    }
                    | TypedStageKind::Scan {
                        identity: count, ..
                    } => count_endpoint_vars(count, uses),
                    TypedStageKind::Match { arms } => {
                        for arm in arms {
                            if let TypedMatchGuard::Call { args, .. } = &arm.guard {
                                for arg in args {
                                    count_endpoint_vars(arg, uses);
                                }
                            }
                            if let TypedMatchTarget::Value(endpoint) = &arm.target {
                                count_endpoint_vars(endpoint, uses);
                            }
                        }
                    }
                    TypedStageKind::Call { .. }
                    | TypedStageKind::Bind { .. }
                    | TypedStageKind::Field { .. }
                    | TypedStageKind::Map { .. }
                    | TypedStageKind::Filter { .. }
                    | TypedStageKind::FaultMap { .. } => {}
                }
            }
        }
        TypedEndpointKind::NodeRef { .. }
        | TypedEndpointKind::Int(_)
        | TypedEndpointKind::Real(_)
        | TypedEndpointKind::Bool(_)
        | TypedEndpointKind::String(_)
        | TypedEndpointKind::Unit => {}
    }
}

fn count_stage_endpoint_vars(stage: &TypedStage, uses: &mut HashMap<String, usize>) {
    match &stage.kind {
        TypedStageKind::Repeat { count, .. }
        | TypedStageKind::Reduce {
            identity: count, ..
        }
        | TypedStageKind::Scan {
            identity: count, ..
        } => count_endpoint_vars(count, uses),
        TypedStageKind::Match { arms } => {
            for arm in arms {
                if let TypedMatchGuard::Call { args, .. } = &arm.guard {
                    for arg in args {
                        count_endpoint_vars(arg, uses);
                    }
                }
                if let TypedMatchTarget::Value(endpoint) = &arm.target {
                    count_endpoint_vars(endpoint, uses);
                }
            }
        }
        TypedStageKind::Call { .. }
        | TypedStageKind::Bind { .. }
        | TypedStageKind::Map { .. }
        | TypedStageKind::Filter { .. }
        | TypedStageKind::FaultMap { .. }
        | TypedStageKind::Field { .. } => {}
    }
}

fn collect_endpoint_var_names(endpoint: &TypedEndpoint, names: &mut BTreeSet<String>) {
    match &endpoint.kind {
        TypedEndpointKind::Variable(name) => {
            names.insert(name.clone());
        }
        TypedEndpointKind::Tuple(items) | TypedEndpointKind::Seq(items) => {
            for item in items {
                collect_endpoint_var_names(item, names);
            }
        }
        TypedEndpointKind::Struct { fields, .. } => {
            for (_, item) in fields {
                collect_endpoint_var_names(item, names);
            }
        }
        TypedEndpointKind::Eval { source, stages } => {
            collect_endpoint_var_names(source, names);
            for stage in stages {
                collect_stage_endpoint_var_names(stage, names);
            }
        }
        TypedEndpointKind::NodeRef { .. }
        | TypedEndpointKind::Int(_)
        | TypedEndpointKind::Real(_)
        | TypedEndpointKind::Bool(_)
        | TypedEndpointKind::String(_)
        | TypedEndpointKind::Unit => {}
    }
}

fn collect_stage_endpoint_var_names(stage: &TypedStage, names: &mut BTreeSet<String>) {
    match &stage.kind {
        TypedStageKind::Repeat { count, .. }
        | TypedStageKind::Reduce {
            identity: count, ..
        }
        | TypedStageKind::Scan {
            identity: count, ..
        } => collect_endpoint_var_names(count, names),
        TypedStageKind::Match { arms } => {
            for arm in arms {
                if let TypedMatchGuard::Call { args, .. } = &arm.guard {
                    for arg in args {
                        collect_endpoint_var_names(arg, names);
                    }
                }
                if let TypedMatchTarget::Value(endpoint) = &arm.target {
                    collect_endpoint_var_names(endpoint, names);
                }
            }
        }
        TypedStageKind::Call { .. }
        | TypedStageKind::Bind { .. }
        | TypedStageKind::Map { .. }
        | TypedStageKind::Filter { .. }
        | TypedStageKind::FaultMap { .. }
        | TypedStageKind::Field { .. } => {}
    }
}

fn collect_binding_target_vars(target: &BindingTarget, names: &mut Vec<String>) {
    match target {
        BindingTarget::Discard => {}
        BindingTarget::Variable(name) => names.push(name.clone()),
        BindingTarget::Tuple(items) => {
            for item in items {
                collect_binding_target_vars(item, names);
            }
        }
    }
}

fn endpoint_contains_empty_seq(endpoint: &TypedEndpoint) -> bool {
    match &endpoint.kind {
        TypedEndpointKind::Seq(items) => {
            items.is_empty() || items.iter().any(endpoint_contains_empty_seq)
        }
        TypedEndpointKind::Tuple(items) => items.iter().any(endpoint_contains_empty_seq),
        TypedEndpointKind::Struct { fields, .. } => fields
            .iter()
            .any(|(_, endpoint)| endpoint_contains_empty_seq(endpoint)),
        TypedEndpointKind::Eval { source, stages } => {
            endpoint_contains_empty_seq(source)
                || stages.iter().any(|stage| match stage {
                    TypedStage {
                        kind: TypedStageKind::Repeat { count, .. },
                        ..
                    }
                    | TypedStage {
                        kind:
                            TypedStageKind::Reduce {
                                identity: count, ..
                            },
                        ..
                    }
                    | TypedStage {
                        kind:
                            TypedStageKind::Scan {
                                identity: count, ..
                            },
                        ..
                    } => endpoint_contains_empty_seq(count),
                    TypedStage {
                        kind: TypedStageKind::Match { arms },
                        ..
                    } => arms.iter().any(|arm| {
                        (match &arm.guard {
                            TypedMatchGuard::Call { args, .. } => {
                                args.iter().any(endpoint_contains_empty_seq)
                            }
                            TypedMatchGuard::Fallback => false,
                        }) || match &arm.target {
                            TypedMatchTarget::Value(endpoint) => {
                                endpoint_contains_empty_seq(endpoint)
                            }
                            TypedMatchTarget::Node { .. } => false,
                        }
                    }),
                    _ => false,
                })
        }
        _ => false,
    }
}

fn is_zero(endpoint: &TypedEndpoint) -> bool {
    match &endpoint.kind {
        TypedEndpointKind::Int(value) => *value == 0,
        TypedEndpointKind::Real(value) => *value == 0.0,
        _ => false,
    }
}

fn matches_pair_source(endpoint: &TypedEndpoint, left: &str, right: &str) -> bool {
    matches!(
        &endpoint.kind,
        TypedEndpointKind::Tuple(items)
            if items.len() == 2
                && matches!(&items[0].kind, TypedEndpointKind::Variable(name) if name == left)
                && matches!(&items[1].kind, TypedEndpointKind::Variable(name) if name == right)
    )
}

fn flatten_add_terms(name: &str, additions: &HashMap<String, (String, String)>) -> Vec<String> {
    if let Some((left, right)) = additions.get(name) {
        let mut out = flatten_add_terms(left, additions);
        out.extend(flatten_add_terms(right, additions));
        out
    } else {
        vec![name.to_string()]
    }
}

fn type_name(ty: &Ty) -> String {
    format!("Fa{}", sanitize_symbol(&type_suffix(ty)))
}

fn format_binding_target_for_error(target: &BindingTarget) -> String {
    match target {
        BindingTarget::Discard => "$".to_string(),
        BindingTarget::Variable(name) => format!("${name}"),
        BindingTarget::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(format_binding_target_for_error)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn binding_target_is_discard(target: &BindingTarget) -> bool {
    matches!(target, BindingTarget::Discard)
}

fn wasm_exportable_input(ty: &Ty) -> bool {
    match ty {
        Ty::Unit | Ty::Int | Ty::Real => true,
        Ty::Tuple(items) => items.iter().all(wasm_exportable_scalar),
        _ => false,
    }
}

fn wasm_exportable_output(ty: &Ty) -> bool {
    wasm_exportable_scalar(ty)
}

fn wasm_exportable_scalar(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Real)
}

#[cfg(not(target_arch = "wasm32"))]
fn export_abi_label(abi: DirectExportAbi) -> &'static str {
    match abi {
        DirectExportAbi::Wasm => "WASM",
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn native_c_input_items<'a>(
    export_name: &str,
    callable: &TypedCallable,
    input_ty: &'a Ty,
) -> Result<Vec<&'a Ty>, String> {
    match callable.inputs.as_slice() {
        [] => Ok(Vec::new()),
        [_] => Ok(vec![input_ty]),
        ports => {
            let Ty::Tuple(items) = input_ty else {
                return Err(format!(
                    "native C export `{export_name}` has multiple inputs but signature input is `{input_ty}`"
                ));
            };
            if items.len() != ports.len() {
                return Err(format!(
                    "native C export `{export_name}` input arity mismatch: signature has {}, declaration has {}",
                    items.len(),
                    ports.len()
                ));
            }
            Ok(items.iter().collect())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_abi_type(registry: &mut TypeRegistry, ty: &Ty) {
    registry.c_type(ty);
    match ty {
        Ty::Seq(item) | Ty::Stream(item) | Ty::Faultable(item) => collect_abi_type(registry, item),
        Ty::Tuple(items) | Ty::OneOf(items) => {
            for item in items {
                collect_abi_type(registry, item);
            }
        }
        Ty::Struct { fields, .. } => {
            for (_, item) in fields {
                collect_abi_type(registry, item);
            }
        }
        Ty::HttpListener => collect_abi_type(registry, &Ty::HttpServerConfig),
        Ty::HttpResponse => {
            collect_abi_type(registry, &Ty::HttpRequest);
            collect_abi_type(registry, &Ty::Seq(Box::new(Ty::Bytes)));
        }
        Ty::SqliteRow => collect_abi_type(registry, &Ty::SqliteValue),
        Ty::Unit
        | Ty::Int
        | Ty::Real
        | Ty::Bool
        | Ty::Bytes
        | Ty::Args
        | Ty::HttpServerConfig
        | Ty::HttpRequest
        | Ty::SqliteConnection
        | Ty::SqliteValue
        | Ty::Fault
        | Ty::Var(_)
        | Ty::EmptySeq => {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn native_c_header_source(registry: &mut TypeRegistry, prototypes: &[String]) -> String {
    let mut out = String::new();
    out.push_str("#ifndef FLOWARROW_NATIVE_LIBRARY_H\n");
    out.push_str("#define FLOWARROW_NATIVE_LIBRARY_H\n\n");
    out.push_str("#include <stdbool.h>\n");
    out.push_str("#include <stddef.h>\n");
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <stdio.h>\n\n");
    out.push_str("typedef struct { int _unused; } FaUnit;\n");
    out.push_str("typedef struct { char *bytes; size_t len; } FaBytes;\n");
    out.push_str("typedef struct { FaBytes message; } FaFault;\n");
    out.push_str("typedef struct { int argc; char **argv; } FaArgs;\n");
    out.push_str("typedef int (*FaStreamNextFn)(void *state, void *out, FaFault *fault);\n");
    out.push_str("typedef int (*FaStreamCloseFn)(void *state, FaFault *fault);\n");
    out.push_str(
        "typedef struct { FILE *file; int fd; FaBytes path; void *state; void *map_fn; FaStreamNextFn next; FaStreamCloseFn close; size_t item_size; bool closed; } FaStream;\n",
    );
    out.push_str("typedef struct { size_t count; FaBytes *items; } FaSeq_Bytes;\n");
    out.push_str("typedef struct { FaBytes f0; FaBytes f1; } FaTuple_Bytes_Bytes;\n");
    out.push_str(
        "typedef struct { size_t count; FaTuple_Bytes_Bytes *items; } FaSeq_Tuple_Bytes_Bytes;\n",
    );
    out.push_str("typedef struct { size_t count; int64_t *items; } FaSeq_Int;\n");
    out.push_str("typedef struct { size_t count; double *items; } FaSeq_Real;\n");
    out.push_str("typedef struct { size_t count; FaFault *items; } FaSeq_Fault;\n");
    out.push_str(
        "typedef struct { bool is_fault; FaFault fault; int64_t value; } FaFaultable_Int;\n",
    );
    out.push_str(
        "typedef struct { bool is_fault; FaFault fault; double value; } FaFaultable_Real;\n",
    );
    out.push_str(
        "typedef struct { bool is_fault; FaFault fault; FaBytes value; } FaFaultable_Bytes;\n",
    );
    out.push_str("typedef struct { bool is_fault; FaFault fault; FaSeq_Bytes value; } FaFaultable_Seq_Bytes;\n");
    out.push_str("typedef struct { bool is_fault; FaFault fault; FaSeq_Tuple_Bytes_Bytes value; } FaFaultable_Seq_Tuple_Bytes_Bytes;\n");
    out.push_str("typedef struct { bool is_fault; FaFault fault; FaStream value; } FaFaultable_Stream_Bytes;\n");
    out.push_str("typedef struct { bool is_fault; FaFault fault; FaSeq_Real value; } FaFaultable_Seq_Real;\n\n");
    out.push_str(&registry.emit_abi_typedefs());
    out.push_str("#ifdef __cplusplus\n");
    out.push_str("extern \"C\" {\n");
    out.push_str("#endif\n\n");
    for prototype in prototypes {
        out.push_str(prototype);
        out.push('\n');
    }
    out.push_str("\n#ifdef __cplusplus\n");
    out.push_str("}\n");
    out.push_str("#endif\n\n");
    out.push_str("#endif\n");
    out
}

fn type_suffix(ty: &Ty) -> String {
    match ty {
        Ty::Unit => "Unit".to_string(),
        Ty::Int => "Int".to_string(),
        Ty::Real => "Real".to_string(),
        Ty::Bool => "Bool".to_string(),
        Ty::Bytes => "Bytes".to_string(),
        Ty::Args => "Args".to_string(),
        Ty::HttpServerConfig => "HttpServerConfig".to_string(),
        Ty::HttpListener => "HttpListener".to_string(),
        Ty::HttpRequest => "HttpRequest".to_string(),
        Ty::HttpResponse => "HttpResponse".to_string(),
        Ty::SqliteConnection => "SqliteConnection".to_string(),
        Ty::SqliteRow => "SqliteRow".to_string(),
        Ty::SqliteValue => "SqliteValue".to_string(),
        Ty::Stream(item) => format!("Stream_{}", type_suffix(item)),
        Ty::Fault => "Fault".to_string(),
        Ty::Faultable(inner) => format!("Faultable_{}", type_suffix(inner)),
        Ty::Seq(item) => format!("Seq_{}", type_suffix(item)),
        Ty::Tuple(items) => format!(
            "Tuple_{}",
            items.iter().map(type_suffix).collect::<Vec<_>>().join("_")
        ),
        Ty::Struct { name, .. } => format!("Struct_{name}"),
        Ty::OneOf(items) => format!(
            "OneOf_{}",
            items.iter().map(type_suffix).collect::<Vec<_>>().join("_")
        ),
        Ty::Var(name) => format!("Var_{name}"),
        Ty::EmptySeq => "EmptySeq".to_string(),
    }
}

fn type_depth(ty: &Ty) -> usize {
    match ty {
        Ty::Seq(item) | Ty::Stream(item) | Ty::Faultable(item) => 1 + type_depth(item),
        Ty::Tuple(items) | Ty::OneOf(items) => 1 + items.iter().map(type_depth).max().unwrap_or(0),
        Ty::Struct { fields, .. } => {
            1 + fields
                .iter()
                .map(|(_, ty)| type_depth(ty))
                .max()
                .unwrap_or(0)
        }
        Ty::EmptySeq => 0,
        _ => 0,
    }
}

fn user_fn_name(name: &str) -> String {
    if name == "main" {
        "flow_program_main".to_string()
    } else {
        format!("flow_node_{}", sanitize_symbol(name))
    }
}

fn c_ident(name: &str) -> String {
    format!("v_{}", sanitize_symbol(name))
}

fn sanitize_symbol(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn c_string(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parser, typecheck};

    fn checked_module(source: &str) -> Module {
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        module
    }

    fn lowered_module(module: &Module) -> LoweredModule {
        LoweredModule::with_stdlib_sources(module).expect("lowered module")
    }

    fn function_body<'a>(runtime_c: &'a str, name: &str) -> &'a str {
        let start = runtime_c.find(name).expect("function name");
        let body_start = runtime_c[start..].find(" {\n").expect("function body") + start;
        let body_end = runtime_c[body_start..]
            .find("\n}\n\n")
            .expect("function end")
            + body_start;
        &runtime_c[body_start..body_end]
    }

    fn extern_visibility_module() -> Module {
        parser::parse(
            r#"
                extern node exposed(value: Int) -> out: Int {
                    $value -> hidden -> $out
                }

                node hidden(value: Int) -> out: Int {
                    $value -> $out
                }
            "#,
        )
        .expect("parse")
    }

    #[test]
    fn typescript_exports_only_extern_nodes() {
        let module = extern_visibility_module();
        let lowered = lowered_module(&module);

        let ts =
            typescript::emit_module(lowered.typed().expect("typed codegen")).expect("typescript");

        assert!(ts.contains("export function exposed(value: bigint): bigint"));
        assert!(ts.contains("\nfunction hidden(value: bigint): bigint"));
        assert!(!ts.contains("export function hidden"));
    }

    #[test]
    fn wasm_exports_only_extern_nodes() {
        let module = extern_visibility_module();
        let lowered = lowered_module(&module);
        let emitted = DirectLlvm::emit_with_options(
            lowered.typed().expect("typed codegen"),
            DirectLlvmOptions {
                emit_entrypoint: false,
                export_abi: Some(DirectExportAbi::Wasm),
                ..DirectLlvmOptions::default()
            },
        )
        .expect("llvm");

        assert_eq!(emitted.symbol_exports, vec!["exposed"]);
        assert!(emitted.llvm.contains("define i64 @exposed(i64"));
        assert!(!emitted.llvm.contains("define i64 @hidden(i64"));
    }

    #[test]
    fn native_c_exports_generate_compound_abi_header() {
        let module = parser::parse(
            r#"
                extern node parts(value: Int) -> (original: Int, doubled: Int) {
                    $value       -> $original
                    ($value, 2)  -> mul -> $doubled
                }

                extern node label() -> value: Bytes {
                    "compound" -> $value
                }

                import std.math { mul }
            "#,
        )
        .expect("parse");
        let lowered = lowered_module(&module);
        let mut codegen = lowered.typed().expect("typed codegen");
        let emitted = codegen.emit_native_cdylib_c().expect("native c");

        assert_eq!(emitted.exports, vec!["parts", "label"]);
        assert!(
            emitted
                .header
                .contains("typedef struct { int64_t f0; int64_t f1; } FaTuple_Int_Int;")
        );
        assert!(
            emitted
                .header
                .contains("FaTuple_Int_Int parts(int64_t v_value);")
        );
        assert!(emitted.header.contains("FaBytes label(void);"));
        assert!(
            emitted
                .source
                .contains("FaTuple_Int_Int parts(int64_t v_value)")
        );
        assert!(emitted.source.contains("FaBytes label(void)"));
    }

    #[test]
    fn llvm_entry_is_only_a_thin_shim_to_unboxed_c_runtime() {
        let module = checked_module(
            r#"
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    0 -> $exit_code
                }
            "#,
        );

        let lowered = lowered_module(&module);
        let llvm = DirectLlvm::emit(lowered.typed().expect("typed codegen")).expect("llvm");

        assert!(llvm.contains("define i32 @flow_unboxed_main(i32"));
        assert!(llvm.contains("define i32 @main(i32"));
        assert!(llvm.contains("call i32 @flow_unboxed_main(i32"));
        assert!(llvm.contains("ret i32"));
    }

    #[test]
    fn runtime_emits_typed_values_and_generated_loops() {
        let module = checked_module(
            r#"
                import std.cli { Args }
                import std.bytes { split_lines }
                import std.predicates { not_empty }
                import std.real { parse_real, format_real }
                import std.math { add }
                import std.io { read_stdin, write_stdout }

                program main(args: Args) -> exit_code: Faultable[Int] {
                    () -> read_stdin -> split_lines -> filter not_empty -> map parse_real -> reduce add(identity: 0.0) -> $total
                    $total -> format_real -> write_stdout -> $exit_code
                }
            "#,
        );

        let runtime_c = emit_runtime_c(&module).expect("runtime c");

        assert!(runtime_c.contains(
            "typedef struct { bool is_fault; FaFault fault; double value; } FaFaultable_Real;"
        ));
        assert!(runtime_c.contains("for (size_t"));
        assert!(!runtime_c.contains("FaValue"));
        assert!(!runtime_c.contains("fa_map("));
        assert!(!runtime_c.contains("fa_reduce("));
    }

    #[test]
    fn pure_maps_emit_parallel_workers() {
        let module = checked_module(
            r#"
                import std.cli { Args }
                import std.math { abs }

                program main(args: Args) -> exit_code: Int {
                    [-1, -2, -3] -> map abs -> $values
                    0 -> $exit_code
                }
            "#,
        );

        let runtime_c = emit_runtime_c(&module).expect("runtime c");

        assert!(runtime_c.contains("fa_parallel_map_worker_"));
        assert!(runtime_c.contains("fa_parallel_for(0,"));
    }

    #[test]
    fn independent_pure_chains_emit_parallel_tasks() {
        let module = parser::parse(
            r#"
                import std.math { add, max, mul }

                struct JobSummary {
                    total_score: Int,
                    peak_score: Int,
                    total_weight: Int,
                    peak_weight: Int,
                }

                extern node score_batch(width: Int) -> summary: JobSummary {
                    (1, $width, 1) -> range_step              -> $jobs
                    $jobs        -> map score_job           -> $scores
                    $jobs        -> map weight_job          -> $weights
                    $scores      -> reduce add(identity: 0) -> $total_score
                    $scores      -> reduce max(identity: 0) -> $peak_score
                    $weights     -> reduce add(identity: 0) -> $total_weight
                    $weights     -> reduce max(identity: 0) -> $peak_weight
                    JobSummary {
                        total_score: $total_score,
                        peak_score: $peak_score,
                        total_weight: $total_weight,
                        peak_weight: $peak_weight
                    } -> $summary
                }

                node score_job(n: Int) -> score: Int {
                    ($n, $n)      -> mul -> $square
                    ($square, $n) -> add -> $score
                }

                node weight_job(n: Int) -> weight: Int {
                    ($n, 2)       -> mul -> $doubled
                    ($doubled, 1) -> add -> $weight
                }
            "#,
        )
        .expect("parse");

        let lowered = lowered_module(&module);
        let mut codegen = lowered.typed().expect("typed codegen");
        let emitted = codegen.emit_native_cdylib_c().expect("native c");

        assert!(emitted.source.contains("fa_parallel_tasks"));
        assert!(emitted.source.contains("fa_parallel_chain_worker_0"));
        assert!(emitted.source.contains("fa_parallel_chain_worker_2"));
        assert!(emitted.source.contains("fa_parallel_tasks(2,"));
        assert!(emitted.source.contains("fa_parallel_tasks(4,"));
    }

    #[test]
    fn matrix_reduction_pipelines_avoid_materialized_intermediates() {
        let module = checked_module(
            r#"
                import std.cli { Args }
                import std.math { add, eq }
                import std.matrix { matmul, matvec, row_sums, sum as matrix_sum }
                import std.vector { sum as vector_sum }

                program main(args: Args) -> exit_code: Int {
                    [[1.0, 2.0], [3.0, 4.0]] -> $left
                    [[5.0, 6.0], [7.0, 8.0]] -> $right
                    [9.0, 10.0] -> $vector
                    ($left, $right) -> matmul -> $product
                    $product -> matrix_sum -> $product_sum
                    ($left, $vector) -> matvec -> $mv
                    $mv -> vector_sum -> $mv_sum
                    $left -> row_sums -> vector_sum -> $row_sum
                    ($product_sum, $mv_sum) -> add -> $partial
                    ($partial, $row_sum) -> add -> $score
                    ($score, 240.0) -> eq -> $ok
                    ($ok, 0, 1) -> select -> $exit_code
                }
            "#,
        );

        let runtime_c = emit_runtime_c(&module).expect("runtime c");
        let main = function_body(&runtime_c, "flow_program_main");

        assert!(!main.contains("flow_node___flow_std_matrix_matmul"));
        assert!(!main.contains("flow_node___flow_std_matrix_matvec"));
        assert!(!main.contains("flow_node___flow_std_matrix_row_sums"));
        assert!(main.contains("for (size_t"));
    }

    #[test]
    fn structs_emit_named_c_shapes_and_field_projection() {
        let module = checked_module(
            r#"
                import std.cli { Args }
                import std.math { add }

                struct Point {
                    x: Int,
                    y: Int,
                }

                node sum_point(point: Point) -> total: Int {
                    $point -> field x -> $x
                    $point -> field y -> $y
                    ($x, $y) -> add -> $total
                }

                program main(args: Args) -> exit_code: Int {
                    Point { x: 20, y: 22 } -> sum_point -> $exit_code
                }
            "#,
        );

        let runtime_c = emit_runtime_c(&module).expect("runtime c");

        assert!(runtime_c.contains("typedef struct { int64_t v_x; int64_t v_y; } FaStruct_Point;"));
        assert!(runtime_c.contains(".v_x"));
        assert!(runtime_c.contains(".v_y"));
    }

    #[test]
    fn structs_emit_typescript_object_shapes() {
        let module = checked_module(
            r#"
                import std.cli { Args }
                import std.math { add }

                struct Point {
                    x: Int,
                    y: Int,
                }

                node sum_point(point: Point) -> total: Int {
                    $point -> field x -> $x
                    $point -> field y -> $y
                    ($x, $y) -> add -> $total
                }

                program main(args: Args) -> exit_code: Int {
                    Point { x: 20, y: 22 } -> sum_point -> $exit_code
                }
            "#,
        );

        let lowered = lowered_module(&module);
        let ts =
            typescript::emit_module(lowered.typed().expect("typed codegen")).expect("typescript");

        assert!(ts.contains("point: { x: bigint; y: bigint }"));
        assert!(ts.contains("point.x"));
        assert!(ts.contains("point.y"));
        assert!(ts.contains("{ x: 20n, y: 22n }"));
    }
}
