use crate::ast::*;
use crate::module_resolver;
use crate::stdlib::{self, RuntimeSupport};
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module as LlvmModule;
use inkwell::types::{AnyType, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

mod llvm_stdlib;

#[allow(dead_code)]
pub fn emit_module(module: &Module) -> Result<String, String> {
    crate::llvm_backend::emit_module(module)
}

pub fn emit_direct_llvm_with_base(module: &Module, base_dir: &Path) -> Result<String, String> {
    let expanded = module_resolver::expand_sources(module, Some(base_dir))?;
    let codegen = TypedCodegen::new(&expanded)?;
    DirectLlvm::emit(codegen)
}

#[allow(dead_code)]
pub fn emit_runtime_c(module: &Module) -> Result<String, String> {
    let expanded = module_resolver::expand_stdlib_sources(module)?;
    TypedCodegen::new(&expanded)?.emit()
}

#[allow(dead_code)]
pub fn emit_runtime_c_with_base(module: &Module, base_dir: &Path) -> Result<String, String> {
    let expanded = module_resolver::expand_sources(module, Some(base_dir))?;
    TypedCodegen::new(&expanded)?.emit()
}

pub fn emit_runtime_support_c_with_base(
    module: &Module,
    base_dir: &Path,
) -> Result<String, String> {
    let expanded = module_resolver::expand_sources(module, Some(base_dir))?;
    let mut codegen = TypedCodegen::new(&expanded)?;
    codegen.emit_runtime_support_c()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Ty {
    Unit,
    Int,
    Real,
    Bool,
    Bytes,
    Args,
    HttpServerConfig,
    HttpListener,
    HttpRequest,
    HttpResponse,
    SqliteConnection,
    SqliteRow,
    SqliteValue,
    Stream(Box<Ty>),
    Fault,
    Faultable(Box<Ty>),
    Seq(Box<Ty>),
    Tuple(Vec<Ty>),
    OneOf(Vec<Ty>),
    Var(String),
    EmptySeq,
}

#[derive(Debug, Clone)]
struct Signature {
    input: Ty,
    output: Ty,
}

#[derive(Debug, Clone)]
struct Value {
    code: String,
    ty: Ty,
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
enum BroadcastSide {
    Left,
    Right,
}

struct TypedCodegen<'a> {
    module: &'a Module,
    temp: usize,
    parallel_helper: usize,
    stream_helper: usize,
    parallel_helpers: String,
    callables: HashMap<String, &'a Callable>,
    signatures: HashMap<String, Signature>,
    stdlib_names: HashMap<String, String>,
    aliases: HashMap<String, Ty>,
    types: TypeRegistry,
}

impl<'a> TypedCodegen<'a> {
    fn new(module: &'a Module) -> Result<Self, String> {
        let mut codegen = Self {
            module,
            temp: 0,
            parallel_helper: 0,
            stream_helper: 0,
            parallel_helpers: String::new(),
            callables: HashMap::new(),
            signatures: HashMap::new(),
            stdlib_names: HashMap::new(),
            aliases: HashMap::new(),
            types: TypeRegistry::default(),
        };
        codegen.collect_imports();
        codegen.collect_type_aliases()?;
        codegen.collect_callables()?;
        Ok(codegen)
    }

    fn emit(mut self) -> Result<String, String> {
        let mut bodies = String::new();
        let mut names = self.callables.keys().cloned().collect::<Vec<_>>();
        names.sort();
        let uses_cv_runtime = self.uses_cv_runtime();
        let uses_http_runtime = self.uses_http_runtime();
        let uses_sqlite_runtime = self.uses_sqlite_runtime();

        for name in &names {
            let sig = self
                .signatures
                .get(name)
                .ok_or_else(|| format!("missing signature for `{name}`"))?;
            self.types.c_type(&sig.input);
            self.types.c_type(&sig.output);
        }
        if uses_cv_runtime {
            let image = cv_image_ty();
            self.types.c_type(&image);
            self.types.c_type(&Ty::Faultable(Box::new(image)));
            self.types.c_type(&Ty::Faultable(Box::new(Ty::Bytes)));
        }
        if uses_http_runtime {
            self.types.c_type(&Ty::HttpServerConfig);
            self.types.c_type(&Ty::HttpListener);
            self.types.c_type(&Ty::HttpRequest);
            self.types.c_type(&Ty::HttpResponse);
            self.types.c_type(&Ty::Tuple(vec![
                Ty::HttpListener,
                Ty::Stream(Box::new(Ty::HttpResponse)),
            ]));
            self.types
                .c_type(&Ty::Tuple(vec![Ty::HttpResponse, Ty::Int]));
            self.types
                .c_type(&Ty::Tuple(vec![Ty::HttpResponse, Ty::Bytes]));
            self.types
                .c_type(&Ty::Tuple(vec![Ty::HttpResponse, Ty::Bytes, Ty::Bytes]));
            self.types
                .c_type(&Ty::Faultable(Box::new(Ty::HttpListener)));
            self.types.c_type(&Ty::Stream(Box::new(Ty::HttpRequest)));
            self.types.c_type(&Ty::Stream(Box::new(Ty::HttpResponse)));
        }
        if uses_sqlite_runtime {
            self.types.c_type(&Ty::SqliteConnection);
            self.types.c_type(&Ty::SqliteRow);
            self.types.c_type(&Ty::SqliteValue);
            let seq_sqlite_value = Ty::Seq(Box::new(Ty::SqliteValue));
            let seq_sqlite_row = Ty::Seq(Box::new(Ty::SqliteRow));
            let stream_sqlite_row = Ty::Stream(Box::new(Ty::SqliteRow));
            let tuple_conn_bool = Ty::Tuple(vec![Ty::SqliteConnection, Ty::Bool]);
            let tuple_conn_int = Ty::Tuple(vec![Ty::SqliteConnection, Ty::Int]);
            let tuple_conn_bytes_params = Ty::Tuple(vec![
                Ty::SqliteConnection,
                Ty::Bytes,
                seq_sqlite_value.clone(),
            ]);
            let tuple_conn_stream_row =
                Ty::Tuple(vec![Ty::SqliteConnection, stream_sqlite_row.clone()]);
            let tuple_conn_seq_row = Ty::Tuple(vec![Ty::SqliteConnection, seq_sqlite_row.clone()]);
            let tuple_row_int = Ty::Tuple(vec![Ty::SqliteRow, Ty::Int]);
            let tuple_row_bytes = Ty::Tuple(vec![Ty::SqliteRow, Ty::Bytes]);
            self.types.c_type(&tuple_conn_bool);
            self.types.c_type(&tuple_conn_int);
            self.types.c_type(&tuple_conn_bytes_params);
            self.types.c_type(&tuple_conn_stream_row);
            self.types.c_type(&tuple_conn_seq_row);
            self.types.c_type(&tuple_row_int);
            self.types.c_type(&tuple_row_bytes);
            self.types
                .c_type(&Ty::Faultable(Box::new(Ty::SqliteConnection)));
            self.types.c_type(&Ty::Faultable(Box::new(Ty::SqliteValue)));
            self.types.c_type(&Ty::Faultable(Box::new(tuple_conn_int)));
            self.types
                .c_type(&Ty::Faultable(Box::new(tuple_conn_stream_row)));
            self.types
                .c_type(&Ty::Faultable(Box::new(tuple_conn_seq_row)));
            self.types.c_type(&seq_sqlite_value);
            self.types.c_type(&seq_sqlite_row);
            self.types.c_type(&stream_sqlite_row);
        }
        self.types.set_use_cv_header(uses_cv_runtime);

        for decl in &self.module.declarations {
            match decl {
                Decl::TypeAlias(_) => {}
                Decl::Node(callable) => self.emit_callable(&mut bodies, callable, false)?,
                Decl::Program(callable) => self.emit_callable(&mut bodies, callable, true)?,
                Decl::Import(_) => {}
            }
        }

        let mut out = String::new();
        emit_preamble(&mut out);
        if self.types.uses_cv_header() {
            stdlib::emit_cv_type_h(&mut out);
        }
        if uses_http_runtime {
            stdlib::emit_http_runtime_h(&mut out);
        }
        if uses_sqlite_runtime {
            stdlib::emit_sqlite_runtime_h(&mut out);
        }
        out.push_str(&self.types.emit_typedefs());
        out.push_str(&self.types.emit_helpers());
        if uses_cv_runtime {
            stdlib::emit_cv_runtime_h(&mut out);
            stdlib::emit_cv_runtime_c(&mut out);
        }
        if uses_http_runtime {
            stdlib::emit_http_runtime_c(&mut out);
        }
        if uses_sqlite_runtime {
            stdlib::emit_sqlite_runtime_c(&mut out);
        }
        for name in &names {
            let sig = self.signatures.get(name).expect("signature");
            let input = self.types.c_type(&sig.input);
            let output = self.types.c_type(&sig.output);
            out.push_str(&format!(
                "static inline {output} {}({input} input);\n",
                user_fn_name(name)
            ));
        }
        out.push('\n');
        out.push_str(&self.parallel_helpers);
        out.push_str(&bodies);
        out.push_str(
            "int flow_unboxed_main(int argc, char **argv) {\n\
  FaArgs args;\n\
  args.argc = argc;\n\
  args.argv = argv;\n\
  ",
        );
        let main_sig = self
            .signatures
            .get("main")
            .ok_or_else(|| "missing `program main`".to_string())?;
        let main_out = self.types.c_type(&main_sig.output);
        out.push_str(&format!("{main_out} result = flow_program_main(args);\n"));
        match &main_sig.output {
            Ty::Faultable(inner) if inner.as_ref() == &Ty::Int => {
                out.push_str("  if (result.is_fault) fa_exit_fault(result.fault);\n  return (int)result.value;\n}\n");
            }
            Ty::Int => out.push_str("  return (int)result;\n}\n"),
            other => return Err(format!("program main output must be Int, found `{other}`")),
        }
        Ok(out)
    }

    fn emit_runtime_support_c(&mut self) -> Result<String, String> {
        let mut out = String::new();
        let uses_cv_runtime = self.uses_cv_runtime();
        let uses_http_runtime = self.uses_http_runtime();
        let uses_sqlite_runtime = self.uses_sqlite_runtime();
        self.collect_runtime_support_types(uses_cv_runtime, uses_http_runtime, uses_sqlite_runtime);
        self.types.set_use_cv_header(uses_cv_runtime);

        emit_preamble(&mut out);
        if uses_cv_runtime {
            stdlib::emit_cv_type_h(&mut out);
        }
        if uses_http_runtime {
            stdlib::emit_http_runtime_h(&mut out);
        }
        if uses_sqlite_runtime {
            stdlib::emit_sqlite_runtime_h(&mut out);
        }
        out.push_str(&self.types.emit_typedefs());
        out.push_str(&self.types.emit_helpers());
        if uses_cv_runtime {
            stdlib::emit_cv_runtime_h(&mut out);
            stdlib::emit_cv_runtime_c(&mut out);
        }
        if uses_http_runtime {
            stdlib::emit_http_runtime_c(&mut out);
        }
        if uses_sqlite_runtime {
            stdlib::emit_sqlite_runtime_c(&mut out);
        }
        out.push_str(
            "static int64_t fa_write_stdout(FaBytes bytes) { return fa_write_bytes(stdout, bytes); }\n\
static int64_t fa_write_stderr(FaBytes bytes) { return fa_write_bytes(stderr, bytes); }\n",
        );
        Ok(out)
    }

    fn collect_runtime_support_types(
        &mut self,
        uses_cv_runtime: bool,
        uses_http_runtime: bool,
        uses_sqlite_runtime: bool,
    ) {
        if uses_cv_runtime {
            let image = cv_image_ty();
            self.types.c_type(&image);
            self.types.c_type(&Ty::Faultable(Box::new(image)));
            self.types.c_type(&Ty::Faultable(Box::new(Ty::Bytes)));
        }
        if uses_http_runtime {
            self.types.c_type(&Ty::HttpServerConfig);
            self.types.c_type(&Ty::HttpListener);
            self.types.c_type(&Ty::HttpRequest);
            self.types.c_type(&Ty::HttpResponse);
            self.types.c_type(&Ty::Tuple(vec![
                Ty::HttpListener,
                Ty::Stream(Box::new(Ty::HttpResponse)),
            ]));
            self.types
                .c_type(&Ty::Tuple(vec![Ty::HttpResponse, Ty::Int]));
            self.types
                .c_type(&Ty::Tuple(vec![Ty::HttpResponse, Ty::Bytes]));
            self.types
                .c_type(&Ty::Tuple(vec![Ty::HttpResponse, Ty::Bytes, Ty::Bytes]));
            self.types
                .c_type(&Ty::Faultable(Box::new(Ty::HttpListener)));
            self.types.c_type(&Ty::Stream(Box::new(Ty::HttpRequest)));
            self.types.c_type(&Ty::Stream(Box::new(Ty::HttpResponse)));
        }
        if uses_sqlite_runtime {
            self.types.c_type(&Ty::SqliteConnection);
            self.types.c_type(&Ty::SqliteRow);
            self.types.c_type(&Ty::SqliteValue);
            let seq_sqlite_value = Ty::Seq(Box::new(Ty::SqliteValue));
            let seq_sqlite_row = Ty::Seq(Box::new(Ty::SqliteRow));
            let stream_sqlite_row = Ty::Stream(Box::new(Ty::SqliteRow));
            let tuple_conn_bool = Ty::Tuple(vec![Ty::SqliteConnection, Ty::Bool]);
            let tuple_conn_int = Ty::Tuple(vec![Ty::SqliteConnection, Ty::Int]);
            let tuple_conn_bytes_params = Ty::Tuple(vec![
                Ty::SqliteConnection,
                Ty::Bytes,
                seq_sqlite_value.clone(),
            ]);
            let tuple_conn_stream_row =
                Ty::Tuple(vec![Ty::SqliteConnection, stream_sqlite_row.clone()]);
            let tuple_conn_seq_row = Ty::Tuple(vec![Ty::SqliteConnection, seq_sqlite_row.clone()]);
            let tuple_row_int = Ty::Tuple(vec![Ty::SqliteRow, Ty::Int]);
            let tuple_row_bytes = Ty::Tuple(vec![Ty::SqliteRow, Ty::Bytes]);
            self.types.c_type(&tuple_conn_bool);
            self.types.c_type(&tuple_conn_int);
            self.types.c_type(&tuple_conn_bytes_params);
            self.types.c_type(&tuple_conn_stream_row);
            self.types.c_type(&tuple_conn_seq_row);
            self.types.c_type(&tuple_row_int);
            self.types.c_type(&tuple_row_bytes);
            self.types
                .c_type(&Ty::Faultable(Box::new(Ty::SqliteConnection)));
            self.types.c_type(&Ty::Faultable(Box::new(Ty::SqliteValue)));
            self.types.c_type(&Ty::Faultable(Box::new(tuple_conn_int)));
            self.types
                .c_type(&Ty::Faultable(Box::new(tuple_conn_stream_row)));
            self.types
                .c_type(&Ty::Faultable(Box::new(tuple_conn_seq_row)));
            self.types.c_type(&seq_sqlite_value);
            self.types.c_type(&seq_sqlite_row);
            self.types.c_type(&stream_sqlite_row);
        }
    }

    fn uses_cv_runtime(&self) -> bool {
        self.module.declarations.iter().any(|decl| {
            let (Decl::Node(callable) | Decl::Program(callable)) = decl else {
                return false;
            };
            callable
                .chains
                .iter()
                .flat_map(|chain| chain.stages.iter())
                .any(|stage| self.stage_uses_cv_runtime(stage))
        })
    }

    fn uses_http_runtime(&self) -> bool {
        self.module.declarations.iter().any(|decl| {
            let (Decl::Node(callable) | Decl::Program(callable)) = decl else {
                return false;
            };
            callable
                .chains
                .iter()
                .flat_map(|chain| chain.stages.iter())
                .any(|stage| self.stage_uses_http_runtime(stage))
        })
    }

    fn stage_uses_cv_runtime(&self, stage: &Stage) -> bool {
        match stage {
            Stage::Endpoint(Endpoint::Name(name))
            | Stage::Map(name)
            | Stage::Filter(name)
            | Stage::Repeat { node: name, .. }
            | Stage::FaultMap { node: name, .. } => self.is_cv_runtime_name(name),
            Stage::Reduce { op, .. } | Stage::Scan { op, .. } => self.is_cv_runtime_name(op),
            Stage::Match { arms } => arms.iter().any(|arm| {
                matches!(&arm.target, MatchTarget::Node(node) if self.is_cv_runtime_name(node))
                    || matches!(
                        &arm.guard,
                        MatchGuard::Call { node, .. } if self.is_cv_runtime_name(node)
                    )
            }),
            Stage::Bind(_) => false,
            Stage::Endpoint(_) => false,
        }
    }

    fn is_cv_runtime_name(&self, name: &str) -> bool {
        matches!(
            self.canonical_name(name).as_str(),
            "decode"
                | "decode_bmp"
                | "decode_jpeg"
                | "decode_png"
                | "decode_pnm"
                | "encode_bmp"
                | "encode_jpeg"
                | "encode_pgm"
                | "encode_png"
                | "encode_ppm"
        )
    }

    fn stage_uses_http_runtime(&self, stage: &Stage) -> bool {
        match stage {
            Stage::Endpoint(Endpoint::Name(name))
            | Stage::Map(name)
            | Stage::Filter(name)
            | Stage::Repeat { node: name, .. }
            | Stage::FaultMap { node: name, .. } => self.is_http_runtime_name(name),
            Stage::Reduce { op, .. } | Stage::Scan { op, .. } => self.is_http_runtime_name(op),
            Stage::Match { arms } => arms.iter().any(|arm| {
                matches!(&arm.target, MatchTarget::Node(node) if self.is_http_runtime_name(node))
                    || matches!(
                        &arm.guard,
                        MatchGuard::Call { node, .. } if self.is_http_runtime_name(node)
                    )
            }),
            Stage::Bind(_) => false,
            Stage::Endpoint(_) => false,
        }
    }

    fn is_http_runtime_name(&self, name: &str) -> bool {
        matches!(
            self.canonical_name(name).as_str(),
            "default_config"
                | "with_tcp_listener"
                | "with_tls"
                | "with_http2"
                | "with_http3"
                | "listen"
                | "requests"
                | "serve"
                | "route"
                | "body"
                | "response"
                | "with_status"
                | "with_header"
                | "text"
                | "json"
                | "not_found"
        )
    }

    fn uses_sqlite_runtime(&self) -> bool {
        self.module.declarations.iter().any(|decl| {
            let (Decl::Node(callable) | Decl::Program(callable)) = decl else {
                return false;
            };
            callable
                .chains
                .iter()
                .flat_map(|chain| chain.stages.iter())
                .any(|stage| self.stage_uses_sqlite_runtime(stage))
        })
    }

    fn stage_uses_sqlite_runtime(&self, stage: &Stage) -> bool {
        match stage {
            Stage::Endpoint(Endpoint::Name(name))
            | Stage::Map(name)
            | Stage::Filter(name)
            | Stage::Repeat { node: name, .. }
            | Stage::FaultMap { node: name, .. } => self.is_sqlite_runtime_name(name),
            Stage::Reduce { op, .. } | Stage::Scan { op, .. } => self.is_sqlite_runtime_name(op),
            Stage::Match { arms } => arms.iter().any(|arm| {
                matches!(&arm.target, MatchTarget::Node(node) if self.is_sqlite_runtime_name(node))
                    || matches!(
                        &arm.guard,
                        MatchGuard::Call { node, .. } if self.is_sqlite_runtime_name(node)
                    )
            }),
            Stage::Bind(_) => false,
            Stage::Endpoint(_) => false,
        }
    }

    fn is_sqlite_runtime_name(&self, name: &str) -> bool {
        matches!(
            self.canonical_name(name).as_str(),
            "sqlite.open"
                | "sqlite.open_readonly"
                | "sqlite.open_memory"
                | "sqlite.close"
                | "sqlite.busy_timeout"
                | "sqlite.foreign_keys"
                | "sqlite.begin"
                | "sqlite.begin_immediate"
                | "sqlite.commit"
                | "sqlite.rollback"
                | "sqlite.null"
                | "sqlite.int"
                | "sqlite.real"
                | "sqlite.text"
                | "sqlite.blob"
                | "sqlite.exec"
                | "sqlite.query"
                | "sqlite.query_all"
                | "sqlite.column_count"
                | "sqlite.column_name"
                | "sqlite.value_at"
                | "sqlite.value_named"
                | "sqlite.kind"
                | "sqlite.is_null"
                | "sqlite.as_int"
                | "sqlite.as_real"
                | "sqlite.as_text"
                | "sqlite.as_blob"
        )
    }

    fn collect_imports(&mut self) {
        for decl in &self.module.declarations {
            let Decl::Import(import) = decl else {
                continue;
            };
            let ImportSource::Module(module) = &import.source else {
                continue;
            };
            match &import.clause {
                ImportClause::Alias(alias) => {
                    for symbol in stdlib::module_symbols(module) {
                        if symbol.kind == stdlib::SymbolKind::Type {
                            self.aliases.insert(
                                format!("{alias}.{}", symbol.name),
                                self.stdlib_codegen_type(symbol.name),
                            );
                        }
                        if symbol.kind == stdlib::SymbolKind::Node
                            && symbol.runtime != RuntimeSupport::Unsupported
                        {
                            let runtime_name = if symbol.module == "std.sqlite" {
                                format!("sqlite.{}", symbol.name)
                            } else {
                                symbol.name.to_string()
                            };
                            self.stdlib_names
                                .insert(format!("{alias}.{}", symbol.name), runtime_name);
                        }
                    }
                }
                ImportClause::Items(items) => {
                    for item in items {
                        if let Some(symbol) = stdlib::find_export(module, &item.name) {
                            if symbol.kind == stdlib::SymbolKind::Type {
                                self.aliases.insert(
                                    item.alias.as_deref().unwrap_or(&item.name).to_string(),
                                    self.stdlib_codegen_type(symbol.name),
                                );
                            }
                            if symbol.kind == stdlib::SymbolKind::Node
                                && symbol.runtime != RuntimeSupport::Unsupported
                            {
                                let runtime_name = if symbol.module == "std.sqlite" {
                                    format!("sqlite.{}", symbol.name)
                                } else {
                                    symbol.name.to_string()
                                };
                                self.stdlib_names.insert(
                                    item.alias.as_deref().unwrap_or(&item.name).to_string(),
                                    runtime_name,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn stdlib_codegen_type(&self, name: &str) -> Ty {
        match name {
            "Stream" => Ty::Stream(Box::new(Ty::Var("V".to_string()))),
            "Args" => Ty::Args,
            "Fault" => Ty::Fault,
            "ServerConfig" => Ty::HttpServerConfig,
            "Listener" => Ty::HttpListener,
            "Request" => Ty::HttpRequest,
            "Response" => Ty::HttpResponse,
            "Connection" => Ty::SqliteConnection,
            "Row" => Ty::SqliteRow,
            "Value" => Ty::SqliteValue,
            _ => parse_type(name).unwrap_or_else(|_| Ty::Var(name.to_string())),
        }
    }

    fn collect_type_aliases(&mut self) -> Result<(), String> {
        let raw = self
            .module
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                Decl::TypeAlias(alias) => Some((alias.name.clone(), alias.ty.clone())),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let mut resolved = HashMap::new();
        for name in raw.keys() {
            let mut resolving = Vec::new();
            let ty = self.resolve_alias(name, &raw, &mut resolved, &mut resolving)?;
            self.aliases.insert(name.clone(), ty);
        }
        Ok(())
    }

    fn resolve_alias(
        &self,
        name: &str,
        raw: &HashMap<String, String>,
        resolved: &mut HashMap<String, Ty>,
        resolving: &mut Vec<String>,
    ) -> Result<Ty, String> {
        if let Some(ty) = resolved.get(name) {
            return Ok(ty.clone());
        }
        if resolving.iter().any(|item| item == name) {
            resolving.push(name.to_string());
            return Err(format!("cyclic type alias `{}`", resolving.join(" -> ")));
        }
        let text = raw
            .get(name)
            .ok_or_else(|| format!("unknown type alias `{name}`"))?;
        resolving.push(name.to_string());
        let ty = self.resolve_alias_type(parse_type(text)?, raw, resolved, resolving)?;
        resolving.pop();
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    }

    fn resolve_alias_type(
        &self,
        ty: Ty,
        raw: &HashMap<String, String>,
        resolved: &mut HashMap<String, Ty>,
        resolving: &mut Vec<String>,
    ) -> Result<Ty, String> {
        match ty {
            Ty::Var(name) => {
                if let Some(known) = builtin_type_alias(&name) {
                    Ok(known)
                } else if raw.contains_key(&name) {
                    self.resolve_alias(&name, raw, resolved, resolving)
                } else {
                    Err(format!("unknown type `{name}`"))
                }
            }
            Ty::Faultable(item) => Ok(Ty::Faultable(Box::new(
                self.resolve_alias_type(*item, raw, resolved, resolving)?,
            ))),
            Ty::Seq(item) => Ok(Ty::Seq(Box::new(
                self.resolve_alias_type(*item, raw, resolved, resolving)?,
            ))),
            Ty::Stream(item) => Ok(Ty::Stream(Box::new(
                self.resolve_alias_type(*item, raw, resolved, resolving)?,
            ))),
            Ty::OneOf(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_alias_type(item, raw, resolved, resolving)?);
                }
                Ok(Ty::OneOf(out))
            }
            Ty::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_alias_type(item, raw, resolved, resolving)?);
                }
                Ok(Ty::Tuple(out))
            }
            other => Ok(other),
        }
    }

    fn collect_callables(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            let (Decl::Node(callable) | Decl::Program(callable)) = decl else {
                continue;
            };
            if self
                .callables
                .insert(callable.name.clone(), callable)
                .is_some()
            {
                return Err(format!("duplicate declaration `{}`", callable.name));
            }
            self.signatures.insert(
                callable.name.clone(),
                Signature {
                    input: self.port_types(&callable.inputs)?,
                    output: self.port_types(&callable.outputs)?,
                },
            );
        }
        if !self.callables.contains_key("main") {
            return Err("missing `program main`".to_string());
        }
        Ok(())
    }

    fn port_types(&self, ports: &[Port]) -> Result<Ty, String> {
        let mut types = ports
            .iter()
            .map(|port| self.parse_declared_type(&port.ty))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match types.len() {
            0 => Ty::Unit,
            1 => types.remove(0),
            _ => Ty::Tuple(types),
        })
    }

    fn parse_declared_type(&self, text: &str) -> Result<Ty, String> {
        self.resolve_declared_type(parse_type(text)?)
    }

    fn parse_signature_type(&self, text: &str) -> Result<Ty, String> {
        self.resolve_signature_type(parse_type(text)?)
    }

    fn resolve_declared_type(&self, ty: Ty) -> Result<Ty, String> {
        match ty {
            Ty::Var(name) => self
                .aliases
                .get(&name)
                .cloned()
                .or_else(|| builtin_type_alias(&name))
                .ok_or_else(|| format!("unknown type `{name}`")),
            Ty::Faultable(item) => Ok(Ty::Faultable(Box::new(self.resolve_declared_type(*item)?))),
            Ty::Seq(item) => Ok(Ty::Seq(Box::new(self.resolve_declared_type(*item)?))),
            Ty::Stream(item) => Ok(Ty::Stream(Box::new(self.resolve_declared_type(*item)?))),
            Ty::OneOf(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_declared_type(item)?);
                }
                Ok(Ty::OneOf(out))
            }
            Ty::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_declared_type(item)?);
                }
                Ok(Ty::Tuple(out))
            }
            other => Ok(other),
        }
    }

    fn resolve_signature_type(&self, ty: Ty) -> Result<Ty, String> {
        match ty {
            Ty::Var(name) => Ok(self
                .aliases
                .get(&name)
                .cloned()
                .or_else(|| builtin_type_alias(&name))
                .unwrap_or(Ty::Var(name))),
            Ty::Faultable(item) => Ok(Ty::Faultable(Box::new(self.resolve_signature_type(*item)?))),
            Ty::Seq(item) => Ok(Ty::Seq(Box::new(self.resolve_signature_type(*item)?))),
            Ty::Stream(item) => Ok(Ty::Stream(Box::new(self.resolve_signature_type(*item)?))),
            Ty::OneOf(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_signature_type(item)?);
                }
                Ok(Ty::OneOf(out))
            }
            Ty::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_signature_type(item)?);
                }
                Ok(Ty::Tuple(out))
            }
            other => Ok(other),
        }
    }

    fn emit_callable(
        &mut self,
        out: &mut String,
        callable: &Callable,
        is_program: bool,
    ) -> Result<(), String> {
        self.temp = 0;
        let symbol = if is_program {
            "flow_program_main".to_string()
        } else {
            user_fn_name(&callable.name)
        };
        let sig = self
            .signatures
            .get(&callable.name)
            .cloned()
            .ok_or_else(|| format!("missing signature for `{}`", callable.name))?;
        let input_ty = self.types.c_type(&sig.input);
        let output_ty = self.types.c_type(&sig.output);
        if !is_program && self.emit_accumulator_fusion(out, callable, &symbol, &sig)? {
            return Ok(());
        }
        out.push_str(&format!(
            "static inline {output_ty} {symbol}({input_ty} input) {{\n"
        ));

        let mut env = HashMap::new();
        match callable.inputs.as_slice() {
            [] => {
                out.push_str("  (void)input;\n");
            }
            [port] => {
                let ty = self.parse_declared_type(&port.ty)?;
                let c_ty = self.types.c_type(&ty);
                let var = c_ident(&port.name);
                out.push_str(&format!("  {c_ty} {var} = input;\n"));
                env.insert(port.name.clone(), Value { code: var, ty });
            }
            ports => {
                for (index, port) in ports.iter().enumerate() {
                    let ty = self.parse_declared_type(&port.ty)?;
                    let c_ty = self.types.c_type(&ty);
                    let var = c_ident(&port.name);
                    out.push_str(&format!("  {c_ty} {var} = input.f{index};\n"));
                    env.insert(port.name.clone(), Value { code: var, ty });
                }
            }
        }

        let chains = fuse_single_use_chains(callable);
        for chain in &chains {
            self.emit_chain(out, chain, &mut env)?;
        }

        let result = self.emit_outputs(out, callable, &env)?;
        out.push_str(&format!("  return {};\n", result.code));
        out.push_str("}\n\n");
        Ok(())
    }

    fn emit_outputs(
        &mut self,
        out: &mut String,
        callable: &Callable,
        env: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        match callable.outputs.as_slice() {
            [] => Err(format!("`{}` must declare an output", callable.name)),
            [output] => {
                let value = env
                    .get(&output.name)
                    .cloned()
                    .ok_or_else(|| format!("output `{}` is never bound", output.name))?;
                let expected = self.parse_declared_type(&output.ty)?;
                self.emit_coerced_value(out, value, &expected)
            }
            outputs => {
                let mut values = Vec::new();
                let mut types = Vec::new();
                for output in outputs {
                    let expected = self.parse_declared_type(&output.ty)?;
                    values.push(
                        env.get(&output.name)
                            .cloned()
                            .ok_or_else(|| format!("output `{}` is never bound", output.name))?,
                    );
                    types.push(expected);
                }
                let ty = Ty::Tuple(types.clone());
                let c_ty = self.types.c_type(&ty);
                let tmp = self.next_temp();
                out.push_str(&format!("  {c_ty} {tmp};\n"));
                for (index, (value, item_ty)) in values.iter().zip(types.iter()).enumerate() {
                    self.emit_assign_value(out, &format!("{tmp}.f{index}"), item_ty, value)?;
                }
                Ok(Value { code: tmp, ty })
            }
        }
    }

    fn emit_coerced_value(
        &mut self,
        out: &mut String,
        value: Value,
        expected: &Ty,
    ) -> Result<Value, String> {
        if &value.ty == expected {
            return Ok(value);
        }
        if !assignable_output_ty(expected, &value.ty) {
            return Err(format!("expected `{expected}`, found `{}`", value.ty));
        }
        let c_ty = self.types.c_type(expected);
        let tmp = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp};\n"));
        self.emit_assign_value(out, &tmp, expected, &value)?;
        Ok(Value {
            code: tmp,
            ty: expected.clone(),
        })
    }

    fn emit_accumulator_fusion(
        &mut self,
        out: &mut String,
        callable: &Callable,
        symbol: &str,
        sig: &Signature,
    ) -> Result<bool, String> {
        let [left_port, right_port, score_port] = callable.inputs.as_slice() else {
            return Ok(false);
        };
        let [out_left, out_right, out_score] = callable.outputs.as_slice() else {
            return Ok(false);
        };
        if self.parse_declared_type(&left_port.ty)? != Ty::Seq(Box::new(Ty::Real))
            || self.parse_declared_type(&right_port.ty)? != Ty::Seq(Box::new(Ty::Real))
            || self.parse_declared_type(&score_port.ty)? != Ty::Real
            || self.parse_declared_type(&out_left.ty)? != Ty::Seq(Box::new(Ty::Real))
            || self.parse_declared_type(&out_right.ty)? != Ty::Seq(Box::new(Ty::Real))
            || self.parse_declared_type(&out_score.ty)? != Ty::Real
        {
            return Ok(false);
        }

        let mut reductions: HashMap<String, ReductionTerm> = HashMap::new();
        let mut additions: HashMap<String, (String, String)> = HashMap::new();
        let mut left_passthrough = false;
        let mut right_passthrough = false;

        for chain in &callable.chains {
            let Some(binding) = final_variable(chain) else {
                return Ok(false);
            };
            let Some(stages) = stages_binding_output(chain, binding) else {
                return Ok(false);
            };
            if stages.is_empty() {
                match (&chain.source, binding) {
                    (Endpoint::Variable(name), out)
                        if name == &left_port.name && out == out_left.name =>
                    {
                        left_passthrough = true;
                        continue;
                    }
                    (Endpoint::Variable(name), out)
                        if name == &right_port.name && out == out_right.name =>
                    {
                        right_passthrough = true;
                        continue;
                    }
                    _ => return Ok(false),
                }
            }
            if let [Stage::Endpoint(Endpoint::Name(name))] = stages {
                if matches_pair_source(&chain.source, &left_port.name, &right_port.name) {
                    match self.fusion_for_name(name) {
                        Some(Fusion::ZipMapReduceAdd(BinaryOp::Mul)) => {
                            reductions.insert(binding.to_string(), ReductionTerm::PairMul);
                            continue;
                        }
                        Some(Fusion::ZipDifferenceSquareSum) => {
                            reductions.insert(binding.to_string(), ReductionTerm::PairDiffSquare);
                            continue;
                        }
                        _ => {}
                    }
                }
                if matches!(&chain.source, Endpoint::Variable(name) if name == &left_port.name)
                    && self.fusion_for_name(name) == Some(Fusion::MapReduceAdd(MapOp::Square))
                {
                    reductions.insert(binding.to_string(), ReductionTerm::LeftSquare);
                    continue;
                }
            }
            if let [Stage::Endpoint(Endpoint::Name(name))] = stages
                && self.is_add(name)
            {
                let Endpoint::Tuple(items) = &chain.source else {
                    return Ok(false);
                };
                let [Endpoint::Variable(left), Endpoint::Variable(right)] = items.as_slice() else {
                    return Ok(false);
                };
                additions.insert(binding.to_string(), (left.clone(), right.clone()));
                continue;
            }
            return Ok(false);
        }

        if !left_passthrough || !right_passthrough || reductions.is_empty() {
            return Ok(false);
        }
        let flattened = flatten_add_terms(&out_score.name, &additions);
        let mut expected = reductions.keys().cloned().collect::<Vec<_>>();
        expected.push(score_port.name.clone());
        expected.sort();
        let mut actual = flattened;
        actual.sort();
        if actual != expected {
            return Ok(false);
        }

        let input_ty = self.types.c_type(&sig.input);
        let output_ty = self.types.c_type(&sig.output);
        out.push_str(&format!(
            "static inline {output_ty} {symbol}({input_ty} input) {{\n"
        ));
        out.push_str("  FaSeq_Real v_left = input.f0;\n");
        out.push_str("  FaSeq_Real v_right = input.f1;\n");
        out.push_str("  double v_score = input.f2;\n");
        out.push_str("  if (v_left.count != v_right.count) fa_die_usage(\"zip: sequences must have the same length\");\n");
        let mut names = reductions.iter().collect::<Vec<_>>();
        names.sort_by(|a, b| a.0.cmp(b.0));
        for (name, _) in &names {
            out.push_str(&format!("  double {} = 0.0;\n", c_ident(name)));
        }
        out.push_str("  for (size_t i = 0; i < v_left.count; i++) {\n");
        out.push_str("    double left = v_left.items[i];\n");
        out.push_str("    double right = v_right.items[i];\n");
        for (name, term) in &names {
            let var = c_ident(name);
            match term {
                ReductionTerm::PairMul => out.push_str(&format!("    {var} += left * right;\n")),
                ReductionTerm::PairDiffSquare => {
                    out.push_str("    double delta = left - right;\n");
                    out.push_str(&format!("    {var} += delta * delta;\n"));
                }
                ReductionTerm::LeftSquare => out.push_str(&format!("    {var} += left * left;\n")),
            }
        }
        out.push_str("  }\n");
        out.push_str(&format!("  {output_ty} out;\n"));
        out.push_str("  out.f0 = v_left;\n");
        out.push_str("  out.f1 = v_right;\n");
        out.push_str("  out.f2 = v_score");
        for name in reductions.keys() {
            out.push_str(&format!(" + {}", c_ident(name)));
        }
        out.push_str(";\n  return out;\n}\n\n");
        Ok(true)
    }

    fn emit_chain(
        &mut self,
        out: &mut String,
        chain: &Chain,
        env: &mut HashMap<String, Value>,
    ) -> Result<(), String> {
        let mut value = if endpoint_contains_empty_seq(&chain.source) {
            if let Some(Stage::Endpoint(Endpoint::Name(name))) = chain.stages.first() {
                let expected = self.call_input_type_for_endpoint(name, &chain.source, env)?;
                self.emit_endpoint_expected(out, &chain.source, env, Some(&expected))?
            } else {
                self.emit_endpoint_expected(out, &chain.source, env, None)?
            }
        } else {
            self.emit_endpoint(out, &chain.source, env)?
        };
        let mut index = 0;
        while index < chain.stages.len() {
            let stage = &chain.stages[index];
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Bind(target) if is_last => {
                    self.emit_bind_target(out, target, value.clone(), env)?;
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    if let Some(Stage::Endpoint(Endpoint::Name(next))) = chain.stages.get(index + 1)
                    {
                        if self.is_matmul_name(name)
                            && self.fusion_for_name(next) == Some(Fusion::NestedSum)
                        {
                            let tmp = self.next_temp();
                            out.push_str(&format!("  double {tmp};\n"));
                            self.emit_fused_matmul_sum(out, &tmp, &value.code, &value.ty)?;
                            value = Value {
                                code: tmp,
                                ty: Ty::Real,
                            };
                            index += 2;
                            continue;
                        }
                        if self.is_matvec_name(name)
                            && self.fusion_for_name(next) == Some(Fusion::Sum)
                        {
                            let tmp = self.next_temp();
                            out.push_str(&format!("  double {tmp};\n"));
                            self.emit_fused_matvec_sum(out, &tmp, &value.code, &value.ty)?;
                            value = Value {
                                code: tmp,
                                ty: Ty::Real,
                            };
                            index += 2;
                            continue;
                        }
                        if self.is_map_sum_callable(name)
                            && self.fusion_for_name(next) == Some(Fusion::Sum)
                        {
                            let tmp = self.next_temp();
                            out.push_str(&format!("  double {tmp};\n"));
                            self.emit_fused_nested_sum(out, &tmp, &value.code, &value.ty)?;
                            value = Value {
                                code: tmp,
                                ty: Ty::Real,
                            };
                            index += 2;
                            continue;
                        }
                    }
                    if let Some(Stage::Map(map_name)) = chain.stages.get(index + 1)
                        && !contains_faultable_ty(&value.ty)
                    {
                        match self.canonical_name(name).as_str() {
                            "broadcast_left" => {
                                value = self.emit_broadcast_map(
                                    out,
                                    map_name,
                                    value.clone(),
                                    BroadcastSide::Left,
                                )?;
                                index += 2;
                                continue;
                            }
                            "broadcast_right" => {
                                value = self.emit_broadcast_map(
                                    out,
                                    map_name,
                                    value.clone(),
                                    BroadcastSide::Right,
                                )?;
                                index += 2;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    value = self.emit_call(out, name, value.clone())?;
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
                }
                Stage::Bind(_) => {
                    return Err("binding targets may only appear as final stages".to_string());
                }
                Stage::Map(name) => {
                    value = self.emit_map(out, name, value.clone())?;
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let (ok_value, fault_value) = self.emit_fault_map(out, node, value.clone())?;
                    if env.insert(ok.clone(), ok_value).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_value).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                Stage::Filter(name) => {
                    value = self.emit_filter(out, name, value.clone())?;
                }
                Stage::Repeat { count, node } => {
                    let count_value = self.emit_endpoint(out, count, env)?;
                    value = self.emit_repeat(out, node, value.clone(), count_value)?;
                }
                Stage::Reduce { op, identity } => {
                    let identity_value = self.emit_endpoint(out, identity, env)?;
                    value = self.emit_reduce(out, op, value.clone(), identity_value)?;
                }
                Stage::Scan { op, identity } => {
                    let identity_value = self.emit_endpoint(out, identity, env)?;
                    value = self.emit_scan(out, op, value.clone(), identity_value)?;
                }
                Stage::Match { arms } => {
                    value = self.emit_match(out, arms, value.clone(), env)?;
                }
            }
            index += 1;
        }
        Ok(())
    }

    fn emit_bind_target(
        &mut self,
        out: &mut String,
        target: &BindingTarget,
        value: Value,
        env: &mut HashMap<String, Value>,
    ) -> Result<(), String> {
        match target {
            BindingTarget::Discard => {}
            BindingTarget::Variable(name) => {
                if env.insert(name.clone(), value).is_some() {
                    return Err(format!("value `{name}` is bound more than once"));
                }
            }
            BindingTarget::Tuple(targets) => match value.ty.clone() {
                Ty::Tuple(items) if items.len() == targets.len() => {
                    for (index, (target, item_ty)) in targets.iter().zip(items.iter()).enumerate() {
                        if binding_target_is_discard(target) {
                            continue;
                        }
                        self.emit_bind_target(
                            out,
                            target,
                            Value {
                                code: format!("{}.f{index}", value.code),
                                ty: item_ty.clone(),
                            },
                            env,
                        )?;
                    }
                }
                Ty::Faultable(inner) => {
                    let Ty::Tuple(items) = inner.as_ref() else {
                        return Err(format!(
                            "binding target `{}` expected tuple input, found `{}`",
                            format_binding_target_for_error(target),
                            value.ty
                        ));
                    };
                    if items.len() != targets.len() {
                        return Err(format!(
                            "binding target `{}` expected {} tuple fields, found {}",
                            format_binding_target_for_error(target),
                            targets.len(),
                            items.len()
                        ));
                    }
                    for (index, (target, item_ty)) in targets.iter().zip(items.iter()).enumerate() {
                        if binding_target_is_discard(target) {
                            continue;
                        }
                        let projected_ty = faultable_projection_ty(item_ty);
                        let projected_c_ty = self.types.c_type(&projected_ty);
                        let tmp = self.next_temp();
                        out.push_str(&format!("  {projected_c_ty} {tmp};\n"));
                        if matches!(item_ty, Ty::Faultable(_)) {
                            out.push_str(&format!(
                                "  if ({}.is_fault) {{ {tmp}.is_fault = true; {tmp}.fault = {}.fault; }} else {{ {tmp} = {}.value.f{index}; }}\n",
                                value.code, value.code, value.code
                            ));
                        } else {
                            out.push_str(&format!(
                                "  if ({}.is_fault) {{ {tmp}.is_fault = true; {tmp}.fault = {}.fault; }} else {{ {tmp}.is_fault = false; {tmp}.value = {}.value.f{index}; }}\n",
                                value.code, value.code, value.code
                            ));
                        }
                        self.emit_bind_target(
                            out,
                            target,
                            Value {
                                code: tmp,
                                ty: projected_ty,
                            },
                            env,
                        )?;
                    }
                }
                Ty::Tuple(items) => {
                    return Err(format!(
                        "binding target `{}` expected {} tuple fields, found {}",
                        format_binding_target_for_error(target),
                        targets.len(),
                        items.len()
                    ));
                }
                other => {
                    return Err(format!(
                        "binding target `{}` expected tuple input, found `{other}`",
                        format_binding_target_for_error(target)
                    ));
                }
            },
        }
        Ok(())
    }

    fn emit_endpoint(
        &mut self,
        out: &mut String,
        endpoint: &Endpoint,
        env: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        self.emit_endpoint_expected(out, endpoint, env, None)
    }

    fn emit_endpoint_expected(
        &mut self,
        out: &mut String,
        endpoint: &Endpoint,
        env: &HashMap<String, Value>,
        expected: Option<&Ty>,
    ) -> Result<Value, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Int(value) => Ok(Value {
                code: value.to_string(),
                ty: Ty::Int,
            }),
            Endpoint::Real(value) => Ok(Value {
                code: format!("{value:.17e}"),
                ty: Ty::Real,
            }),
            Endpoint::Bool(value) => Ok(Value {
                code: if *value { "true" } else { "false" }.to_string(),
                ty: Ty::Bool,
            }),
            Endpoint::String(value) => Ok(Value {
                code: format!("fa_bytes_literal(\"{}\", {})", c_string(value), value.len()),
                ty: Ty::Bytes,
            }),
            Endpoint::Unit => Ok(Value {
                code: "fa_unit()".to_string(),
                ty: Ty::Unit,
            }),
            Endpoint::Tuple(items) => {
                let expected_items = match expected {
                    Some(Ty::Tuple(expected_items)) if expected_items.len() == items.len() => {
                        Some(expected_items.as_slice())
                    }
                    _ => None,
                };
                let values = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        self.emit_endpoint_expected(
                            out,
                            item,
                            env,
                            expected_items.and_then(|items| items.get(index)),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let ty = Ty::Tuple(values.iter().map(|value| value.ty.clone()).collect());
                let c_ty = self.types.c_type(&ty);
                let tmp = self.next_temp();
                out.push_str(&format!("  {c_ty} {tmp};\n"));
                for (index, value) in values.iter().enumerate() {
                    out.push_str(&format!("  {tmp}.f{index} = {};\n", value.code));
                }
                Ok(Value { code: tmp, ty })
            }
            Endpoint::Seq(items) => {
                if items.is_empty() {
                    let Some(seq_ty @ Ty::Seq(_)) = expected else {
                        return Err("empty sequence literals need a type context".to_string());
                    };
                    if contains_type_var(seq_ty) {
                        return Err(
                            "empty sequence literals need a concrete type context".to_string()
                        );
                    }
                    let c_ty = self.types.c_type(seq_ty);
                    let new_fn = self.types.seq_new_name(seq_ty)?;
                    let tmp = self.next_temp();
                    out.push_str(&format!("  {c_ty} {tmp} = {new_fn}(0);\n"));
                    return Ok(Value {
                        code: tmp,
                        ty: seq_ty.clone(),
                    });
                }
                let inferred_item;
                let expected_item = match expected {
                    Some(Ty::Seq(item)) => Some(item.as_ref()),
                    _ if items.iter().any(endpoint_contains_empty_seq) => {
                        match self.endpoint_value_type(endpoint, env)? {
                            Ty::Seq(item)
                                if !contains_empty_seq(&item) && !contains_type_var(&item) =>
                            {
                                inferred_item = *item;
                                Some(&inferred_item)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let values = items
                    .iter()
                    .map(|item| self.emit_endpoint_expected(out, item, env, expected_item))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut item_ty = values[0].ty.clone();
                for value in values.iter().skip(1) {
                    item_ty = sequence_item_type(&item_ty, &value.ty)?;
                }
                if let Some(Ty::Seq(expected_item)) = expected {
                    item_ty = expected_item.as_ref().clone();
                }
                let seq_ty = Ty::Seq(Box::new(item_ty.clone()));
                let c_ty = self.types.c_type(&seq_ty);
                let new_fn = self.types.seq_new_name(&seq_ty)?;
                let tmp = self.next_temp();
                out.push_str(&format!("  {c_ty} {tmp} = {new_fn}({});\n", values.len()));
                for (index, value) in values.iter().enumerate() {
                    self.emit_assign_value(out, &format!("{tmp}.items[{index}]"), &item_ty, value)?;
                }
                Ok(Value {
                    code: tmp,
                    ty: seq_ty,
                })
            }
            Endpoint::Eval { source, stages } => {
                let mut value = if endpoint_contains_empty_seq(source) {
                    if let Some(Stage::Endpoint(Endpoint::Name(name))) = stages.first() {
                        let expected = self.call_input_type_for_endpoint(name, source, env)?;
                        self.emit_endpoint_expected(out, source, env, Some(&expected))?
                    } else {
                        self.emit_endpoint_expected(out, source, env, None)?
                    }
                } else {
                    self.emit_endpoint(out, source, env)?
                };
                for stage in stages {
                    match stage {
                        Stage::Endpoint(Endpoint::Name(name)) => {
                            if contains_empty_seq(&value.ty) {
                                let expected = self.call_input_type_for_value(name, &value.ty)?;
                                value =
                                    self.emit_endpoint_expected(out, source, env, Some(&expected))?;
                            }
                            value = self.emit_call(out, name, value)?;
                        }
                        Stage::Endpoint(Endpoint::Variable(_)) => {
                            return Err("inline evaluations cannot bind values".to_string());
                        }
                        Stage::Bind(_) => {
                            return Err("inline evaluations cannot bind values".to_string());
                        }
                        Stage::Endpoint(_) => {
                            return Err(
                                "non-name endpoints may only appear as inline evaluation sources"
                                    .to_string(),
                            );
                        }
                        Stage::Map(name) => {
                            value = self.emit_map(out, name, value)?;
                        }
                        Stage::FaultMap { .. } => {
                            return Err("inline evaluations cannot use `fault map`".to_string());
                        }
                        Stage::Filter(name) => {
                            value = self.emit_filter(out, name, value)?;
                        }
                        Stage::Repeat { count, node } => {
                            let count_value = self.emit_endpoint(out, count, env)?;
                            value = self.emit_repeat(out, node, value, count_value)?;
                        }
                        Stage::Reduce { op, identity } => {
                            let identity_value = self.emit_endpoint(out, identity, env)?;
                            value = self.emit_reduce(out, op, value, identity_value)?;
                        }
                        Stage::Scan { op, identity } => {
                            let identity_value = self.emit_endpoint(out, identity, env)?;
                            value = self.emit_scan(out, op, value, identity_value)?;
                        }
                        Stage::Match { arms } => {
                            value = self.emit_match(out, arms, value, env)?;
                        }
                    }
                }
                Ok(value)
            }
        }
    }

    fn emit_assign_value(
        &mut self,
        out: &mut String,
        target: &str,
        target_ty: &Ty,
        value: &Value,
    ) -> Result<(), String> {
        match (target_ty, &value.ty) {
            (Ty::Faultable(inner), value_ty) if inner.as_ref() == value_ty => {
                out.push_str(&format!("  {target}.is_fault = false;\n"));
                out.push_str(&format!("  {target}.value = {};\n", value.code));
            }
            (Ty::Faultable(inner), value_ty)
                if unwrap_faultable_tuple(value_ty)
                    .as_ref()
                    .is_some_and(|unwrapped| unwrapped == inner.as_ref()) =>
            {
                let unwrapped_ty = inner.as_ref();
                let unwrapped_c_ty = self.types.c_type(unwrapped_ty);
                let unwrapped = self.next_temp();
                out.push_str(&format!("  {target}.is_fault = false;\n"));
                emit_fault_checks_for_value(out, target, &value.code, value_ty);
                out.push_str(&format!("  if (!{target}.is_fault) {{\n"));
                out.push_str(&format!("    {unwrapped_c_ty} {unwrapped};\n"));
                emit_unwrap_faultable_value(out, &unwrapped, &value.code, value_ty, "    ");
                out.push_str(&format!("    {target}.value = {unwrapped};\n"));
                out.push_str("  }\n");
            }
            _ => out.push_str(&format!("  {target} = {};\n", value.code)),
        }
        Ok(())
    }

    fn emit_call(&mut self, out: &mut String, name: &str, input: Value) -> Result<Value, String> {
        let output_ty = self.call_output_type(name, &input.ty)?;
        let c_ty = self.types.c_type(&output_ty);
        let tmp = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp};\n"));
        self.emit_assign_call(out, &tmp, &output_ty, name, &input.code, &input.ty)?;
        Ok(Value {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_match(
        &mut self,
        out: &mut String,
        arms: &[MatchArm],
        subject: Value,
        env: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        let output_ty = self.match_output_type(arms, &subject.ty, env)?;
        let c_ty = self.types.c_type(&output_ty);
        let target = self.next_temp();
        out.push_str(&format!("  {c_ty} {target};\n"));

        for (index, arm) in arms.iter().enumerate() {
            match &arm.guard {
                MatchGuard::Fallback => {
                    if index + 1 != arms.len() {
                        return Err("`match` fallback arm must be last".to_string());
                    }
                    if index == 0 {
                        out.push_str("  {\n");
                    } else {
                        out.push_str("  else {\n");
                    }
                    self.emit_assign_match_target(
                        out,
                        &target,
                        &output_ty,
                        &arm.target,
                        &subject,
                        env,
                    )?;
                    out.push_str("  }\n");
                }
                MatchGuard::Call { node, args } => {
                    if index == 0 {
                        out.push_str("  {\n");
                    } else {
                        out.push_str("  else {\n");
                    }
                    let guard_input =
                        self.emit_match_guard_input(out, subject.clone(), args, env)?;
                    let guard_output_ty = self.call_output_type(node, &guard_input.ty)?;
                    if guard_output_ty != Ty::Bool {
                        return Err(format!(
                            "match guard `{node}` result expected `Bool`, found `{guard_output_ty}`"
                        ));
                    }
                    let guard = self.next_temp();
                    out.push_str(&format!("  bool {guard};\n"));
                    self.emit_assign_call(
                        out,
                        &guard,
                        &Ty::Bool,
                        node,
                        &guard_input.code,
                        &guard_input.ty,
                    )?;
                    out.push_str(&format!("  if ({guard}) {{\n"));
                    self.emit_assign_match_target(
                        out,
                        &target,
                        &output_ty,
                        &arm.target,
                        &subject,
                        env,
                    )?;
                    out.push_str("  }\n");
                }
            }
        }
        for _ in arms
            .iter()
            .filter(|arm| !matches!(arm.guard, MatchGuard::Fallback))
        {
            out.push_str("  }\n");
        }
        Ok(Value {
            code: target,
            ty: output_ty,
        })
    }

    fn match_target_type(
        &self,
        target: &MatchTarget,
        subject_ty: &Ty,
        env: &HashMap<String, Value>,
    ) -> Result<Ty, String> {
        match target {
            MatchTarget::Node(node) => self.call_output_type(node, subject_ty),
            MatchTarget::Value(endpoint) => self.endpoint_value_type(endpoint, env),
        }
    }

    fn match_output_type(
        &self,
        arms: &[MatchArm],
        subject_ty: &Ty,
        env: &HashMap<String, Value>,
    ) -> Result<Ty, String> {
        let mut output = None;
        for arm in arms {
            let arm_ty = self.match_target_type(&arm.target, subject_ty, env)?;
            output = Some(if let Some(current) = output {
                common_assignable_output_ty(
                    &current,
                    &arm_ty,
                    &format!("match arm `{}` result", format_match_target(&arm.target)),
                )?
            } else {
                arm_ty
            });
        }
        output.ok_or_else(|| "`match` must contain at least one arm".to_string())
    }

    fn endpoint_value_type(
        &self,
        endpoint: &Endpoint,
        env: &HashMap<String, Value>,
    ) -> Result<Ty, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .map(|value| value.ty.clone())
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Int(_) => Ok(Ty::Int),
            Endpoint::Real(_) => Ok(Ty::Real),
            Endpoint::Bool(_) => Ok(Ty::Bool),
            Endpoint::String(_) => Ok(Ty::Bytes),
            Endpoint::Unit => Ok(Ty::Unit),
            Endpoint::Tuple(items) => {
                let mut types = Vec::with_capacity(items.len());
                for item in items {
                    types.push(self.endpoint_value_type(item, env)?);
                }
                Ok(Ty::Tuple(types))
            }
            Endpoint::Seq(items) => {
                let mut item_ty = None;
                for item in items {
                    let ty = self.endpoint_value_type(item, env)?;
                    if let Some(expected) = &item_ty {
                        item_ty = Some(sequence_item_type(expected, &ty)?);
                    } else {
                        item_ty = Some(ty);
                    }
                }
                match item_ty {
                    Some(item_ty) => Ok(Ty::Seq(Box::new(item_ty))),
                    None => Ok(Ty::EmptySeq),
                }
            }
            Endpoint::Eval { source, stages } => self.inline_eval_value_type(source, stages, env),
        }
    }

    fn inline_eval_value_type(
        &self,
        source: &Endpoint,
        stages: &[Stage],
        env: &HashMap<String, Value>,
    ) -> Result<Ty, String> {
        let mut value_ty = self.endpoint_value_type(source, env)?;
        for stage in stages {
            match stage {
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value_ty = self.call_output_type(name, &value_ty)?;
                }
                Stage::Endpoint(Endpoint::Variable(_)) => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                Stage::Bind(_) => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                Stage::Endpoint(_) => {
                    return Err(
                        "non-name endpoints may only appear as inline evaluation sources"
                            .to_string(),
                    );
                }
                Stage::Map(name) => {
                    let output_item_ty = match &value_ty {
                        Ty::Seq(item_ty) | Ty::Stream(item_ty) => {
                            self.call_output_type(name, item_ty)?
                        }
                        _ => return Err(format!("`map {name}` expected Seq or Stream input")),
                    };
                    value_ty = match value_ty {
                        Ty::Seq(_) => Ty::Seq(Box::new(output_item_ty)),
                        Ty::Stream(_) => Ty::Stream(Box::new(output_item_ty)),
                        _ => unreachable!(),
                    };
                }
                Stage::FaultMap { .. } => {
                    return Err("inline evaluations cannot use `fault map`".to_string());
                }
                Stage::Filter(name) => {
                    let Ty::Seq(item_ty) = &value_ty else {
                        return Err(format!("`filter {name}` expected Seq input"));
                    };
                    let predicate_ty = self.call_output_type(name, item_ty)?;
                    if predicate_ty != Ty::Bool {
                        return Err(format!(
                            "`filter {name}` predicate expected `Bool`, found `{predicate_ty}`"
                        ));
                    }
                }
                Stage::Repeat { node, .. } => {
                    value_ty = self.call_output_type(node, &value_ty)?;
                }
                Stage::Reduce { op, identity } => {
                    let Ty::Seq(item_ty) = &value_ty else {
                        return Err(format!("`reduce {op}` expected Seq input"));
                    };
                    let identity_ty = self.endpoint_value_type(identity, env)?;
                    if item_ty.as_ref() != &identity_ty {
                        return Err(format!(
                            "`reduce {op}` identity expected `{item_ty}`, found `{identity_ty}`"
                        ));
                    }
                    value_ty = self.call_output_type(op, item_ty)?;
                }
                Stage::Scan { op, identity } => {
                    let Ty::Seq(item_ty) = &value_ty else {
                        return Err(format!("`scan {op}` expected Seq input"));
                    };
                    let identity_ty = self.endpoint_value_type(identity, env)?;
                    if item_ty.as_ref() != &identity_ty {
                        return Err(format!(
                            "`scan {op}` identity expected `{item_ty}`, found `{identity_ty}`"
                        ));
                    }
                }
                Stage::Match { arms } => {
                    value_ty = self.match_output_type(arms, &value_ty, env)?;
                }
            }
        }
        Ok(value_ty)
    }

    fn emit_assign_match_target(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        arm_target: &MatchTarget,
        subject: &Value,
        env: &HashMap<String, Value>,
    ) -> Result<(), String> {
        match arm_target {
            MatchTarget::Node(node) => {
                self.emit_assign_call(out, target, output_ty, node, &subject.code, &subject.ty)
            }
            MatchTarget::Value(endpoint) => {
                let value = self.emit_endpoint(out, endpoint, env)?;
                self.emit_assign_value(out, target, output_ty, &value)
            }
        }
    }

    fn emit_match_guard_input(
        &mut self,
        out: &mut String,
        subject: Value,
        args: &[Endpoint],
        env: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        if args.is_empty() {
            return Ok(subject);
        }
        let mut values = Vec::with_capacity(args.len() + 1);
        values.push(subject);
        for arg in args {
            values.push(self.emit_endpoint(out, arg, env)?);
        }
        let ty = Ty::Tuple(values.iter().map(|value| value.ty.clone()).collect());
        let c_ty = self.types.c_type(&ty);
        let tmp = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp};\n"));
        for (index, value) in values.iter().enumerate() {
            out.push_str(&format!("  {tmp}.f{index} = {};\n", value.code));
        }
        Ok(Value { code: tmp, ty })
    }

    fn emit_assign_call(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        name: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        if let (Ty::Faultable(input_inner), Ty::Faultable(output_inner)) = (input_ty, output_ty) {
            out.push_str(&format!("  if ({input}.is_fault) {{\n"));
            out.push_str(&format!("    {target}.is_fault = true;\n"));
            out.push_str(&format!("    {target}.fault = {input}.fault;\n"));
            out.push_str("  } else {\n");
            let plain_output = if let Some(signature) = self.signatures.get(name) {
                signature.output.clone()
            } else {
                builtin_output_type_plain(&self.canonical_name(name), input_inner)?
            };
            if matches!(plain_output, Ty::Faultable(_)) {
                self.emit_assign_call_plain(
                    out,
                    target,
                    output_ty,
                    name,
                    &format!("{input}.value"),
                    input_inner,
                )?;
            } else {
                out.push_str(&format!("    {target}.is_fault = false;\n"));
                self.emit_assign_call_plain(
                    out,
                    &format!("{target}.value"),
                    output_inner,
                    name,
                    &format!("{input}.value"),
                    input_inner,
                )?;
            }
            out.push_str("  }\n");
            return Ok(());
        }
        if let (Some(unwrapped_input), Ty::Faultable(output_inner)) =
            (unwrap_faultable_tuple(input_ty), output_ty)
        {
            let unwrapped_c_ty = self.types.c_type(&unwrapped_input);
            let unwrapped = self.next_temp();
            out.push_str(&format!("  {target}.is_fault = false;\n"));
            if matches!(input_ty, Ty::Tuple(_)) {
                emit_fault_checks_for_value(out, target, input, input_ty);
                out.push_str(&format!("  if (!{target}.is_fault) {{\n"));
                out.push_str(&format!("    {unwrapped_c_ty} {unwrapped};\n"));
                emit_unwrap_faultable_value(out, &unwrapped, input, input_ty, "    ");
                let plain_output = if let Some(signature) = self.signatures.get(name) {
                    signature.output.clone()
                } else {
                    builtin_output_type_plain(&self.canonical_name(name), &unwrapped_input)?
                };
                if matches!(plain_output, Ty::Faultable(_)) {
                    self.emit_assign_call_plain(
                        out,
                        target,
                        output_ty,
                        name,
                        &unwrapped,
                        &unwrapped_input,
                    )?;
                } else {
                    self.emit_assign_call_plain(
                        out,
                        &format!("{target}.value"),
                        output_inner,
                        name,
                        &unwrapped,
                        &unwrapped_input,
                    )?;
                }
                out.push_str("  }\n");
                return Ok(());
            }
        }
        if let Ty::Faultable(output_inner) = output_ty {
            let plain_output = if let Some(signature) = self.signatures.get(name) {
                signature.output.clone()
            } else {
                builtin_output_type_plain(&self.canonical_name(name), input_ty)?
            };
            if &plain_output == output_inner.as_ref() {
                let c_ty = self.types.c_type(output_inner);
                let tmp = self.next_temp();
                out.push_str(&format!("  {c_ty} {tmp};\n"));
                self.emit_assign_call_plain(out, &tmp, output_inner, name, input, input_ty)?;
                self.emit_assign_value(
                    out,
                    target,
                    output_ty,
                    &Value {
                        code: tmp,
                        ty: output_inner.as_ref().clone(),
                    },
                )?;
                return Ok(());
            }
        }
        self.emit_assign_call_plain(out, target, output_ty, name, input, input_ty)
    }

    fn emit_assign_call_plain(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        name: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        if self.callables.contains_key(name) {
            if let Some(fusion) = self.fusion_for_name(name) {
                self.emit_fusion_assign(out, target, output_ty, &fusion, input, input_ty)?;
                return Ok(());
            }
            out.push_str(&format!("  {target} = {}({input});\n", user_fn_name(name)));
            return Ok(());
        }
        let canonical = self.canonical_name(name);
        self.emit_builtin_assign(out, target, output_ty, &canonical, input, input_ty)
    }

    fn emit_map(&mut self, out: &mut String, name: &str, input: Value) -> Result<Value, String> {
        if let Ty::Faultable(inner) = input.ty.clone() {
            if !matches!(inner.as_ref(), Ty::Seq(_) | Ty::Stream(_)) {
                return Err(format!("`map {name}` expected Seq or Stream input"));
            };
            let inner_value = Value {
                code: format!("{}.value", input.code),
                ty: inner.as_ref().clone(),
            };
            let mapped_item_ty = match inner.as_ref() {
                Ty::Seq(item_ty) | Ty::Stream(item_ty) => self.call_output_type(name, item_ty)?,
                _ => unreachable!(),
            };
            let mapped_ty = match inner.as_ref() {
                Ty::Seq(_) => Ty::Seq(Box::new(mapped_item_ty)),
                Ty::Stream(_) => Ty::Stream(Box::new(mapped_item_ty)),
                _ => unreachable!(),
            };
            let output_ty = Ty::Faultable(Box::new(mapped_ty));
            let c_ty = self.types.c_type(&output_ty);
            let tmp = self.next_temp();
            out.push_str(&format!("  {c_ty} {tmp};\n"));
            out.push_str(&format!("  if ({}.is_fault) {{\n", input.code));
            out.push_str(&format!("    {tmp}.is_fault = true;\n"));
            out.push_str(&format!("    {tmp}.fault = {}.fault;\n", input.code));
            out.push_str("  } else {\n");
            out.push_str(&format!("    {tmp}.is_fault = false;\n"));
            let mapped = self.emit_map(out, name, inner_value)?;
            out.push_str(&format!("    {tmp}.value = {};\n", mapped.code));
            out.push_str("  }\n");
            return Ok(Value {
                code: tmp,
                ty: output_ty,
            });
        }
        if let Ty::Stream(item_ty) = input.ty.clone() {
            let output_item_ty = self.call_output_type(name, &item_ty)?;
            let output_ty = Ty::Stream(Box::new(output_item_ty.clone()));
            let c_ty = self.types.c_type(&output_ty);
            let tmp = self.next_temp();
            out.push_str(&format!("  {c_ty} {tmp} = {input};\n", input = input.code));
            let helper = self.emit_stream_map_helper(name, &item_ty, &output_item_ty)?;
            let ctx_ty = format!("{helper}_Ctx");
            let ctx = self.next_temp();
            out.push_str(&format!("  if ({tmp}.next) {{\n"));
            out.push_str(&format!(
                "    {ctx_ty} *{ctx} = ({ctx_ty} *)calloc(1, sizeof({ctx_ty}));\n"
            ));
            out.push_str(&format!("    if (!{ctx}) fa_die_alloc();\n"));
            out.push_str(&format!("    {ctx}->upstream = {};\n", input.code));
            out.push_str(&format!("    {ctx}->closed = false;\n"));
            out.push_str(&format!("    {tmp}.state = {ctx};\n"));
            out.push_str(&format!("    {tmp}.map_fn = NULL;\n"));
            out.push_str(&format!("    {tmp}.next = {helper}_next;\n"));
            out.push_str(&format!("    {tmp}.close = {helper}_close;\n"));
            out.push_str(&format!(
                "    {tmp}.item_size = sizeof({});\n",
                self.types.c_type(&output_item_ty)
            ));
            out.push_str(&format!("    {tmp}.closed = false;\n"));
            out.push_str("  } else {\n");
            out.push_str(&format!(
                "    {tmp}.map_fn = (void *){};\n",
                user_fn_name(name)
            ));
            out.push_str("  }\n");
            return Ok(Value {
                code: tmp,
                ty: output_ty,
            });
        }
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`map {name}` expected Seq or Stream input"));
        };
        let output_item_ty = self.call_output_type(name, &item_ty)?;
        let output_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let c_ty = self.types.c_type(&output_ty);
        let item_c_ty = self.types.c_type(&output_item_ty);
        let new_fn = self.types.seq_new_name(&output_ty)?;
        let tmp = self.next_temp();
        if self.is_parallel_safe_name(name, &mut HashSet::new()) {
            let worker = self.emit_parallel_map_helper(
                name,
                &input.ty,
                &item_ty,
                &output_ty,
                &output_item_ty,
            )?;
            let ctx_ty = format!("{worker}_Ctx");
            let ctx = self.next_temp();
            out.push_str(&format!(
                "  {c_ty} {tmp} = {new_fn}({}.count);\n",
                input.code
            ));
            out.push_str(&format!("  {ctx_ty} {ctx};\n"));
            out.push_str(&format!("  {ctx}.input = {};\n", input.code));
            out.push_str(&format!("  {ctx}.output = {tmp};\n"));
            out.push_str(&format!(
                "  fa_parallel_for(0, {}.count, FA_PARALLEL_FOR_GRAIN, {worker}, &{ctx});\n",
                input.code
            ));
            return Ok(Value {
                code: tmp,
                ty: output_ty,
            });
        }
        let item_tmp = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!(
            "  {c_ty} {tmp} = {new_fn}({}.count);\n",
            input.code
        ));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
            input.code
        ));
        out.push_str(&format!("    {item_c_ty} {item_tmp};\n"));
        self.emit_assign_call(
            out,
            &item_tmp,
            &output_item_ty,
            name,
            &format!("{}.items[{i}]", input.code),
            &item_ty,
        )?;
        out.push_str(&format!("    {tmp}.items[{i}] = {item_tmp};\n"));
        out.push_str("  }\n");
        Ok(Value {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_stream_map_helper(
        &mut self,
        name: &str,
        input_item_ty: &Ty,
        output_item_ty: &Ty,
    ) -> Result<String, String> {
        let helper = format!("fa_stream_map_helper_{}", self.stream_helper);
        self.stream_helper += 1;
        let input_c_ty = self.types.c_type(input_item_ty);
        let output_c_ty = self.types.c_type(output_item_ty);
        let mut body = String::new();
        body.push_str(&format!(
            "typedef struct {{ FaStream upstream; bool closed; }} {helper}_Ctx;\n"
        ));
        body.push_str(&format!(
            "static int {helper}_next(void *ctx_ptr, void *out_item, FaFault *fault) {{\n"
        ));
        body.push_str(&format!("  {helper}_Ctx *ctx = ({helper}_Ctx *)ctx_ptr;\n"));
        body.push_str("  if (!ctx || ctx->closed || !ctx->upstream.next) return 0;\n");
        body.push_str(&format!("  {input_c_ty} input_item;\n"));
        body.push_str(
            "  int status = ctx->upstream.next(ctx->upstream.state, &input_item, fault);\n",
        );
        body.push_str("  if (status <= 0) return status;\n");
        body.push_str(&format!("  {output_c_ty} mapped_item;\n"));
        self.emit_assign_call(
            &mut body,
            "mapped_item",
            output_item_ty,
            name,
            "input_item",
            input_item_ty,
        )?;
        body.push_str(&format!("  *({output_c_ty} *)out_item = mapped_item;\n"));
        body.push_str("  return 1;\n");
        body.push_str("}\n");
        body.push_str(&format!(
            "static int {helper}_close(void *ctx_ptr, FaFault *fault) {{\n"
        ));
        body.push_str(&format!("  {helper}_Ctx *ctx = ({helper}_Ctx *)ctx_ptr;\n"));
        body.push_str("  if (!ctx || ctx->closed) return 0;\n");
        body.push_str("  ctx->closed = true;\n");
        body.push_str("  return fa_stream_close(&ctx->upstream, fault);\n");
        body.push_str("}\n\n");
        self.parallel_helpers.push_str(&body);
        Ok(helper)
    }

    fn emit_broadcast_map(
        &mut self,
        out: &mut String,
        name: &str,
        input: Value,
        side: BroadcastSide,
    ) -> Result<Value, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("broadcast-map fusion expected tuple input".to_string());
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err("broadcast-map fusion expected pair input".to_string());
        };

        let (seq_field, broadcast_field, item_ty, broadcast_ty) = match (side, left_ty, right_ty) {
            (BroadcastSide::Left, _, Ty::Seq(item_ty)) => {
                ("f1", "f0", item_ty.as_ref().clone(), left_ty.clone())
            }
            (BroadcastSide::Right, Ty::Seq(item_ty), _) => {
                ("f0", "f1", item_ty.as_ref().clone(), right_ty.clone())
            }
            (BroadcastSide::Left, _, _) => {
                return Err("broadcast_left expected (A,Seq[B]) input".to_string());
            }
            (BroadcastSide::Right, _, _) => {
                return Err("broadcast_right expected (Seq[A],B) input".to_string());
            }
        };

        let pair_ty = match side {
            BroadcastSide::Left => Ty::Tuple(vec![broadcast_ty, item_ty.clone()]),
            BroadcastSide::Right => Ty::Tuple(vec![item_ty.clone(), broadcast_ty]),
        };
        let output_item_ty = self.call_output_type(name, &pair_ty)?;
        let output_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let output_c_ty = self.types.c_type(&output_ty);
        let new_fn = self.types.seq_new_name(&output_ty)?;
        let tmp = self.next_temp();

        out.push_str(&format!(
            "  {output_c_ty} {tmp} = {new_fn}({}.{seq_field}.count);\n",
            input.code
        ));
        if self.is_parallel_safe_name(name, &mut HashSet::new()) {
            let worker = self.emit_parallel_broadcast_map_helper(
                name,
                &input.ty,
                &pair_ty,
                &output_ty,
                &output_item_ty,
                side,
            )?;
            let ctx_ty = format!("{worker}_Ctx");
            let ctx = self.next_temp();
            out.push_str(&format!("  {ctx_ty} {ctx};\n"));
            out.push_str(&format!("  {ctx}.input = {};\n", input.code));
            out.push_str(&format!("  {ctx}.output = {tmp};\n"));
            out.push_str(&format!(
                "  fa_parallel_for(0, {input}.{seq_field}.count, FA_PARALLEL_FOR_GRAIN, {worker}, &{ctx});\n",
                input = input.code
            ));
            return Ok(Value {
                code: tmp,
                ty: output_ty,
            });
        }

        let pair_c_ty = self.types.c_type(&pair_ty);
        let item_c_ty = self.types.c_type(&output_item_ty);
        let pair = self.next_temp();
        let item = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {}.{seq_field}.count; {i}++) {{\n",
            input.code
        ));
        out.push_str(&format!("    {pair_c_ty} {pair};\n"));
        match side {
            BroadcastSide::Left => {
                out.push_str(&format!(
                    "    {pair}.f0 = {}.{broadcast_field};\n",
                    input.code
                ));
                out.push_str(&format!(
                    "    {pair}.f1 = {}.{seq_field}.items[{i}];\n",
                    input.code
                ));
            }
            BroadcastSide::Right => {
                out.push_str(&format!(
                    "    {pair}.f0 = {}.{seq_field}.items[{i}];\n",
                    input.code
                ));
                out.push_str(&format!(
                    "    {pair}.f1 = {}.{broadcast_field};\n",
                    input.code
                ));
            }
        }
        out.push_str(&format!("    {item_c_ty} {item};\n"));
        self.emit_assign_call(out, &item, &output_item_ty, name, &pair, &pair_ty)?;
        out.push_str(&format!("    {tmp}.items[{i}] = {item};\n"));
        out.push_str("  }\n");
        Ok(Value {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_parallel_map_helper(
        &mut self,
        name: &str,
        input_ty: &Ty,
        item_ty: &Ty,
        output_ty: &Ty,
        output_item_ty: &Ty,
    ) -> Result<String, String> {
        let id = self.parallel_helper;
        self.parallel_helper += 1;
        let worker = format!("fa_parallel_map_worker_{id}");
        let ctx_ty = format!("{worker}_Ctx");
        let input_c_ty = self.types.c_type(input_ty);
        let output_c_ty = self.types.c_type(output_ty);
        let item_c_ty = self.types.c_type(output_item_ty);

        let mut helper = String::new();
        helper.push_str(&format!(
            "typedef struct {{ {input_c_ty} input; {output_c_ty} output; }} {ctx_ty};\n"
        ));
        helper.push_str(&format!(
            "static void {worker}(void *ctx_ptr, size_t start, size_t end) {{\n"
        ));
        helper.push_str(&format!("  {ctx_ty} *ctx = ({ctx_ty} *)ctx_ptr;\n"));
        helper.push_str("  for (size_t i = start; i < end; i++) {\n");
        helper.push_str(&format!("    {item_c_ty} item;\n"));
        self.emit_assign_call(
            &mut helper,
            "item",
            output_item_ty,
            name,
            "ctx->input.items[i]",
            item_ty,
        )?;
        helper.push_str("    ctx->output.items[i] = item;\n");
        helper.push_str("  }\n");
        helper.push_str("}\n\n");
        self.parallel_helpers.push_str(&helper);
        Ok(worker)
    }

    fn emit_parallel_broadcast_map_helper(
        &mut self,
        name: &str,
        input_ty: &Ty,
        pair_ty: &Ty,
        output_ty: &Ty,
        output_item_ty: &Ty,
        side: BroadcastSide,
    ) -> Result<String, String> {
        let id = self.parallel_helper;
        self.parallel_helper += 1;
        let worker = format!("fa_parallel_map_worker_{id}");
        let ctx_ty = format!("{worker}_Ctx");
        let input_c_ty = self.types.c_type(input_ty);
        let pair_c_ty = self.types.c_type(pair_ty);
        let output_c_ty = self.types.c_type(output_ty);
        let item_c_ty = self.types.c_type(output_item_ty);

        let mut helper = String::new();
        helper.push_str(&format!(
            "typedef struct {{ {input_c_ty} input; {output_c_ty} output; }} {ctx_ty};\n"
        ));
        helper.push_str(&format!(
            "static void {worker}(void *ctx_ptr, size_t start, size_t end) {{\n"
        ));
        helper.push_str(&format!("  {ctx_ty} *ctx = ({ctx_ty} *)ctx_ptr;\n"));
        helper.push_str("  for (size_t i = start; i < end; i++) {\n");
        helper.push_str(&format!("    {pair_c_ty} pair;\n"));
        match side {
            BroadcastSide::Left => {
                helper.push_str("    pair.f0 = ctx->input.f0;\n");
                helper.push_str("    pair.f1 = ctx->input.f1.items[i];\n");
            }
            BroadcastSide::Right => {
                helper.push_str("    pair.f0 = ctx->input.f0.items[i];\n");
                helper.push_str("    pair.f1 = ctx->input.f1;\n");
            }
        }
        helper.push_str(&format!("    {item_c_ty} item;\n"));
        self.emit_assign_call(&mut helper, "item", output_item_ty, name, "pair", pair_ty)?;
        helper.push_str("    ctx->output.items[i] = item;\n");
        helper.push_str("  }\n");
        helper.push_str("}\n\n");
        self.parallel_helpers.push_str(&helper);
        Ok(worker)
    }

    fn emit_filter(&mut self, out: &mut String, name: &str, input: Value) -> Result<Value, String> {
        if let Ty::Faultable(inner) = input.ty.clone() {
            if !matches!(inner.as_ref(), Ty::Seq(_)) {
                return Err(format!("`filter {name}` expected Seq input"));
            }
            let output_ty = input.ty.clone();
            let c_ty = self.types.c_type(&output_ty);
            let tmp = self.next_temp();
            out.push_str(&format!("  {c_ty} {tmp};\n"));
            out.push_str(&format!("  if ({}.is_fault) {{\n", input.code));
            out.push_str(&format!("    {tmp}.is_fault = true;\n"));
            out.push_str(&format!("    {tmp}.fault = {}.fault;\n", input.code));
            out.push_str("  } else {\n");
            out.push_str(&format!("    {tmp}.is_fault = false;\n"));
            let filtered = self.emit_filter(
                out,
                name,
                Value {
                    code: format!("{}.value", input.code),
                    ty: inner.as_ref().clone(),
                },
            )?;
            out.push_str(&format!("    {tmp}.value = {};\n", filtered.code));
            out.push_str("  }\n");
            return Ok(Value {
                code: tmp,
                ty: output_ty,
            });
        }
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`filter {name}` expected Seq input"));
        };
        let c_ty = self.types.c_type(&input.ty);
        let new_fn = self.types.seq_new_name(&input.ty)?;
        let tmp = self.next_temp();
        let keep = self.next_temp();
        let count = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!(
            "  {c_ty} {tmp} = {new_fn}({}.count);\n",
            input.code
        ));
        out.push_str(&format!("  size_t {count} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
            input.code
        ));
        out.push_str(&format!("    bool {keep};\n"));
        self.emit_assign_call(
            out,
            &keep,
            &Ty::Bool,
            name,
            &format!("{}.items[{i}]", input.code),
            &item_ty,
        )?;
        out.push_str(&format!(
            "    if ({keep}) {tmp}.items[{count}++] = {}.items[{i}];\n",
            input.code
        ));
        out.push_str("  }\n");
        out.push_str(&format!("  {tmp}.count = {count};\n"));
        Ok(Value {
            code: tmp,
            ty: input.ty,
        })
    }

    fn emit_fault_map(
        &mut self,
        out: &mut String,
        name: &str,
        input: Value,
    ) -> Result<(Value, Value), String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`fault map {name}` expected Seq input"));
        };
        let output_item_ty = self.call_output_type(name, &item_ty)?;
        let Ty::Faultable(ok_item_ty) = output_item_ty else {
            return Err(format!("`fault map {name}` expected faultable output"));
        };
        let ok_ty = Ty::Seq(ok_item_ty.clone());
        let fault_ty = Ty::Seq(Box::new(Ty::Fault));
        let ok_c_ty = self.types.c_type(&ok_ty);
        let fault_c_ty = self.types.c_type(&fault_ty);
        let result_c_ty = self.types.c_type(&Ty::Faultable(ok_item_ty.clone()));
        let ok_new = self.types.seq_new_name(&ok_ty)?;
        let fault_new = self.types.seq_new_name(&fault_ty)?;
        let ok = self.next_temp();
        let faults = self.next_temp();
        let ok_count = self.next_temp();
        let fault_count = self.next_temp();
        let result = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!(
            "  {ok_c_ty} {ok} = {ok_new}({}.count);\n",
            input.code
        ));
        out.push_str(&format!(
            "  {fault_c_ty} {faults} = {fault_new}({}.count);\n",
            input.code
        ));
        out.push_str(&format!("  size_t {ok_count} = 0;\n"));
        out.push_str(&format!("  size_t {fault_count} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
            input.code
        ));
        out.push_str(&format!("    {result_c_ty} {result};\n"));
        self.emit_assign_call(
            out,
            &result,
            &Ty::Faultable(ok_item_ty.clone()),
            name,
            &format!("{}.items[{i}]", input.code),
            &item_ty,
        )?;
        out.push_str(&format!("    if ({result}.is_fault) {{\n"));
        if matches!(
            self.canonical_name(name).as_str(),
            "parse_real" | "parse_int"
        ) {
            out.push_str(&format!(
                "      {faults}.items[{fault_count}++] = fa_fault_with_line({i} + 1, {result}.fault);\n"
            ));
        } else {
            out.push_str(&format!(
                "      {faults}.items[{fault_count}++] = {result}.fault;\n"
            ));
        }
        out.push_str("    } else {\n");
        out.push_str(&format!(
            "      {ok}.items[{ok_count}++] = {result}.value;\n"
        ));
        out.push_str("    }\n");
        out.push_str("  }\n");
        out.push_str(&format!("  {ok}.count = {ok_count};\n"));
        out.push_str(&format!("  {faults}.count = {fault_count};\n"));
        Ok((
            Value {
                code: ok,
                ty: ok_ty,
            },
            Value {
                code: faults,
                ty: fault_ty,
            },
        ))
    }

    fn emit_reduce(
        &mut self,
        out: &mut String,
        op: &str,
        input: Value,
        identity: Value,
    ) -> Result<Value, String> {
        if let Ty::Faultable(inner) = input.ty.clone() {
            let Ty::Seq(_) = inner.as_ref() else {
                return Err(format!("`reduce {op}` expected Seq input"));
            };
            let reduced = self.emit_reduce(
                out,
                op,
                Value {
                    code: format!("{}.value", input.code),
                    ty: inner.as_ref().clone(),
                },
                identity,
            )?;
            let output_ty = match reduced.ty {
                Ty::Faultable(_) => reduced.ty.clone(),
                ref other => Ty::Faultable(Box::new(other.clone())),
            };
            let c_ty = self.types.c_type(&output_ty);
            let tmp = self.next_temp();
            out.push_str(&format!("  {c_ty} {tmp};\n"));
            out.push_str(&format!("  if ({}.is_fault) {{\n", input.code));
            out.push_str(&format!("    {tmp}.is_fault = true;\n"));
            out.push_str(&format!("    {tmp}.fault = {}.fault;\n", input.code));
            out.push_str("  } else {\n");
            match &reduced.ty {
                Ty::Faultable(_) => out.push_str(&format!("    {tmp} = {};\n", reduced.code)),
                _ => {
                    out.push_str(&format!("    {tmp}.is_fault = false;\n"));
                    out.push_str(&format!("    {tmp}.value = {};\n", reduced.code));
                }
            }
            out.push_str("  }\n");
            return Ok(Value {
                code: tmp,
                ty: output_ty,
            });
        }
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`reduce {op}` expected Seq input"));
        };
        let canonical = self.canonical_name(op);
        if canonical == "add" {
            return self.emit_reduce_add(out, input, *item_ty, identity);
        }
        if canonical == "min" || canonical == "max" {
            return self.emit_reduce_min_max(out, &canonical, input, *item_ty, identity);
        }
        if canonical == "concat_bytes" {
            let tmp = self.next_temp();
            out.push_str(&format!(
                "  FaBytes {tmp} = fa_reduce_concat_bytes({}, {});\n",
                input.code, identity.code
            ));
            return Ok(Value {
                code: tmp,
                ty: Ty::Bytes,
            });
        }
        Err(format!("unsupported reduce op `{op}`"))
    }

    fn emit_reduce_min_max(
        &mut self,
        out: &mut String,
        op: &str,
        input: Value,
        item_ty: Ty,
        identity: Value,
    ) -> Result<Value, String> {
        let (plain_ty, faultable) = match item_ty {
            Ty::Faultable(inner) => (*inner, true),
            other => (other, false),
        };
        let output_ty = if faultable {
            Ty::Faultable(Box::new(plain_ty.clone()))
        } else {
            plain_ty.clone()
        };
        let c_ty = self.types.c_type(&output_ty);
        let tmp = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp};\n"));
        if faultable {
            out.push_str(&format!("  {tmp}.is_fault = false;\n"));
            out.push_str(&format!("  {tmp}.value = {};\n", identity.code));
            out.push_str(&format!(
                "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
                input.code
            ));
            out.push_str(&format!("    if ({}.items[{i}].is_fault) {{ {tmp}.is_fault = true; {tmp}.fault = {}.items[{i}].fault; break; }}\n", input.code, input.code));
            out.push_str(&format!(
                "    {tmp}.value = {};\n",
                min_max_expr(
                    op,
                    &format!("{tmp}.value"),
                    &format!("{}.items[{i}].value", input.code),
                    &plain_ty
                )
            ));
            out.push_str("  }\n");
        } else {
            out.push_str(&format!("  {tmp} = {};\n", identity.code));
            out.push_str(&format!(
                "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
                input.code
            ));
            out.push_str(&format!(
                "    {tmp} = {};\n",
                min_max_expr(op, &tmp, &format!("{}.items[{i}]", input.code), &plain_ty)
            ));
            out.push_str("  }\n");
        }
        Ok(Value {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_reduce_add(
        &mut self,
        out: &mut String,
        input: Value,
        item_ty: Ty,
        identity: Value,
    ) -> Result<Value, String> {
        let (plain_ty, faultable) = match item_ty {
            Ty::Faultable(inner) => (*inner, true),
            other => (other, false),
        };
        let output_ty = if faultable {
            Ty::Faultable(Box::new(plain_ty.clone()))
        } else {
            plain_ty.clone()
        };
        let c_ty = self.types.c_type(&output_ty);
        let tmp = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp};\n"));
        if faultable {
            out.push_str(&format!("  {tmp}.is_fault = false;\n"));
            out.push_str(&format!("  {tmp}.value = {};\n", identity.code));
            out.push_str(&format!(
                "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
                input.code
            ));
            out.push_str(&format!("    if ({}.items[{i}].is_fault) {{ {tmp}.is_fault = true; {tmp}.fault = {}.items[{i}].fault; break; }}\n", input.code, input.code));
            out.push_str(&format!(
                "    {tmp}.value = {};\n",
                add_expr(
                    &format!("{tmp}.value"),
                    &format!("{}.items[{i}].value", input.code),
                    &plain_ty
                )
            ));
            out.push_str("  }\n");
        } else {
            out.push_str(&format!("  {tmp} = {};\n", identity.code));
            out.push_str(&format!(
                "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
                input.code
            ));
            out.push_str(&format!(
                "    {tmp} = {};\n",
                add_expr(&tmp, &format!("{}.items[{i}]", input.code), &plain_ty)
            ));
            out.push_str("  }\n");
        }
        Ok(Value {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_scan(
        &mut self,
        out: &mut String,
        op: &str,
        input: Value,
        identity: Value,
    ) -> Result<Value, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`scan {op}` expected Seq input"));
        };
        let output_ty = Ty::Seq(item_ty.clone());
        let c_ty = self.types.c_type(&output_ty);
        let item_c_ty = self.types.c_type(&item_ty);
        let pair_ty = Ty::Tuple(vec![*item_ty.clone(), *item_ty.clone()]);
        let pair_c_ty = self.types.c_type(&pair_ty);
        let new_fn = self.types.seq_new_name(&output_ty)?;
        let tmp = self.next_temp();
        let state = self.next_temp();
        let pair = self.next_temp();
        let result = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!(
            "  {c_ty} {tmp} = {new_fn}({}.count);\n",
            input.code
        ));
        out.push_str(&format!("  {item_c_ty} {state} = {};\n", identity.code));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {}.count; {i}++) {{\n",
            input.code
        ));
        out.push_str(&format!("    {pair_c_ty} {pair};\n"));
        out.push_str(&format!("    {pair}.f0 = {state};\n"));
        out.push_str(&format!("    {pair}.f1 = {}.items[{i}];\n", input.code));
        out.push_str(&format!("    {item_c_ty} {result};\n"));
        self.emit_assign_call(out, &result, &item_ty, op, &pair, &pair_ty)?;
        out.push_str(&format!("    {state} = {result};\n"));
        out.push_str(&format!("    {tmp}.items[{i}] = {state};\n"));
        out.push_str("  }\n");
        Ok(Value {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_repeat(
        &mut self,
        out: &mut String,
        node: &str,
        input: Value,
        count: Value,
    ) -> Result<Value, String> {
        let c_ty = self.types.c_type(&input.ty);
        let tmp = self.next_temp();
        let next = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp} = {};\n", input.code));
        out.push_str(&format!(
            "  for (int64_t {i} = 0; {i} < {}; {i}++) {{\n",
            count.code
        ));
        out.push_str(&format!("    {c_ty} {next};\n"));
        self.emit_assign_call(out, &next, &input.ty, node, &tmp, &input.ty)?;
        out.push_str(&format!("    {tmp} = {next};\n"));
        out.push_str("  }\n");
        Ok(Value {
            code: tmp,
            ty: input.ty,
        })
    }

    fn emit_builtin_assign(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        name: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        match name {
            "argv" => out.push_str(&format!("  {target} = fa_argv({input});\n")),
            "flag_present" => out.push_str(&format!(
                "  {target} = fa_flag_present({input}.f0, {input}.f1);\n"
            )),
            "flag_value" => out.push_str(&format!(
                "  {target} = fa_flag_value({input}.f0, {input}.f1);\n"
            )),
            "read_stdin" => out.push_str(&format!("  {target} = fa_read_stdin();\n")),
            "write_stdout" => {
                out.push_str(&format!("  {target} = fa_write_bytes(stdout, {input});\n"))
            }
            "write_stderr" => {
                out.push_str(&format!("  {target} = fa_write_bytes(stderr, {input});\n"))
            }
            "read_file" => out.push_str(&format!("  {target} = fa_read_file({input});\n")),
            "write_file" => out.push_str(&format!(
                "  {target} = fa_write_file({input}.f0, {input}.f1);\n"
            )),
            "exists" => out.push_str(&format!("  {target} = fa_path_exists({input});\n")),
            "is_file" => out.push_str(&format!("  {target} = fa_path_is_file({input});\n")),
            "is_dir" => out.push_str(&format!("  {target} = fa_path_is_dir({input});\n")),
            "file_size" => out.push_str(&format!("  {target} = fa_path_file_size({input});\n")),
            "join_path" => out.push_str(&format!(
                "  {target} = fa_join_path({input}.f0, {input}.f1);\n"
            )),
            "basename" => out.push_str(&format!("  {target} = fa_basename({input});\n")),
            "dirname" => out.push_str(&format!("  {target} = fa_dirname({input});\n")),
            "list_dir" => out.push_str(&format!("  {target} = fa_list_dir({input});\n")),
            "walk_files" => out.push_str(&format!("  {target} = fa_walk_files({input});\n")),
            "read_files" => out.push_str(&format!("  {target} = fa_read_files({input});\n")),
            "open_file" => out.push_str(&format!("  {target} = fa_open_file({input});\n")),
            "size" => out.push_str(&format!("  {target} = fa_stream_size({input});\n")),
            "read_at" => out.push_str(&format!(
                "  {target} = fa_stream_read_at({input}.f0, {input}.f1, {input}.f2);\n"
            )),
            "copy_to_file" => out.push_str(&format!(
                "  {target} = fa_copy_stream_to_file({input}.f0, {input}.f1);\n"
            )),
            "close" => out.push_str(&format!("  {target} = fa_close_stream({input});\n")),
            "to_seq" => self.emit_stream_to_seq(out, target, output_ty, input, input_ty)?,
            "drain" => self.emit_stream_drain(out, target, input, input_ty)?,
            "default_config" => out.push_str(&format!("  {target} = fa_http_default_config();\n")),
            "with_tcp_listener" => out.push_str(&format!(
                "  {target} = fa_http_with_tcp_listener({input}.f0, {input}.f1, {input}.f2);\n"
            )),
            "with_tls" => out.push_str(&format!(
                "  {target} = fa_http_with_tls({input}.f0, {input}.f1, {input}.f2);\n"
            )),
            "with_http2" => out.push_str(&format!(
                "  {target} = fa_http_with_http2({input}.f0, {input}.f1);\n"
            )),
            "with_http3" => out.push_str(&format!(
                "  {target} = fa_http_with_http3({input}.f0, {input}.f1);\n"
            )),
            "listen" => out.push_str(&format!("  {target} = fa_http_listen({input});\n")),
            "requests" => out.push_str(&format!("  {target} = fa_http_requests({input});\n")),
            "serve" => out.push_str(&format!("  {target} = fa_http_serve({input});\n")),
            "route" => out.push_str(&format!("  {target} = fa_http_route({input});\n")),
            "body" => out.push_str(&format!("  {target} = fa_http_body({input});\n")),
            "response" => out.push_str(&format!("  {target} = fa_http_response({input});\n")),
            "with_status" => out.push_str(&format!("  {target} = fa_http_with_status({input});\n")),
            "with_header" => out.push_str(&format!("  {target} = fa_http_with_header({input});\n")),
            "text" => out.push_str(&format!("  {target} = fa_http_text({input});\n")),
            "json" => out.push_str(&format!("  {target} = fa_http_json({input});\n")),
            "not_found" => out.push_str(&format!("  {target} = fa_http_not_found({input});\n")),
            "sqlite.open" => out.push_str(&format!("  {target} = fa_sqlite_open({input});\n")),
            "sqlite.open_readonly" => {
                out.push_str(&format!("  {target} = fa_sqlite_open_readonly({input});\n"))
            }
            "sqlite.open_memory" => {
                out.push_str(&format!("  {target} = fa_sqlite_open_memory({input});\n"))
            }
            "sqlite.close" => out.push_str(&format!("  {target} = fa_sqlite_close({input});\n")),
            "sqlite.busy_timeout" => {
                out.push_str(&format!("  {target} = fa_sqlite_busy_timeout({input});\n"))
            }
            "sqlite.foreign_keys" => {
                out.push_str(&format!("  {target} = fa_sqlite_foreign_keys({input});\n"))
            }
            "sqlite.begin" => out.push_str(&format!("  {target} = fa_sqlite_begin({input});\n")),
            "sqlite.begin_immediate" => out.push_str(&format!(
                "  {target} = fa_sqlite_begin_immediate({input});\n"
            )),
            "sqlite.commit" => out.push_str(&format!("  {target} = fa_sqlite_commit({input});\n")),
            "sqlite.rollback" => {
                out.push_str(&format!("  {target} = fa_sqlite_rollback({input});\n"))
            }
            "sqlite.null" => out.push_str(&format!("  {target} = fa_sqlite_null({input});\n")),
            "sqlite.int" => out.push_str(&format!("  {target} = fa_sqlite_int({input});\n")),
            "sqlite.real" => out.push_str(&format!("  {target} = fa_sqlite_real({input});\n")),
            "sqlite.text" => out.push_str(&format!("  {target} = fa_sqlite_text({input});\n")),
            "sqlite.blob" => out.push_str(&format!("  {target} = fa_sqlite_blob({input});\n")),
            "sqlite.exec" => out.push_str(&format!("  {target} = fa_sqlite_exec({input});\n")),
            "sqlite.query" => out.push_str(&format!("  {target} = fa_sqlite_query({input});\n")),
            "sqlite.query_all" => {
                out.push_str(&format!("  {target} = fa_sqlite_query_all({input});\n"))
            }
            "sqlite.column_count" => {
                out.push_str(&format!("  {target} = fa_sqlite_column_count({input});\n"))
            }
            "sqlite.column_name" => {
                out.push_str(&format!("  {target} = fa_sqlite_column_name({input});\n"))
            }
            "sqlite.value_at" => {
                out.push_str(&format!("  {target} = fa_sqlite_value_at({input});\n"))
            }
            "sqlite.value_named" => {
                out.push_str(&format!("  {target} = fa_sqlite_value_named({input});\n"))
            }
            "sqlite.kind" => out.push_str(&format!("  {target} = fa_sqlite_kind({input});\n")),
            "sqlite.is_null" => {
                out.push_str(&format!("  {target} = fa_sqlite_is_null({input});\n"))
            }
            "sqlite.as_int" => out.push_str(&format!("  {target} = fa_sqlite_as_int({input});\n")),
            "sqlite.as_real" => {
                out.push_str(&format!("  {target} = fa_sqlite_as_real({input});\n"))
            }
            "sqlite.as_text" => {
                out.push_str(&format!("  {target} = fa_sqlite_as_text({input});\n"))
            }
            "sqlite.as_blob" => {
                out.push_str(&format!("  {target} = fa_sqlite_as_blob({input});\n"))
            }
            "split_lines" => out.push_str(&format!("  {target} = fa_split_lines({input});\n")),
            "trim" => out.push_str(&format!("  {target} = fa_trim({input});\n")),
            "contains" => out.push_str(&format!(
                "  {target} = fa_bytes_contains({input}.f0, {input}.f1);\n"
            )),
            "starts_with" => out.push_str(&format!(
                "  {target} = fa_bytes_starts_with({input}.f0, {input}.f1);\n"
            )),
            "ends_with" => out.push_str(&format!(
                "  {target} = fa_bytes_ends_with({input}.f0, {input}.f1);\n"
            )),
            "index_of" => out.push_str(&format!(
                "  {target} = fa_index_of({input}.f0, {input}.f1);\n"
            )),
            "last_index_of" => out.push_str(&format!(
                "  {target} = fa_last_index_of({input}.f0, {input}.f1);\n"
            )),
            "slice" if matches!(input_ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int, Ty::Int])) =>
            {
                out.push_str(&format!(
                    "  {target} = fa_bytes_slice({input}.f0, {input}.f1, {input}.f2);\n"
                ));
            }
            "take" if matches!(input_ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) =>
            {
                out.push_str(&format!(
                    "  {target} = fa_bytes_take({input}.f0, {input}.f1);\n"
                ));
            }
            "drop" if matches!(input_ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) =>
            {
                out.push_str(&format!(
                    "  {target} = fa_bytes_drop({input}.f0, {input}.f1);\n"
                ));
            }
            "replace" => out.push_str(&format!(
                "  {target} = fa_bytes_replace({input}.f0, {input}.f1, {input}.f2);\n"
            )),
            "repeat_bytes" => {
                out.push_str(&format!(
                    "  {target} = fa_bytes_repeat({input}.f0, {input}.f1);\n"
                ));
            }
            "ascii_lower" => out.push_str(&format!("  {target} = fa_ascii_lower({input});\n")),
            "ascii_upper" => out.push_str(&format!("  {target} = fa_ascii_upper({input});\n")),
            "split_on" => out.push_str(&format!(
                "  {target} = fa_split_on({input}.f0, {input}.f1);\n"
            )),
            "strip_prefix" => out.push_str(&format!(
                "  {target} = fa_strip_prefix({input}.f0, {input}.f1);\n"
            )),
            "strip_suffix" => out.push_str(&format!(
                "  {target} = fa_strip_suffix({input}.f0, {input}.f1);\n"
            )),
            "bytes_to_codes" => {
                out.push_str(&format!("  {target} = fa_bytes_to_codes({input});\n"))
            }
            "codes_to_bytes" => {
                out.push_str(&format!("  {target} = fa_codes_to_bytes({input});\n"))
            }
            "byte_length" => out.push_str(&format!("  {target} = (int64_t){input}.len;\n")),
            "concat_bytes" if matches!(output_ty, Ty::Faultable(inner) if inner.as_ref() == &Ty::Bytes) =>
            {
                self.emit_faultable_concat_bytes(out, target, input);
            }
            "concat_bytes" => out.push_str(&format!("  {target} = fa_concat_bytes({input});\n")),
            "join_bytes" => out.push_str(&format!(
                "  {target} = fa_join_bytes({input}.f0, {input}.f1);\n"
            )),
            "parse_int" => out.push_str(&format!("  {target} = fa_parse_int({input});\n")),
            "parse_real" => out.push_str(&format!("  {target} = fa_parse_real({input});\n")),
            "from_int" => out.push_str(&format!("  {target} = (double){input};\n")),
            "format_int" => {
                self.emit_format_faultable_or_plain(out, target, input, input_ty, "fa_format_int")?
            }
            "format_real" => {
                self.emit_format_faultable_or_plain(out, target, input, input_ty, "fa_format_real")?
            }
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                out.push_str(&format!(
                    "  {target} = {};\n",
                    numeric_binary_expr(name, input, output_ty)
                ));
            }
            "neg" | "abs" | "sqrt" | "exp" | "sin" | "cos" => {
                out.push_str(&format!(
                    "  {target} = {};\n",
                    numeric_unary_expr(name, input, output_ty)
                ));
            }
            "eq" | "lt" | "gt" | "le" | "ge" => {
                out.push_str(&format!("  {target} = {};\n", compare_expr(name, input)));
            }
            "not_empty" => out.push_str(&format!("  {target} = {input}.len > 0;\n")),
            "is_empty" if matches!(input_ty, Ty::Bytes) => {
                out.push_str(&format!("  {target} = {input}.len == 0;\n"))
            }
            "and" => out.push_str(&format!("  {target} = {input}.f0 && {input}.f1;\n")),
            "or" => out.push_str(&format!("  {target} = {input}.f0 || {input}.f1;\n")),
            "xor" => out.push_str(&format!("  {target} = {input}.f0 != {input}.f1;\n")),
            "not" => out.push_str(&format!("  {target} = !{input};\n")),
            "all" => self.emit_all_any(out, target, input, true),
            "any" => self.emit_all_any(out, target, input, false),
            "has_faults" => out.push_str(&format!("  {target} = {input}.count > 0;\n")),
            "format_faults" => out.push_str(&format!("  {target} = fa_format_faults({input});\n")),
            "expect" => {
                if matches!(input_ty, Ty::Faultable(_)) {
                    out.push_str(&format!(
                        "  if ({input}.is_fault) fa_die_usage({input}.fault.message.bytes); else {target} = {input}.value;\n"
                    ));
                } else {
                    out.push_str(&format!("  {target} = {input};\n"));
                }
            }
            "collect" => self.emit_collect(out, target, output_ty, input, input_ty)?,
            "select" => out.push_str(&format!(
                "  {target} = {input}.f0 ? {input}.f1 : {input}.f2;\n"
            )),
            "length" => out.push_str(&format!("  {target} = (int64_t){input}.count;\n")),
            "is_empty" if matches!(input_ty, Ty::Seq(_)) => {
                out.push_str(&format!("  {target} = {input}.count == 0;\n"))
            }
            "inner_length" => self.emit_inner_length(out, target, input),
            "first" => out.push_str(&format!("  {target} = {input}.f0;\n")),
            "second" => out.push_str(&format!("  {target} = {input}.f1;\n")),
            "swap" => {
                out.push_str(&format!("  {target}.f0 = {input}.f1;\n"));
                out.push_str(&format!("  {target}.f1 = {input}.f0;\n"));
            }
            "zip" => self.emit_zip(out, target, output_ty, input, input_ty)?,
            "broadcast_left" => {
                self.emit_broadcast_left(out, target, output_ty, input, input_ty)?
            }
            "broadcast_right" => {
                self.emit_broadcast_right(out, target, output_ty, input, input_ty)?
            }
            "transpose" => self.emit_transpose(out, target, output_ty, input, input_ty)?,
            "flatten" => self.emit_flatten(out, target, output_ty, input, input_ty)?,
            "group_by_id" => self.emit_group_by_id(out, target, output_ty, input, input_ty)?,
            "shift_right" => self.emit_shift_right(out, target, output_ty, input, input_ty)?,
            "shift_left" => self.emit_shift_left(out, target, output_ty, input, input_ty)?,
            "head" => self.emit_head(out, target, output_ty, input, input_ty)?,
            "tail" => self.emit_tail(out, target, output_ty, input, input_ty)?,
            "reverse" => self.emit_reverse(out, target, output_ty, input, input_ty)?,
            "take" => self.emit_take(out, target, output_ty, input, input_ty)?,
            "drop" => self.emit_drop(out, target, output_ty, input, input_ty)?,
            "fill" => self.emit_fill(out, target, output_ty, input, input_ty)?,
            "slice" => self.emit_slice(out, target, output_ty, input, input_ty)?,
            "last" => self.emit_last(out, target, output_ty, input, input_ty)?,
            "get" => self.emit_get(out, target, output_ty, input, input_ty)?,
            "get_or" => self.emit_get_or(out, target, output_ty, input, input_ty)?,
            "at" => self.emit_at(out, target, output_ty, input, input_ty)?,
            "append" => self.emit_append(out, target, output_ty, input, input_ty)?,
            "set" => self.emit_set(out, target, output_ty, input, input_ty)?,
            "concat" => self.emit_seq_concat(out, target, output_ty, input, input_ty)?,
            "range_step" => out.push_str(&format!(
                "  {target} = fa_range_step({input}.f0, {input}.f1, {input}.f2);\n"
            )),
            "decode" => out.push_str(&format!("  {target} = fa_cv_decode({input});\n")),
            "decode_bmp" => out.push_str(&format!("  {target} = fa_cv_decode_bmp({input});\n")),
            "decode_jpeg" => out.push_str(&format!("  {target} = fa_cv_decode_jpeg({input});\n")),
            "decode_png" => out.push_str(&format!("  {target} = fa_cv_decode_png({input});\n")),
            "decode_pnm" => out.push_str(&format!("  {target} = fa_cv_decode_pnm({input});\n")),
            "encode_bmp" => out.push_str(&format!("  {target} = fa_cv_encode_bmp({input});\n")),
            "encode_jpeg" => out.push_str(&format!("  {target} = fa_cv_encode_jpeg({input});\n")),
            "encode_pgm" => out.push_str(&format!("  {target} = fa_cv_encode_pgm({input});\n")),
            "encode_png" => out.push_str(&format!("  {target} = fa_cv_encode_png({input});\n")),
            "encode_ppm" => out.push_str(&format!("  {target} = fa_cv_encode_ppm({input});\n")),
            "bit_and" => out.push_str(&format!("  {target} = {input}.f0 & {input}.f1;\n")),
            "bit_or" => out.push_str(&format!("  {target} = {input}.f0 | {input}.f1;\n")),
            "bit_xor" => out.push_str(&format!("  {target} = {input}.f0 ^ {input}.f1;\n")),
            "bit_shl" => out.push_str(&format!("  {target} = {input}.f0 << {input}.f1;\n")),
            "bit_shr" => out.push_str(&format!("  {target} = {input}.f0 >> {input}.f1;\n")),
            other => return Err(format!("unsupported builtin `{other}`")),
        }
        Ok(())
    }

    fn emit_format_faultable_or_plain(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
        formatter: &str,
    ) -> Result<(), String> {
        match input_ty {
            Ty::Faultable(_) => {
                out.push_str(&format!("  if ({input}.is_fault) {{\n"));
                out.push_str(&format!("    {target}.is_fault = true;\n"));
                out.push_str(&format!("    {target}.fault = {input}.fault;\n"));
                out.push_str("  } else {\n");
                out.push_str(&format!("    {target}.is_fault = false;\n"));
                out.push_str(&format!(
                    "    {target}.value = {formatter}({input}.value);\n"
                ));
                out.push_str("  }\n");
            }
            _ => out.push_str(&format!("  {target} = {formatter}({input});\n")),
        }
        Ok(())
    }

    fn emit_all_any(&mut self, out: &mut String, target: &str, input: &str, all: bool) {
        let i = self.next_temp();
        out.push_str(&format!(
            "  {target} = {};\n",
            if all { "true" } else { "false" }
        ));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        if all {
            out.push_str(&format!(
                "    if (!{input}.items[{i}]) {{ {target} = false; break; }}\n"
            ));
        } else {
            out.push_str(&format!(
                "    if ({input}.items[{i}]) {{ {target} = true; break; }}\n"
            ));
        }
        out.push_str("  }\n");
    }

    fn emit_faultable_concat_bytes(&mut self, out: &mut String, target: &str, input: &str) {
        let ok_values = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!("  {target}.is_fault = false;\n"));
        out.push_str(&format!(
            "  FaSeq_Bytes {ok_values} = FaSeq_Bytes_new({input}.count);\n"
        ));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        out.push_str(&format!("    if ({input}.items[{i}].is_fault) {{ {target}.is_fault = true; {target}.fault = {input}.items[{i}].fault; break; }}\n"));
        out.push_str(&format!(
            "    {ok_values}.items[{i}] = {input}.items[{i}].value;\n"
        ));
        out.push_str("  }\n");
        out.push_str(&format!(
            "  if (!{target}.is_fault) {target}.value = fa_concat_bytes({ok_values});\n"
        ));
    }

    fn emit_collect(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Seq(item_ty) = input_ty else {
            return Err("collect expected sequence input".to_string());
        };
        let Ty::Faultable(ok_ty) = item_ty.as_ref() else {
            return Err("collect expected Seq[Faultable[V]] input".to_string());
        };
        let Ty::Faultable(seq_ty) = output_ty else {
            return Err("collect expected faultable sequence output".to_string());
        };
        let new_fn = self.types.seq_new_name(seq_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target}.is_fault = false;\n"));
        out.push_str(&format!("  {target}.value = {new_fn}({input}.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        out.push_str(&format!("    if ({input}.items[{i}].is_fault) {{ {target}.is_fault = true; {target}.fault = {input}.items[{i}].fault; break; }}\n"));
        self.emit_assign_value(
            out,
            &format!("{target}.value.items[{i}]"),
            ok_ty,
            &Value {
                code: format!("{input}.items[{i}].value"),
                ty: ok_ty.as_ref().clone(),
            },
        )?;
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_stream_to_seq(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Stream(item_ty) = input_ty else {
            return Err("to_seq expected stream input".to_string());
        };
        let Ty::Faultable(seq_ty) = output_ty else {
            return Err("to_seq expected faultable sequence output".to_string());
        };
        let Ty::Seq(seq_item_ty) = seq_ty.as_ref() else {
            return Err("to_seq expected sequence output".to_string());
        };
        if seq_item_ty.as_ref() != item_ty.as_ref() {
            return Err("to_seq stream item/output item mismatch".to_string());
        }
        let item_c_ty = self.types.c_type(item_ty);
        let seq_new = self.types.seq_new_name(seq_ty)?;
        let cap = self.next_temp();
        let count = self.next_temp();
        let items = self.next_temp();
        let status = self.next_temp();
        let item = self.next_temp();
        let fault = self.next_temp();
        let close_fault = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!("  {target}.is_fault = false;\n"));
        out.push_str(&format!("  size_t {cap} = 8;\n"));
        out.push_str(&format!("  size_t {count} = 0;\n"));
        out.push_str(&format!(
            "  {item_c_ty} *{items} = ({item_c_ty} *)calloc({cap}, sizeof({item_c_ty}));\n"
        ));
        out.push_str(&format!("  if (!{items}) fa_die_alloc();\n"));
        out.push_str(&format!("  if (!{input}.next) {{\n"));
        out.push_str(&format!(
            "    {target}.is_fault = true; {target}.fault = fa_fault_cstr(\"stream.to_seq: stream is not pull-readable\");\n"
        ));
        out.push_str("  } else {\n");
        out.push_str("    for (;;) {\n");
        out.push_str(&format!(
            "      if ({count} == {cap}) {{ {cap} *= 2; {item_c_ty} *next_items = ({item_c_ty} *)realloc({items}, {cap} * sizeof({item_c_ty})); if (!next_items) fa_die_alloc(); {items} = next_items; }}\n"
        ));
        out.push_str(&format!("      {item_c_ty} {item};\n"));
        out.push_str(&format!("      FaFault {fault};\n"));
        out.push_str(&format!(
            "      int {status} = {input}.next({input}.state, &{item}, &{fault});\n"
        ));
        out.push_str(&format!("      if ({status} < 0) {{ {target}.is_fault = true; {target}.fault = {fault}; break; }}\n"));
        out.push_str(&format!("      if ({status} == 0) break;\n"));
        out.push_str(&format!("      {items}[{count}++] = {item};\n"));
        out.push_str("    }\n");
        out.push_str(&format!("    FaFault {close_fault};\n"));
        out.push_str(&format!("    if (fa_stream_close(&{input}, &{close_fault}) != 0 && !{target}.is_fault) {{ {target}.is_fault = true; {target}.fault = {close_fault}; }}\n"));
        out.push_str("  }\n");
        out.push_str(&format!("  if (!{target}.is_fault) {{\n"));
        out.push_str(&format!("    {target}.value = {seq_new}({count});\n"));
        out.push_str(&format!(
            "    for (size_t {i} = 0; {i} < {count}; {i}++) {target}.value.items[{i}] = {items}[{i}];\n"
        ));
        out.push_str("  }\n");
        out.push_str(&format!("  free({items});\n"));
        Ok(())
    }

    fn emit_stream_drain(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Stream(item_ty) = input_ty else {
            return Err("drain expected stream input".to_string());
        };
        let item_c_ty = self.types.c_type(item_ty);
        let item = self.next_temp();
        let fault = self.next_temp();
        let close_fault = self.next_temp();
        let status = self.next_temp();
        out.push_str(&format!("  {target}.is_fault = false;\n"));
        out.push_str(&format!("  {target}.value = 0;\n"));
        out.push_str(&format!("  if (!{input}.next) {{\n"));
        out.push_str(&format!(
            "    {target}.is_fault = true; {target}.fault = fa_fault_cstr(\"stream.drain: stream is not pull-readable\");\n"
        ));
        out.push_str("  } else {\n");
        out.push_str("    for (;;) {\n");
        out.push_str(&format!("      {item_c_ty} {item};\n"));
        out.push_str(&format!("      FaFault {fault};\n"));
        out.push_str(&format!(
            "      int {status} = {input}.next({input}.state, &{item}, &{fault});\n"
        ));
        out.push_str(&format!("      if ({status} < 0) {{ {target}.is_fault = true; {target}.fault = {fault}; break; }}\n"));
        out.push_str(&format!("      if ({status} == 0) break;\n"));
        out.push_str("    }\n");
        out.push_str(&format!("    FaFault {close_fault};\n"));
        out.push_str(&format!("    if (fa_stream_close(&{input}, &{close_fault}) != 0 && !{target}.is_fault) {{ {target}.is_fault = true; {target}.fault = {close_fault}; }}\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_zip(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("zip expected tuple input".to_string());
        };
        let [Ty::Seq(_), Ty::Seq(_)] = items.as_slice() else {
            return Err("zip expected sequence inputs".to_string());
        };
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target}.items[{i}].f0 = {input}.f0.items[{i}];\n"
        ));
        out.push_str(&format!(
            "    {target}.items[{i}].f1 = {input}.f1.items[{i}];\n"
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_broadcast_left(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("broadcast_left expected tuple input".to_string());
        };
        let [_, Ty::Seq(_)] = items.as_slice() else {
            return Err("broadcast_left expected (A,Seq[B]) input".to_string());
        };
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.f1.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f1.count; {i}++) {{\n"
        ));
        out.push_str(&format!("    {target}.items[{i}].f0 = {input}.f0;\n"));
        out.push_str(&format!(
            "    {target}.items[{i}].f1 = {input}.f1.items[{i}];\n"
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_broadcast_right(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("broadcast_right expected tuple input".to_string());
        };
        let [Ty::Seq(_), _] = items.as_slice() else {
            return Err("broadcast_right expected (Seq[A],B) input".to_string());
        };
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target}.items[{i}].f0 = {input}.f0.items[{i}];\n"
        ));
        out.push_str(&format!("    {target}.items[{i}].f1 = {input}.f1;\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_transpose(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Seq(row_ty) = input_ty else {
            return Err("transpose expected sequence input".to_string());
        };
        let Ty::Seq(_) = row_ty.as_ref() else {
            return Err("transpose expected nested sequence input".to_string());
        };
        let out_new = self.types.seq_new_name(output_ty)?;
        let row_new = self.types.seq_new_name(row_ty)?;
        let rows = self.next_temp();
        let cols = self.next_temp();
        let r = self.next_temp();
        let c = self.next_temp();
        out.push_str(&format!("  size_t {rows} = {input}.count;\n"));
        out.push_str(&format!(
            "  size_t {cols} = {rows} == 0 ? 0 : {input}.items[0].count;\n"
        ));
        out.push_str(&format!("  for (size_t {r} = 0; {r} < {rows}; {r}++) {{\n"));
        out.push_str(&format!("    if ({input}.items[{r}].count != {cols}) fa_die_usage(\"transpose: rows must have the same length\");\n"));
        out.push_str("  }\n");
        out.push_str(&format!("  {target} = {out_new}({cols});\n"));
        out.push_str(&format!("  for (size_t {c} = 0; {c} < {cols}; {c}++) {{\n"));
        out.push_str(&format!("    {target}.items[{c}] = {row_new}({rows});\n"));
        out.push_str(&format!(
            "    for (size_t {r} = 0; {r} < {rows}; {r}++) {{\n"
        ));
        out.push_str(&format!(
            "      {target}.items[{c}].items[{r}] = {input}.items[{r}].items[{c}];\n"
        ));
        out.push_str("    }\n");
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_flatten(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Seq(row_ty) = input_ty else {
            return Err("flatten expected sequence input".to_string());
        };
        let Ty::Seq(_) = row_ty.as_ref() else {
            return Err("flatten expected nested sequence input".to_string());
        };
        let new_fn = self.types.seq_new_name(output_ty)?;
        let total = self.next_temp();
        let offset = self.next_temp();
        let r = self.next_temp();
        let c = self.next_temp();
        out.push_str(&format!("  size_t {total} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {r} = 0; {r} < {input}.count; {r}++) {total} += {input}.items[{r}].count;\n"
        ));
        out.push_str(&format!("  {target} = {new_fn}({total});\n"));
        out.push_str(&format!("  size_t {offset} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {r} = 0; {r} < {input}.count; {r}++) {{\n"
        ));
        out.push_str(&format!(
            "    for (size_t {c} = 0; {c} < {input}.items[{r}].count; {c}++) {{\n"
        ));
        out.push_str(&format!(
            "      {target}.items[{offset}++] = {input}.items[{r}].items[{c}];\n"
        ));
        out.push_str("    }\n");
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_inner_length(&mut self, out: &mut String, target: &str, input: &str) {
        out.push_str(&format!(
            "  {target} = {input}.count == 0 ? 0 : (int64_t){input}.items[0].count;\n"
        ));
    }

    fn emit_group_by_id(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("group_by_id expected tuple input".to_string());
        };
        let [Ty::Seq(value_ty), Ty::Seq(id_ty)] = items.as_slice() else {
            return Err("group_by_id expected sequence inputs".to_string());
        };
        if id_ty.as_ref() != &Ty::Int {
            return Err("group_by_id expected Seq[Int] ids".to_string());
        }
        let group_ty = Ty::Seq(value_ty.clone());
        let group_new = self.types.seq_new_name(&group_ty)?;
        let out_new = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        let groups = self.next_temp();
        let prev = self.next_temp();
        let run_start = self.next_temp();
        let group_index = self.next_temp();
        let len = self.next_temp();
        let j = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"group_by_id: values and ids must have the same length\");\n"));
        out.push_str(&format!(
            "  size_t {groups} = {input}.f0.count == 0 ? 0 : 1;\n"
        ));
        out.push_str(&format!("  if ({input}.f0.count > 0) {{\n"));
        out.push_str(&format!("    int64_t {prev} = {input}.f1.items[0];\n"));
        out.push_str(&format!(
            "    for (size_t {i} = 1; {i} < {input}.f1.count; {i}++) {{\n"
        ));
        out.push_str(&format!("      if ({input}.f1.items[{i}] < {prev}) fa_die_usage(\"group_by_id: ids must be non-decreasing\");\n"));
        out.push_str(&format!("      if ({input}.f1.items[{i}] != {prev}) {{ {groups}++; {prev} = {input}.f1.items[{i}]; }}\n"));
        out.push_str("    }\n");
        out.push_str("  }\n");
        out.push_str(&format!("  {target} = {out_new}({groups});\n"));
        out.push_str(&format!(
            "  size_t {run_start} = 0;\n  size_t {group_index} = 0;\n"
        ));
        out.push_str(&format!(
            "  for (size_t {i} = 1; {i} <= {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!("    if ({i} == {input}.f0.count || {input}.f1.items[{i}] != {input}.f1.items[{run_start}]) {{\n"));
        out.push_str(&format!("      size_t {len} = {i} - {run_start};\n"));
        out.push_str(&format!(
            "      {target}.items[{group_index}] = {group_new}({len});\n"
        ));
        out.push_str(&format!("      for (size_t {j} = 0; {j} < {len}; {j}++) {target}.items[{group_index}].items[{j}] = {input}.f0.items[{run_start} + {j}];\n"));
        out.push_str(&format!(
            "      {group_index}++;\n      {run_start} = {i};\n"
        ));
        out.push_str("    }\n  }\n");
        Ok(())
    }

    fn emit_shift_right(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count);\n"));
        out.push_str(&format!("  if ({input}.f0.count > 0) {{\n"));
        out.push_str(&format!("    {target}.items[0] = {input}.f1;\n"));
        out.push_str(&format!("    for (size_t {i} = 1; {i} < {input}.f0.count; {i}++) {target}.items[{i}] = {input}.f0.items[{i} - 1];\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_shift_left(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count);\n"));
        out.push_str(&format!("  if ({input}.f0.count > 0) {{\n"));
        out.push_str(&format!("    for (size_t {i} = 0; {i} + 1 < {input}.f0.count; {i}++) {target}.items[{i}] = {input}.f0.items[{i} + 1];\n"));
        out.push_str(&format!(
            "    {target}.items[{input}.f0.count - 1] = {input}.f1;\n"
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_head(
        &mut self,
        out: &mut String,
        target: &str,
        _output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        out.push_str(&format!("  if ({input}.count == 0) {{ {target}.is_fault = true; {target}.fault = fa_fault_cstr(\"head: empty sequence\"); }} else {{ {target}.is_fault = false; {target}.value = {input}.items[0]; }}\n"));
        Ok(())
    }

    fn emit_tail(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!(
            "  {target} = {new_fn}({input}.count == 0 ? 0 : {input}.count - 1);\n"
        ));
        out.push_str(&format!(
            "  for (size_t {i} = 1; {i} < {input}.count; {i}++) {target}.items[{i} - 1] = {input}.items[{i}];\n"
        ));
        Ok(())
    }

    fn emit_reverse(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        if !matches!(input_ty, Ty::Seq(_)) {
            return Err("reverse expected sequence input".to_string());
        }
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {target}.items[{i}] = {input}.items[{input}.count - 1 - {i}];\n"
        ));
        Ok(())
    }

    fn emit_take(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("take expected tuple input".to_string());
        };
        if !matches!(items.as_slice(), [Ty::Seq(_), Ty::Int]) {
            return Err("take expected (Seq[V],Int) input".to_string());
        }
        let new_fn = self.types.seq_new_name(output_ty)?;
        let count = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!(
            "  if ({input}.f1 < 0) fa_die_usage(\"take: count must be non-negative\");\n"
        ));
        out.push_str(&format!(
            "  size_t {count} = (size_t){input}.f1 > {input}.f0.count ? {input}.f0.count : (size_t){input}.f1;\n"
        ));
        out.push_str(&format!("  {target} = {new_fn}({count});\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {count}; {i}++) {target}.items[{i}] = {input}.f0.items[{i}];\n"
        ));
        Ok(())
    }

    fn emit_drop(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("drop expected tuple input".to_string());
        };
        if !matches!(items.as_slice(), [Ty::Seq(_), Ty::Int]) {
            return Err("drop expected (Seq[V],Int) input".to_string());
        }
        let new_fn = self.types.seq_new_name(output_ty)?;
        let offset = self.next_temp();
        let count = self.next_temp();
        let i = self.next_temp();
        out.push_str(&format!(
            "  if ({input}.f1 < 0) fa_die_usage(\"drop: count must be non-negative\");\n"
        ));
        out.push_str(&format!(
            "  size_t {offset} = (size_t){input}.f1 > {input}.f0.count ? {input}.f0.count : (size_t){input}.f1;\n"
        ));
        out.push_str(&format!(
            "  size_t {count} = {input}.f0.count - {offset};\n"
        ));
        out.push_str(&format!("  {target} = {new_fn}({count});\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {count}; {i}++) {target}.items[{i}] = {input}.f0.items[{offset} + {i}];\n"
        ));
        Ok(())
    }

    fn emit_fill(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("fill expected tuple input".to_string());
        };
        if !matches!(items.as_slice(), [_, Ty::Int]) {
            return Err("fill expected (V,Int) input".to_string());
        }
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!(
            "  if ({input}.f1 < 0) fa_die_usage(\"fill: count must be non-negative\");\n"
        ));
        out.push_str(&format!("  {target} = {new_fn}((size_t){input}.f1);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {target}.count; {i}++) {target}.items[{i}] = {input}.f0;\n"
        ));
        Ok(())
    }

    fn emit_slice(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!(
            "  if ({input}.f1 < 0 || {input}.f2 < {input}.f1 || (size_t){input}.f2 > {input}.f0.count) fa_die_usage(\"slice: index out of range\");\n"
        ));
        out.push_str(&format!(
            "  {target} = {new_fn}((size_t)({input}.f2 - {input}.f1));\n"
        ));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {target}.count; {i}++) {target}.items[{i}] = {input}.f0.items[(size_t){input}.f1 + {i}];\n"
        ));
        Ok(())
    }

    fn emit_last(
        &mut self,
        out: &mut String,
        target: &str,
        _output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        out.push_str(&format!("  if ({input}.count == 0) {{ {target}.is_fault = true; {target}.fault = fa_fault_cstr(\"last: empty sequence\"); }} else {{ {target}.is_fault = false; {target}.value = {input}.items[{input}.count - 1]; }}\n"));
        Ok(())
    }

    fn emit_at(
        &mut self,
        out: &mut String,
        target: &str,
        _output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        out.push_str(&format!("  if ({input}.f1 < 0 || (size_t){input}.f1 >= {input}.f0.count) {{ {target}.is_fault = true; {target}.fault = fa_fault_cstr(\"at: index out of range\"); }} else {{ {target}.is_fault = false; {target}.value = {input}.f0.items[{input}.f1]; }}\n"));
        Ok(())
    }

    fn emit_get(
        &mut self,
        out: &mut String,
        target: &str,
        _output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        out.push_str(&format!(
            "  if ({input}.f1 < 0 || (size_t){input}.f1 >= {input}.f0.count) fa_die_usage(\"get: index out of range\");\n"
        ));
        out.push_str(&format!("  {target} = {input}.f0.items[{input}.f1];\n"));
        Ok(())
    }

    fn emit_get_or(
        &mut self,
        out: &mut String,
        target: &str,
        _output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        out.push_str(&format!(
            "  {target} = ({input}.f1 < 0 || (size_t){input}.f1 >= {input}.f0.count) ? {input}.f2 : {input}.f0.items[{input}.f1];\n"
        ));
        Ok(())
    }

    fn emit_append(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count + 1);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {target}.items[{i}] = {input}.f0.items[{i}];\n"
        ));
        out.push_str(&format!(
            "  {target}.items[{input}.f0.count] = {input}.f1;\n"
        ));
        Ok(())
    }

    fn emit_set(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!(
            "  if ({input}.f1 < 0 || (size_t){input}.f1 >= {input}.f0.count) fa_die_usage(\"set: index out of range\");\n"
        ));
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {target}.items[{i}] = {input}.f0.items[{i}];\n"
        ));
        out.push_str(&format!("  {target}.items[{input}.f1] = {input}.f2;\n"));
        Ok(())
    }

    fn emit_seq_concat(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        input: &str,
        _input_ty: &Ty,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        let j = self.next_temp();
        out.push_str(&format!(
            "  {target} = {new_fn}({input}.f0.count + {input}.f1.count);\n"
        ));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {target}.items[{i}] = {input}.f0.items[{i}];\n"
        ));
        out.push_str(&format!(
            "  for (size_t {j} = 0; {j} < {input}.f1.count; {j}++) {target}.items[{input}.f0.count + {j}] = {input}.f1.items[{j}];\n"
        ));
        Ok(())
    }

    fn fusion_for_name(&self, name: &str) -> Option<Fusion> {
        let callable = self.callables.get(name)?;
        self.fusion_for_callable(callable, &mut HashSet::new())
    }

    fn fusion_for_callable(
        &self,
        callable: &Callable,
        visiting: &mut HashSet<String>,
    ) -> Option<Fusion> {
        if !visiting.insert(callable.name.clone()) {
            return None;
        }
        let fusion = self.fusion_for_callable_inner(callable, visiting);
        visiting.remove(&callable.name);
        fusion
    }

    fn fusion_for_callable_inner(
        &self,
        callable: &Callable,
        visiting: &mut HashSet<String>,
    ) -> Option<Fusion> {
        if let Some(fusion) = self.mean_fusion(callable, visiting) {
            return Some(fusion);
        }
        let [output] = callable.outputs.as_slice() else {
            return None;
        };
        let [chain] = callable.chains.as_slice() else {
            return None;
        };
        let stages = stages_binding_output(chain, &output.name)?;
        match stages {
            [Stage::Reduce { op, identity }] if self.is_add(op) && is_zero(identity) => {
                Some(Fusion::Sum)
            }
            [Stage::Map(node), Stage::Endpoint(Endpoint::Name(next))]
                if self.called_fusion(node, visiting) == Some(Fusion::Sum)
                    && self.called_fusion(next, visiting) == Some(Fusion::Sum) =>
            {
                Some(Fusion::NestedSum)
            }
            [Stage::Map(node)] => self.unary_op_for_node(node).map(Fusion::MapUnary),
            [Stage::Map(node), Stage::Reduce { op, identity }]
                if self.is_add(op) && is_zero(identity) =>
            {
                self.map_reduce_op_for_node(node).map(Fusion::MapReduceAdd)
            }
            [Stage::Map(node), Stage::Endpoint(Endpoint::Name(next))]
                if self.called_fusion(next, visiting) == Some(Fusion::Sum) =>
            {
                self.map_reduce_op_for_node(node).map(Fusion::MapReduceAdd)
            }
            [Stage::Endpoint(Endpoint::Name(zip)), Stage::Map(node)] if self.is_zip(zip) => {
                if self.binary_eq_for_node(node) {
                    Some(Fusion::ZipAllEqual)
                } else {
                    self.binary_op_for_node(node).map(Fusion::ZipMap)
                }
            }
            [
                Stage::Endpoint(Endpoint::Name(zip)),
                Stage::Map(node),
                Stage::Reduce { op, identity },
            ] if self.is_zip(zip) && self.is_add(op) && is_zero(identity) => {
                self.binary_op_for_node(node).map(Fusion::ZipMapReduceAdd)
            }
            [
                Stage::Endpoint(Endpoint::Name(zip)),
                Stage::Map(node),
                Stage::Endpoint(Endpoint::Name(all)),
            ] if self.is_zip(zip) && self.is_all(all) && self.binary_eq_for_node(node) => {
                Some(Fusion::ZipAllEqual)
            }
            [
                Stage::Endpoint(Endpoint::Name(first)),
                Stage::Endpoint(Endpoint::Name(second)),
            ] => {
                let first_fusion = self.called_fusion(first, visiting);
                let second_fusion = self.called_fusion(second, visiting);
                if first_fusion == Some(Fusion::ZipMap(BinaryOp::Sub))
                    && second_fusion == Some(Fusion::MapReduceAdd(MapOp::Square))
                {
                    return Some(Fusion::ZipDifferenceSquareSum);
                }
                if self.is_sqrt(second) {
                    return first_fusion.map(|fusion| Fusion::Sqrt(Box::new(fusion)));
                }
                None
            }
            [Stage::Endpoint(Endpoint::Name(name))] if self.is_sqrt(name) => {
                Some(Fusion::Sqrt(Box::new(Fusion::Sum)))
            }
            _ => None,
        }
    }

    fn mean_fusion(&self, callable: &Callable, visiting: &mut HashSet<String>) -> Option<Fusion> {
        let [input] = callable.inputs.as_slice() else {
            return None;
        };
        let [output] = callable.outputs.as_slice() else {
            return None;
        };
        let [sum_chain, length_chain, div_chain] = callable.chains.as_slice() else {
            return None;
        };
        let sum_binding = final_variable(sum_chain)?;
        let length_binding = final_variable(length_chain)?;
        if !matches!(&sum_chain.source, Endpoint::Variable(name) if name == &input.name) {
            return None;
        }
        if !matches!(&length_chain.source, Endpoint::Variable(name) if name == &input.name) {
            return None;
        }
        let sum_stages = stages_binding_output(sum_chain, sum_binding)?;
        let length_stages = stages_binding_output(length_chain, length_binding)?;
        if !matches!(sum_stages, [Stage::Endpoint(Endpoint::Name(name))] if self.called_fusion(name, visiting) == Some(Fusion::Sum))
        {
            return None;
        }
        if !matches!(length_stages, [Stage::Endpoint(Endpoint::Name(name))] if self.is_length(name))
        {
            return None;
        }
        let div_stages = stages_binding_output(div_chain, &output.name)?;
        if !matches!(div_stages, [Stage::Endpoint(Endpoint::Name(name))] if self.is_div(name)) {
            return None;
        }
        if !matches!(
            &div_chain.source,
            Endpoint::Tuple(items)
                if items.len() == 2
                    && matches!(&items[0], Endpoint::Variable(name) if name == sum_binding)
                    && matches!(&items[1], Endpoint::Variable(name) if name == length_binding)
        ) {
            return None;
        }
        Some(Fusion::Mean)
    }

    fn emit_fusion_assign(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        fusion: &Fusion,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        match fusion {
            Fusion::Sum => self.emit_fused_sum(out, target, input, input_ty),
            Fusion::NestedSum => self.emit_fused_nested_sum(out, target, input, input_ty),
            Fusion::Mean => self.emit_fused_mean(out, target, input),
            Fusion::MapUnary(op) => self.emit_fused_map_unary(out, target, output_ty, *op, input),
            Fusion::ZipMap(op) => self.emit_fused_zip_map(out, target, output_ty, *op, input),
            Fusion::ZipMapReduceAdd(op) => self.emit_fused_zip_map_reduce(out, target, *op, input),
            Fusion::MapReduceAdd(op) => self.emit_fused_map_reduce(out, target, *op, input),
            Fusion::ZipAllEqual => self.emit_fused_zip_all_equal(out, target, input),
            Fusion::ZipDifferenceSquareSum => {
                self.emit_fused_zip_difference_square_sum(out, target, input)
            }
            Fusion::Sqrt(inner) => {
                let tmp = self.next_temp();
                out.push_str(&format!("  double {tmp};\n"));
                self.emit_fusion_assign(out, &tmp, &Ty::Real, inner, input, input_ty)?;
                out.push_str(&format!("  {target} = sqrt({tmp});\n"));
                Ok(())
            }
        }
    }

    fn emit_fused_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Seq(item_ty) = input_ty else {
            return Err("sum fusion expected sequence input".to_string());
        };
        let i = self.next_temp();
        out.push_str(&format!("  {target} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target} = {};\n",
            add_expr(target, &format!("{input}.items[{i}]"), item_ty)
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_nested_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Seq(row_ty) = input_ty else {
            return Err("nested sum fusion expected sequence input".to_string());
        };
        let Ty::Seq(item_ty) = row_ty.as_ref() else {
            return Err("nested sum fusion expected nested sequence input".to_string());
        };
        let r = self.next_temp();
        let c = self.next_temp();
        out.push_str(&format!("  {target} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {r} = 0; {r} < {input}.count; {r}++) {{\n"
        ));
        out.push_str(&format!(
            "    for (size_t {c} = 0; {c} < {input}.items[{r}].count; {c}++) {{\n"
        ));
        out.push_str(&format!(
            "      {target} = {};\n",
            add_expr(target, &format!("{input}.items[{r}].items[{c}]"), item_ty)
        ));
        out.push_str("    }\n");
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_matvec_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("matvec-sum fusion expected tuple input".to_string());
        };
        let [Ty::Seq(row_ty), Ty::Seq(_)] = items.as_slice() else {
            return Err("matvec-sum fusion expected (matrix, vector) input".to_string());
        };
        let Ty::Seq(item_ty) = row_ty.as_ref() else {
            return Err("matvec-sum fusion expected matrix input".to_string());
        };
        if item_ty.as_ref() != &Ty::Real {
            return Err("matvec-sum fusion expected real matrix input".to_string());
        }

        let row = self.next_temp();
        let col = self.next_temp();
        let dot = self.next_temp();
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {row} = 0; {row} < {input}.f0.count; {row}++) {{\n"
        ));
        out.push_str(&format!(
            "    if ({input}.f0.items[{row}].count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"
        ));
        out.push_str(&format!("    double {dot} = 0.0;\n"));
        out.push_str(&format!(
            "    for (size_t {col} = 0; {col} < {input}.f1.count; {col}++) {{\n"
        ));
        out.push_str(&format!(
            "      {dot} += {input}.f0.items[{row}].items[{col}] * {input}.f1.items[{col}];\n"
        ));
        out.push_str("    }\n");
        out.push_str(&format!("    {target} += {dot};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_matmul_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("matmul-sum fusion expected tuple input".to_string());
        };
        let [Ty::Seq(left_row_ty), Ty::Seq(right_row_ty)] = items.as_slice() else {
            return Err("matmul-sum fusion expected matrix pair input".to_string());
        };
        if !matches!(left_row_ty.as_ref(), Ty::Seq(item_ty) if item_ty.as_ref() == &Ty::Real)
            || !matches!(right_row_ty.as_ref(), Ty::Seq(item_ty) if item_ty.as_ref() == &Ty::Real)
        {
            return Err("matmul-sum fusion expected real matrix inputs".to_string());
        }

        let inner = self.next_temp();
        let cols = self.next_temp();
        let check = self.next_temp();
        let k = self.next_temp();
        let row = self.next_temp();
        let col = self.next_temp();
        let left_sum = self.next_temp();
        let right_sum = self.next_temp();

        out.push_str(&format!("  size_t {inner} = {input}.f1.count;\n"));
        out.push_str(&format!(
            "  size_t {cols} = {inner} == 0 ? 0 : {input}.f1.items[0].count;\n"
        ));
        out.push_str(&format!(
            "  for (size_t {check} = 0; {check} < {inner}; {check}++) {{\n"
        ));
        out.push_str(&format!(
            "    if ({input}.f1.items[{check}].count != {cols}) fa_die_usage(\"transpose: rows must have the same length\");\n"
        ));
        out.push_str("  }\n");
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!("  if ({cols} > 0) {{\n"));
        out.push_str(&format!(
            "    for (size_t {k} = 0; {k} < {inner}; {k}++) {{\n"
        ));
        out.push_str(&format!("      double {left_sum} = 0.0;\n"));
        out.push_str(&format!(
            "      for (size_t {row} = 0; {row} < {input}.f0.count; {row}++) {{\n"
        ));
        out.push_str(&format!(
            "        if ({input}.f0.items[{row}].count != {inner}) fa_die_usage(\"zip: sequences must have the same length\");\n"
        ));
        out.push_str(&format!(
            "        {left_sum} += {input}.f0.items[{row}].items[{k}];\n"
        ));
        out.push_str("      }\n");
        out.push_str(&format!("      double {right_sum} = 0.0;\n"));
        out.push_str(&format!(
            "      for (size_t {col} = 0; {col} < {cols}; {col}++) {{\n"
        ));
        out.push_str(&format!(
            "        {right_sum} += {input}.f1.items[{k}].items[{col}];\n"
        ));
        out.push_str("      }\n");
        out.push_str(&format!("      {target} += {left_sum} * {right_sum};\n"));
        out.push_str("    }\n");
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_mean(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
    ) -> Result<(), String> {
        let total = self.next_temp();
        out.push_str(&format!("  double {total} = 0.0;\n"));
        self.emit_fused_sum(out, &total, input, &Ty::Seq(Box::new(Ty::Real)))?;
        out.push_str(&format!("  {target} = {total} / (double){input}.count;\n"));
        Ok(())
    }

    fn emit_fused_map_unary(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        op: UnaryOp,
        input: &str,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        let expr = match op {
            UnaryOp::Neg => format!("-({input}.items[{i}])"),
            UnaryOp::Abs => format!("fabs({input}.items[{i}])"),
        };
        out.push_str(&format!("    {target}.items[{i}] = {expr};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_map(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        op: BinaryOp,
        input: &str,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target}.items[{i}] = {};\n",
            binary_op_expr(
                op,
                &format!("{input}.f0.items[{i}]"),
                &format!("{input}.f1.items[{i}]")
            )
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_map_reduce(
        &mut self,
        out: &mut String,
        target: &str,
        op: BinaryOp,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target} += {};\n",
            binary_op_expr(
                op,
                &format!("{input}.f0.items[{i}]"),
                &format!("{input}.f1.items[{i}]")
            )
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_map_reduce(
        &mut self,
        out: &mut String,
        target: &str,
        op: MapOp,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        let value = format!("{input}.items[{i}]");
        let expr = match op {
            MapOp::Square => format!("({value} * {value})"),
            MapOp::Abs => format!("fabs({value})"),
        };
        out.push_str(&format!("    {target} += {expr};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_all_equal(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = true;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!("    if ({input}.f0.items[{i}] != {input}.f1.items[{i}]) {{ {target} = false; break; }}\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_difference_square_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        let delta = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    double {delta} = {input}.f0.items[{i}] - {input}.f1.items[{i}];\n"
        ));
        out.push_str(&format!("    {target} += {delta} * {delta};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn called_fusion(&self, name: &str, visiting: &mut HashSet<String>) -> Option<Fusion> {
        self.callables
            .get(name)
            .and_then(|callable| self.fusion_for_callable(callable, visiting))
    }

    fn unary_op_for_node(&self, name: &str) -> Option<UnaryOp> {
        let op = self.direct_single_builtin(name)?;
        match op.as_str() {
            "neg" => Some(UnaryOp::Neg),
            "abs" => Some(UnaryOp::Abs),
            _ => None,
        }
    }

    fn map_reduce_op_for_node(&self, name: &str) -> Option<MapOp> {
        if self.is_square_node(name) {
            return Some(MapOp::Square);
        }
        if self.unary_op_for_node(name) == Some(UnaryOp::Abs) {
            return Some(MapOp::Abs);
        }
        None
    }

    fn binary_op_for_node(&self, name: &str) -> Option<BinaryOp> {
        let op = self.direct_single_builtin(name)?;
        match op.as_str() {
            "add" => Some(BinaryOp::Add),
            "sub" => Some(BinaryOp::Sub),
            "mul" => Some(BinaryOp::Mul),
            "div" => Some(BinaryOp::Div),
            _ => None,
        }
    }

    fn binary_eq_for_node(&self, name: &str) -> bool {
        self.direct_single_builtin(name)
            .map(|op| op == "eq")
            .unwrap_or(false)
    }

    fn is_map_sum_callable(&self, name: &str) -> bool {
        let Some(callable) = self.callables.get(name) else {
            return false;
        };
        let [input] = callable.inputs.as_slice() else {
            return false;
        };
        let [output] = callable.outputs.as_slice() else {
            return false;
        };
        let [chain] = callable.chains.as_slice() else {
            return false;
        };
        if !matches!(&chain.source, Endpoint::Variable(name) if name == &input.name) {
            return false;
        }
        let Some([Stage::Map(node)]) = stages_binding_output(chain, &output.name) else {
            return false;
        };
        self.fusion_for_name(node) == Some(Fusion::Sum)
    }

    fn is_matmul_name(&self, name: &str) -> bool {
        name == "__flow_std_matrix_matmul"
    }

    fn is_matvec_name(&self, name: &str) -> bool {
        name == "__flow_std_matrix_matvec"
    }

    fn direct_single_builtin(&self, name: &str) -> Option<String> {
        let callable = self.callables.get(name)?;
        let [input] = callable.inputs.as_slice() else {
            return None;
        };
        let [output] = callable.outputs.as_slice() else {
            return None;
        };
        let [chain] = callable.chains.as_slice() else {
            return None;
        };
        if !matches!(&chain.source, Endpoint::Variable(name) if name == &input.name) {
            return None;
        }
        let [Stage::Endpoint(Endpoint::Name(op))] = stages_binding_output(chain, &output.name)?
        else {
            return None;
        };
        Some(self.canonical_name(op))
    }

    fn is_square_node(&self, name: &str) -> bool {
        let Some(callable) = self.callables.get(name) else {
            return false;
        };
        let [input] = callable.inputs.as_slice() else {
            return false;
        };
        let [output] = callable.outputs.as_slice() else {
            return false;
        };
        let [chain] = callable.chains.as_slice() else {
            return false;
        };
        if !matches!(
            &chain.source,
            Endpoint::Tuple(items)
                if items.len() == 2
                    && matches!(&items[0], Endpoint::Variable(name) if name == &input.name)
                    && matches!(&items[1], Endpoint::Variable(name) if name == &input.name)
        ) {
            return false;
        }
        matches!(
            stages_binding_output(chain, &output.name),
            Some([Stage::Endpoint(Endpoint::Name(op))]) if self.is_mul(op)
        )
    }

    fn is_add(&self, name: &str) -> bool {
        self.canonical_name(name) == "add"
    }

    fn is_mul(&self, name: &str) -> bool {
        self.canonical_name(name) == "mul"
    }

    fn is_div(&self, name: &str) -> bool {
        self.canonical_name(name) == "div"
    }

    fn is_sqrt(&self, name: &str) -> bool {
        self.canonical_name(name) == "sqrt"
    }

    fn is_zip(&self, name: &str) -> bool {
        self.canonical_name(name) == "zip"
    }

    fn is_all(&self, name: &str) -> bool {
        self.canonical_name(name) == "all"
    }

    fn is_length(&self, name: &str) -> bool {
        self.canonical_name(name) == "length"
    }

    fn is_parallel_safe_name(&self, name: &str, visiting: &mut HashSet<String>) -> bool {
        if let Some(callable) = self.callables.get(name) {
            return self.is_parallel_safe_callable(callable, visiting);
        }
        self.is_parallel_safe_builtin(&self.canonical_name(name))
    }

    fn is_parallel_safe_callable(
        &self,
        callable: &Callable,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if !visiting.insert(callable.name.clone()) {
            return false;
        }
        let safe = callable.chains.iter().all(|chain| {
            self.is_parallel_safe_endpoint(&chain.source, visiting)
                && chain
                    .stages
                    .iter()
                    .all(|stage| self.is_parallel_safe_stage(stage, visiting))
        });
        visiting.remove(&callable.name);
        safe
    }

    fn is_parallel_safe_stage(&self, stage: &Stage, visiting: &mut HashSet<String>) -> bool {
        match stage {
            Stage::Endpoint(endpoint) => self.is_parallel_safe_endpoint(endpoint, visiting),
            Stage::Bind(_) => true,
            Stage::Map(name) | Stage::Filter(name) => self.is_parallel_safe_name(name, visiting),
            Stage::FaultMap { node, .. } => self.is_parallel_safe_name(node, visiting),
            Stage::Repeat { count, node } => {
                self.is_parallel_safe_endpoint(count, visiting)
                    && self.is_parallel_safe_name(node, visiting)
            }
            Stage::Reduce { op, identity } | Stage::Scan { op, identity } => {
                self.is_parallel_safe_endpoint(identity, visiting)
                    && self.is_parallel_safe_name(op, visiting)
            }
            Stage::Match { arms } => arms.iter().all(|arm| {
                let target_safe = match &arm.target {
                    MatchTarget::Node(node) => self.is_parallel_safe_name(node, visiting),
                    MatchTarget::Value(endpoint) => {
                        self.is_parallel_safe_endpoint(endpoint, visiting)
                    }
                };
                target_safe
                    && match &arm.guard {
                        MatchGuard::Call { node, args } => {
                            self.is_parallel_safe_name(node, visiting)
                                && args
                                    .iter()
                                    .all(|arg| self.is_parallel_safe_endpoint(arg, visiting))
                        }
                        MatchGuard::Fallback => true,
                    }
            }),
        }
    }

    fn is_parallel_safe_endpoint(
        &self,
        endpoint: &Endpoint,
        visiting: &mut HashSet<String>,
    ) -> bool {
        match endpoint {
            Endpoint::Name(name) => self.is_parallel_safe_name(name, visiting),
            Endpoint::Tuple(items) | Endpoint::Seq(items) => items
                .iter()
                .all(|item| self.is_parallel_safe_endpoint(item, visiting)),
            Endpoint::Eval { source, stages } => {
                self.is_parallel_safe_endpoint(source, visiting)
                    && stages.iter().all(|stage| match stage {
                        Stage::Endpoint(Endpoint::Name(name)) => {
                            self.is_parallel_safe_name(name, visiting)
                        }
                        Stage::Endpoint(endpoint) => {
                            self.is_parallel_safe_endpoint(endpoint, visiting)
                        }
                        Stage::Bind(_) => true,
                        Stage::Map(name)
                        | Stage::Filter(name)
                        | Stage::Repeat { node: name, .. }
                        | Stage::Reduce { op: name, .. }
                        | Stage::Scan { op: name, .. } => {
                            self.is_parallel_safe_name(name, visiting)
                        }
                        Stage::FaultMap { node, .. } => self.is_parallel_safe_name(node, visiting),
                        Stage::Match { .. } => false,
                    })
            }
            Endpoint::Variable(_)
            | Endpoint::Int(_)
            | Endpoint::Real(_)
            | Endpoint::Bool(_)
            | Endpoint::String(_)
            | Endpoint::Unit => true,
        }
    }

    fn is_parallel_safe_builtin(&self, name: &str) -> bool {
        !matches!(
            name,
            "read_file"
                | "write_file"
                | "exists"
                | "is_file"
                | "is_dir"
                | "file_size"
                | "list_dir"
                | "walk_files"
                | "read_files"
                | "open_file"
                | "size"
                | "read_at"
                | "copy_to_file"
                | "close"
                | "read_stdin"
                | "write_stdout"
                | "write_stderr"
        )
    }

    fn call_output_type(&self, name: &str, input: &Ty) -> Result<Ty, String> {
        if let Some(signature) = self.signatures.get(name) {
            if &signature.input == input && contains_faultable_ty(&signature.input) {
                return Ok(signature.output.clone());
            }
            if (matches!(input, Ty::Faultable(_)) || unwrap_faultable_tuple(input).is_some())
                && !matches!(signature.output, Ty::Faultable(_))
            {
                return Ok(Ty::Faultable(Box::new(signature.output.clone())));
            }
            return Ok(signature.output.clone());
        }
        let canonical = self.canonical_name(name);
        builtin_output_type(&canonical, input)
    }

    fn call_input_type_for_endpoint(
        &self,
        name: &str,
        endpoint: &Endpoint,
        env: &HashMap<String, Value>,
    ) -> Result<Ty, String> {
        let actual = self.endpoint_value_type(endpoint, env)?;
        self.call_input_type_for_value(name, &actual)
    }

    fn call_input_type_for_value(&self, name: &str, actual: &Ty) -> Result<Ty, String> {
        let signatures = self.call_signatures(name)?;
        let mut last_error = None;
        for signature in signatures {
            let mut vars = HashMap::new();
            match match_input_types(&signature.input, actual, &mut vars) {
                Ok(()) => {
                    let input = substitute_ty(&signature.input, &vars).ok_or_else(|| {
                        format!("`{name}` input type contains unresolved type variables")
                    })?;
                    if contains_type_var(&input) {
                        return Err(
                            "empty sequence literals need a concrete type context".to_string()
                        );
                    }
                    return Ok(input_context_ty(&input, actual));
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| format!("cannot infer input type for `{name}`")))
    }

    fn call_signatures(&self, name: &str) -> Result<Vec<Signature>, String> {
        if let Some(signature) = self.signatures.get(name) {
            return Ok(vec![signature.clone()]);
        }
        let canonical = self.canonical_name(name);
        let (module, symbol_name) = if let Some(symbol_name) = canonical.strip_prefix("sqlite.") {
            ("std.sqlite", symbol_name)
        } else {
            stdlib::all_symbols()
                .find(|symbol| symbol.kind == stdlib::SymbolKind::Node && symbol.name == canonical)
                .map(|symbol| (symbol.module, symbol.name))
                .ok_or_else(|| format!("unknown node `{name}`"))?
        };
        let symbol = stdlib::find_export(module, symbol_name)
            .ok_or_else(|| format!("unknown node `{name}`"))?;
        let input = symbol
            .input
            .ok_or_else(|| format!("stdlib node `{name}` has no input type"))?;
        let output = symbol
            .output
            .ok_or_else(|| format!("stdlib node `{name}` has no output type"))?;
        Ok(vec![Signature {
            input: self.parse_signature_type(input)?,
            output: self.parse_signature_type(output)?,
        }])
    }

    fn canonical_name(&self, name: &str) -> String {
        self.stdlib_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    fn next_temp(&mut self) -> String {
        let tmp = format!("t{}", self.temp);
        self.temp += 1;
        tmp
    }
}

#[derive(Clone)]
struct LlvmValue<'ctx> {
    value: BasicValueEnum<'ctx>,
    ty: Ty,
}

struct DirectLlvm<'ctx, 'a> {
    context: &'ctx Context,
    module: LlvmModule<'ctx>,
    builder: Builder<'ctx>,
    codegen: TypedCodegen<'a>,
    types: LlvmTypeRegistry<'ctx>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    stream_helper: usize,
}

impl<'a> DirectLlvm<'_, 'a> {
    fn emit(codegen: TypedCodegen<'a>) -> Result<String, String> {
        let context = Context::create();
        let module = context.create_module("flowarrow");
        let builder = context.create_builder();
        let types = LlvmTypeRegistry::new(&context);
        let mut direct = DirectLlvm {
            context: &context,
            module,
            builder,
            codegen,
            types,
            functions: HashMap::new(),
            stream_helper: 0,
        };
        direct.declare_callables()?;
        direct.emit_callables()?;
        direct.emit_entrypoint()?;
        Ok(direct.module.print_to_string().to_string())
    }
}

impl<'ctx, 'a> DirectLlvm<'ctx, 'a> {
    fn declare_callables(&mut self) -> Result<(), String> {
        let names = self.codegen.callables.keys().cloned().collect::<Vec<_>>();
        for name in names {
            let sig = self
                .codegen
                .signatures
                .get(&name)
                .ok_or_else(|| format!("missing signature for `{name}`"))?
                .clone();
            let output_ty = self.types.basic_type(&sig.output)?;
            let input_ty = self.types.basic_type(&sig.input)?;
            let function_ty = output_ty.fn_type(&[input_ty.into()], false);
            let function = self
                .module
                .add_function(&user_fn_name(&name), function_ty, None);
            self.functions.insert(name, function);
        }
        Ok(())
    }

    fn emit_callables(&mut self) -> Result<(), String> {
        let mut names = self.codegen.callables.keys().cloned().collect::<Vec<_>>();
        names.sort();
        for name in names {
            let callable = *self
                .codegen
                .callables
                .get(&name)
                .ok_or_else(|| format!("missing callable `{name}`"))?;
            self.emit_callable(callable)?;
        }
        Ok(())
    }

    fn emit_callable(&mut self, callable: &Callable) -> Result<(), String> {
        let function = *self
            .functions
            .get(&callable.name)
            .ok_or_else(|| format!("missing LLVM function for `{}`", callable.name))?;
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        let sig = self
            .codegen
            .signatures
            .get(&callable.name)
            .ok_or_else(|| format!("missing signature for `{}`", callable.name))?
            .clone();
        let input = function
            .get_nth_param(0)
            .ok_or_else(|| format!("missing input parameter for `{}`", callable.name))?;
        let mut env = HashMap::new();
        match callable.inputs.as_slice() {
            [] => {}
            [port] => {
                env.insert(
                    port.name.clone(),
                    LlvmValue {
                        value: input,
                        ty: sig.input.clone(),
                    },
                );
            }
            ports => {
                let Ty::Tuple(items) = &sig.input else {
                    return Err(format!("callable `{}` expected tuple input", callable.name));
                };
                for (index, (port, ty)) in ports.iter().zip(items.iter()).enumerate() {
                    let value = self
                        .builder
                        .build_extract_value(input.into_struct_value(), index as u32, &port.name)
                        .map_err(|error| {
                            format!(
                                "LLVM backend failed to extract input `{}`: {error}",
                                port.name
                            )
                        })?;
                    env.insert(
                        port.name.clone(),
                        LlvmValue {
                            value,
                            ty: ty.clone(),
                        },
                    );
                }
            }
        }

        for chain in &callable.chains {
            self.emit_chain(chain, &mut env)?;
        }
        let result = self.emit_outputs(callable, &env, &sig.output)?;
        self.builder
            .build_return(Some(&result.value))
            .map_err(|error| {
                format!(
                    "LLVM backend failed to return from `{}`: {error}",
                    callable.name
                )
            })?;
        Ok(())
    }

    fn emit_outputs(
        &mut self,
        callable: &Callable,
        env: &HashMap<String, LlvmValue<'ctx>>,
        expected_ty: &Ty,
    ) -> Result<LlvmValue<'ctx>, String> {
        match callable.outputs.as_slice() {
            [] => {
                let value = self.types.basic_type(&Ty::Unit)?.const_zero();
                self.coerce_value_to_ty(
                    LlvmValue {
                        value,
                        ty: Ty::Unit,
                    },
                    expected_ty,
                )
            }
            [port] => {
                let value = env
                    .get(&port.name)
                    .cloned()
                    .ok_or_else(|| format!("output `{}` is not bound", port.name))?;
                self.coerce_value_to_ty(value, expected_ty)
            }
            ports => {
                let values = ports
                    .iter()
                    .map(|port| {
                        env.get(&port.name)
                            .cloned()
                            .ok_or_else(|| format!("output `{}` is not bound", port.name))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let Ty::Tuple(expected_items) = expected_ty else {
                    return Err(format!(
                        "callable `{}` has multiple outputs but signature output is `{expected_ty}`",
                        callable.name
                    ));
                };
                if expected_items.len() != values.len() {
                    return Err(format!(
                        "callable `{}` output arity mismatch: signature has {}, callable has {}",
                        callable.name,
                        expected_items.len(),
                        values.len()
                    ));
                }
                let mut out = self
                    .types
                    .basic_type(expected_ty)?
                    .into_struct_type()
                    .const_zero();
                for (index, (value, expected_item)) in
                    values.into_iter().zip(expected_items.iter()).enumerate()
                {
                    let value = self.coerce_value_to_ty(value, expected_item)?;
                    out = self
                        .builder
                        .build_insert_value(out, value.value, index as u32, "out")
                        .map_err(|error| {
                            format!("LLVM backend failed to assemble output tuple: {error}")
                        })?
                        .into_struct_value();
                }
                Ok(LlvmValue {
                    value: out.into(),
                    ty: expected_ty.clone(),
                })
            }
        }
    }

    fn emit_chain(
        &mut self,
        chain: &Chain,
        env: &mut HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<(), String> {
        let mut value = if endpoint_contains_empty_seq(&chain.source) {
            if let Some(Stage::Endpoint(Endpoint::Name(name))) = chain.stages.first() {
                let actual = self.endpoint_type(&chain.source, env)?;
                let expected = self.codegen.call_input_type_for_value(name, &actual)?;
                self.emit_endpoint_expected(&chain.source, env, Some(&expected))?
            } else {
                self.emit_endpoint(&chain.source, env)?
            }
        } else {
            self.emit_endpoint(&chain.source, env)?
        };
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Bind(target) if is_last => self.bind_target(target, value.clone(), env)?,
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value = self.emit_call(name, value)?;
                }
                Stage::Map(name) => {
                    value = self.emit_map(name, value)?;
                }
                Stage::Filter(name) => {
                    value = self.emit_filter(name, value)?;
                }
                Stage::Reduce { op, identity } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_reduce(op, value, identity)?;
                }
                Stage::Scan { op, identity } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_scan(op, value, identity)?;
                }
                Stage::Repeat { count, node } => {
                    let count = self.emit_endpoint(count, env)?;
                    value = self.emit_repeat(node, value, count)?;
                }
                Stage::Match { arms } => {
                    value = self.emit_match(arms, value, env)?;
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let (ok_value, fault_value) = self.emit_fault_map(node, value.clone())?;
                    if env.insert(ok.clone(), ok_value).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_value).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
                }
                Stage::Bind(_) => {
                    return Err("binding targets may only appear as final stages".to_string());
                }
            }
        }
        Ok(())
    }

    fn emit_endpoint(
        &mut self,
        endpoint: &Endpoint,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
        self.emit_endpoint_expected(endpoint, env, None)
    }

    fn emit_endpoint_expected(
        &mut self,
        endpoint: &Endpoint,
        env: &HashMap<String, LlvmValue<'ctx>>,
        expected: Option<&Ty>,
    ) -> Result<LlvmValue<'ctx>, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Int(value) => Ok(LlvmValue {
                value: self
                    .context
                    .i64_type()
                    .const_int(*value as u64, true)
                    .into(),
                ty: Ty::Int,
            }),
            Endpoint::Real(value) => Ok(LlvmValue {
                value: self.context.f64_type().const_float(*value).into(),
                ty: Ty::Real,
            }),
            Endpoint::Bool(value) => Ok(LlvmValue {
                value: self
                    .context
                    .i8_type()
                    .const_int(if *value { 1 } else { 0 }, false)
                    .into(),
                ty: Ty::Bool,
            }),
            Endpoint::Unit => Ok(LlvmValue {
                value: self.types.basic_type(&Ty::Unit)?.const_zero(),
                ty: Ty::Unit,
            }),
            Endpoint::Tuple(items) => {
                let values = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let expected = match expected {
                            Some(Ty::Tuple(items)) => items.get(index),
                            _ => None,
                        };
                        self.emit_endpoint_expected(item, env, expected)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let ty = expected.cloned().unwrap_or_else(|| {
                    Ty::Tuple(values.iter().map(|value| value.ty.clone()).collect())
                });
                let mut out = self.types.basic_type(&ty)?.into_struct_type().const_zero();
                let Ty::Tuple(expected_items) = &ty else {
                    return Err(format!("tuple literal expected tuple type, found `{ty}`"));
                };
                for (index, (value, expected_item)) in
                    values.into_iter().zip(expected_items.iter()).enumerate()
                {
                    let value = self.coerce_value_to_ty(value, expected_item)?;
                    out = self
                        .builder
                        .build_insert_value(out, value.value, index as u32, "tuple")
                        .map_err(|error| {
                            format!("LLVM backend failed to assemble tuple literal: {error}")
                        })?
                        .into_struct_value();
                }
                Ok(LlvmValue {
                    value: out.into(),
                    ty,
                })
            }
            Endpoint::String(_) | Endpoint::Seq(_) => {
                self.emit_literal_endpoint_expected(endpoint, env, expected)
            }
            Endpoint::Eval { source, stages } => self.emit_inline_eval(source, stages, env),
        }
    }

    fn emit_inline_eval(
        &mut self,
        source: &Endpoint,
        stages: &[Stage],
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let mut value = if endpoint_contains_empty_seq(source) {
            if let Some(Stage::Endpoint(Endpoint::Name(name))) = stages.first() {
                let actual = self.endpoint_type(source, env)?;
                let expected = self.codegen.call_input_type_for_value(name, &actual)?;
                self.emit_endpoint_expected(source, env, Some(&expected))?
            } else {
                self.emit_endpoint(source, env)?
            }
        } else {
            self.emit_endpoint(source, env)?
        };
        for stage in stages {
            match stage {
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value = self.emit_call(name, value)?;
                }
                Stage::Endpoint(Endpoint::Variable(_)) | Stage::Bind(_) => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                Stage::Endpoint(_) => {
                    return Err(
                        "non-name endpoints may only appear as inline evaluation sources"
                            .to_string(),
                    );
                }
                Stage::Map(name) => value = self.emit_map(name, value)?,
                Stage::FaultMap { .. } => {
                    return Err("inline evaluations cannot use `fault map`".to_string());
                }
                Stage::Filter(name) => value = self.emit_filter(name, value)?,
                Stage::Reduce { op, identity } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_reduce(op, value, identity)?;
                }
                Stage::Scan { op, identity } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_scan(op, value, identity)?;
                }
                Stage::Repeat { count, node } => {
                    let count = self.emit_endpoint(count, env)?;
                    value = self.emit_repeat(node, value, count)?;
                }
                Stage::Match { arms } => {
                    value = self.emit_match(arms, value, env)?;
                }
            }
        }
        Ok(value)
    }

    fn emit_literal_endpoint_expected(
        &mut self,
        endpoint: &Endpoint,
        env: &HashMap<String, LlvmValue<'ctx>>,
        expected: Option<&Ty>,
    ) -> Result<LlvmValue<'ctx>, String> {
        match endpoint {
            Endpoint::String(value) => {
                let global =
                    self.builder
                        .build_global_string_ptr(value, "str")
                        .map_err(|error| {
                            format!("LLVM backend failed to build string literal: {error}")
                        })?;
                let pair_ty = self.runtime_pair_type();
                let fn_value = self.runtime_function(
                    "fa_bytes_literal",
                    Some(pair_ty.into()),
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                    ],
                )?;
                let len = self.context.i64_type().const_int(value.len() as u64, false);
                let call = self
                    .builder
                    .build_call(
                        fn_value,
                        &[global.as_pointer_value().into(), len.into()],
                        "bytes",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to call fa_bytes_literal: {error}")
                    })?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "fa_bytes_literal did not return a value".to_string())?;
                let call = self.runtime_pair_to_value(call, &Ty::Bytes)?;
                Ok(LlvmValue {
                    value: call,
                    ty: Ty::Bytes,
                })
            }
            Endpoint::Seq(items) => {
                if items.is_empty() {
                    let Some(seq_ty @ Ty::Seq(_)) = expected else {
                        return Err("empty sequence literals need a type context".to_string());
                    };
                    return self.emit_seq_new(seq_ty, self.context.i64_type().const_zero());
                }
                let values = items
                    .iter()
                    .map(|item| {
                        let expected_item = match expected {
                            Some(Ty::Seq(item)) => Some(item.as_ref()),
                            _ => None,
                        };
                        self.emit_endpoint_expected(item, env, expected_item)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut item_ty = values[0].ty.clone();
                for value in values.iter().skip(1) {
                    item_ty = sequence_item_type(&item_ty, &value.ty)?;
                }
                let seq_ty = Ty::Seq(Box::new(item_ty.clone()));
                let count = self
                    .context
                    .i64_type()
                    .const_int(values.len() as u64, false);
                let seq = self.emit_seq_new(&seq_ty, count)?;
                for (index, value) in values.into_iter().enumerate() {
                    let value = self.coerce_value_to_ty(value, &item_ty)?;
                    self.store_seq_item(
                        seq.value,
                        &seq_ty,
                        self.context.i64_type().const_int(index as u64, false),
                        value.value,
                    )?;
                }
                Ok(seq)
            }
            _ => unreachable!(),
        }
    }

    fn endpoint_type(
        &self,
        endpoint: &Endpoint,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<Ty, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .map(|value| value.ty.clone())
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Int(_) => Ok(Ty::Int),
            Endpoint::Real(_) => Ok(Ty::Real),
            Endpoint::Bool(_) => Ok(Ty::Bool),
            Endpoint::String(_) => Ok(Ty::Bytes),
            Endpoint::Unit => Ok(Ty::Unit),
            Endpoint::Tuple(items) => items
                .iter()
                .map(|item| self.endpoint_type(item, env))
                .collect::<Result<Vec<_>, _>>()
                .map(Ty::Tuple),
            Endpoint::Seq(items) => {
                let mut item_ty = None;
                for item in items {
                    let ty = self.endpoint_type(item, env)?;
                    item_ty = Some(if let Some(current) = item_ty {
                        sequence_item_type(&current, &ty)?
                    } else {
                        ty
                    });
                }
                Ok(item_ty
                    .map(|ty| Ty::Seq(Box::new(ty)))
                    .unwrap_or(Ty::EmptySeq))
            }
            Endpoint::Eval { source, stages } => self.inline_eval_type(source, stages, env),
        }
    }

    fn inline_eval_type(
        &self,
        source: &Endpoint,
        stages: &[Stage],
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<Ty, String> {
        let mut value_ty = self.endpoint_type(source, env)?;
        for stage in stages {
            match stage {
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value_ty = self.codegen.call_output_type(name, &value_ty)?;
                }
                Stage::Endpoint(Endpoint::Variable(_)) | Stage::Bind(_) => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                Stage::Endpoint(_) => {
                    return Err(
                        "non-name endpoints may only appear as inline evaluation sources"
                            .to_string(),
                    );
                }
                Stage::Map(name) => {
                    value_ty = match &value_ty {
                        Ty::Seq(item_ty) => {
                            Ty::Seq(Box::new(self.codegen.call_output_type(name, item_ty)?))
                        }
                        Ty::Stream(item_ty) => {
                            Ty::Stream(Box::new(self.codegen.call_output_type(name, item_ty)?))
                        }
                        _ => return Err(format!("`map {name}` expected Seq or Stream input")),
                    };
                }
                Stage::FaultMap { .. } => {
                    return Err("inline evaluations cannot use `fault map`".to_string());
                }
                Stage::Filter(name) => {
                    let Ty::Seq(item_ty) = &value_ty else {
                        return Err(format!("`filter {name}` expected Seq input"));
                    };
                    let predicate_ty = self.codegen.call_output_type(name, item_ty)?;
                    if predicate_ty != Ty::Bool {
                        return Err(format!(
                            "`filter {name}` predicate expected `Bool`, found `{predicate_ty}`"
                        ));
                    }
                }
                Stage::Reduce { op, identity } => {
                    let Ty::Seq(item_ty) = &value_ty else {
                        return Err(format!("`reduce {op}` expected Seq input"));
                    };
                    let identity_ty = self.endpoint_type(identity, env)?;
                    if item_ty.as_ref() != &identity_ty {
                        return Err(format!(
                            "`reduce {op}` identity expected `{item_ty}`, found `{identity_ty}`"
                        ));
                    }
                    value_ty = self.codegen.call_output_type(op, item_ty)?;
                }
                Stage::Scan { op, identity } => {
                    let Ty::Seq(item_ty) = &value_ty else {
                        return Err(format!("`scan {op}` expected Seq input"));
                    };
                    let identity_ty = self.endpoint_type(identity, env)?;
                    if item_ty.as_ref() != &identity_ty {
                        return Err(format!(
                            "`scan {op}` identity expected `{item_ty}`, found `{identity_ty}`"
                        ));
                    }
                }
                Stage::Repeat { node, .. } => {
                    value_ty = self.codegen.call_output_type(node, &value_ty)?;
                }
                Stage::Match { arms } => {
                    value_ty = self.match_output_type_llvm(arms, &value_ty, env)?;
                }
            }
        }
        Ok(value_ty)
    }

    fn emit_match(
        &mut self,
        arms: &[MatchArm],
        subject: LlvmValue<'ctx>,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let output_ty = self.match_output_type_llvm(arms, &subject.ty, env)?;
        let output_llvm_ty = self.types.basic_type(&output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "match.result")
            .map_err(|error| format!("LLVM backend failed to allocate match result: {error}"))?;
        let function = self.current_function()?;
        let after_block = self.context.append_basic_block(function, "match.after");

        for (index, arm) in arms.iter().enumerate() {
            let arm_block = self.context.append_basic_block(function, "match.arm");
            let next_block = match &arm.guard {
                MatchGuard::Fallback => {
                    if index + 1 != arms.len() {
                        return Err("`match` fallback arm must be last".to_string());
                    }
                    self.builder
                        .build_unconditional_branch(arm_block)
                        .map_err(|error| {
                            format!("LLVM backend failed to branch to match fallback: {error}")
                        })?;
                    None
                }
                MatchGuard::Call { node, args } => {
                    let next_block = self.context.append_basic_block(function, "match.next");
                    let guard_input = self.emit_match_guard_input(subject.clone(), args, env)?;
                    let guard = self.emit_call(node, guard_input)?;
                    if guard.ty != Ty::Bool {
                        return Err(format!(
                            "match guard `{node}` result expected `Bool`, found `{}`",
                            guard.ty
                        ));
                    }
                    let guard_bit = self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            guard.value.into_int_value(),
                            self.context.i8_type().const_zero(),
                            "match.guard",
                        )
                        .map_err(|error| {
                            format!("LLVM backend failed to compare match guard: {error}")
                        })?;
                    self.builder
                        .build_conditional_branch(guard_bit, arm_block, next_block)
                        .map_err(|error| {
                            format!("LLVM backend failed to branch on match guard: {error}")
                        })?;
                    Some(next_block)
                }
            };

            self.builder.position_at_end(arm_block);
            let value = self.emit_match_target(&arm.target, subject.clone(), env)?;
            let value = self.coerce_value_to_ty(value, &output_ty)?;
            self.builder
                .build_store(out_ptr, value.value)
                .map_err(|error| format!("LLVM backend failed to store match result: {error}"))?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| format!("LLVM backend failed to leave match arm: {error}"))?;
            if let Some(next_block) = next_block {
                self.builder.position_at_end(next_block);
            } else if index + 1 == arms.len() {
                self.builder.position_at_end(after_block);
            }
        }

        if arms.is_empty() {
            return Err("`match` must contain at least one arm".to_string());
        }
        self.builder.position_at_end(after_block);
        let result = self
            .builder
            .build_load(output_llvm_ty, out_ptr, "match.result")
            .map_err(|error| format!("LLVM backend failed to load match result: {error}"))?;
        Ok(LlvmValue {
            value: result,
            ty: output_ty,
        })
    }

    fn match_output_type_llvm(
        &self,
        arms: &[MatchArm],
        subject_ty: &Ty,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<Ty, String> {
        let mut output = None;
        for arm in arms {
            let arm_ty = match &arm.target {
                MatchTarget::Node(node) => self.codegen.call_output_type(node, subject_ty)?,
                MatchTarget::Value(endpoint) => self.endpoint_type(endpoint, env)?,
            };
            output = Some(if let Some(current) = output {
                common_assignable_output_ty(
                    &current,
                    &arm_ty,
                    &format!("match arm `{}` result", format_match_target(&arm.target)),
                )?
            } else {
                arm_ty
            });
        }
        output.ok_or_else(|| "`match` must contain at least one arm".to_string())
    }

    fn emit_match_target(
        &mut self,
        target: &MatchTarget,
        subject: LlvmValue<'ctx>,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
        match target {
            MatchTarget::Node(node) => self.emit_call(node, subject),
            MatchTarget::Value(endpoint) => self.emit_endpoint(endpoint, env),
        }
    }

    fn emit_match_guard_input(
        &mut self,
        subject: LlvmValue<'ctx>,
        args: &[Endpoint],
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
        if args.is_empty() {
            return Ok(subject);
        }
        let mut values = Vec::with_capacity(args.len() + 1);
        values.push(subject);
        for arg in args {
            values.push(self.emit_endpoint(arg, env)?);
        }
        let ty = Ty::Tuple(values.iter().map(|value| value.ty.clone()).collect());
        let mut out = self.types.basic_type(&ty)?.into_struct_type().const_zero();
        for (index, value) in values.iter().enumerate() {
            out = self
                .builder
                .build_insert_value(out, value.value, index as u32, "match.guard")
                .map_err(|error| {
                    format!("LLVM backend failed to build match guard input: {error}")
                })?
                .into_struct_value();
        }
        Ok(LlvmValue {
            value: out.into(),
            ty,
        })
    }

    fn bind_target(
        &mut self,
        target: &BindingTarget,
        value: LlvmValue<'ctx>,
        env: &mut HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<(), String> {
        match target {
            BindingTarget::Discard => Ok(()),
            BindingTarget::Variable(name) => {
                if env.insert(name.clone(), value).is_some() {
                    return Err(format!("value `{name}` is bound more than once"));
                }
                Ok(())
            }
            BindingTarget::Tuple(targets) => {
                if let Ty::Faultable(inner) = value.ty.clone() {
                    let Ty::Tuple(items) = inner.as_ref() else {
                        return Err("tuple binding expected tuple value".to_string());
                    };
                    let faultable = value.value.into_struct_value();
                    let flag = self
                        .builder
                        .build_extract_value(faultable, 0, "is_fault")
                        .map_err(|error| {
                            format!("LLVM backend failed to extract tuple fault flag: {error}")
                        })?
                        .into_int_value();
                    let fault = self
                        .builder
                        .build_extract_value(faultable, 1, "fault")
                        .map_err(|error| {
                            format!("LLVM backend failed to extract tuple fault: {error}")
                        })?;
                    let inner_value = self
                        .builder
                        .build_extract_value(faultable, 2, "value")
                        .map_err(|error| {
                            format!("LLVM backend failed to extract tuple value: {error}")
                        })?;
                    for (index, (target, ty)) in targets.iter().zip(items.iter()).enumerate() {
                        if binding_target_is_discard(target) {
                            continue;
                        }
                        let field = self
                            .builder
                            .build_extract_value(
                                inner_value.into_struct_value(),
                                index as u32,
                                "field",
                            )
                            .map_err(|error| {
                                format!("LLVM backend failed to extract tuple binding: {error}")
                            })?;
                        let wrapped = self.faultable_value_with_flag(ty, flag, fault, field)?;
                        self.bind_target(
                            target,
                            LlvmValue {
                                value: wrapped,
                                ty: Ty::Faultable(Box::new(ty.clone())),
                            },
                            env,
                        )?;
                    }
                    return Ok(());
                }
                let Ty::Tuple(items) = value.ty.clone() else {
                    return Err("tuple binding expected tuple value".to_string());
                };
                for (index, (target, ty)) in targets.iter().zip(items.iter()).enumerate() {
                    if binding_target_is_discard(target) {
                        continue;
                    }
                    let field = self
                        .builder
                        .build_extract_value(value.value.into_struct_value(), index as u32, "field")
                        .map_err(|error| {
                            format!("LLVM backend failed to extract tuple binding: {error}")
                        })?;
                    self.bind_target(
                        target,
                        LlvmValue {
                            value: field,
                            ty: ty.clone(),
                        },
                        env,
                    )?;
                }
                Ok(())
            }
        }
    }

    fn emit_call(&mut self, name: &str, input: LlvmValue<'ctx>) -> Result<LlvmValue<'ctx>, String> {
        let output_ty = self.codegen.call_output_type(name, &input.ty)?;
        if let Some(function) = self.functions.get(name).copied() {
            let signature = self
                .codegen
                .signatures
                .get(name)
                .ok_or_else(|| format!("missing signature for `{name}`"))?
                .clone();
            let result =
                self.emit_user_function_call(name, function, &signature, input, &output_ty)?;
            return Ok(LlvmValue {
                value: result,
                ty: output_ty,
            });
        }
        self.emit_builtin_call(&self.codegen.canonical_name(name), input, output_ty)
    }

    fn emit_user_function_call(
        &mut self,
        name: &str,
        function: FunctionValue<'ctx>,
        signature: &Signature,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if input.ty == signature.input {
            return self
                .builder
                .build_call(function, &[input.value.into()], "call")
                .map_err(|error| format!("LLVM backend failed to call `{name}`: {error}"))?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| format!("LLVM callable `{name}` did not return a value"));
        }

        if unwrap_faultable_tuple(&input.ty)
            .as_ref()
            .is_some_and(|plain| plain == &signature.input)
        {
            let wrapped = self.coerce_faultable_tuple_to_faultable(input, &signature.input)?;
            return self.emit_user_function_call(name, function, signature, wrapped, output_ty);
        }

        let Ty::Faultable(input_inner) = input.ty.clone() else {
            return Err(format!(
                "direct LLVM backend cannot pass `{}` to `{name}` expecting `{}`",
                input.ty, signature.input
            ));
        };
        if input_inner.as_ref() != &signature.input {
            return Err(format!(
                "direct LLVM backend cannot pass `{}` to `{name}` expecting `{}`",
                input.ty, signature.input
            ));
        }
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "faultable input to `{name}` expected faultable output"
            ));
        };
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "faultable.user_call")
            .map_err(|error| format!("LLVM backend failed to allocate user call: {error}"))?;
        let current = self.current_function()?;
        let fault_block = self.context.append_basic_block(current, "user_call.fault");
        let ok_block = self.context.append_basic_block(current, "user_call.ok");
        let after_block = self.context.append_basic_block(current, "user_call.after");
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on user call fault: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store user call fault: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave user call fault: {error}"))?;

        self.builder.position_at_end(ok_block);
        let plain_input = self.extract_faultable_value(input.value)?;
        let plain_result = self
            .builder
            .build_call(function, &[plain_input.into()], "call")
            .map_err(|error| format!("LLVM backend failed to call `{name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("LLVM callable `{name}` did not return a value"))?;
        let ok = if &signature.output == output_inner.as_ref() {
            self.faultable_value(output_inner, false, None, Some(plain_result))?
        } else if &signature.output == output_ty {
            plain_result
        } else {
            return Err(format!(
                "direct LLVM backend cannot wrap `{}` as `{output_ty}`",
                signature.output
            ));
        };
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store user call value: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave user call ok: {error}"))?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "user_call.result")
            .map_err(|error| format!("LLVM backend failed to load user call result: {error}"))
    }

    fn emit_faultable_plain_builtin_call(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
        plain_output_ty: &Ty,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Faultable(input_inner) = input.ty.clone() else {
            return Err(format!(
                "faultable builtin wrapper expected faultable input to `{name}`"
            ));
        };
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "faultable builtin wrapper expected faultable output from `{name}`"
            ));
        };
        if output_inner.as_ref() != plain_output_ty && output_ty != plain_output_ty {
            return Err(format!("faultable builtin `{name}` output mismatch"));
        }
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "faultable.builtin")
            .map_err(|error| {
                format!("LLVM backend failed to allocate faultable builtin: {error}")
            })?;
        let function = self.current_function()?;
        let fault_block = self.context.append_basic_block(function, "builtin.fault");
        let ok_block = self.context.append_basic_block(function, "builtin.ok");
        let after_block = self.context.append_basic_block(function, "builtin.after");
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on faultable builtin: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| {
                format!("LLVM backend failed to store faultable builtin fault: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable builtin fault: {error}")
            })?;

        self.builder.position_at_end(ok_block);
        let plain_input = self.extract_faultable_value(input.value)?;
        let plain = self.emit_builtin_call(
            name,
            LlvmValue {
                value: plain_input,
                ty: input_inner.as_ref().clone(),
            },
            plain_output_ty.clone(),
        )?;
        let ok = if plain_output_ty == output_ty {
            plain.value
        } else {
            self.faultable_value(output_inner, false, None, Some(plain.value))?
        };
        self.builder.build_store(out_ptr, ok).map_err(|error| {
            format!("LLVM backend failed to store faultable builtin value: {error}")
        })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable builtin ok: {error}")
            })?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "faultable.builtin")
            .map_err(|error| format!("LLVM backend failed to load faultable builtin: {error}"))
    }

    fn emit_builtin_call(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
        output_ty: Ty,
    ) -> Result<LlvmValue<'ctx>, String> {
        self.emit_stdlib_builtin_call(name, input, output_ty)
    }

    fn emit_map(&mut self, name: &str, input: LlvmValue<'ctx>) -> Result<LlvmValue<'ctx>, String> {
        if let Ty::Faultable(inner) = input.ty.clone() {
            let plain_output_ty = match inner.as_ref() {
                Ty::Seq(item_ty) => {
                    let mapped_item_ty = self.codegen.call_output_type(name, item_ty)?;
                    Ty::Seq(Box::new(mapped_item_ty))
                }
                Ty::Stream(item_ty) => {
                    let mapped_item_ty = self.codegen.call_output_type(name, item_ty)?;
                    Ty::Stream(Box::new(mapped_item_ty))
                }
                _ => {
                    return Err(format!("`map {name}` expected Seq or Stream input"));
                }
            };
            let output_ty = Ty::Faultable(Box::new(plain_output_ty.clone()));
            let output_llvm_ty = self.types.basic_type(&output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "faultable.map")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate faultable map: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self.context.append_basic_block(function, "map.fault");
            let ok_block = self.context.append_basic_block(function, "map.ok");
            let after_block = self.context.append_basic_block(function, "map.after");
            let is_fault = self.extract_faultable_is_fault(input.value)?;
            self.builder
                .build_conditional_branch(is_fault, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable map: {error}")
                })?;
            self.builder.position_at_end(fault_block);
            let fault = self.extract_faultable_fault(input.value)?;
            let faulted = self.faultable_value(&plain_output_ty, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable map fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable map fault: {error}")
                })?;
            self.builder.position_at_end(ok_block);
            let plain_input = self.extract_faultable_value(input.value)?;
            let mapped = self.emit_map(
                name,
                LlvmValue {
                    value: plain_input,
                    ty: inner.as_ref().clone(),
                },
            )?;
            let ok = self.faultable_value(&plain_output_ty, false, None, Some(mapped.value))?;
            self.builder.build_store(out_ptr, ok).map_err(|error| {
                format!("LLVM backend failed to store faultable map value: {error}")
            })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable map ok: {error}")
                })?;
            self.builder.position_at_end(after_block);
            let value = self
                .builder
                .build_load(output_llvm_ty, out_ptr, "map.result")
                .map_err(|error| {
                    format!("LLVM backend failed to load faultable map result: {error}")
                })?;
            return Ok(LlvmValue {
                value,
                ty: output_ty,
            });
        }
        if let Ty::Stream(item_ty) = input.ty.clone() {
            let output_item_ty = self.codegen.call_output_type(name, &item_ty)?;
            let output_ty = Ty::Stream(Box::new(output_item_ty.clone()));
            let function = self.functions.get(name).copied().ok_or_else(|| {
                format!("direct LLVM backend cannot map stream with builtin `{name}`")
            })?;
            let output_item_llvm_ty = self.types.basic_type(&output_item_ty)?;
            let mut item_size = output_item_llvm_ty.size_of().ok_or_else(|| {
                format!("LLVM backend cannot compute stream item size for `{name}`")
            })?;
            if item_size.get_type().get_bit_width() != 64 {
                item_size = self
                    .builder
                    .build_int_z_extend(item_size, self.context.i64_type(), "stream.item_size")
                    .map_err(|error| {
                        format!("LLVM backend failed to extend stream item size: {error}")
                    })?;
            }
            let stream_ty = self.types.basic_type(&output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(stream_ty, "stream.map")
                .map_err(|error| format!("LLVM backend failed to allocate stream map: {error}"))?;
            let stream = input.value.into_struct_value();
            let next = self
                .builder
                .build_extract_value(stream, 5, "stream.next")
                .map_err(|error| format!("LLVM backend failed to extract stream next: {error}"))?
                .into_pointer_value();
            let has_next = self
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    next,
                    self.context.ptr_type(AddressSpace::default()).const_null(),
                    "stream.has_next",
                )
                .map_err(|error| format!("LLVM backend failed to test stream next: {error}"))?;
            let current = self.current_function()?;
            let pull_block = self.context.append_basic_block(current, "stream.map_pull");
            let direct_block = self
                .context
                .append_basic_block(current, "stream.map_direct");
            let after_block = self.context.append_basic_block(current, "stream.map_after");
            self.builder
                .build_conditional_branch(has_next, pull_block, direct_block)
                .map_err(|error| format!("LLVM backend failed to branch on stream map: {error}"))?;

            self.builder.position_at_end(pull_block);
            let (next_fn, close_fn, ctx_ty) =
                self.emit_stream_map_helper(name, &item_ty, &output_item_ty)?;
            let ctx_size = ctx_ty
                .size_of()
                .ok_or_else(|| "LLVM backend cannot compute stream map context size".to_string())?;
            let calloc = self.runtime_function(
                "calloc",
                Some(self.context.ptr_type(AddressSpace::default()).into()),
                &[
                    self.context.i64_type().into(),
                    self.context.i64_type().into(),
                ],
            )?;
            let ctx = self
                .builder
                .build_call(
                    calloc,
                    &[
                        self.context.i64_type().const_int(1, false).into(),
                        ctx_size.into(),
                    ],
                    "stream.ctx",
                )
                .map_err(|error| {
                    format!("LLVM backend failed to allocate stream context: {error}")
                })?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "calloc did not return a value".to_string())?
                .into_pointer_value();
            let upstream_ptr = self
                .builder
                .build_struct_gep(ctx_ty, ctx, 0, "stream.upstream")
                .map_err(|error| {
                    format!("LLVM backend failed to build stream context gep: {error}")
                })?;
            self.builder
                .build_store(upstream_ptr, input.value)
                .map_err(|error| {
                    format!("LLVM backend failed to store stream upstream: {error}")
                })?;
            let mut mapped_pull = stream;
            mapped_pull = self
                .builder
                .build_insert_value(mapped_pull, ctx, 3, "stream.state")
                .map_err(|error| format!("LLVM backend failed to set stream state: {error}"))?
                .into_struct_value();
            mapped_pull = self
                .builder
                .build_insert_value(
                    mapped_pull,
                    self.context.ptr_type(AddressSpace::default()).const_null(),
                    4,
                    "stream.map_fn",
                )
                .map_err(|error| format!("LLVM backend failed to clear stream map fn: {error}"))?
                .into_struct_value();
            mapped_pull = self
                .builder
                .build_insert_value(
                    mapped_pull,
                    next_fn.as_global_value().as_pointer_value(),
                    5,
                    "stream.next",
                )
                .map_err(|error| format!("LLVM backend failed to set stream next: {error}"))?
                .into_struct_value();
            mapped_pull = self
                .builder
                .build_insert_value(
                    mapped_pull,
                    close_fn.as_global_value().as_pointer_value(),
                    6,
                    "stream.close",
                )
                .map_err(|error| format!("LLVM backend failed to set stream close: {error}"))?
                .into_struct_value();
            mapped_pull = self
                .builder
                .build_insert_value(mapped_pull, item_size, 7, "stream.item_size")
                .map_err(|error| format!("LLVM backend failed to set stream item size: {error}"))?
                .into_struct_value();
            mapped_pull = self
                .builder
                .build_insert_value(
                    mapped_pull,
                    self.context.i8_type().const_zero(),
                    8,
                    "stream.closed",
                )
                .map_err(|error| format!("LLVM backend failed to set stream closed: {error}"))?
                .into_struct_value();
            self.builder
                .build_store(out_ptr, mapped_pull)
                .map_err(|error| {
                    format!("LLVM backend failed to store pull stream map: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave pull stream map: {error}")
                })?;

            self.builder.position_at_end(direct_block);
            let function =
                self.stream_direct_map_function(name, &item_ty, &output_item_ty, function)?;
            let mut mapped_direct = stream;
            mapped_direct = self
                .builder
                .build_insert_value(
                    mapped_direct,
                    function.as_global_value().as_pointer_value(),
                    4,
                    "stream.map_fn",
                )
                .map_err(|error| format!("LLVM backend failed to attach stream map fn: {error}"))?
                .into_struct_value();
            mapped_direct = self
                .builder
                .build_insert_value(mapped_direct, item_size, 7, "stream.item_size")
                .map_err(|error| format!("LLVM backend failed to set stream item size: {error}"))?
                .into_struct_value();
            self.builder
                .build_store(out_ptr, mapped_direct)
                .map_err(|error| {
                    format!("LLVM backend failed to store direct stream map: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave direct stream map: {error}")
                })?;
            self.builder.position_at_end(after_block);
            let stream = self
                .builder
                .build_load(stream_ty, out_ptr, "stream.map")
                .map_err(|error| format!("LLVM backend failed to load stream map: {error}"))?;
            return Ok(LlvmValue {
                value: stream,
                ty: output_ty,
            });
        }

        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`map {name}` expected Seq input"));
        };
        let output_item_ty = self.codegen.call_output_type(name, &item_ty)?;
        let output_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let count = self.seq_count(input.value)?;
        let output = self.emit_seq_new(&output_ty, count)?;

        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "map.loop");
        let body_block = self.context.append_basic_block(function, "map.body");
        let after_block = self.context.append_basic_block(function, "map.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate map index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize map index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to map loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load map index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "map.cond")
            .map_err(|error| format!("LLVM backend failed to compare map index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in map loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let mapped = self.emit_call(
            name,
            LlvmValue {
                value: item,
                ty: item_ty.as_ref().clone(),
            },
        )?;
        self.store_seq_item(output.value, &output_ty, i, mapped.value)?;
        let next = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment map index: {error}"))?;
        self.builder
            .build_store(i_ptr, next)
            .map_err(|error| format!("LLVM backend failed to store map index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue map loop: {error}"))?;

        self.builder.position_at_end(after_block);
        Ok(output)
    }

    fn stream_direct_map_function(
        &mut self,
        name: &str,
        input_item_ty: &Ty,
        output_item_ty: &Ty,
        function: FunctionValue<'ctx>,
    ) -> Result<FunctionValue<'ctx>, String> {
        if !matches!(input_item_ty, Ty::HttpRequest) || !matches!(output_item_ty, Ty::HttpResponse)
        {
            return Ok(function);
        }

        let wrapper_id = self.stream_helper;
        self.stream_helper += 1;
        let wrapper_name = format!("flow_http_handler_{wrapper_id}");
        if let Some(existing) = self.module.get_function(&wrapper_name) {
            return Ok(existing);
        }

        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let wrapper_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let wrapper = self.module.add_function(&wrapper_name, wrapper_ty, None);
        let output_ty = self.types.basic_type(output_item_ty)?;
        let sret = self.context.create_type_attribute(
            Attribute::get_named_enum_kind_id("sret"),
            output_ty.as_any_type_enum(),
        );
        wrapper.add_attribute(AttributeLoc::Param(0), sret);
        let restore_block = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(wrapper, "entry");
        self.builder.position_at_end(entry);

        let out_ptr = wrapper
            .get_nth_param(0)
            .ok_or_else(|| "HTTP handler wrapper missing output pointer".to_string())?
            .into_pointer_value();
        let input_ptr = wrapper
            .get_nth_param(1)
            .ok_or_else(|| "HTTP handler wrapper missing input pointer".to_string())?
            .into_pointer_value();
        let input_ty = self.types.basic_type(input_item_ty)?;
        let input = self
            .builder
            .build_load(input_ty, input_ptr, "http.request")
            .map_err(|error| format!("LLVM backend failed to load HTTP request: {error}"))?;
        let output = self
            .builder
            .build_call(function, &[input.into()], "http.response")
            .map_err(|error| format!("LLVM backend failed to call HTTP handler `{name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("HTTP handler `{name}` did not return a value"))?;
        let output = self.coerce_value_to_ty(
            LlvmValue {
                value: output,
                ty: self.codegen.call_output_type(name, input_item_ty)?,
            },
            output_item_ty,
        )?;
        self.builder
            .build_store(out_ptr, output.value)
            .map_err(|error| format!("LLVM backend failed to store HTTP response: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return HTTP wrapper: {error}"))?;

        if let Some(block) = restore_block {
            self.builder.position_at_end(block);
        }
        Ok(wrapper)
    }

    fn emit_stream_map_helper(
        &mut self,
        name: &str,
        input_item_ty: &Ty,
        output_item_ty: &Ty,
    ) -> Result<(FunctionValue<'ctx>, FunctionValue<'ctx>, StructType<'ctx>), String> {
        let helper_id = self.stream_helper;
        self.stream_helper += 1;
        let helper = format!("flow_stream_map_{helper_id}");
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let stream_ty = self
            .types
            .basic_type(&Ty::Stream(Box::new(input_item_ty.clone())))?
            .into_struct_type();
        let ctx_ty = self.context.struct_type(&[stream_ty.into()], false);
        let next_ty = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), ptr_ty.into()], false);
        let close_ty = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let next_fn = self
            .module
            .add_function(&format!("{helper}_next"), next_ty, None);
        let close_fn = self
            .module
            .add_function(&format!("{helper}_close"), close_ty, None);
        let mapped = *self
            .functions
            .get(name)
            .ok_or_else(|| format!("missing stream map function `{name}`"))?;
        let restore_block = self.builder.get_insert_block();

        let entry = self.context.append_basic_block(next_fn, "entry");
        let no_next_block = self.context.append_basic_block(next_fn, "no_next");
        let call_block = self.context.append_basic_block(next_fn, "call_next");
        let map_block = self.context.append_basic_block(next_fn, "map");
        let done_block = self.context.append_basic_block(next_fn, "done");
        self.builder.position_at_end(entry);
        let ctx = next_fn
            .get_nth_param(0)
            .ok_or_else(|| "stream map next missing ctx".to_string())?
            .into_pointer_value();
        let out_item = next_fn
            .get_nth_param(1)
            .ok_or_else(|| "stream map next missing output".to_string())?
            .into_pointer_value();
        let fault = next_fn
            .get_nth_param(2)
            .ok_or_else(|| "stream map next missing fault".to_string())?
            .into_pointer_value();
        let upstream_ptr = self
            .builder
            .build_struct_gep(ctx_ty, ctx, 0, "upstream.ptr")
            .map_err(|error| format!("LLVM backend failed to gep stream upstream: {error}"))?;
        let upstream = self
            .builder
            .build_load(stream_ty, upstream_ptr, "upstream")
            .map_err(|error| format!("LLVM backend failed to load stream upstream: {error}"))?
            .into_struct_value();
        let upstream_next = self
            .builder
            .build_extract_value(upstream, 5, "upstream.next")
            .map_err(|error| format!("LLVM backend failed to extract stream next: {error}"))?
            .into_pointer_value();
        let has_next = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                upstream_next,
                ptr_ty.const_null(),
                "has_next",
            )
            .map_err(|error| format!("LLVM backend failed to test stream next: {error}"))?;
        self.builder
            .build_conditional_branch(has_next, call_block, no_next_block)
            .map_err(|error| format!("LLVM backend failed to branch in stream next: {error}"))?;

        self.builder.position_at_end(no_next_block);
        self.builder
            .build_return(Some(&i32_ty.const_zero()))
            .map_err(|error| format!("LLVM backend failed to return stream no-next: {error}"))?;

        self.builder.position_at_end(call_block);
        let input_llvm_ty = self.types.basic_type(input_item_ty)?;
        let input_ptr = self
            .builder
            .build_alloca(input_llvm_ty, "input_item")
            .map_err(|error| {
                format!("LLVM backend failed to allocate stream input item: {error}")
            })?;
        let upstream_state = self
            .builder
            .build_extract_value(upstream, 3, "upstream.state")
            .map_err(|error| format!("LLVM backend failed to extract stream state: {error}"))?
            .into_pointer_value();
        let status = self
            .builder
            .build_indirect_call(
                next_ty,
                upstream_next,
                &[upstream_state.into(), input_ptr.into(), fault.into()],
                "upstream.status",
            )
            .map_err(|error| format!("LLVM backend failed to call upstream stream: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "stream next did not return a value".to_string())?
            .into_int_value();
        let ok = self
            .builder
            .build_int_compare(
                IntPredicate::SGT,
                status,
                i32_ty.const_zero(),
                "stream.next_ok",
            )
            .map_err(|error| format!("LLVM backend failed to test stream status: {error}"))?;
        self.builder
            .build_conditional_branch(ok, map_block, done_block)
            .map_err(|error| format!("LLVM backend failed to branch on stream status: {error}"))?;

        self.builder.position_at_end(map_block);
        let input_item = self
            .builder
            .build_load(input_llvm_ty, input_ptr, "input_item")
            .map_err(|error| format!("LLVM backend failed to load stream input item: {error}"))?;
        let output = self
            .builder
            .build_call(mapped, &[input_item.into()], "mapped_item")
            .map_err(|error| {
                format!("LLVM backend failed to call stream mapper `{name}`: {error}")
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("stream mapper `{name}` did not return a value"))?;
        let output = self.coerce_value_to_ty(
            LlvmValue {
                value: output,
                ty: self.codegen.call_output_type(name, input_item_ty)?,
            },
            output_item_ty,
        )?;
        self.builder
            .build_store(out_item, output.value)
            .map_err(|error| format!("LLVM backend failed to store mapped stream item: {error}"))?;
        self.builder
            .build_return(Some(&i32_ty.const_int(1, false)))
            .map_err(|error| {
                format!("LLVM backend failed to return mapped stream status: {error}")
            })?;

        self.builder.position_at_end(done_block);
        self.builder
            .build_return(Some(&status))
            .map_err(|error| format!("LLVM backend failed to return stream status: {error}"))?;

        let close_entry = self.context.append_basic_block(close_fn, "entry");
        let close_call = self.context.append_basic_block(close_fn, "call");
        let close_done = self.context.append_basic_block(close_fn, "done");
        self.builder.position_at_end(close_entry);
        let ctx = close_fn
            .get_nth_param(0)
            .ok_or_else(|| "stream map close missing ctx".to_string())?
            .into_pointer_value();
        let fault = close_fn
            .get_nth_param(1)
            .ok_or_else(|| "stream map close missing fault".to_string())?
            .into_pointer_value();
        let upstream_ptr = self
            .builder
            .build_struct_gep(ctx_ty, ctx, 0, "upstream.ptr")
            .map_err(|error| format!("LLVM backend failed to gep close upstream: {error}"))?;
        let upstream = self
            .builder
            .build_load(stream_ty, upstream_ptr, "upstream")
            .map_err(|error| format!("LLVM backend failed to load close upstream: {error}"))?
            .into_struct_value();
        let upstream_close = self
            .builder
            .build_extract_value(upstream, 6, "upstream.close")
            .map_err(|error| format!("LLVM backend failed to extract stream close: {error}"))?
            .into_pointer_value();
        let has_close = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                upstream_close,
                ptr_ty.const_null(),
                "has_close",
            )
            .map_err(|error| format!("LLVM backend failed to test stream close: {error}"))?;
        self.builder
            .build_conditional_branch(has_close, close_call, close_done)
            .map_err(|error| format!("LLVM backend failed to branch in stream close: {error}"))?;
        self.builder.position_at_end(close_call);
        let upstream_state = self
            .builder
            .build_extract_value(upstream, 3, "upstream.state")
            .map_err(|error| format!("LLVM backend failed to extract close state: {error}"))?
            .into_pointer_value();
        let status = self
            .builder
            .build_indirect_call(
                close_ty,
                upstream_close,
                &[upstream_state.into(), fault.into()],
                "upstream.close_status",
            )
            .map_err(|error| format!("LLVM backend failed to call upstream close: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "stream close did not return a value".to_string())?;
        self.builder.build_return(Some(&status)).map_err(|error| {
            format!("LLVM backend failed to return stream close status: {error}")
        })?;
        self.builder.position_at_end(close_done);
        self.builder
            .build_return(Some(&i32_ty.const_zero()))
            .map_err(|error| format!("LLVM backend failed to return stream close done: {error}"))?;

        if let Some(block) = restore_block {
            self.builder.position_at_end(block);
        }
        Ok((next_fn, close_fn, ctx_ty))
    }

    fn emit_filter(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        if let Ty::Faultable(inner) = input.ty.clone() {
            let Ty::Seq(_) = inner.as_ref() else {
                return Err(format!("`filter {name}` expected Seq input"));
            };
            let output_ty = Ty::Faultable(inner.clone());
            let output_llvm_ty = self.types.basic_type(&output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "faultable.filter")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate faultable filter: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self.context.append_basic_block(function, "filter.fault");
            let ok_block = self.context.append_basic_block(function, "filter.ok");
            let after_block = self.context.append_basic_block(function, "filter.after");
            let is_fault = self.extract_faultable_is_fault(input.value)?;
            self.builder
                .build_conditional_branch(is_fault, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable filter: {error}")
                })?;

            self.builder.position_at_end(fault_block);
            let fault = self.extract_faultable_fault(input.value)?;
            let faulted = self.faultable_value(inner.as_ref(), true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable filter fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable filter fault: {error}")
                })?;

            self.builder.position_at_end(ok_block);
            let plain_input = self.extract_faultable_value(input.value)?;
            let filtered = self.emit_filter(
                name,
                LlvmValue {
                    value: plain_input,
                    ty: inner.as_ref().clone(),
                },
            )?;
            let ok = self.faultable_value(inner.as_ref(), false, None, Some(filtered.value))?;
            self.builder.build_store(out_ptr, ok).map_err(|error| {
                format!("LLVM backend failed to store faultable filter value: {error}")
            })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable filter ok: {error}")
                })?;

            self.builder.position_at_end(after_block);
            let value = self
                .builder
                .build_load(output_llvm_ty, out_ptr, "filter.result")
                .map_err(|error| {
                    format!("LLVM backend failed to load faultable filter result: {error}")
                })?;
            return Ok(LlvmValue {
                value,
                ty: output_ty,
            });
        }
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`filter {name}` expected Seq input"));
        };
        let predicate_ty = self.codegen.call_output_type(name, item_ty.as_ref())?;
        if predicate_ty != Ty::Bool {
            return Err(format!(
                "`filter {name}` predicate expected Bool, found `{predicate_ty}`"
            ));
        }
        let output_ty = input.ty.clone();
        let count = self.seq_count(input.value)?;
        let output = self.emit_seq_new(&output_ty, count)?;

        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "filter.loop");
        let body_block = self.context.append_basic_block(function, "filter.body");
        let keep_block = self.context.append_basic_block(function, "filter.keep");
        let skip_block = self.context.append_basic_block(function, "filter.skip");
        let after_block = self.context.append_basic_block(function, "filter.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate filter index: {error}"))?;
        let out_i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "out_i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate filter output index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize filter index: {error}"))?;
        self.builder
            .build_store(out_i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize filter output index: {error}")
            })?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to filter loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load filter index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "filter.cond")
            .map_err(|error| format!("LLVM backend failed to compare filter index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in filter loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let keep = self.emit_call(
            name,
            LlvmValue {
                value: item,
                ty: item_ty.as_ref().clone(),
            },
        )?;
        let keep = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                keep.value.into_int_value(),
                self.context.i8_type().const_zero(),
                "keep",
            )
            .map_err(|error| format!("LLVM backend failed to compare filter predicate: {error}"))?;
        self.builder
            .build_conditional_branch(keep, keep_block, skip_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on filter predicate: {error}")
            })?;

        self.builder.position_at_end(keep_block);
        let out_i = self
            .builder
            .build_load(self.context.i64_type(), out_i_ptr, "out_i")
            .map_err(|error| format!("LLVM backend failed to load filter output index: {error}"))?
            .into_int_value();
        self.store_seq_item(output.value, &output_ty, out_i, item)?;
        let next_out_i = self
            .builder
            .build_int_add(
                out_i,
                self.context.i64_type().const_int(1, false),
                "next_out",
            )
            .map_err(|error| format!("LLVM backend failed to increment filter output: {error}"))?;
        self.builder
            .build_store(out_i_ptr, next_out_i)
            .map_err(|error| {
                format!("LLVM backend failed to store filter output index: {error}")
            })?;
        self.builder
            .build_unconditional_branch(skip_block)
            .map_err(|error| format!("LLVM backend failed to leave filter keep: {error}"))?;

        self.builder.position_at_end(skip_block);
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment filter index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store filter index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue filter loop: {error}"))?;

        self.builder.position_at_end(after_block);
        let out_count = self
            .builder
            .build_load(self.context.i64_type(), out_i_ptr, "out_count")
            .map_err(|error| format!("LLVM backend failed to load filter count: {error}"))?;
        let filtered = self
            .builder
            .build_insert_value(output.value.into_struct_value(), out_count, 0, "filtered")
            .map_err(|error| format!("LLVM backend failed to set filter count: {error}"))?;
        Ok(LlvmValue {
            value: filtered.as_basic_value_enum(),
            ty: output_ty,
        })
    }

    fn emit_fault_map(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<(LlvmValue<'ctx>, LlvmValue<'ctx>), String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`fault map {name}` expected Seq input"));
        };
        let output_item_ty = self.codegen.call_output_type(name, item_ty.as_ref())?;
        let Ty::Faultable(ok_item_ty) = output_item_ty else {
            return Err(format!("`fault map {name}` expected faultable output"));
        };
        let ok_ty = Ty::Seq(ok_item_ty.clone());
        let fault_ty = Ty::Seq(Box::new(Ty::Fault));
        let count = self.seq_count(input.value)?;
        let ok_seq = self.emit_seq_new(&ok_ty, count)?;
        let fault_seq = self.emit_seq_new(&fault_ty, count)?;

        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "fault_map.loop");
        let body_block = self.context.append_basic_block(function, "fault_map.body");
        let fault_block = self.context.append_basic_block(function, "fault_map.fault");
        let ok_block = self.context.append_basic_block(function, "fault_map.ok");
        let next_block = self.context.append_basic_block(function, "fault_map.next");
        let after_block = self.context.append_basic_block(function, "fault_map.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate fault map index: {error}"))?;
        let ok_i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "ok_i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate fault map ok index: {error}")
            })?;
        let fault_i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "fault_i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate fault map fault index: {error}")
            })?;
        for ptr in [i_ptr, ok_i_ptr, fault_i_ptr] {
            self.builder
                .build_store(ptr, self.context.i64_type().const_zero())
                .map_err(|error| {
                    format!("LLVM backend failed to initialize fault map index: {error}")
                })?;
        }
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to fault map loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load fault map index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "fault_map.cond")
            .map_err(|error| format!("LLVM backend failed to compare fault map index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in fault map loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let result = self.emit_call(
            name,
            LlvmValue {
                value: item,
                ty: item_ty.as_ref().clone(),
            },
        )?;
        let is_fault = self.extract_faultable_is_fault(result.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on fault map result: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let mut fault = self.extract_faultable_fault(result.value)?;
        if matches!(
            self.codegen.canonical_name(name).as_str(),
            "parse_real" | "parse_int"
        ) {
            let runtime_fault = self.value_to_runtime_arg(fault, &Ty::Fault)?;
            let fn_value = self.runtime_function(
                "fa_fault_with_line",
                Some(self.runtime_pair_type().into()),
                &[
                    self.context.i64_type().into(),
                    self.runtime_pair_type().into(),
                ],
            )?;
            let line = self
                .builder
                .build_int_add(i, self.context.i64_type().const_int(1, false), "line")
                .map_err(|error| format!("LLVM backend failed to build fault line: {error}"))?;
            let with_line = self
                .builder
                .build_call(
                    fn_value,
                    &[line.into(), runtime_fault.into()],
                    "fault_with_line",
                )
                .map_err(|error| {
                    format!("LLVM backend failed to call fa_fault_with_line: {error}")
                })?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "fa_fault_with_line did not return a value".to_string())?;
            fault = self.runtime_pair_to_value(with_line, &Ty::Fault)?;
        }
        let fault_i = self
            .builder
            .build_load(self.context.i64_type(), fault_i_ptr, "fault_i")
            .map_err(|error| format!("LLVM backend failed to load fault index: {error}"))?
            .into_int_value();
        self.store_seq_item(fault_seq.value, &fault_ty, fault_i, fault)?;
        let next_fault_i = self
            .builder
            .build_int_add(
                fault_i,
                self.context.i64_type().const_int(1, false),
                "next_fault",
            )
            .map_err(|error| format!("LLVM backend failed to increment fault index: {error}"))?;
        self.builder
            .build_store(fault_i_ptr, next_fault_i)
            .map_err(|error| format!("LLVM backend failed to store fault index: {error}"))?;
        self.builder
            .build_unconditional_branch(next_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave fault map fault block: {error}")
            })?;

        self.builder.position_at_end(ok_block);
        let ok_value = self.extract_faultable_value(result.value)?;
        let ok_i = self
            .builder
            .build_load(self.context.i64_type(), ok_i_ptr, "ok_i")
            .map_err(|error| format!("LLVM backend failed to load ok index: {error}"))?
            .into_int_value();
        self.store_seq_item(ok_seq.value, &ok_ty, ok_i, ok_value)?;
        let next_ok_i = self
            .builder
            .build_int_add(ok_i, self.context.i64_type().const_int(1, false), "next_ok")
            .map_err(|error| format!("LLVM backend failed to increment ok index: {error}"))?;
        self.builder
            .build_store(ok_i_ptr, next_ok_i)
            .map_err(|error| format!("LLVM backend failed to store ok index: {error}"))?;
        self.builder
            .build_unconditional_branch(next_block)
            .map_err(|error| format!("LLVM backend failed to leave fault map ok block: {error}"))?;

        self.builder.position_at_end(next_block);
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| {
                format!("LLVM backend failed to increment fault map index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store fault map index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue fault map loop: {error}"))?;

        self.builder.position_at_end(after_block);
        let ok_count = self
            .builder
            .build_load(self.context.i64_type(), ok_i_ptr, "ok_count")
            .map_err(|error| format!("LLVM backend failed to load ok count: {error}"))?;
        let fault_count = self
            .builder
            .build_load(self.context.i64_type(), fault_i_ptr, "fault_count")
            .map_err(|error| format!("LLVM backend failed to load fault count: {error}"))?;
        let ok_value = self
            .builder
            .build_insert_value(ok_seq.value.into_struct_value(), ok_count, 0, "ok_seq")
            .map_err(|error| format!("LLVM backend failed to set ok sequence count: {error}"))?
            .as_basic_value_enum();
        let fault_value = self
            .builder
            .build_insert_value(
                fault_seq.value.into_struct_value(),
                fault_count,
                0,
                "fault_seq",
            )
            .map_err(|error| format!("LLVM backend failed to set fault sequence count: {error}"))?
            .as_basic_value_enum();
        Ok((
            LlvmValue {
                value: ok_value,
                ty: ok_ty,
            },
            LlvmValue {
                value: fault_value,
                ty: fault_ty,
            },
        ))
    }

    fn emit_repeat(
        &mut self,
        node: &str,
        input: LlvmValue<'ctx>,
        count: LlvmValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let Ty::Int = count.ty else {
            return Err(format!(
                "`repeat {node}` count expected Int, found `{}`",
                count.ty
            ));
        };
        let value_ty = self.types.basic_type(&input.ty)?;
        let state_ptr = self
            .builder
            .build_alloca(value_ty, "repeat.state")
            .map_err(|error| format!("LLVM backend failed to allocate repeat state: {error}"))?;
        self.builder
            .build_store(state_ptr, input.value)
            .map_err(|error| format!("LLVM backend failed to initialize repeat state: {error}"))?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "repeat.loop");
        let body_block = self.context.append_basic_block(function, "repeat.body");
        let after_block = self.context.append_basic_block(function, "repeat.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate repeat index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize repeat index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to repeat loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load repeat index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(
                IntPredicate::SLT,
                i,
                count.value.into_int_value(),
                "repeat.cond",
            )
            .map_err(|error| format!("LLVM backend failed to compare repeat index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in repeat loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let state = self
            .builder
            .build_load(value_ty, state_ptr, "state")
            .map_err(|error| format!("LLVM backend failed to load repeat state: {error}"))?;
        let next_state = self.emit_call(
            node,
            LlvmValue {
                value: state,
                ty: input.ty.clone(),
            },
        )?;
        self.builder
            .build_store(state_ptr, next_state.value)
            .map_err(|error| format!("LLVM backend failed to store repeat state: {error}"))?;
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment repeat index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store repeat index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue repeat loop: {error}"))?;

        self.builder.position_at_end(after_block);
        let result = self
            .builder
            .build_load(value_ty, state_ptr, "repeat.result")
            .map_err(|error| format!("LLVM backend failed to load repeat result: {error}"))?;
        Ok(LlvmValue {
            value: result,
            ty: input.ty,
        })
    }

    fn emit_scan(
        &mut self,
        op: &str,
        input: LlvmValue<'ctx>,
        identity: LlvmValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`scan {op}` expected Seq input"));
        };
        let item_ty = item_ty.as_ref().clone();
        let output_ty = Ty::Seq(Box::new(item_ty.clone()));
        let count = self.seq_count(input.value)?;
        let output = self.emit_seq_new(&output_ty, count)?;
        let item_llvm_ty = self.types.basic_type(&item_ty)?;
        let state_ptr = self
            .builder
            .build_alloca(item_llvm_ty, "scan.state")
            .map_err(|error| format!("LLVM backend failed to allocate scan state: {error}"))?;
        let identity = self.coerce_value_to_ty(identity, &item_ty)?;
        self.builder
            .build_store(state_ptr, identity.value)
            .map_err(|error| format!("LLVM backend failed to initialize scan state: {error}"))?;

        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "scan.loop");
        let body_block = self.context.append_basic_block(function, "scan.body");
        let after_block = self.context.append_basic_block(function, "scan.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate scan index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize scan index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to scan loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load scan index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "scan.cond")
            .map_err(|error| format!("LLVM backend failed to compare scan index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in scan loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let state = self
            .builder
            .build_load(item_llvm_ty, state_ptr, "state")
            .map_err(|error| format!("LLVM backend failed to load scan state: {error}"))?;
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let pair_ty = Ty::Tuple(vec![item_ty.clone(), item_ty.clone()]);
        let mut pair = self
            .types
            .basic_type(&pair_ty)?
            .into_struct_type()
            .const_zero();
        pair = self
            .builder
            .build_insert_value(pair, state, 0, "scan.pair")
            .map_err(|error| format!("LLVM backend failed to build scan pair: {error}"))?
            .into_struct_value();
        pair = self
            .builder
            .build_insert_value(pair, item, 1, "scan.pair")
            .map_err(|error| format!("LLVM backend failed to build scan pair: {error}"))?
            .into_struct_value();
        let next_state = self.emit_call(
            op,
            LlvmValue {
                value: pair.into(),
                ty: pair_ty,
            },
        )?;
        self.builder
            .build_store(state_ptr, next_state.value)
            .map_err(|error| format!("LLVM backend failed to store scan state: {error}"))?;
        self.store_seq_item(output.value, &output_ty, i, next_state.value)?;
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment scan index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store scan index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue scan loop: {error}"))?;

        self.builder.position_at_end(after_block);
        Ok(output)
    }

    fn emit_reduce(
        &mut self,
        op: &str,
        input: LlvmValue<'ctx>,
        identity: LlvmValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`reduce {op}` expected Seq input"));
        };
        let canonical = self.codegen.canonical_name(op);
        if canonical == "add" {
            return self.emit_reduce_add_llvm(op, input, item_ty.as_ref().clone(), identity);
        }
        if canonical == "min" || canonical == "max" {
            return self.emit_reduce_min_max_llvm(
                &canonical,
                input,
                item_ty.as_ref().clone(),
                identity,
            );
        }
        let output_ty = self.codegen.call_output_type(op, &item_ty)?;
        if canonical == "concat_bytes" {
            let value = self.emit_reduce_concat_bytes(input, identity)?;
            return Ok(LlvmValue {
                value,
                ty: output_ty,
            });
        }
        let state_ptr = self
            .builder
            .build_alloca(self.types.basic_type(&output_ty)?, "reduce.state")
            .map_err(|error| format!("LLVM backend failed to allocate reduce state: {error}"))?;
        self.builder
            .build_store(state_ptr, identity.value)
            .map_err(|error| format!("LLVM backend failed to initialize reduce state: {error}"))?;
        let count = self.seq_count(input.value)?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "reduce.loop");
        let body_block = self.context.append_basic_block(function, "reduce.body");
        let after_block = self.context.append_basic_block(function, "reduce.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate reduce index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize reduce index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to reduce loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load reduce index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "reduce.cond")
            .map_err(|error| format!("LLVM backend failed to compare reduce index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in reduce loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let state = self
            .builder
            .build_load(self.types.basic_type(&output_ty)?, state_ptr, "state")
            .map_err(|error| format!("LLVM backend failed to load reduce state: {error}"))?;
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let pair_ty = Ty::Tuple(vec![output_ty.clone(), item_ty.as_ref().clone()]);
        let mut pair = self
            .types
            .basic_type(&pair_ty)?
            .into_struct_type()
            .const_zero();
        pair = self
            .builder
            .build_insert_value(pair, state, 0, "pair")
            .map_err(|error| format!("LLVM backend failed to build reduce pair: {error}"))?
            .into_struct_value();
        pair = self
            .builder
            .build_insert_value(pair, item, 1, "pair")
            .map_err(|error| format!("LLVM backend failed to build reduce pair: {error}"))?
            .into_struct_value();
        let next_state = self.emit_call(
            op,
            LlvmValue {
                value: pair.into(),
                ty: pair_ty,
            },
        )?;
        self.builder
            .build_store(state_ptr, next_state.value)
            .map_err(|error| format!("LLVM backend failed to store reduce state: {error}"))?;
        let next = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment reduce index: {error}"))?;
        self.builder
            .build_store(i_ptr, next)
            .map_err(|error| format!("LLVM backend failed to store reduce index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue reduce loop: {error}"))?;

        self.builder.position_at_end(after_block);
        let result = self
            .builder
            .build_load(
                self.types.basic_type(&output_ty)?,
                state_ptr,
                "reduce.result",
            )
            .map_err(|error| format!("LLVM backend failed to load reduce result: {error}"))?;
        Ok(LlvmValue {
            value: result,
            ty: output_ty,
        })
    }

    fn emit_reduce_add_llvm(
        &mut self,
        _op: &str,
        input: LlvmValue<'ctx>,
        item_ty: Ty,
        identity: LlvmValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let (plain_ty, faultable) = match item_ty {
            Ty::Faultable(inner) => (*inner, true),
            other => (other, false),
        };
        let output_ty = if faultable {
            Ty::Faultable(Box::new(plain_ty.clone()))
        } else {
            plain_ty.clone()
        };
        let output_llvm_ty = self.types.basic_type(&output_ty)?;
        let state_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "reduce.state")
            .map_err(|error| format!("LLVM backend failed to allocate reduce state: {error}"))?;
        let initial = if faultable {
            self.faultable_value(&plain_ty, false, None, Some(identity.value))?
        } else {
            identity.value
        };
        self.builder
            .build_store(state_ptr, initial)
            .map_err(|error| format!("LLVM backend failed to initialize reduce state: {error}"))?;

        let count = self.seq_count(input.value)?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "reduce.add.loop");
        let body_block = self.context.append_basic_block(function, "reduce.add.body");
        let after_block = self
            .context
            .append_basic_block(function, "reduce.add.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate reduce index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize reduce index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to reduce loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load reduce index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "reduce.cond")
            .map_err(|error| format!("LLVM backend failed to compare reduce index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in reduce loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        if faultable {
            let state = self
                .builder
                .build_load(output_llvm_ty, state_ptr, "state")
                .map_err(|error| format!("LLVM backend failed to load reduce state: {error}"))?;
            let state_fault = self.extract_faultable_is_fault(state)?;
            let item_fault = self.extract_faultable_is_fault(item)?;
            let any_fault = self
                .builder
                .build_or(state_fault, item_fault, "any.fault")
                .map_err(|error| format!("LLVM backend failed to combine fault flags: {error}"))?;
            let state_value = self.extract_faultable_value(state)?;
            let item_value = self.extract_faultable_value(item)?;
            let sum_value = self.emit_add_values(state_value, item_value, &plain_ty)?;
            let item_fault_value = self.extract_faultable_fault(item)?;
            let state_fault_value = self.extract_faultable_fault(state)?;
            let selected_fault = self
                .builder
                .build_select(item_fault, item_fault_value, state_fault_value, "fault")
                .map_err(|error| format!("LLVM backend failed to select reduce fault: {error}"))?;
            let next =
                self.faultable_value(&plain_ty, true, Some(selected_fault), Some(sum_value))?;
            let any_fault_i8 = self
                .builder
                .build_int_z_extend(any_fault, self.context.i8_type(), "fault.flag")
                .map_err(|error| format!("LLVM backend failed to extend fault flag: {error}"))?;
            let next = self
                .builder
                .build_insert_value(next.into_struct_value(), any_fault_i8, 0, "fault.flag")
                .map_err(|error| format!("LLVM backend failed to set reduce fault flag: {error}"))?
                .as_basic_value_enum();
            self.builder
                .build_store(state_ptr, next)
                .map_err(|error| format!("LLVM backend failed to store reduce state: {error}"))?;
        } else {
            let state = self
                .builder
                .build_load(output_llvm_ty, state_ptr, "state")
                .map_err(|error| format!("LLVM backend failed to load reduce state: {error}"))?;
            let sum = self.emit_add_values(state, item, &plain_ty)?;
            self.builder
                .build_store(state_ptr, sum)
                .map_err(|error| format!("LLVM backend failed to store reduce state: {error}"))?;
        }
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment reduce index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store reduce index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue reduce loop: {error}"))?;

        self.builder.position_at_end(after_block);
        let result = self
            .builder
            .build_load(output_llvm_ty, state_ptr, "reduce.result")
            .map_err(|error| format!("LLVM backend failed to load reduce result: {error}"))?;
        Ok(LlvmValue {
            value: result,
            ty: output_ty,
        })
    }

    fn emit_reduce_min_max_llvm(
        &mut self,
        op: &str,
        input: LlvmValue<'ctx>,
        item_ty: Ty,
        identity: LlvmValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        if matches!(item_ty, Ty::Faultable(_)) {
            return Err(format!(
                "direct LLVM backend does not yet support faultable reduce {op}"
            ));
        }
        let output_llvm_ty = self.types.basic_type(&item_ty)?;
        let state_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "reduce.state")
            .map_err(|error| format!("LLVM backend failed to allocate reduce state: {error}"))?;
        let identity = self.coerce_value_to_ty(identity, &item_ty)?;
        self.builder
            .build_store(state_ptr, identity.value)
            .map_err(|error| format!("LLVM backend failed to initialize reduce state: {error}"))?;
        let count = self.seq_count(input.value)?;
        let function = self.current_function()?;
        let loop_block = self
            .context
            .append_basic_block(function, "reduce.minmax.loop");
        let body_block = self
            .context
            .append_basic_block(function, "reduce.minmax.body");
        let after_block = self
            .context
            .append_basic_block(function, "reduce.minmax.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate reduce index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize reduce index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to reduce loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load reduce index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "reduce.cond")
            .map_err(|error| format!("LLVM backend failed to compare reduce index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in reduce loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let state = self
            .builder
            .build_load(output_llvm_ty, state_ptr, "state")
            .map_err(|error| format!("LLVM backend failed to load reduce state: {error}"))?;
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let next = self.emit_min_max_values(op, state, item, &item_ty)?;
        self.builder
            .build_store(state_ptr, next)
            .map_err(|error| format!("LLVM backend failed to store reduce state: {error}"))?;
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment reduce index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store reduce index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue reduce loop: {error}"))?;

        self.builder.position_at_end(after_block);
        let result = self
            .builder
            .build_load(output_llvm_ty, state_ptr, "reduce.result")
            .map_err(|error| format!("LLVM backend failed to load reduce result: {error}"))?;
        Ok(LlvmValue {
            value: result,
            ty: item_ty,
        })
    }

    fn emit_numeric_binary(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = &input.ty else {
            return Err(format!("`{name}` expected tuple input"));
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err(format!("`{name}` expected pair input"));
        };
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        if matches!(left_ty, Ty::Real) || matches!(right_ty, Ty::Real) {
            let left = self.to_real(left, left_ty)?;
            let right = self.to_real(right, right_ty)?;
            let result = match name {
                "add" => self.builder.build_float_add(left, right, "add"),
                "sub" => self.builder.build_float_sub(left, right, "sub"),
                "mul" => self.builder.build_float_mul(left, right, "mul"),
                "div" => self.builder.build_float_div(left, right, "div"),
                "rem" => self.builder.build_float_rem(left, right, "rem"),
                "min" => {
                    let cmp = self
                        .builder
                        .build_float_compare(inkwell::FloatPredicate::OLT, left, right, "min")
                        .map_err(|error| format!("LLVM backend failed to compare min: {error}"))?;
                    return self
                        .builder
                        .build_select(cmp, left, right, "min")
                        .map_err(|error| format!("LLVM backend failed to select min: {error}"));
                }
                "max" => {
                    let cmp = self
                        .builder
                        .build_float_compare(inkwell::FloatPredicate::OGT, left, right, "max")
                        .map_err(|error| format!("LLVM backend failed to compare max: {error}"))?;
                    return self
                        .builder
                        .build_select(cmp, left, right, "max")
                        .map_err(|error| format!("LLVM backend failed to select max: {error}"));
                }
                _ => unreachable!(),
            }
            .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?;
            Ok(result.into())
        } else {
            let left = left.into_int_value();
            let right = right.into_int_value();
            let result = match name {
                "add" => self.builder.build_int_add(left, right, "add"),
                "sub" => self.builder.build_int_sub(left, right, "sub"),
                "mul" => self.builder.build_int_mul(left, right, "mul"),
                "div" => self.builder.build_int_signed_div(left, right, "div"),
                "rem" => self.builder.build_int_signed_rem(left, right, "rem"),
                "min" => {
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::SLT, left, right, "min")
                        .map_err(|error| format!("LLVM backend failed to compare min: {error}"))?;
                    return self
                        .builder
                        .build_select(cmp, left, right, "min")
                        .map_err(|error| format!("LLVM backend failed to select min: {error}"));
                }
                "max" => {
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::SGT, left, right, "max")
                        .map_err(|error| format!("LLVM backend failed to compare max: {error}"))?;
                    return self
                        .builder
                        .build_select(cmp, left, right, "max")
                        .map_err(|error| format!("LLVM backend failed to select max: {error}"));
                }
                _ => unreachable!(),
            }
            .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?;
            Ok(result.into())
        }
    }

    fn emit_from_int(&mut self, input: LlvmValue<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        if input.ty != Ty::Int {
            return Err(format!("from_int expected Int, found `{}`", input.ty));
        }
        Ok(self
            .builder
            .build_signed_int_to_float(
                input.value.into_int_value(),
                self.context.f64_type(),
                "from_int",
            )
            .map_err(|error| format!("LLVM backend failed to build from_int: {error}"))?
            .into())
    }

    fn emit_numeric_unary(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match name {
            "neg" => match input.ty {
                Ty::Int => Ok(self
                    .builder
                    .build_int_neg(input.value.into_int_value(), "neg")
                    .map_err(|error| format!("LLVM backend failed to build neg: {error}"))?
                    .into()),
                Ty::Real => Ok(self
                    .builder
                    .build_float_neg(input.value.into_float_value(), "neg")
                    .map_err(|error| format!("LLVM backend failed to build neg: {error}"))?
                    .into()),
                ref other => Err(format!("neg expected numeric input, found `{other}`")),
            },
            "abs" => match input.ty {
                Ty::Int => {
                    let value = input.value.into_int_value();
                    let negative = self
                        .builder
                        .build_int_compare(
                            IntPredicate::SLT,
                            value,
                            self.context.i64_type().const_zero(),
                            "abs.negative",
                        )
                        .map_err(|error| format!("LLVM backend failed to compare abs: {error}"))?;
                    let negated = self
                        .builder
                        .build_int_neg(value, "abs.neg")
                        .map_err(|error| format!("LLVM backend failed to negate abs: {error}"))?;
                    self.builder
                        .build_select(negative, negated, value, "abs")
                        .map_err(|error| format!("LLVM backend failed to select abs: {error}"))
                }
                Ty::Real => {
                    let value = input.value.into_float_value();
                    let negative = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OLT,
                            value,
                            self.context.f64_type().const_float(0.0),
                            "abs.negative",
                        )
                        .map_err(|error| format!("LLVM backend failed to compare abs: {error}"))?;
                    let negated = self
                        .builder
                        .build_float_neg(value, "abs.neg")
                        .map_err(|error| format!("LLVM backend failed to negate abs: {error}"))?;
                    self.builder
                        .build_select(negative, negated, value, "abs")
                        .map_err(|error| format!("LLVM backend failed to select abs: {error}"))
                }
                ref other => Err(format!("abs expected numeric input, found `{other}`")),
            },
            "sqrt" | "exp" | "sin" | "cos" => {
                if output_ty != &Ty::Real {
                    return Err(format!("{name} expected Real output, found `{output_ty}`"));
                }
                let value = self.to_real(input.value, &input.ty)?;
                let fn_value = self.runtime_function(
                    name,
                    Some(self.context.f64_type().into()),
                    &[self.context.f64_type().into()],
                )?;
                self.builder
                    .build_call(fn_value, &[value.into()], name)
                    .map_err(|error| format!("LLVM backend failed to call `{name}`: {error}"))?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("runtime function `{name}` did not return a value"))
            }
            _ => unreachable!(),
        }
    }

    fn emit_int_binary(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = &input.ty else {
            return Err(format!("`{name}` expected tuple input"));
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err(format!("`{name}` expected pair input"));
        };
        if left_ty != &Ty::Int || right_ty != &Ty::Int {
            return Err(format!("`{name}` expected Int operands"));
        }
        let left = self.extract_tuple_field(&input, 0)?.into_int_value();
        let right = self.extract_tuple_field(&input, 1)?.into_int_value();
        let result = match name {
            "bit_and" => self.builder.build_and(left, right, "bit_and"),
            "bit_or" => self.builder.build_or(left, right, "bit_or"),
            "bit_xor" => self.builder.build_xor(left, right, "bit_xor"),
            "bit_shl" => self.builder.build_left_shift(left, right, "bit_shl"),
            "bit_shr" => self
                .builder
                .build_right_shift(left, right, false, "bit_shr"),
            _ => unreachable!(),
        }
        .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?;
        Ok(result.into())
    }

    fn emit_compare(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = &input.ty else {
            return Err(format!("`{name}` expected tuple input"));
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err(format!("`{name}` expected pair input"));
        };
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        let bit = if matches!(left_ty, Ty::Real) || matches!(right_ty, Ty::Real) {
            let left = self.to_real(left, left_ty)?;
            let right = self.to_real(right, right_ty)?;
            let pred = match name {
                "eq" => inkwell::FloatPredicate::OEQ,
                "lt" => inkwell::FloatPredicate::OLT,
                "gt" => inkwell::FloatPredicate::OGT,
                "le" => inkwell::FloatPredicate::OLE,
                "ge" => inkwell::FloatPredicate::OGE,
                _ => unreachable!(),
            };
            self.builder
                .build_float_compare(pred, left, right, "cmp")
                .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?
        } else {
            let pred = match name {
                "eq" => IntPredicate::EQ,
                "lt" => IntPredicate::SLT,
                "gt" => IntPredicate::SGT,
                "le" => IntPredicate::SLE,
                "ge" => IntPredicate::SGE,
                _ => unreachable!(),
            };
            self.builder
                .build_int_compare(pred, left.into_int_value(), right.into_int_value(), "cmp")
                .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?
        };
        Ok(self
            .builder
            .build_int_z_extend(bit, self.context.i8_type(), "bool")
            .map_err(|error| format!("LLVM backend failed to extend bool: {error}"))?
            .into())
    }

    fn emit_bool_binary(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let left = self.extract_tuple_field(&input, 0)?.into_int_value();
        let right = self.extract_tuple_field(&input, 1)?.into_int_value();
        let result = match name {
            "and" => self.builder.build_and(left, right, "and"),
            "or" => self.builder.build_or(left, right, "or"),
            "xor" => self.builder.build_xor(left, right, "xor"),
            _ => unreachable!(),
        }
        .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?;
        Ok(result.into())
    }

    fn emit_select(&mut self, input: LlvmValue<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let cond = self.extract_tuple_field(&input, 0)?.into_int_value();
        let then_value = self.extract_tuple_field(&input, 1)?;
        let else_value = self.extract_tuple_field(&input, 2)?;
        let bit = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                cond,
                self.context.i8_type().const_zero(),
                "cond",
            )
            .map_err(|error| format!("LLVM backend failed to build select condition: {error}"))?;
        self.builder
            .build_select(bit, then_value, else_value, "select")
            .map_err(|error| format!("LLVM backend failed to build select: {error}"))
    }

    fn emit_tuple_project(
        &mut self,
        input: LlvmValue<'ctx>,
        index: usize,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some(plain_input_ty) = unwrap_faultable_tuple(&input.ty) {
            let wrapped = self.coerce_faultable_tuple_to_faultable(input, &plain_input_ty)?;
            return self.emit_tuple_project(wrapped, index, output_ty);
        }
        if let Ty::Faultable(inner) = input.ty.clone() {
            let Ty::Tuple(items) = inner.as_ref() else {
                return Err("tuple projection expected tuple input".to_string());
            };
            let field_ty = items
                .get(index)
                .ok_or_else(|| "tuple projection index out of bounds".to_string())?;
            let Ty::Faultable(output_inner) = output_ty else {
                return Err(format!(
                    "faultable tuple projection expected faultable output, found `{output_ty}`"
                ));
            };
            if output_inner.as_ref() != field_ty {
                return Err(format!(
                    "tuple projection expected `{}` output, found `{output_ty}`",
                    Ty::Faultable(Box::new(field_ty.clone()))
                ));
            }
            let faultable = input.value.into_struct_value();
            let flag = self
                .builder
                .build_extract_value(faultable, 0, "is_fault")
                .map_err(|error| {
                    format!("LLVM backend failed to extract tuple projection fault flag: {error}")
                })?
                .into_int_value();
            let fault = self
                .builder
                .build_extract_value(faultable, 1, "fault")
                .map_err(|error| {
                    format!("LLVM backend failed to extract tuple projection fault: {error}")
                })?;
            let inner_value = self
                .builder
                .build_extract_value(faultable, 2, "value")
                .map_err(|error| {
                    format!("LLVM backend failed to extract tuple projection value: {error}")
                })?;
            let field = self
                .builder
                .build_extract_value(inner_value.into_struct_value(), index as u32, "field")
                .map_err(|error| {
                    format!("LLVM backend failed to extract tuple projection field: {error}")
                })?;
            return self.faultable_value_with_flag(field_ty, flag, fault, field);
        }
        self.extract_tuple_field(&input, index as u32)
    }

    fn emit_add_values(
        &mut self,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            Ty::Real => Ok(self
                .builder
                .build_float_add(left.into_float_value(), right.into_float_value(), "add")
                .map_err(|error| format!("LLVM backend failed to build real add: {error}"))?
                .into()),
            Ty::Int => Ok(self
                .builder
                .build_int_add(left.into_int_value(), right.into_int_value(), "add")
                .map_err(|error| format!("LLVM backend failed to build int add: {error}"))?
                .into()),
            other => Err(format!("add expected numeric value, found `{other}`")),
        }
    }

    fn emit_min_max_values(
        &mut self,
        op: &str,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            Ty::Real => {
                let pred = if op == "min" {
                    inkwell::FloatPredicate::OLT
                } else {
                    inkwell::FloatPredicate::OGT
                };
                let left = left.into_float_value();
                let right = right.into_float_value();
                let cmp = self
                    .builder
                    .build_float_compare(pred, left, right, op)
                    .map_err(|error| format!("LLVM backend failed to compare {op}: {error}"))?;
                self.builder
                    .build_select(cmp, left, right, op)
                    .map_err(|error| format!("LLVM backend failed to select {op}: {error}"))
            }
            Ty::Int => {
                let pred = if op == "min" {
                    IntPredicate::SLT
                } else {
                    IntPredicate::SGT
                };
                let left = left.into_int_value();
                let right = right.into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(pred, left, right, op)
                    .map_err(|error| format!("LLVM backend failed to compare {op}: {error}"))?;
                self.builder
                    .build_select(cmp, left, right, op)
                    .map_err(|error| format!("LLVM backend failed to select {op}: {error}"))
            }
            other => Err(format!("{op} expected numeric value, found `{other}`")),
        }
    }

    fn coerce_value_to_ty(
        &mut self,
        value: LlvmValue<'ctx>,
        expected_ty: &Ty,
    ) -> Result<LlvmValue<'ctx>, String> {
        if &value.ty == expected_ty {
            return Ok(value);
        }

        match (expected_ty, value.ty.clone()) {
            (Ty::Real, Ty::Int) => {
                let real = self
                    .builder
                    .build_signed_int_to_float(
                        value.value.into_int_value(),
                        self.context.f64_type(),
                        "int_to_real",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to coerce Int to Real: {error}")
                    })?;
                Ok(LlvmValue {
                    value: real.into(),
                    ty: Ty::Real,
                })
            }
            (Ty::Faultable(inner), actual)
                if unwrap_faultable_tuple(&actual)
                    .as_ref()
                    .is_some_and(|unwrapped| unwrapped == inner.as_ref()) =>
            {
                return self.coerce_faultable_tuple_to_faultable(value, inner);
            }
            (Ty::Faultable(inner), actual)
                if inner.as_ref() == &actual
                    || (inner.as_ref() == &Ty::Real && actual == Ty::Int) =>
            {
                let plain = self.coerce_value_to_ty(value, inner)?;
                let wrapped = self.faultable_value(inner, false, None, Some(plain.value))?;
                Ok(LlvmValue {
                    value: wrapped,
                    ty: expected_ty.clone(),
                })
            }
            (Ty::Tuple(expected_items), Ty::Tuple(actual_items))
                if expected_items.len() == actual_items.len() =>
            {
                let mut out = self
                    .types
                    .basic_type(expected_ty)?
                    .into_struct_type()
                    .const_zero();
                for (index, (expected_item, actual_item)) in
                    expected_items.iter().zip(actual_items.iter()).enumerate()
                {
                    let field = self
                        .builder
                        .build_extract_value(value.value.into_struct_value(), index as u32, "field")
                        .map_err(|error| {
                            format!("LLVM backend failed to extract value for coercion: {error}")
                        })?;
                    let field = self.coerce_value_to_ty(
                        LlvmValue {
                            value: field,
                            ty: actual_item.clone(),
                        },
                        expected_item,
                    )?;
                    out = self
                        .builder
                        .build_insert_value(out, field.value, index as u32, "field")
                        .map_err(|error| {
                            format!("LLVM backend failed to insert coerced tuple field: {error}")
                        })?
                        .into_struct_value();
                }
                Ok(LlvmValue {
                    value: out.into(),
                    ty: expected_ty.clone(),
                })
            }
            (Ty::Seq(expected_item), Ty::Seq(actual_item))
                if expected_item.as_ref() == actual_item.as_ref() =>
            {
                Ok(LlvmValue {
                    value: value.value,
                    ty: expected_ty.clone(),
                })
            }
            _ => Err(format!(
                "direct LLVM backend cannot coerce `{}` to `{expected_ty}`",
                value.ty
            )),
        }
    }

    fn coerce_faultable_tuple_to_faultable(
        &mut self,
        value: LlvmValue<'ctx>,
        inner_ty: &Ty,
    ) -> Result<LlvmValue<'ctx>, String> {
        let Ty::Tuple(actual_items) = value.ty.clone() else {
            return Err(format!(
                "expected tuple with faultable fields, found `{}`",
                value.ty
            ));
        };
        let Ty::Tuple(inner_items) = inner_ty else {
            return Err(format!("expected tuple inner type, found `{inner_ty}`"));
        };
        if actual_items.len() != inner_items.len() {
            return Err("faultable tuple arity mismatch".to_string());
        }
        let output_ty = Ty::Faultable(Box::new(inner_ty.clone()));
        let output_llvm_ty = self.types.basic_type(&output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "faultable.tuple")
            .map_err(|error| format!("LLVM backend failed to allocate faultable tuple: {error}"))?;
        let function = self.current_function()?;
        let fault_block = self.context.append_basic_block(function, "tuple.fault");
        let ok_block = self.context.append_basic_block(function, "tuple.ok");
        let after_block = self.context.append_basic_block(function, "tuple.after");
        let mut fault_cond = self.context.bool_type().const_zero();
        let mut first_fault = None;
        for (index, actual_item) in actual_items.iter().enumerate() {
            let field = self
                .builder
                .build_extract_value(value.value.into_struct_value(), index as u32, "field")
                .map_err(|error| format!("LLVM backend failed to extract tuple field: {error}"))?;
            self.collect_nested_fault_state(field, actual_item, &mut fault_cond, &mut first_fault)?;
        }
        self.builder
            .build_conditional_branch(fault_cond, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch on tuple faults: {error}"))?;
        self.builder.position_at_end(fault_block);
        let fault =
            first_fault.ok_or_else(|| "faultable tuple had no faultable fields".to_string())?;
        let faulted = self.faultable_value(inner_ty, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store tuple fault: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave tuple fault: {error}"))?;
        self.builder.position_at_end(ok_block);
        let mut inner = self
            .types
            .basic_type(inner_ty)?
            .into_struct_type()
            .const_zero();
        for (index, (actual_item, inner_item)) in
            actual_items.iter().zip(inner_items.iter()).enumerate()
        {
            let field = self
                .builder
                .build_extract_value(value.value.into_struct_value(), index as u32, "field")
                .map_err(|error| format!("LLVM backend failed to extract tuple field: {error}"))?;
            let field = self.strip_nested_faultable_value(field, actual_item)?;
            let field = self.coerce_value_to_ty(field, inner_item)?;
            inner = self
                .builder
                .build_insert_value(inner, field.value, index as u32, "field")
                .map_err(|error| format!("LLVM backend failed to build tuple value: {error}"))?
                .into_struct_value();
        }
        let ok = self.faultable_value(inner_ty, false, None, Some(inner.into()))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store tuple value: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave tuple ok: {error}"))?;
        self.builder.position_at_end(after_block);
        let result = self
            .builder
            .build_load(output_llvm_ty, out_ptr, "faultable.tuple")
            .map_err(|error| format!("LLVM backend failed to load tuple value: {error}"))?;
        Ok(LlvmValue {
            value: result,
            ty: output_ty,
        })
    }

    pub(super) fn collect_nested_fault_state(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
        fault_cond: &mut IntValue<'ctx>,
        selected_fault: &mut Option<BasicValueEnum<'ctx>>,
    ) -> Result<(), String> {
        match ty {
            Ty::Faultable(_) => {
                let is_fault = self.extract_faultable_is_fault(value)?;
                let fault = self.extract_faultable_fault(value)?;
                let prior_fault_cond = *fault_cond;
                *fault_cond = self
                    .builder
                    .build_or(*fault_cond, is_fault, "nested.fault")
                    .map_err(|error| {
                        format!("LLVM backend failed to combine nested faults: {error}")
                    })?;
                *selected_fault = Some(if let Some(previous) = selected_fault.take() {
                    self.builder
                        .build_select(prior_fault_cond, previous, fault, "nested.fault_value")
                        .map_err(|error| {
                            format!("LLVM backend failed to select nested fault value: {error}")
                        })?
                } else {
                    fault
                });
                Ok(())
            }
            Ty::Tuple(items) => {
                let tuple = value.into_struct_value();
                for (index, item) in items.iter().enumerate() {
                    let field = self
                        .builder
                        .build_extract_value(tuple, index as u32, "nested.field")
                        .map_err(|error| {
                            format!("LLVM backend failed to extract nested tuple field: {error}")
                        })?;
                    self.collect_nested_fault_state(field, item, fault_cond, selected_fault)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(super) fn strip_nested_faultable_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> Result<LlvmValue<'ctx>, String> {
        match ty {
            Ty::Faultable(inner) => Ok(LlvmValue {
                value: self.extract_faultable_value(value)?,
                ty: inner.as_ref().clone(),
            }),
            Ty::Tuple(items) => {
                let tuple = value.into_struct_value();
                let mut fields = Vec::with_capacity(items.len());
                let mut field_tys = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let field = self
                        .builder
                        .build_extract_value(tuple, index as u32, "nested.field")
                        .map_err(|error| {
                            format!("LLVM backend failed to extract nested tuple field: {error}")
                        })?;
                    let field = self.strip_nested_faultable_value(field, item)?;
                    field_tys.push(field.ty);
                    fields.push(field.value);
                }
                let plain_ty = Ty::Tuple(field_tys);
                let mut plain = self
                    .types
                    .basic_type(&plain_ty)?
                    .into_struct_type()
                    .const_zero();
                for (index, field) in fields.into_iter().enumerate() {
                    plain = self
                        .builder
                        .build_insert_value(plain, field, index as u32, "nested.tuple")
                        .map_err(|error| {
                            format!("LLVM backend failed to build nested plain tuple: {error}")
                        })?
                        .into_struct_value();
                }
                Ok(LlvmValue {
                    value: plain.into(),
                    ty: plain_ty,
                })
            }
            _ => Ok(LlvmValue {
                value,
                ty: ty.clone(),
            }),
        }
    }

    fn faultable_value(
        &mut self,
        inner_ty: &Ty,
        is_fault: bool,
        fault: Option<BasicValueEnum<'ctx>>,
        value: Option<BasicValueEnum<'ctx>>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let faultable_ty = Ty::Faultable(Box::new(inner_ty.clone()));
        let mut out = self
            .types
            .basic_type(&faultable_ty)?
            .into_struct_type()
            .const_zero();
        let flag = self
            .context
            .i8_type()
            .const_int(if is_fault { 1 } else { 0 }, false);
        out = self
            .builder
            .build_insert_value(out, flag, 0, "faultable")
            .map_err(|error| format!("LLVM backend failed to build faultable flag: {error}"))?
            .into_struct_value();
        if let Some(fault) = fault {
            out = self
                .builder
                .build_insert_value(out, fault, 1, "faultable")
                .map_err(|error| format!("LLVM backend failed to build faultable fault: {error}"))?
                .into_struct_value();
        }
        if let Some(value) = value {
            out = self
                .builder
                .build_insert_value(out, value, 2, "faultable")
                .map_err(|error| format!("LLVM backend failed to build faultable value: {error}"))?
                .into_struct_value();
        }
        Ok(out.into())
    }

    fn faultable_value_with_flag(
        &mut self,
        inner_ty: &Ty,
        flag: IntValue<'ctx>,
        fault: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let faultable_ty = Ty::Faultable(Box::new(inner_ty.clone()));
        let mut out = self
            .types
            .basic_type(&faultable_ty)?
            .into_struct_type()
            .const_zero();
        out = self
            .builder
            .build_insert_value(out, flag, 0, "faultable")
            .map_err(|error| format!("LLVM backend failed to build faultable flag: {error}"))?
            .into_struct_value();
        out = self
            .builder
            .build_insert_value(out, fault, 1, "faultable")
            .map_err(|error| format!("LLVM backend failed to build faultable fault: {error}"))?
            .into_struct_value();
        out = self
            .builder
            .build_insert_value(out, value, 2, "faultable")
            .map_err(|error| format!("LLVM backend failed to build faultable value: {error}"))?
            .into_struct_value();
        Ok(out.into())
    }

    fn extract_faultable_is_fault(
        &mut self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let flag = self
            .builder
            .build_extract_value(value.into_struct_value(), 0, "is_fault")
            .map_err(|error| format!("LLVM backend failed to extract fault flag: {error}"))?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                flag,
                self.context.i8_type().const_zero(),
                "fault",
            )
            .map_err(|error| format!("LLVM backend failed to compare fault flag: {error}"))
    }

    fn extract_faultable_fault(
        &mut self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_extract_value(value.into_struct_value(), 1, "fault")
            .map_err(|error| format!("LLVM backend failed to extract fault: {error}"))
    }

    fn extract_faultable_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_extract_value(value.into_struct_value(), 2, "value")
            .map_err(|error| format!("LLVM backend failed to extract faultable value: {error}"))
    }

    fn extract_tuple_field(
        &mut self,
        input: &LlvmValue<'ctx>,
        index: u32,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_extract_value(input.value.into_struct_value(), index, "field")
            .map_err(|error| format!("LLVM backend failed to extract tuple field: {error}"))
    }

    fn to_real(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> Result<inkwell::values::FloatValue<'ctx>, String> {
        match ty {
            Ty::Real => Ok(value.into_float_value()),
            Ty::Int => self
                .builder
                .build_signed_int_to_float(
                    value.into_int_value(),
                    self.context.f64_type(),
                    "sitofp",
                )
                .map_err(|error| format!("LLVM backend failed to cast Int to Real: {error}")),
            other => Err(format!("expected numeric value, found `{other}`")),
        }
    }

    fn current_function(&self) -> Result<FunctionValue<'ctx>, String> {
        self.builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .ok_or_else(|| "LLVM backend has no current function".to_string())
    }

    fn runtime_function(
        &mut self,
        name: &str,
        ret: Option<BasicTypeEnum<'ctx>>,
        args: &[BasicTypeEnum<'ctx>],
    ) -> Result<FunctionValue<'ctx>, String> {
        if let Some(function) = self.module.get_function(name) {
            return Ok(function);
        }
        let arg_types = args
            .iter()
            .copied()
            .map(Into::into)
            .collect::<Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>>>();
        let function_ty = match ret {
            Some(ret) => ret.fn_type(&arg_types, false),
            None => self.context.void_type().fn_type(&arg_types, false),
        };
        Ok(self.module.add_function(name, function_ty, None))
    }

    fn calloc_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("calloc") {
            return function;
        }
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        self.module.add_function(
            "calloc",
            ptr_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false),
            None,
        )
    }

    fn emit_seq_new(
        &mut self,
        seq_ty: &Ty,
        count: IntValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let Ty::Seq(item_ty) = seq_ty else {
            return Err(format!("expected sequence type, found `{seq_ty}`"));
        };
        let item_llvm_ty = self.types.basic_type(item_ty)?;
        let elem_size = item_llvm_ty
            .size_of()
            .ok_or_else(|| format!("cannot compute size of `{item_ty}`"))?;
        let nonzero_count = self
            .builder
            .build_select(
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        count,
                        self.context.i64_type().const_zero(),
                        "empty",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to compare sequence count: {error}")
                    })?,
                self.context.i64_type().const_int(1, false),
                count,
                "alloc.count",
            )
            .map_err(|error| format!("LLVM backend failed to select allocation count: {error}"))?
            .into_int_value();
        let calloc = self.calloc_function();
        let items = self
            .builder
            .build_call(calloc, &[nonzero_count.into(), elem_size.into()], "items")
            .map_err(|error| format!("LLVM backend failed to call calloc: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "calloc did not return a value".to_string())?;
        let mut seq = self
            .types
            .basic_type(seq_ty)?
            .into_struct_type()
            .const_zero();
        seq = self
            .builder
            .build_insert_value(seq, count, 0, "seq")
            .map_err(|error| format!("LLVM backend failed to set sequence count: {error}"))?
            .into_struct_value();
        seq = self
            .builder
            .build_insert_value(seq, items, 1, "seq")
            .map_err(|error| format!("LLVM backend failed to set sequence items: {error}"))?
            .into_struct_value();
        Ok(LlvmValue {
            value: seq.into(),
            ty: seq_ty.clone(),
        })
    }

    fn seq_count(&mut self, seq: BasicValueEnum<'ctx>) -> Result<IntValue<'ctx>, String> {
        Ok(self
            .builder
            .build_extract_value(seq.into_struct_value(), 0, "count")
            .map_err(|error| format!("LLVM backend failed to extract sequence count: {error}"))?
            .into_int_value())
    }

    fn seq_items(&mut self, seq: BasicValueEnum<'ctx>) -> Result<PointerValue<'ctx>, String> {
        Ok(self
            .builder
            .build_extract_value(seq.into_struct_value(), 1, "items")
            .map_err(|error| format!("LLVM backend failed to extract sequence items: {error}"))?
            .into_pointer_value())
    }

    fn load_seq_item(
        &mut self,
        seq: BasicValueEnum<'ctx>,
        seq_ty: &Ty,
        index: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Seq(item_ty) = seq_ty else {
            return Err(format!("expected sequence type, found `{seq_ty}`"));
        };
        let item_llvm_ty = self.types.basic_type(item_ty)?;
        let items = self.seq_items(seq)?;
        let ptr = unsafe {
            self.builder
                .build_gep(item_llvm_ty, items, &[index], "item.ptr")
                .map_err(|error| {
                    format!("LLVM backend failed to compute sequence item ptr: {error}")
                })?
        };
        self.builder
            .build_load(item_llvm_ty, ptr, "item")
            .map_err(|error| format!("LLVM backend failed to load sequence item: {error}"))
    }

    fn store_seq_item(
        &mut self,
        seq: BasicValueEnum<'ctx>,
        seq_ty: &Ty,
        index: IntValue<'ctx>,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let Ty::Seq(item_ty) = seq_ty else {
            return Err(format!("expected sequence type, found `{seq_ty}`"));
        };
        let item_llvm_ty = self.types.basic_type(item_ty)?;
        let items = self.seq_items(seq)?;
        let ptr = unsafe {
            self.builder
                .build_gep(item_llvm_ty, items, &[index], "item.ptr")
                .map_err(|error| {
                    format!("LLVM backend failed to compute sequence item ptr: {error}")
                })?
        };
        self.builder
            .build_store(ptr, value)
            .map_err(|error| format!("LLVM backend failed to store sequence item: {error}"))?;
        Ok(())
    }

    fn emit_entrypoint(&mut self) -> Result<(), String> {
        let Some(program) = self.codegen.module.declarations.iter().find_map(|decl| {
            if let Decl::Program(callable) = decl {
                Some(callable)
            } else {
                None
            }
        }) else {
            return Ok(());
        };
        let program_fn = *self
            .functions
            .get(&program.name)
            .ok_or_else(|| "missing program function".to_string())?;
        let i32_ty = self.context.i32_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let main_ty = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into()], false);
        let flow_main = self.module.add_function("flow_unboxed_main", main_ty, None);
        let block = self.context.append_basic_block(flow_main, "entry");
        self.builder.position_at_end(block);
        let argc = flow_main
            .get_nth_param(0)
            .ok_or_else(|| "missing argc".to_string())?;
        let argv = flow_main
            .get_nth_param(1)
            .ok_or_else(|| "missing argv".to_string())?;
        let args_ty = self.types.basic_type(&Ty::Args)?;
        let mut args = args_ty.into_struct_type().const_zero();
        args = self
            .builder
            .build_insert_value(args, argc, 0, "argc")
            .map_err(|error| format!("LLVM backend failed to build Args.argc: {error}"))?
            .into_struct_value();
        args = self
            .builder
            .build_insert_value(args, argv, 1, "argv")
            .map_err(|error| format!("LLVM backend failed to build Args.argv: {error}"))?
            .into_struct_value();
        let result = self
            .builder
            .build_call(program_fn, &[args.into()], "program")
            .map_err(|error| format!("LLVM backend failed to call program main: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "program main did not return a value".to_string())?;
        let exit = match self
            .codegen
            .signatures
            .get(&program.name)
            .map(|sig| sig.output.clone())
        {
            Some(Ty::Int) => self
                .builder
                .build_int_truncate(result.into_int_value(), i32_ty, "exit")
                .map_err(|error| format!("LLVM backend failed to truncate exit code: {error}"))?,
            Some(Ty::Faultable(inner)) if inner.as_ref() == &Ty::Int => {
                let fault_block = self.context.append_basic_block(flow_main, "fault");
                let ok_block = self.context.append_basic_block(flow_main, "ok");
                let is_fault = self.extract_faultable_is_fault(result)?;
                self.builder
                    .build_conditional_branch(is_fault, fault_block, ok_block)
                    .map_err(|error| {
                        format!("LLVM backend failed to branch on program fault: {error}")
                    })?;
                self.builder.position_at_end(fault_block);
                let fault = self.extract_faultable_fault(result)?;
                let fault = self.value_to_runtime_arg(fault, &Ty::Fault)?;
                let exit_fault = self.runtime_function(
                    "fa_exit_fault",
                    None,
                    &[self.runtime_pair_type().into()],
                )?;
                self.builder
                    .build_call(exit_fault, &[fault.into()], "exit_fault")
                    .map_err(|error| {
                        format!("LLVM backend failed to call fa_exit_fault: {error}")
                    })?;
                self.builder.build_unreachable().map_err(|error| {
                    format!("LLVM backend failed to build unreachable: {error}")
                })?;
                self.builder.position_at_end(ok_block);
                let exit_value = self.extract_faultable_value(result)?.into_int_value();
                self.builder
                    .build_int_truncate(exit_value, i32_ty, "exit")
                    .map_err(|error| {
                        format!("LLVM backend failed to truncate exit code: {error}")
                    })?
            }
            other => {
                return Err(format!(
                    "unsupported program output for direct LLVM: {other:?}"
                ));
            }
        };
        self.builder
            .build_return(Some(&exit))
            .map_err(|error| format!("LLVM backend failed to return entrypoint: {error}"))?;

        let c_main = self.module.add_function("main", main_ty, None);
        let c_main_block = self.context.append_basic_block(c_main, "entry");
        self.builder.position_at_end(c_main_block);
        let c_argc = c_main
            .get_nth_param(0)
            .ok_or_else(|| "missing main argc".to_string())?;
        let c_argv = c_main
            .get_nth_param(1)
            .ok_or_else(|| "missing main argv".to_string())?;
        let c_exit = self
            .builder
            .build_call(flow_main, &[c_argc.into(), c_argv.into()], "exit")
            .map_err(|error| format!("LLVM backend failed to call flow_unboxed_main: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "flow_unboxed_main did not return a value".to_string())?;
        self.builder
            .build_return(Some(&c_exit))
            .map_err(|error| format!("LLVM backend failed to return main: {error}"))?;
        Ok(())
    }
}

struct LlvmTypeRegistry<'ctx> {
    context: &'ctx Context,
    structs: HashMap<String, StructType<'ctx>>,
}

impl<'ctx> LlvmTypeRegistry<'ctx> {
    fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            structs: HashMap::new(),
        }
    }

    fn basic_type(&mut self, ty: &Ty) -> Result<BasicTypeEnum<'ctx>, String> {
        match ty {
            Ty::Unit => Ok(self
                .struct_type(ty, &[self.context.i32_type().into()])?
                .into()),
            Ty::Int => Ok(self.context.i64_type().into()),
            Ty::Real | Ty::OneOf(_) => Ok(self.context.f64_type().into()),
            Ty::Bool => Ok(self.context.i8_type().into()),
            Ty::Bytes => Ok(self
                .struct_type(
                    ty,
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                    ],
                )?
                .into()),
            Ty::Args => Ok(self
                .struct_type(
                    ty,
                    &[
                        self.context.i32_type().into(),
                        self.context.ptr_type(AddressSpace::default()).into(),
                    ],
                )?
                .into()),
            Ty::Fault => {
                let bytes = self.basic_type(&Ty::Bytes)?;
                Ok(self.struct_type(ty, &[bytes])?.into())
            }
            Ty::Seq(item) => {
                self.basic_type(item)?;
                Ok(self
                    .struct_type(
                        ty,
                        &[
                            self.context.i64_type().into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        ],
                    )?
                    .into())
            }
            Ty::Tuple(items) => {
                let fields = items
                    .iter()
                    .map(|item| self.basic_type(item))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(self.struct_type(ty, &fields)?.into())
            }
            Ty::Faultable(inner) => {
                let fault = self.basic_type(&Ty::Fault)?;
                let inner = self.basic_type(inner)?;
                Ok(self
                    .struct_type(ty, &[self.context.i8_type().into(), fault, inner])?
                    .into())
            }
            Ty::Stream(_) => {
                let bytes = self.basic_type(&Ty::Bytes)?;
                let ptr = self.context.ptr_type(AddressSpace::default()).into();
                Ok(self
                    .struct_type(
                        ty,
                        &[
                            ptr,
                            self.context.i32_type().into(),
                            bytes,
                            ptr,
                            ptr,
                            ptr,
                            ptr,
                            self.context.i64_type().into(),
                            self.context.i8_type().into(),
                        ],
                    )?
                    .into())
            }
            Ty::SqliteConnection => Ok(self
                .struct_type(ty, &[self.context.ptr_type(AddressSpace::default()).into()])?
                .into()),
            Ty::SqliteValue => {
                let bytes = self.basic_type(&Ty::Bytes)?;
                Ok(self
                    .struct_type(
                        ty,
                        &[
                            self.context.i32_type().into(),
                            self.context.i64_type().into(),
                            self.context.f64_type().into(),
                            bytes,
                        ],
                    )?
                    .into())
            }
            Ty::SqliteRow => Ok(self
                .struct_type(
                    ty,
                    &[
                        self.context.i64_type().into(),
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.ptr_type(AddressSpace::default()).into(),
                    ],
                )?
                .into()),
            Ty::HttpServerConfig => {
                let bytes = self.basic_type(&Ty::Bytes)?;
                Ok(self
                    .struct_type(
                        ty,
                        &[
                            bytes,
                            self.context.i64_type().into(),
                            self.context.i8_type().into(),
                            bytes,
                            bytes,
                            self.context.i8_type().into(),
                            self.context.i8_type().into(),
                        ],
                    )?
                    .into())
            }
            Ty::HttpListener => {
                let config = self.basic_type(&Ty::HttpServerConfig)?;
                Ok(self
                    .struct_type(
                        ty,
                        &[
                            config,
                            self.context.ptr_type(AddressSpace::default()).into(),
                        ],
                    )?
                    .into())
            }
            Ty::HttpRequest => {
                let bytes = self.basic_type(&Ty::Bytes)?;
                Ok(self
                    .struct_type(
                        ty,
                        &[
                            bytes,
                            bytes,
                            bytes,
                            self.context.ptr_type(AddressSpace::default()).into(),
                        ],
                    )?
                    .into())
            }
            Ty::HttpResponse => {
                let request = self.basic_type(&Ty::HttpRequest)?;
                let bytes = self.basic_type(&Ty::Bytes)?;
                let seq_bytes = self.basic_type(&Ty::Seq(Box::new(Ty::Bytes)))?;
                Ok(self
                    .struct_type(
                        ty,
                        &[
                            request,
                            self.context.i64_type().into(),
                            seq_bytes,
                            seq_bytes,
                            bytes,
                            bytes,
                        ],
                    )?
                    .into())
            }
            Ty::Var(_) | Ty::EmptySeq => {
                Err(format!("direct LLVM cannot lower unresolved type `{ty}`"))
            }
        }
    }

    fn struct_type(
        &mut self,
        ty: &Ty,
        fields: &[BasicTypeEnum<'ctx>],
    ) -> Result<StructType<'ctx>, String> {
        let name = type_name(ty);
        if let Some(existing) = self.structs.get(&name) {
            return Ok(*existing);
        }
        let field_types = fields.to_vec();
        let struct_ty = self.context.struct_type(&field_types, false);
        self.structs.insert(name, struct_ty);
        Ok(struct_ty)
    }
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
                        "static {name} {name}_new(size_t count) {{\n  {name} seq;\n  seq.count = count;\n  seq.items = ({item_ty} *)calloc(count ? count : 1, sizeof({item_ty}));\n  if (!seq.items) fa_die_alloc();\n  return seq;\n}}\n\n"
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

fn sequence_item_type(left: &Ty, right: &Ty) -> Result<Ty, String> {
    if left == right {
        return Ok(left.clone());
    }
    match (left, right) {
        (Ty::EmptySeq, other) | (other, Ty::EmptySeq) => Ok(other.clone()),
        (Ty::Faultable(inner), other) | (other, Ty::Faultable(inner))
            if inner.as_ref() == other =>
        {
            Ok(Ty::Faultable(inner.clone()))
        }
        (Ty::Int, Ty::Real) | (Ty::Real, Ty::Int) => Ok(Ty::Real),
        _ => Err(format!(
            "sequence literal item type mismatch: `{left}` vs `{right}`"
        )),
    }
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
        _ => false,
    }
}

fn common_assignable_output_ty(current: &Ty, next: &Ty, label: &str) -> Result<Ty, String> {
    if assignable_output_ty(current, next) {
        return Ok(current.clone());
    }
    if assignable_output_ty(next, current) {
        return Ok(next.clone());
    }
    Err(format!("{label} expected `{current}`, found `{next}`"))
}

fn format_match_target(target: &MatchTarget) -> String {
    match target {
        MatchTarget::Node(node) => node.clone(),
        MatchTarget::Value(endpoint) => format_endpoint_for_error_codegen(endpoint),
    }
}

fn format_endpoint_for_error_codegen(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Variable(name) => format!("${name}"),
        Endpoint::Name(name) => name.clone(),
        Endpoint::Int(value) => value.to_string(),
        Endpoint::Real(value) => value.to_string(),
        Endpoint::Bool(value) => value.to_string(),
        Endpoint::String(value) => format!("\"{value}\""),
        Endpoint::Unit => "()".to_string(),
        Endpoint::Tuple(_) => "(...)".to_string(),
        Endpoint::Seq(_) => "[...]".to_string(),
        Endpoint::Eval { .. } => "(inline eval)".to_string(),
    }
}

fn substitute_ty(ty: &Ty, vars: &HashMap<String, Ty>) -> Option<Ty> {
    match ty {
        Ty::Var(name) => vars
            .get(name)
            .cloned()
            .or_else(|| Some(Ty::Var(name.clone()))),
        Ty::Faultable(item) => Some(Ty::Faultable(Box::new(substitute_ty(item, vars)?))),
        Ty::Seq(item) => Some(Ty::Seq(Box::new(substitute_ty(item, vars)?))),
        Ty::Stream(item) => Some(Ty::Stream(Box::new(substitute_ty(item, vars)?))),
        Ty::OneOf(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute_ty(item, vars)?);
            }
            Some(Ty::OneOf(out))
        }
        Ty::Tuple(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute_ty(item, vars)?);
            }
            Some(Ty::Tuple(out))
        }
        other => Some(other.clone()),
    }
}

fn contains_type_var(input: &Ty) -> bool {
    match input {
        Ty::Var(_) => true,
        Ty::Faultable(item) | Ty::Seq(item) | Ty::Stream(item) => contains_type_var(item),
        Ty::Tuple(items) | Ty::OneOf(items) => items.iter().any(contains_type_var),
        _ => false,
    }
}

fn contains_empty_seq(input: &Ty) -> bool {
    match input {
        Ty::EmptySeq => true,
        Ty::Faultable(item) | Ty::Seq(item) | Ty::Stream(item) => contains_empty_seq(item),
        Ty::Tuple(items) | Ty::OneOf(items) => items.iter().any(contains_empty_seq),
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
    matches!(name, "FaTuple_HttpRequest_Bytes_Bytes")
}

fn is_sqlite_runtime_type_name(name: &str) -> bool {
    matches!(name, "FaSqliteConnection" | "FaSqliteRow" | "FaSqliteValue")
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

fn stages_binding_output<'a>(chain: &'a Chain, output: &str) -> Option<&'a [Stage]> {
    let (last, stages) = chain.stages.split_last()?;
    match last {
        Stage::Bind(BindingTarget::Discard) => None,
        Stage::Bind(BindingTarget::Variable(name)) if name == output => Some(stages),
        _ => None,
    }
}

fn final_variable(chain: &Chain) -> Option<&str> {
    match chain.stages.last()? {
        Stage::Bind(BindingTarget::Discard) => None,
        Stage::Bind(BindingTarget::Variable(name)) => Some(name),
        _ => None,
    }
}

fn fuse_single_use_chains(callable: &Callable) -> Vec<Chain> {
    let mut chains = callable.chains.clone();
    loop {
        let mut uses = HashMap::new();
        for chain in &chains {
            count_endpoint_vars(&chain.source, &mut uses);
            for stage in &chain.stages {
                match stage {
                    Stage::Reduce { identity, .. } | Stage::Scan { identity, .. } => {
                        count_endpoint_vars(identity, &mut uses);
                    }
                    Stage::Repeat { count, .. } => count_endpoint_vars(count, &mut uses),
                    Stage::Match { arms } => {
                        for arm in arms {
                            if let MatchGuard::Call { args, .. } = &arm.guard {
                                for arg in args {
                                    count_endpoint_vars(arg, &mut uses);
                                }
                            }
                        }
                    }
                    Stage::Endpoint(_)
                    | Stage::Bind(_)
                    | Stage::Map(_)
                    | Stage::Filter(_)
                    | Stage::FaultMap { .. } => {}
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
                |chain| matches!(&chain.source, Endpoint::Variable(name) if name == &binding),
            ) else {
                continue;
            };
            if producer_index == consumer_index {
                continue;
            }

            let mut stages = chains[producer_index].stages.clone();
            stages.pop();
            stages.extend(chains[consumer_index].stages.clone());
            chains[consumer_index] = Chain {
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

fn count_endpoint_vars(endpoint: &Endpoint, uses: &mut HashMap<String, usize>) {
    match endpoint {
        Endpoint::Variable(name) => {
            *uses.entry(name.clone()).or_insert(0) += 1;
        }
        Endpoint::Tuple(items) | Endpoint::Seq(items) => {
            for item in items {
                count_endpoint_vars(item, uses);
            }
        }
        Endpoint::Eval { source, stages } => {
            count_endpoint_vars(source, uses);
            for stage in stages {
                match stage {
                    Stage::Repeat { count, .. }
                    | Stage::Reduce {
                        identity: count, ..
                    }
                    | Stage::Scan {
                        identity: count, ..
                    } => count_endpoint_vars(count, uses),
                    Stage::Match { arms } => {
                        for arm in arms {
                            if let MatchGuard::Call { args, .. } = &arm.guard {
                                for arg in args {
                                    count_endpoint_vars(arg, uses);
                                }
                            }
                            if let MatchTarget::Value(endpoint) = &arm.target {
                                count_endpoint_vars(endpoint, uses);
                            }
                        }
                    }
                    Stage::Bind(_) => {}
                    _ => {}
                }
            }
        }
        Endpoint::Name(_)
        | Endpoint::Int(_)
        | Endpoint::Real(_)
        | Endpoint::Bool(_)
        | Endpoint::String(_)
        | Endpoint::Unit => {}
    }
}

fn endpoint_contains_empty_seq(endpoint: &Endpoint) -> bool {
    match endpoint {
        Endpoint::Seq(items) => items.is_empty() || items.iter().any(endpoint_contains_empty_seq),
        Endpoint::Tuple(items) => items.iter().any(endpoint_contains_empty_seq),
        Endpoint::Eval { source, stages } => {
            endpoint_contains_empty_seq(source)
                || stages.iter().any(|stage| match stage {
                    Stage::Repeat { count, .. }
                    | Stage::Reduce {
                        identity: count, ..
                    }
                    | Stage::Scan {
                        identity: count, ..
                    } => endpoint_contains_empty_seq(count),
                    Stage::Match { arms } => arms.iter().any(|arm| {
                        (match &arm.guard {
                            MatchGuard::Call { args, .. } => {
                                args.iter().any(endpoint_contains_empty_seq)
                            }
                            MatchGuard::Fallback => false,
                        }) || match &arm.target {
                            MatchTarget::Value(endpoint) => endpoint_contains_empty_seq(endpoint),
                            MatchTarget::Node(_) => false,
                        }
                    }),
                    _ => false,
                })
        }
        _ => false,
    }
}

fn is_zero(endpoint: &Endpoint) -> bool {
    match endpoint {
        Endpoint::Int(value) => *value == 0,
        Endpoint::Real(value) => *value == 0.0,
        _ => false,
    }
}

fn matches_pair_source(endpoint: &Endpoint, left: &str, right: &str) -> bool {
    matches!(
        endpoint,
        Endpoint::Tuple(items)
            if items.len() == 2
                && matches!(&items[0], Endpoint::Variable(name) if name == left)
                && matches!(&items[1], Endpoint::Variable(name) if name == right)
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

fn parse_type(text: &str) -> Result<Ty, String> {
    TypeParser {
        chars: text.chars().collect(),
        pos: 0,
    }
    .parse()
}

fn builtin_type_alias(name: &str) -> Option<Ty> {
    match name {
        "Number" => Some(Ty::OneOf(vec![Ty::Int, Ty::Real])),
        _ => None,
    }
}

struct TypeParser {
    chars: Vec<char>,
    pos: usize,
}

impl TypeParser {
    fn parse(&mut self) -> Result<Ty, String> {
        let mut items = vec![self.parse_atom()?];
        while self.eat('|') {
            items.push(self.parse_atom()?);
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Ty::OneOf(items)
        })
    }

    fn parse_atom(&mut self) -> Result<Ty, String> {
        self.skip_ws();
        if self.eat('(') {
            let mut items = Vec::new();
            if self.eat(')') {
                return Ok(Ty::Unit);
            }
            loop {
                items.push(self.parse()?);
                if self.eat(',') {
                    continue;
                }
                self.expect(')')?;
                break;
            }
            return Ok(Ty::Tuple(items));
        }
        let name = self.ident()?;
        if name == "Seq" && self.eat('[') {
            let item = self.parse()?;
            self.expect(']')?;
            return Ok(Ty::Seq(Box::new(item)));
        }
        if name == "Faultable" && self.eat('[') {
            let item = self.parse()?;
            self.expect(']')?;
            return Ok(Ty::Faultable(Box::new(item)));
        }
        if name.rsplit('.').next() == Some("Stream") && self.eat('[') {
            let item = self.parse()?;
            self.expect(']')?;
            return Ok(Ty::Stream(Box::new(item)));
        }
        if name == "sqlite.Connection" {
            return Ok(Ty::SqliteConnection);
        }
        if name == "sqlite.Row" {
            return Ok(Ty::SqliteRow);
        }
        if name == "sqlite.Value" {
            return Ok(Ty::SqliteValue);
        }
        if name == "http.ServerConfig" {
            return Ok(Ty::HttpServerConfig);
        }
        if name == "http.Listener" {
            return Ok(Ty::HttpListener);
        }
        if name == "http.Request" {
            return Ok(Ty::HttpRequest);
        }
        if name == "http.Response" {
            return Ok(Ty::HttpResponse);
        }
        let base_name = name.rsplit('.').next().unwrap_or(&name);
        Ok(match base_name {
            "Unit" | "void" => Ty::Unit,
            "Int" | "i8" | "i16" | "i32" | "i64" => Ty::Int,
            "Real" | "f16" | "float" | "double" => Ty::Real,
            "Bool" | "i1" => Ty::Bool,
            "Bytes" | "ptr" => Ty::Bytes,
            "Fault" => Ty::Fault,
            _ => Ty::Var(name),
        })
    }

    fn ident(&mut self) -> Result<String, String> {
        self.skip_ws();
        let start = self.pos;
        while self
            .chars
            .get(self.pos)
            .map(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '.')
            .unwrap_or(false)
        {
            self.pos += 1;
        }
        if self.pos == start {
            return Err("expected type name".to_string());
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn eat(&mut self, ch: char) -> bool {
        self.skip_ws();
        if self.chars.get(self.pos) == Some(&ch) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, ch: char) -> Result<(), String> {
        if self.eat(ch) {
            Ok(())
        } else {
            Err(format!("expected `{ch}` in type"))
        }
    }

    fn skip_ws(&mut self) {
        while self
            .chars
            .get(self.pos)
            .map(|ch| ch.is_whitespace())
            .unwrap_or(false)
        {
            self.pos += 1;
        }
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

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Unit => write!(f, "Unit"),
            Ty::Int => write!(f, "Int"),
            Ty::Real => write!(f, "Real"),
            Ty::Bool => write!(f, "Bool"),
            Ty::Bytes => write!(f, "Bytes"),
            Ty::Args => write!(f, "Args"),
            Ty::HttpServerConfig => write!(f, "http.ServerConfig"),
            Ty::HttpListener => write!(f, "http.Listener"),
            Ty::HttpRequest => write!(f, "http.Request"),
            Ty::HttpResponse => write!(f, "http.Response"),
            Ty::SqliteConnection => write!(f, "sqlite.Connection"),
            Ty::SqliteRow => write!(f, "sqlite.Row"),
            Ty::SqliteValue => write!(f, "sqlite.Value"),
            Ty::Stream(item) => write!(f, "Stream[{item}]"),
            Ty::Fault => write!(f, "Fault"),
            Ty::Faultable(item) => write!(f, "Faultable[{item}]"),
            Ty::Seq(item) => write!(f, "Seq[{item}]"),
            Ty::Tuple(items) => write!(
                f,
                "({})",
                items
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Ty::OneOf(items) => write!(
                f,
                "{}",
                items
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("|")
            ),
            Ty::Var(name) => write!(f, "{name}"),
            Ty::EmptySeq => write!(f, "[]"),
        }
    }
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

    fn function_body<'a>(runtime_c: &'a str, name: &str) -> &'a str {
        let start = runtime_c.find(name).expect("function name");
        let body_start = runtime_c[start..].find(" {\n").expect("function body") + start;
        let body_end = runtime_c[body_start..]
            .find("\n}\n\n")
            .expect("function end")
            + body_start;
        &runtime_c[body_start..body_end]
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

        let llvm = emit_module(&module).expect("llvm");

        assert!(llvm.contains("declare i32 @flow_unboxed_main(i32, ptr)"));
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

        assert!(runtime_c.contains("fa_parallel_map_worker_0"));
        assert!(runtime_c.contains("fa_parallel_for(0,"));
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
}
