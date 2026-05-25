use super::*;

fn gpu_reduce_op(op: &str) -> u32 {
    match op {
        "add" => 0,
        "min" => 1,
        "max" => 2,
        _ => unreachable!("unsupported GPU reduce op"),
    }
}

impl<'a> TypedCodegen<'a> {
    pub(super) fn from_typed(typed: &'a TypedModule) -> Result<Self, String> {
        Self::from_typed_with_gpu(typed, false)
    }

    pub(super) fn from_typed_with_gpu(typed: &'a TypedModule, gpu: bool) -> Result<Self, String> {
        let mut codegen = Self {
            module: typed.module(),
            typed,
            temp: 0,
            parallel_helper: 0,
            stream_helper: 0,
            parallel_helpers: String::new(),
            callables: HashMap::new(),
            foreign_js: HashSet::new(),
            foreign_c: HashMap::new(),
            signatures: HashMap::new(),
            stdlib_names: HashMap::new(),
            aliases: HashMap::new(),
            types: TypeRegistry::default(),
            gpu_enabled: gpu,
            gpu_plan: if gpu {
                gpu::GpuPlan::analyze(typed)
            } else {
                gpu::GpuPlan::empty()
            },
        };
        codegen.collect_imports()?;
        codegen.collect_type_aliases()?;
        codegen.collect_foreigns()?;
        codegen.collect_callables()?;
        Ok(codegen)
    }

    pub(super) fn emit(mut self) -> Result<String, String> {
        if self.module.declarations.iter().any(|decl| {
            matches!(
                decl,
                Decl::Foreign(ForeignBlock {
                    target: ForeignTarget::Js,
                    ..
                })
            )
        }) {
            return Err(
                "foreign js declarations are supported only by the TypeScript and JavaScript backends"
                    .to_string(),
            );
        }
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
        let mut foreign_names = self.foreign_c.keys().cloned().collect::<Vec<_>>();
        foreign_names.sort();
        for name in &foreign_names {
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
        self.types.set_use_cv_header(uses_cv_runtime);

        let mut callables = self.typed.callables.iter().collect::<Vec<_>>();
        callables.sort_by(|left, right| left.name.cmp(&right.name));
        for callable in callables {
            self.emit_callable(&mut bodies, callable)?;
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
        if self.gpu_enabled {
            out.push_str("extern void fa_gpu_require_device(void);\n");
            out.push_str("typedef struct { size_t count; const double *items; } FaGpuSliceF64;\n");
            out.push_str("extern double fa_gpu_repeat_vector_accum_f64(const char *wgsl, const double *left, size_t left_count, const double *right, size_t right_count, double score, int64_t iterations);\n");
            out.push_str("extern double fa_gpu_repeat_matrix_accum_f64(const char *wgsl, const FaGpuSliceF64 *left_rows, size_t left_count, const FaGpuSliceF64 *right_rows, size_t right_count, const double *vector, size_t vector_count, double score, int64_t iterations);\n");
        }
        if !self.gpu_plan.is_empty() {
            out.push_str("extern void fa_gpu_map_i64(const char *wgsl, const int64_t *input, int64_t *output, size_t count);\n");
            out.push_str("extern void fa_gpu_map_f64(const char *wgsl, const double *input, double *output, size_t count);\n");
            out.push_str(&self.gpu_plan.emit_c_manifest());
        }
        if self.gpu_enabled {
            out.push_str("extern int64_t fa_gpu_reduce_i64(uint32_t op, const int64_t *input, size_t count, int64_t identity);\n");
            out.push_str("extern double fa_gpu_reduce_f64(uint32_t op, const double *input, size_t count, double identity);\n");
            out.push_str("extern int64_t fa_gpu_range_map_reduce_i64(const char *map_expr, int64_t start, int64_t stop, int64_t step, uint32_t op, int64_t identity);\n");
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
        for name in &foreign_names {
            let sig = self.signatures.get(name).expect("signature");
            let input = self.types.c_type(&sig.input);
            let output = self.types.c_type(&sig.output);
            let symbol = &self.foreign_c.get(name).expect("foreign c binding").symbol;
            out.push_str(&format!("extern {output} {symbol}({input} input);\n"));
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
        if self.gpu_enabled {
            out.push_str("fa_gpu_require_device();\n  ");
        }
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

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn emit_native_cdylib_c(&mut self) -> Result<NativeCdylibOutput, String> {
        let mut bodies = String::new();
        let uses_cv_runtime = self.uses_cv_runtime();
        let uses_http_runtime = self.uses_http_runtime();
        let uses_sqlite_runtime = self.uses_sqlite_runtime();
        let mut names = self.callables.keys().cloned().collect::<Vec<_>>();
        names.sort();
        for name in &names {
            let sig = self
                .signatures
                .get(name)
                .ok_or_else(|| format!("missing signature for `{name}`"))?;
            self.types.c_type(&sig.input);
            self.types.c_type(&sig.output);
        }
        let mut foreign_names = self.foreign_c.keys().cloned().collect::<Vec<_>>();
        foreign_names.sort();
        for name in &foreign_names {
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
        self.types.set_use_cv_header(uses_cv_runtime);

        let mut callables = self.typed.callables.iter().collect::<Vec<_>>();
        callables.sort_by(|left, right| left.name.cmp(&right.name));
        for callable in callables {
            self.emit_callable(&mut bodies, callable)?;
        }
        let exports = self.exported_node_names();
        let header = self.native_c_header()?;
        let wrappers = self.emit_native_c_export_wrappers(&exports)?;

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
        if self.gpu_enabled {
            out.push_str("extern void fa_gpu_require_device(void);\n");
            out.push_str("typedef struct { size_t count; const double *items; } FaGpuSliceF64;\n");
            out.push_str("extern double fa_gpu_repeat_vector_accum_f64(const char *wgsl, const double *left, size_t left_count, const double *right, size_t right_count, double score, int64_t iterations);\n");
            out.push_str("extern double fa_gpu_repeat_matrix_accum_f64(const char *wgsl, const FaGpuSliceF64 *left_rows, size_t left_count, const FaGpuSliceF64 *right_rows, size_t right_count, const double *vector, size_t vector_count, double score, int64_t iterations);\n");
        }
        if !self.gpu_plan.is_empty() {
            out.push_str("extern void fa_gpu_map_i64(const char *wgsl, const int64_t *input, int64_t *output, size_t count);\n");
            out.push_str("extern void fa_gpu_map_f64(const char *wgsl, const double *input, double *output, size_t count);\n");
            out.push_str(&self.gpu_plan.emit_c_manifest());
        }
        if self.gpu_enabled {
            out.push_str("extern int64_t fa_gpu_reduce_i64(uint32_t op, const int64_t *input, size_t count, int64_t identity);\n");
            out.push_str("extern double fa_gpu_reduce_f64(uint32_t op, const double *input, size_t count, double identity);\n");
            out.push_str("extern int64_t fa_gpu_range_map_reduce_i64(const char *map_expr, int64_t start, int64_t stop, int64_t step, uint32_t op, int64_t identity);\n");
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
        for name in &foreign_names {
            let sig = self.signatures.get(name).expect("signature");
            let input = self.types.c_type(&sig.input);
            let output = self.types.c_type(&sig.output);
            let symbol = &self.foreign_c.get(name).expect("foreign c binding").symbol;
            out.push_str(&format!("extern {output} {symbol}({input} input);\n"));
        }
        out.push('\n');
        if self.gpu_enabled {
            out.push_str("#if defined(__GNUC__) || defined(__clang__)\n__attribute__((constructor)) static void fa_gpu_cdylib_init(void) { fa_gpu_require_device(); }\n#endif\n\n");
        }
        out.push_str(&self.parallel_helpers);
        out.push_str(&bodies);
        out.push_str(&wrappers);

        Ok(NativeCdylibOutput {
            source: out,
            header,
            exports,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn emit_native_c_export_wrappers(&mut self, exports: &[String]) -> Result<String, String> {
        let mut out = String::new();
        for name in exports {
            let callable = self.exported_node(name)?;
            let inputs = callable.inputs.clone();
            let sig = self
                .signatures
                .get(name)
                .ok_or_else(|| format!("missing signature for native C export `{name}`"))?
                .clone();
            let input_items = native_c_input_items(name, callable, &sig.input)?
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            let output = self.types.c_type(&sig.output);
            let params = if input_items.is_empty() {
                "void".to_string()
            } else {
                inputs
                    .iter()
                    .zip(input_items.iter())
                    .map(|(port, ty)| {
                        let c_ty = self.types.c_type(ty);
                        format!("{c_ty} {}", c_ident(&port.name))
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            out.push_str(&format!(
                "{output} {}({params}) {{\n",
                sanitize_symbol(name)
            ));
            match input_items.as_slice() {
                [] => {
                    out.push_str(&format!("  return {}(fa_unit());\n", user_fn_name(name)));
                }
                [_] => {
                    let arg = c_ident(&inputs[0].name);
                    out.push_str(&format!("  return {}({arg});\n", user_fn_name(name)));
                }
                _ => {
                    let input_ty = self.types.c_type(&sig.input);
                    out.push_str(&format!("  {input_ty} input;\n"));
                    for (index, port) in inputs.iter().enumerate() {
                        out.push_str(&format!("  input.f{index} = {};\n", c_ident(&port.name)));
                    }
                    out.push_str(&format!("  return {}(input);\n", user_fn_name(name)));
                }
            }
            out.push_str("}\n\n");
        }
        Ok(out)
    }

    fn exported_node_names(&self) -> Vec<String> {
        self.typed
            .callables
            .iter()
            .filter_map(|callable| {
                if matches!(callable.kind, crate::typecheck::TypedCallableKind::Node)
                    && callable.is_extern
                    && !callable.name.starts_with("__flow_")
                {
                    Some(callable.name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn exported_node(&self, name: &str) -> Result<&TypedCallable, String> {
        self.typed
            .callables
            .iter()
            .find(|callable| callable.name == name)
            .ok_or_else(|| format!("missing declaration for native C export `{name}`"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn native_c_header(&self) -> Result<String, String> {
        let mut registry = TypeRegistry::default();
        let mut prototypes = Vec::new();
        for name in self.exported_node_names() {
            let callable = self.exported_node(&name)?;
            let sig = self
                .signatures
                .get(&name)
                .ok_or_else(|| format!("missing signature for native C export `{name}`"))?;
            collect_abi_type(&mut registry, &sig.output);
            let output = registry.c_type(&sig.output);
            let input_items = native_c_input_items(&name, callable, &sig.input)?;
            let params = if input_items.is_empty() {
                "void".to_string()
            } else {
                callable
                    .inputs
                    .iter()
                    .zip(input_items)
                    .map(|(port, ty)| {
                        collect_abi_type(&mut registry, ty);
                        let c_ty = registry.c_type(ty);
                        format!("{c_ty} {}", c_ident(&port.name))
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            prototypes.push(format!("{output} {}({params});", sanitize_symbol(&name)));
        }
        Ok(native_c_header_source(&mut registry, &prototypes))
    }

    pub(super) fn emit_runtime_support_c(&mut self) -> Result<String, String> {
        let mut out = String::new();
        let uses_cv_runtime = self.uses_cv_runtime();
        let uses_http_runtime = self.uses_http_runtime();
        let uses_sqlite_runtime = self.uses_sqlite_runtime();
        self.collect_runtime_support_types(uses_cv_runtime);
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

    fn collect_runtime_support_types(&mut self, uses_cv_runtime: bool) {
        if uses_cv_runtime {
            let image = cv_image_ty();
            self.types.c_type(&image);
            self.types.c_type(&Ty::Faultable(Box::new(image)));
            self.types.c_type(&Ty::Faultable(Box::new(Ty::Bytes)));
        }
    }

    fn uses_cv_runtime(&self) -> bool {
        self.typed.callables.iter().any(|callable| {
            callable
                .chains
                .iter()
                .flat_map(|chain| chain.stages.iter())
                .any(|stage| self.stage_uses_cv_runtime(stage))
        })
    }

    fn uses_http_runtime(&self) -> bool {
        self.typed.callables.iter().any(|callable| {
            callable
                .chains
                .iter()
                .flat_map(|chain| chain.stages.iter())
                .any(|stage| self.stage_uses_http_runtime(stage))
        })
    }

    fn stage_uses_cv_runtime(&self, stage: &TypedStage) -> bool {
        self.typed_stage_uses_runtime(stage, |this, name| this.is_cv_runtime_name(name))
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

    fn stage_uses_http_runtime(&self, stage: &TypedStage) -> bool {
        self.typed_stage_uses_runtime(stage, |this, name| this.is_http_runtime_name(name))
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
        self.typed.callables.iter().any(|callable| {
            callable
                .chains
                .iter()
                .flat_map(|chain| chain.stages.iter())
                .any(|stage| self.stage_uses_sqlite_runtime(stage))
        })
    }

    fn stage_uses_sqlite_runtime(&self, stage: &TypedStage) -> bool {
        self.typed_stage_uses_runtime(stage, |this, name| this.is_sqlite_runtime_name(name))
    }

    fn typed_stage_uses_runtime(
        &self,
        stage: &TypedStage,
        uses_name: impl Fn(&Self, &str) -> bool,
    ) -> bool {
        match &stage.kind {
            TypedStageKind::Call { name, .. }
            | TypedStageKind::Map { name, .. }
            | TypedStageKind::Filter { name, .. }
            | TypedStageKind::Repeat { node: name, .. }
            | TypedStageKind::FaultMap { node: name, .. } => uses_name(self, name),
            TypedStageKind::Reduce { op, .. } | TypedStageKind::Scan { op, .. } => {
                uses_name(self, op)
            }
            TypedStageKind::Match { arms } => arms.iter().any(|arm| {
                matches!(&arm.target, TypedMatchTarget::Node { name, .. } if uses_name(self, name))
                    || matches!(
                        &arm.guard,
                        TypedMatchGuard::Call { node, .. } if uses_name(self, node)
                    )
            }),
            TypedStageKind::Bind { .. } | TypedStageKind::Field { .. } => false,
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

    fn collect_imports(&mut self) -> Result<(), String> {
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
                                stdlib_type_symbol(symbol.name)?,
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
                                    stdlib_type_symbol(symbol.name)?,
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
        Ok(())
    }

    fn collect_type_aliases(&mut self) -> Result<(), String> {
        let mut raw_aliases = HashMap::new();
        let mut raw_structs = HashMap::new();
        for decl in &self.module.declarations {
            match decl {
                Decl::TypeAlias(alias) => {
                    raw_aliases.insert(alias.name.clone(), alias.ty.clone());
                }
                Decl::Struct(struct_decl) => {
                    raw_structs.insert(struct_decl.name.clone(), struct_decl.fields.clone());
                }
                _ => {}
            }
        }
        let mut resolved = HashMap::new();
        for name in raw_structs.keys() {
            let mut resolving = Vec::new();
            let ty = self.resolve_struct_type(
                name,
                &raw_aliases,
                &raw_structs,
                &mut resolved,
                &mut resolving,
            )?;
            self.aliases.insert(name.clone(), ty);
        }
        for name in raw_aliases.keys() {
            let mut resolving = Vec::new();
            let ty = self.resolve_alias(
                name,
                &raw_aliases,
                &raw_structs,
                &mut resolved,
                &mut resolving,
            )?;
            self.aliases.insert(name.clone(), ty);
        }
        Ok(())
    }

    fn resolve_alias(
        &self,
        name: &str,
        raw_aliases: &HashMap<String, String>,
        raw_structs: &HashMap<String, Vec<Port>>,
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
        let text = raw_aliases
            .get(name)
            .ok_or_else(|| format!("unknown type alias `{name}`"))?;
        resolving.push(name.to_string());
        let ty = self.resolve_alias_type(
            parse_type(text)?,
            raw_aliases,
            raw_structs,
            resolved,
            resolving,
        )?;
        resolving.pop();
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    }

    fn resolve_struct_type(
        &self,
        name: &str,
        raw_aliases: &HashMap<String, String>,
        raw_structs: &HashMap<String, Vec<Port>>,
        resolved: &mut HashMap<String, Ty>,
        resolving: &mut Vec<String>,
    ) -> Result<Ty, String> {
        if let Some(ty) = resolved.get(name) {
            return Ok(ty.clone());
        }
        if resolving.iter().any(|item| item == name) {
            resolving.push(name.to_string());
            return Err(format!(
                "cyclic type declaration `{}`",
                resolving.join(" -> ")
            ));
        }
        let fields = raw_structs
            .get(name)
            .ok_or_else(|| format!("unknown struct `{name}`"))?;
        resolving.push(name.to_string());
        let mut out = Vec::with_capacity(fields.len());
        for field in fields {
            out.push((
                field.name.clone(),
                self.resolve_alias_type(
                    parse_type(&field.ty)?,
                    raw_aliases,
                    raw_structs,
                    resolved,
                    resolving,
                )?,
            ));
        }
        resolving.pop();
        let ty = Ty::Struct {
            name: name.to_string(),
            fields: out,
        };
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    }

    fn resolve_alias_type(
        &self,
        ty: Ty,
        raw_aliases: &HashMap<String, String>,
        raw_structs: &HashMap<String, Vec<Port>>,
        resolved: &mut HashMap<String, Ty>,
        resolving: &mut Vec<String>,
    ) -> Result<Ty, String> {
        match ty {
            Ty::Var(name) => {
                if raw_aliases.contains_key(&name) {
                    self.resolve_alias(&name, raw_aliases, raw_structs, resolved, resolving)
                } else if raw_structs.contains_key(&name) {
                    self.resolve_struct_type(&name, raw_aliases, raw_structs, resolved, resolving)
                } else {
                    Err(format!("unknown type `{name}`"))
                }
            }
            Ty::Faultable(item) => Ok(Ty::Faultable(Box::new(self.resolve_alias_type(
                *item,
                raw_aliases,
                raw_structs,
                resolved,
                resolving,
            )?))),
            Ty::Seq(item) => Ok(Ty::Seq(Box::new(self.resolve_alias_type(
                *item,
                raw_aliases,
                raw_structs,
                resolved,
                resolving,
            )?))),
            Ty::Stream(item) => Ok(Ty::Stream(Box::new(self.resolve_alias_type(
                *item,
                raw_aliases,
                raw_structs,
                resolved,
                resolving,
            )?))),
            Ty::OneOf(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_alias_type(
                        item,
                        raw_aliases,
                        raw_structs,
                        resolved,
                        resolving,
                    )?);
                }
                Ok(Ty::OneOf(out))
            }
            Ty::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_alias_type(
                        item,
                        raw_aliases,
                        raw_structs,
                        resolved,
                        resolving,
                    )?);
                }
                Ok(Ty::Tuple(out))
            }
            Ty::Struct { name, fields } => {
                let mut out = Vec::with_capacity(fields.len());
                for (field, ty) in fields {
                    out.push((
                        field,
                        self.resolve_alias_type(ty, raw_aliases, raw_structs, resolved, resolving)?,
                    ));
                }
                Ok(Ty::Struct { name, fields: out })
            }
            other => Ok(other),
        }
    }

    fn collect_callables(&mut self) -> Result<(), String> {
        for callable in &self.typed.callables {
            if self
                .callables
                .insert(callable.name.clone(), callable)
                .is_some()
            {
                return Err(format!("duplicate declaration `{}`", callable.name));
            }
            let signature = self.typed_signature(&callable.name)?;
            if self
                .signatures
                .insert(callable.name.clone(), signature)
                .is_some()
            {
                return Err(format!("duplicate declaration `{}`", callable.name));
            }
        }
        Ok(())
    }

    fn collect_foreigns(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            let Decl::Foreign(foreign) = decl else {
                continue;
            };
            for node in &foreign.nodes {
                match (&foreign.target, &foreign.source) {
                    (ForeignTarget::Js, ForeignSource::Module(_) | ForeignSource::Global(_)) => {
                        if !self.foreign_js.insert(node.name.clone()) {
                            return Err(format!("duplicate declaration `{}`", node.name));
                        }
                    }
                    (ForeignTarget::C, ForeignSource::CHeader { header, source }) => {
                        if self
                            .foreign_c
                            .insert(
                                node.name.clone(),
                                ForeignCBinding {
                                    symbol: node.symbol.clone(),
                                    header: header.clone(),
                                    source: source.clone(),
                                },
                            )
                            .is_some()
                        {
                            return Err(format!("duplicate declaration `{}`", node.name));
                        }
                    }
                    _ => {
                        return Err(format!(
                            "foreign declaration `{}` has an incompatible target/source",
                            node.name
                        ));
                    }
                }
                let signature = self.typed_signature(&node.name)?;
                if self
                    .signatures
                    .insert(node.name.clone(), signature)
                    .is_some()
                {
                    return Err(format!("duplicate declaration `{}`", node.name));
                }
            }
        }
        Ok(())
    }

    fn typed_signature(&self, name: &str) -> Result<Signature, String> {
        self.typed
            .signature_for(name)
            .ok_or_else(|| format!("missing typed signature for `{name}`"))
    }

    pub(super) fn parse_declared_type(&self, text: &str) -> Result<Ty, String> {
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
            Ty::Struct { name, fields } => {
                let mut out = Vec::with_capacity(fields.len());
                for (field, ty) in fields {
                    out.push((field, self.resolve_declared_type(ty)?));
                }
                Ok(Ty::Struct { name, fields: out })
            }
            other => Ok(other),
        }
    }

    fn resolve_signature_type(&self, ty: Ty) -> Result<Ty, String> {
        match ty {
            Ty::Var(name) => Ok(self.aliases.get(&name).cloned().unwrap_or(Ty::Var(name))),
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
            Ty::Struct { name, fields } => {
                let mut out = Vec::with_capacity(fields.len());
                for (field, ty) in fields {
                    out.push((field, self.resolve_signature_type(ty)?));
                }
                Ok(Ty::Struct { name, fields: out })
            }
            other => Ok(other),
        }
    }

    fn emit_callable(&mut self, out: &mut String, callable: &TypedCallable) -> Result<(), String> {
        self.validate_gpu_host_callable(callable)?;
        self.temp = 0;
        let symbol = if matches!(callable.kind, crate::typecheck::TypedCallableKind::Program) {
            "flow_program_main".to_string()
        } else {
            user_fn_name(&callable.name)
        };
        let sig = callable.signature.clone();
        let input_ty = self.types.c_type(&sig.input);
        let output_ty = self.types.c_type(&sig.output);
        if matches!(callable.kind, crate::typecheck::TypedCallableKind::Node)
            && self.emit_accumulator_fusion(out, callable, &symbol, &sig)?
        {
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
                let ty = port.ty.clone();
                let c_ty = self.types.c_type(&ty);
                let var = c_ident(&port.name);
                out.push_str(&format!("  {c_ty} {var} = input;\n"));
                env.insert(port.name.clone(), Value { code: var, ty });
            }
            ports => {
                for (index, port) in ports.iter().enumerate() {
                    let ty = port.ty.clone();
                    let c_ty = self.types.c_type(&ty);
                    let var = c_ident(&port.name);
                    out.push_str(&format!("  {c_ty} {var} = input.f{index};\n"));
                    env.insert(port.name.clone(), Value { code: var, ty });
                }
            }
        }

        let chains = fuse_single_use_chains(callable);
        let mut fused_callable = callable.clone();
        fused_callable.chains = chains.clone();
        let fused_reductions = self.gpu_plan.range_map_reductions(&fused_callable);
        let fused_by_reduce = fused_reductions
            .iter()
            .cloned()
            .map(|reduction| (reduction.reduce_chain, reduction))
            .collect::<HashMap<_, _>>();
        let fused_skip = fused_reductions
            .iter()
            .flat_map(|reduction| {
                [
                    reduction.range_chain,
                    reduction.map_chain,
                    reduction.reduce_chain,
                ]
            })
            .collect::<HashSet<_>>();
        let mut chain_index = 0;
        while chain_index < chains.len() {
            if let Some(reduction) = fused_by_reduce.get(&chain_index) {
                self.emit_gpu_range_map_reduction(out, reduction, &mut env)?;
                chain_index += 1;
                continue;
            }
            if fused_skip.contains(&chain_index) {
                chain_index += 1;
                continue;
            }
            let batch_len = self.parallel_chain_batch_len(&chains[chain_index..], &env)?;
            let batch_crosses_fused_chain =
                (chain_index..chain_index + batch_len).any(|index| fused_skip.contains(&index));
            if batch_len > 1 && !batch_crosses_fused_chain {
                self.emit_parallel_chain_batch(
                    out,
                    &chains[chain_index..chain_index + batch_len],
                    &mut env,
                )?;
                chain_index += batch_len;
            } else {
                self.emit_chain(out, &chains[chain_index], &mut env)?;
                chain_index += 1;
            }
        }

        let result = self.emit_outputs(out, callable, &env)?;
        out.push_str(&format!("  return {};\n", result.code));
        out.push_str("}\n\n");
        Ok(())
    }

    fn emit_outputs(
        &mut self,
        out: &mut String,
        callable: &TypedCallable,
        env: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        match callable.outputs.as_slice() {
            [] => Err(format!("`{}` must declare an output", callable.name)),
            [output] => {
                let value = env
                    .get(&output.name)
                    .cloned()
                    .ok_or_else(|| format!("output `{}` is never bound", output.name))?;
                let expected = output.ty.clone();
                self.emit_coerced_value(out, value, &expected)
            }
            outputs => {
                let mut values = Vec::new();
                let mut types = Vec::new();
                for output in outputs {
                    let expected = output.ty.clone();
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
        callable: &TypedCallable,
        symbol: &str,
        sig: &Signature,
    ) -> Result<bool, String> {
        let [left_port, right_port, score_port] = callable.inputs.as_slice() else {
            return Ok(false);
        };
        let [out_left, out_right, out_score] = callable.outputs.as_slice() else {
            return Ok(false);
        };
        if left_port.ty != Ty::Seq(Box::new(Ty::Real))
            || right_port.ty != Ty::Seq(Box::new(Ty::Real))
            || score_port.ty != Ty::Real
            || out_left.ty != Ty::Seq(Box::new(Ty::Real))
            || out_right.ty != Ty::Seq(Box::new(Ty::Real))
            || out_score.ty != Ty::Real
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
                match (&chain.source.kind, binding) {
                    (TypedEndpointKind::Variable(name), out)
                        if name == &left_port.name && out == out_left.name =>
                    {
                        left_passthrough = true;
                        continue;
                    }
                    (TypedEndpointKind::Variable(name), out)
                        if name == &right_port.name && out == out_right.name =>
                    {
                        right_passthrough = true;
                        continue;
                    }
                    _ => return Ok(false),
                }
            }
            if let [stage] = stages
                && let TypedStageKind::Call { name, .. } = &stage.kind
            {
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
                if matches!(&chain.source.kind, TypedEndpointKind::Variable(name) if name == &left_port.name)
                    && self.fusion_for_name(name) == Some(Fusion::MapReduceAdd(MapOp::Square))
                {
                    reductions.insert(binding.to_string(), ReductionTerm::LeftSquare);
                    continue;
                }
            }
            if let [stage] = stages
                && let TypedStageKind::Call { name, .. } = &stage.kind
                && self.is_add(name)
            {
                let TypedEndpointKind::Tuple(items) = &chain.source.kind else {
                    return Ok(false);
                };
                let [left, right] = items.as_slice() else {
                    return Ok(false);
                };
                let (TypedEndpointKind::Variable(left), TypedEndpointKind::Variable(right)) =
                    (&left.kind, &right.kind)
                else {
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

    fn parallel_chain_batch_len(
        &self,
        chains: &[TypedChain],
        env: &HashMap<String, Value>,
    ) -> Result<usize, String> {
        let mut produced = HashSet::new();
        let mut len = 0;
        for chain in chains {
            let Some(outputs) = self.parallel_chain_outputs(chain, env, &produced)? else {
                break;
            };
            for output in outputs {
                produced.insert(output);
            }
            len += 1;
        }
        Ok(if len > 1 { len } else { 0 })
    }

    fn parallel_chain_outputs(
        &self,
        chain: &TypedChain,
        env: &HashMap<String, Value>,
        produced: &HashSet<String>,
    ) -> Result<Option<Vec<String>>, String> {
        let Some(TypedStageKind::Bind { target }) = chain.stages.last().map(|stage| &stage.kind)
        else {
            return Ok(None);
        };
        let mut outputs = Vec::new();
        collect_binding_target_vars(target, &mut outputs);
        if outputs.is_empty()
            || outputs
                .iter()
                .any(|name| env.contains_key(name) || produced.contains(name))
        {
            return Ok(None);
        }
        let mut inputs = HashMap::new();
        count_endpoint_vars(&chain.source, &mut inputs);
        for stage in &chain.stages {
            count_stage_endpoint_vars(stage, &mut inputs);
        }
        if inputs
            .keys()
            .any(|name| !env.contains_key(name) || produced.contains(name))
        {
            return Ok(None);
        }
        if !self.is_parallel_safe_endpoint(&chain.source, &mut HashSet::new())
            || !chain
                .stages
                .iter()
                .all(|stage| self.is_parallel_safe_stage(stage, &mut HashSet::new()))
        {
            return Ok(None);
        }
        match self.chain_final_value_type(chain, env) {
            Ok(Some(_)) => Ok(Some(outputs)),
            Ok(None) | Err(_) => Ok(None),
        }
    }

    fn emit_parallel_chain_batch(
        &mut self,
        out: &mut String,
        chains: &[TypedChain],
        env: &mut HashMap<String, Value>,
    ) -> Result<(), String> {
        let mut helpers = Vec::new();
        for chain in chains {
            helpers.push(self.emit_parallel_chain_helper(chain, env)?);
        }
        for helper in &helpers {
            out.push_str(&format!("  {} {};\n", helper.ctx_ty, helper.ctx));
            for input in &helper.inputs {
                out.push_str(&format!(
                    "  {}.{} = {};\n",
                    helper.ctx, input.field, input.value.code
                ));
            }
        }
        let fns = self.next_temp();
        let ctxs = self.next_temp();
        out.push_str(&format!(
            "  FaParallelTaskFn {fns}[{}] = {{ {} }};\n",
            helpers.len(),
            helpers
                .iter()
                .map(|helper| helper.worker.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!(
            "  void *{ctxs}[{}] = {{ {} }};\n",
            helpers.len(),
            helpers
                .iter()
                .map(|helper| format!("&{}", helper.ctx))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!(
            "  fa_parallel_tasks({}, {fns}, {ctxs});\n",
            helpers.len()
        ));
        for helper in helpers {
            self.emit_bind_target(
                out,
                &helper.target,
                Value {
                    code: format!("{}.result", helper.ctx),
                    ty: helper.output_ty,
                },
                env,
            )?;
        }
        Ok(())
    }

    fn emit_parallel_chain_helper(
        &mut self,
        chain: &TypedChain,
        env: &HashMap<String, Value>,
    ) -> Result<ParallelChainHelper, String> {
        let id = self.parallel_helper;
        self.parallel_helper += 1;
        let worker = format!("fa_parallel_chain_worker_{id}");
        let ctx_ty = format!("{worker}_Ctx");
        let ctx = self.next_temp();
        let output_ty = self
            .chain_final_value_type(chain, env)?
            .ok_or_else(|| "parallel chain expected final binding".to_string())?;
        let output_c_ty = self.types.c_type(&output_ty);
        let mut input_names = BTreeSet::new();
        collect_endpoint_var_names(&chain.source, &mut input_names);
        for stage in &chain.stages {
            collect_stage_endpoint_var_names(stage, &mut input_names);
        }
        let mut inputs = Vec::new();
        for (index, name) in input_names.into_iter().enumerate() {
            let value = env
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("parallel chain input `{name}` is unavailable"))?;
            inputs.push(ParallelChainInput {
                name,
                field: format!("in_{index}"),
                c_ty: self.types.c_type(&value.ty),
                value,
            });
        }

        let mut helper = String::new();
        helper.push_str("typedef struct { ");
        for input in &inputs {
            helper.push_str(&format!("{} {}; ", input.c_ty, input.field));
        }
        helper.push_str(&format!("{output_c_ty} result; }} {ctx_ty};\n"));
        helper.push_str(&format!(
            "static void {worker}(void *ctx_ptr) {{\n  {ctx_ty} *ctx = ({ctx_ty} *)ctx_ptr;\n"
        ));
        let mut helper_env = HashMap::new();
        for input in &inputs {
            helper_env.insert(
                input.name.clone(),
                Value {
                    code: format!("ctx->{}", input.field),
                    ty: input.value.ty.clone(),
                },
            );
        }
        let (target, value) = self
            .emit_chain_final_value(&mut helper, chain, &mut helper_env)?
            .ok_or_else(|| "parallel chain expected final binding".to_string())?;
        helper.push_str(&format!("  ctx->result = {};\n}}\n\n", value.code));
        self.parallel_helpers.push_str(&helper);
        Ok(ParallelChainHelper {
            worker,
            ctx_ty,
            ctx,
            inputs,
            target,
            output_ty,
        })
    }

    fn emit_chain_final_value(
        &mut self,
        out: &mut String,
        chain: &TypedChain,
        env: &mut HashMap<String, Value>,
    ) -> Result<Option<(BindingTarget, Value)>, String> {
        let Some(TypedStageKind::Bind { target }) = chain.stages.last().map(|stage| &stage.kind)
        else {
            return Ok(None);
        };
        let source_expected = self.chain_source_expected_type(chain)?;
        let mut value =
            self.emit_endpoint_expected(out, &chain.source, env, source_expected.as_ref())?;
        for stage in &chain.stages[..chain.stages.len() - 1] {
            match &stage.kind {
                TypedStageKind::Call { name, .. } => {
                    value = self.emit_call(out, name, value.clone())?;
                }
                TypedStageKind::Map { name, .. } => {
                    value = self.emit_map(out, name, value.clone())?;
                }
                TypedStageKind::Filter { name, .. } => {
                    value = self.emit_filter(out, name, value.clone())?;
                }
                TypedStageKind::Field { name } => {
                    value = self.emit_field(name, value.clone())?;
                }
                TypedStageKind::Repeat { count, node, .. } => {
                    let count_value = self.emit_endpoint(out, count, env)?;
                    value = self.emit_repeat(out, node, value.clone(), count_value)?;
                }
                TypedStageKind::Reduce { op, identity, .. } => {
                    let identity_value = self.emit_endpoint(out, identity, env)?;
                    value = self.emit_reduce(out, op, value.clone(), identity_value)?;
                }
                TypedStageKind::Scan { op, identity, .. } => {
                    let identity_value = self.emit_endpoint(out, identity, env)?;
                    value = self.emit_scan(out, op, value.clone(), identity_value)?;
                }
                TypedStageKind::Match { arms } => {
                    value = self.emit_match(out, arms, stage.output.clone(), value.clone(), env)?;
                }
                TypedStageKind::Bind { .. } | TypedStageKind::FaultMap { .. } => return Ok(None),
            }
        }
        Ok(Some((target.clone(), value)))
    }

    fn chain_final_value_type(
        &self,
        chain: &TypedChain,
        env: &HashMap<String, Value>,
    ) -> Result<Option<Ty>, String> {
        let Some(last) = chain.stages.last() else {
            return Ok(None);
        };
        if !matches!(last.kind, TypedStageKind::Bind { .. }) {
            return Ok(None);
        }
        let _ = env;
        Ok(Some(last.output.clone()))
    }

    fn emit_chain(
        &mut self,
        out: &mut String,
        chain: &TypedChain,
        env: &mut HashMap<String, Value>,
    ) -> Result<(), String> {
        let source_expected = self.chain_source_expected_type(chain)?;
        let mut value =
            self.emit_endpoint_expected(out, &chain.source, env, source_expected.as_ref())?;
        let mut index = 0;
        while index < chain.stages.len() {
            let stage = &chain.stages[index];
            let is_last = index + 1 == chain.stages.len();
            match &stage.kind {
                TypedStageKind::Bind { target } if is_last => {
                    self.emit_bind_target(out, target, value.clone(), env)?;
                }
                TypedStageKind::Call { name, .. } => {
                    if let Some(TypedStageKind::Call { name: next, .. }) =
                        chain.stages.get(index + 1).map(|stage| &stage.kind)
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
                    if let Some(TypedStageKind::Map { name: map_name, .. }) =
                        chain.stages.get(index + 1).map(|stage| &stage.kind)
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
                TypedStageKind::Bind { .. } => {
                    return Err("binding targets may only appear as final stages".to_string());
                }
                TypedStageKind::Map { name, .. } => {
                    value = self.emit_map(out, name, value.clone())?;
                }
                TypedStageKind::FaultMap {
                    node, ok, fault, ..
                } => {
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
                TypedStageKind::Filter { name, .. } => {
                    value = self.emit_filter(out, name, value.clone())?;
                }
                TypedStageKind::Field { name } => {
                    value = self.emit_field(name, value.clone())?;
                }
                TypedStageKind::Repeat { count, node, .. } => {
                    let count_value = self.emit_endpoint(out, count, env)?;
                    value = self.emit_repeat(out, node, value.clone(), count_value)?;
                }
                TypedStageKind::Reduce { op, identity, .. } => {
                    let identity_value = self.emit_endpoint(out, identity, env)?;
                    value = self.emit_reduce(out, op, value.clone(), identity_value)?;
                }
                TypedStageKind::Scan { op, identity, .. } => {
                    let identity_value = self.emit_endpoint(out, identity, env)?;
                    value = self.emit_scan(out, op, value.clone(), identity_value)?;
                }
                TypedStageKind::Match { arms } => {
                    value = self.emit_match(out, arms, stage.output.clone(), value.clone(), env)?;
                }
            }
            index += 1;
        }
        Ok(())
    }

    fn emit_gpu_range_map_reduction(
        &mut self,
        out: &mut String,
        reduction: &gpu::GpuRangeMapReduction,
        env: &mut HashMap<String, Value>,
    ) -> Result<(), String> {
        let gpu::GpuScalarKind::I32 = reduction.map_kernel.scalar else {
            return Err("GPU range reductions currently require Int range map kernels".to_string());
        };
        if reduction.output_ty != Ty::Int {
            return Err(format!(
                "GPU range reduction expected Int output, found `{}`",
                reduction.output_ty
            ));
        }
        let range_ty = Ty::Tuple(vec![Ty::Int, Ty::Int, Ty::Int]);
        let range =
            self.emit_endpoint_expected(out, &reduction.range_source, env, Some(&range_ty))?;
        let identity = self.emit_endpoint(out, &reduction.identity, env)?;
        let tmp = self.next_temp();
        out.push_str(&format!(
            "  int64_t {tmp} = fa_gpu_range_map_reduce_i64(\"{}\", {}.f0, {}.f1, {}.f2, {}, {});\n",
            c_string(&reduction.map_kernel.map_expr),
            range.code,
            range.code,
            range.code,
            gpu_reduce_op(&reduction.op),
            identity.code
        ));
        if env
            .insert(
                reduction.output_name.clone(),
                Value {
                    code: tmp,
                    ty: reduction.output_ty.clone(),
                },
            )
            .is_some()
        {
            return Err(format!(
                "value `{}` is bound more than once",
                reduction.output_name
            ));
        }
        Ok(())
    }

    fn chain_source_expected_type(&self, chain: &TypedChain) -> Result<Option<Ty>, String> {
        if !contains_empty_seq(&chain.source.ty) {
            return Ok(None);
        }
        match chain.stages.first().map(|stage| &stage.kind) {
            Some(TypedStageKind::Call { name, .. }) => Ok(Some(
                self.call_input_type_for_value(name, &chain.source.ty)?,
            )),
            Some(_) => Ok(chain.stages.first().map(|stage| stage.input.clone())),
            None => Ok(None),
        }
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
        endpoint: &TypedEndpoint,
        env: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        self.emit_endpoint_expected(out, endpoint, env, None)
    }

    fn emit_endpoint_expected(
        &mut self,
        out: &mut String,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, Value>,
        expected: Option<&Ty>,
    ) -> Result<Value, String> {
        match &endpoint.kind {
            TypedEndpointKind::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            TypedEndpointKind::NodeRef { name, .. } => {
                Err(format!("expected value, found node `{name}`"))
            }
            TypedEndpointKind::Int(value) => Ok(Value {
                code: value.to_string(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Real(value) => Ok(Value {
                code: format!("{value:.17e}"),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Bool(value) => Ok(Value {
                code: if *value { "true" } else { "false" }.to_string(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::String(value) => Ok(Value {
                code: format!(
                    "fa_bytes_borrowed(\"{}\", {})",
                    c_string(value),
                    value.len()
                ),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Unit => Ok(Value {
                code: "fa_unit()".to_string(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Tuple(items) => {
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
                let ty = expected.cloned().unwrap_or_else(|| endpoint.ty.clone());
                let c_ty = self.types.c_type(&ty);
                let tmp = self.next_temp();
                out.push_str(&format!("  {c_ty} {tmp};\n"));
                let Ty::Tuple(item_tys) = &ty else {
                    return Err(format!("tuple literal expected tuple type, found `{ty}`"));
                };
                for (index, (value, item_ty)) in values.iter().zip(item_tys.iter()).enumerate() {
                    self.emit_assign_value(out, &format!("{tmp}.f{index}"), item_ty, value)?;
                }
                Ok(Value { code: tmp, ty })
            }
            TypedEndpointKind::Seq(items) => {
                if items.is_empty() {
                    let seq_ty = match expected {
                        Some(seq_ty @ Ty::Seq(_)) => seq_ty,
                        Some(other) => {
                            return Err(format!(
                                "empty sequence literal expected Seq context, found `{other}`"
                            ));
                        }
                        None if matches!(endpoint.ty, Ty::Seq(_)) => &endpoint.ty,
                        None => {
                            return Err("empty sequence literals need a type context".to_string());
                        }
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
                    _ if items.iter().any(endpoint_contains_empty_seq) => match &endpoint.ty {
                        Ty::Seq(item) if !contains_empty_seq(item) && !contains_type_var(item) => {
                            inferred_item = item.as_ref().clone();
                            Some(&inferred_item)
                        }
                        _ => None,
                    },
                    _ if matches!(endpoint.ty, Ty::Seq(_)) => match &endpoint.ty {
                        Ty::Seq(item) if !contains_type_var(item) => {
                            inferred_item = item.as_ref().clone();
                            Some(&inferred_item)
                        }
                        _ => None,
                    },
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
            TypedEndpointKind::Struct { name, fields, .. } => {
                self.emit_struct_literal(out, name, fields, env, expected)
            }
            TypedEndpointKind::Eval { source, stages } => {
                let source_expected = self.inline_source_expected_type(source, stages)?;
                let mut value =
                    self.emit_endpoint_expected(out, source, env, source_expected.as_ref())?;
                for stage in stages {
                    match &stage.kind {
                        TypedStageKind::Call { name, .. } => {
                            value = self.emit_call(out, name, value)?;
                        }
                        TypedStageKind::Bind { .. } => {
                            return Err("inline evaluations cannot bind values".to_string());
                        }
                        TypedStageKind::Map { name, .. } => {
                            value = self.emit_map(out, name, value)?;
                        }
                        TypedStageKind::FaultMap { .. } => {
                            return Err("inline evaluations cannot use `fault map`".to_string());
                        }
                        TypedStageKind::Filter { name, .. } => {
                            value = self.emit_filter(out, name, value)?;
                        }
                        TypedStageKind::Field { name } => {
                            value = self.emit_field(name, value)?;
                        }
                        TypedStageKind::Repeat { count, node, .. } => {
                            let count_value = self.emit_endpoint(out, count, env)?;
                            value = self.emit_repeat(out, node, value, count_value)?;
                        }
                        TypedStageKind::Reduce { op, identity, .. } => {
                            let identity_value = self.emit_endpoint(out, identity, env)?;
                            value = self.emit_reduce(out, op, value, identity_value)?;
                        }
                        TypedStageKind::Scan { op, identity, .. } => {
                            let identity_value = self.emit_endpoint(out, identity, env)?;
                            value = self.emit_scan(out, op, value, identity_value)?;
                        }
                        TypedStageKind::Match { arms } => {
                            value = self.emit_match(out, arms, stage.output.clone(), value, env)?;
                        }
                    }
                }
                Ok(value)
            }
        }
    }

    fn inline_source_expected_type(
        &self,
        source: &TypedEndpoint,
        stages: &[TypedStage],
    ) -> Result<Option<Ty>, String> {
        if !contains_empty_seq(&source.ty) {
            return Ok(None);
        }
        match stages.first().map(|stage| &stage.kind) {
            Some(TypedStageKind::Call { name, .. }) => {
                Ok(Some(self.call_input_type_for_value(name, &source.ty)?))
            }
            Some(_) => Ok(stages.first().map(|stage| stage.input.clone())),
            None => Ok(None),
        }
    }

    fn emit_struct_literal(
        &mut self,
        out: &mut String,
        name: &str,
        fields: &[(String, TypedEndpoint)],
        env: &HashMap<String, Value>,
        expected: Option<&Ty>,
    ) -> Result<Value, String> {
        let ty = expected
            .cloned()
            .or_else(|| self.aliases.get(name).cloned())
            .ok_or_else(|| format!("unknown struct `{name}`"))?;
        let Ty::Struct {
            fields: expected_fields,
            ..
        } = &ty
        else {
            return Err(format!("`{name}` is not a struct"));
        };
        let c_ty = self.types.c_type(&ty);
        let tmp = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp};\n"));
        for (field, field_ty) in expected_fields {
            let (_, endpoint) = fields
                .iter()
                .find(|(candidate, _)| candidate == field)
                .ok_or_else(|| format!("struct `{name}` literal missing field `{field}`"))?;
            let value = self.emit_endpoint_expected(out, endpoint, env, Some(field_ty))?;
            self.emit_assign_value(out, &format!("{tmp}.{}", c_ident(field)), field_ty, &value)?;
        }
        Ok(Value { code: tmp, ty })
    }

    fn emit_field(&self, field: &str, value: Value) -> Result<Value, String> {
        let Ty::Struct { name, fields } = value.ty.clone() else {
            return Err(format!(
                "field `{field}` expected struct input, found `{}`",
                value.ty
            ));
        };
        let (_, ty) = fields
            .iter()
            .find(|(candidate, _)| candidate == field)
            .ok_or_else(|| format!("struct `{name}` has no field `{field}`"))?;
        Ok(Value {
            code: format!("{}.{}", value.code, c_ident(field)),
            ty: ty.clone(),
        })
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
        arms: &[TypedMatchArm],
        output_ty: Ty,
        subject: Value,
        env: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        let c_ty = self.types.c_type(&output_ty);
        let target = self.next_temp();
        out.push_str(&format!("  {c_ty} {target};\n"));

        for (index, arm) in arms.iter().enumerate() {
            match &arm.guard {
                TypedMatchGuard::Fallback => {
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
                TypedMatchGuard::Call { node, args, .. } => {
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
            .filter(|arm| !matches!(arm.guard, TypedMatchGuard::Fallback))
        {
            out.push_str("  }\n");
        }
        Ok(Value {
            code: target,
            ty: output_ty,
        })
    }

    fn emit_assign_match_target(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        arm_target: &TypedMatchTarget,
        subject: &Value,
        env: &HashMap<String, Value>,
    ) -> Result<(), String> {
        match arm_target {
            TypedMatchTarget::Node { name, .. } => {
                self.emit_assign_call(out, target, output_ty, name, &subject.code, &subject.ty)
            }
            TypedMatchTarget::Value(endpoint) => {
                let value = self.emit_endpoint(out, endpoint, env)?;
                self.emit_assign_value(out, target, output_ty, &value)
            }
        }
    }

    fn emit_match_guard_input(
        &mut self,
        out: &mut String,
        subject: Value,
        args: &[TypedEndpoint],
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
        if let Some(binding) = self.foreign_c.get(name) {
            out.push_str(&format!("  {target} = {}({input});\n", binding.symbol));
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
                "    {ctx_ty} *{ctx} = ({ctx_ty} *)fa_calloc(1, sizeof({ctx_ty}));\n"
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
        if let Some(kernel) = self
            .gpu_plan
            .kernel_for_map(name, item_ty.as_ref(), &output_item_ty)
        {
            let map_fn = match kernel.scalar {
                gpu::GpuScalarKind::I32 => "fa_gpu_map_i64",
                gpu::GpuScalarKind::F32 => "fa_gpu_map_f64",
            };
            out.push_str(&format!(
                "  {c_ty} {tmp} = {new_fn}({}.count);\n",
                input.code
            ));
            out.push_str(&format!(
                "  {map_fn}({}_wgsl, {}.items, {tmp}.items, {}.count);\n",
                kernel.id, input.code, input.code
            ));
            return Ok(Value {
                code: tmp,
                ty: output_ty,
            });
        }
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
        if self.gpu_enabled && matches!(canonical.as_str(), "add" | "min" | "max") {
            match item_ty.as_ref() {
                Ty::Int => {
                    let tmp = self.next_temp();
                    out.push_str(&format!(
                        "  int64_t {tmp} = fa_gpu_reduce_i64({}, {}.items, {}.count, {});\n",
                        gpu_reduce_op(&canonical),
                        input.code,
                        input.code,
                        identity.code
                    ));
                    return Ok(Value {
                        code: tmp,
                        ty: Ty::Int,
                    });
                }
                Ty::Real => {
                    let tmp = self.next_temp();
                    out.push_str(&format!(
                        "  double {tmp} = fa_gpu_reduce_f64({}, {}.items, {}.count, {});\n",
                        gpu_reduce_op(&canonical),
                        input.code,
                        input.code,
                        identity.code
                    ));
                    return Ok(Value {
                        code: tmp,
                        ty: Ty::Real,
                    });
                }
                _ => {}
            }
        }
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
        if let Some(plan) = self.gpu_repeat_accumulator(node, &input.ty) {
            return self.emit_gpu_repeat_accumulator(out, plan, input, count);
        }
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

    fn emit_gpu_repeat_accumulator(
        &mut self,
        out: &mut String,
        plan: GpuRepeatAccumulator,
        input: Value,
        count: Value,
    ) -> Result<Value, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("GPU repeat accumulator expected tuple state".to_string());
        };
        let c_ty = self.types.c_type(&input.ty);
        let tmp = self.next_temp();
        let iter = self.next_temp();
        out.push_str(&format!("  {c_ty} {tmp} = {};\n", input.code));
        out.push_str(&format!("  int64_t {iter} = {};\n", count.code));
        out.push_str(&format!("  if ({iter} > 0) {{\n"));
        match plan.kind {
            GpuRepeatAccumulatorKind::VectorScore => {
                if items.len() != 3 {
                    return Err("GPU vector accumulator expected three tuple fields".to_string());
                }
                out.push_str(&format!(
                    "    {tmp}.f2 = fa_gpu_repeat_vector_accum_f64({}, {}.f0.items, {}.f0.count, {}.f1.items, {}.f1.count, {}.f2, {iter});\n",
                    c_string_literal(&plan.wgsl),
                    input.code,
                    input.code,
                    input.code,
                    input.code,
                    input.code
                ));
            }
            GpuRepeatAccumulatorKind::MatrixScore => {
                if items.len() != 4 {
                    return Err("GPU matrix accumulator expected four tuple fields".to_string());
                }
                out.push_str(&format!(
                    "    {tmp}.f3 = fa_gpu_repeat_matrix_accum_f64({}, (const FaGpuSliceF64 *){}.f0.items, {}.f0.count, (const FaGpuSliceF64 *){}.f1.items, {}.f1.count, {}.f2.items, {}.f2.count, {}.f3, {iter});\n",
                    c_string_literal(&plan.wgsl),
                    input.code,
                    input.code,
                    input.code,
                    input.code,
                    input.code,
                    input.code,
                    input.code
                ));
            }
        }
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
            "  {item_c_ty} *{items} = ({item_c_ty} *)fa_calloc({cap}, sizeof({item_c_ty}));\n"
        ));
        out.push_str(&format!("  if (!{items}) fa_die_alloc();\n"));
        out.push_str(&format!("  if (!{input}.next) {{\n"));
        out.push_str(&format!(
            "    {target}.is_fault = true; {target}.fault = fa_fault_cstr(\"stream.to_seq: stream is not pull-readable\");\n"
        ));
        out.push_str("  } else {\n");
        out.push_str("    for (;;) {\n");
        out.push_str(&format!(
                "      if ({count} == {cap}) {{ {cap} *= 2; {item_c_ty} *next_items = ({item_c_ty} *)fa_realloc({items}, {cap} * sizeof({item_c_ty})); {items} = next_items; }}\n"
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
        out.push_str(&format!("  fa_free({items});\n"));
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

    pub(super) fn is_parallel_safe_name(&self, name: &str, visiting: &mut HashSet<String>) -> bool {
        if let Some(callable) = self.callables.get(name) {
            return self.is_parallel_safe_callable(callable, visiting);
        }
        self.is_parallel_safe_builtin(&self.canonical_name(name))
    }

    fn is_parallel_safe_callable(
        &self,
        callable: &TypedCallable,
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

    fn is_parallel_safe_stage(&self, stage: &TypedStage, visiting: &mut HashSet<String>) -> bool {
        match &stage.kind {
            TypedStageKind::Bind { .. } | TypedStageKind::Field { .. } => true,
            TypedStageKind::Call { name, .. }
            | TypedStageKind::Map { name, .. }
            | TypedStageKind::Filter { name, .. } => self.is_parallel_safe_name(name, visiting),
            TypedStageKind::FaultMap { node, .. } => self.is_parallel_safe_name(node, visiting),
            TypedStageKind::Repeat { count, node, .. } => {
                self.is_parallel_safe_endpoint(count, visiting)
                    && self.is_parallel_safe_name(node, visiting)
            }
            TypedStageKind::Reduce { op, identity, .. }
            | TypedStageKind::Scan { op, identity, .. } => {
                self.is_parallel_safe_endpoint(identity, visiting)
                    && self.is_parallel_safe_name(op, visiting)
            }
            TypedStageKind::Match { arms } => arms.iter().all(|arm| {
                let target_safe = match &arm.target {
                    TypedMatchTarget::Node { name, .. } => {
                        self.is_parallel_safe_name(name, visiting)
                    }
                    TypedMatchTarget::Value(endpoint) => {
                        self.is_parallel_safe_endpoint(endpoint, visiting)
                    }
                };
                target_safe
                    && match &arm.guard {
                        TypedMatchGuard::Call { node, args, .. } => {
                            self.is_parallel_safe_name(node, visiting)
                                && args
                                    .iter()
                                    .all(|arg| self.is_parallel_safe_endpoint(arg, visiting))
                        }
                        TypedMatchGuard::Fallback => true,
                    }
            }),
        }
    }

    fn is_parallel_safe_endpoint(
        &self,
        endpoint: &TypedEndpoint,
        visiting: &mut HashSet<String>,
    ) -> bool {
        match &endpoint.kind {
            TypedEndpointKind::NodeRef { name, .. } => self.is_parallel_safe_name(name, visiting),
            TypedEndpointKind::Tuple(items) | TypedEndpointKind::Seq(items) => items
                .iter()
                .all(|item| self.is_parallel_safe_endpoint(item, visiting)),
            TypedEndpointKind::Struct { fields, .. } => fields
                .iter()
                .all(|(_, item)| self.is_parallel_safe_endpoint(item, visiting)),
            TypedEndpointKind::Eval { source, stages } => {
                self.is_parallel_safe_endpoint(source, visiting)
                    && stages
                        .iter()
                        .all(|stage| self.is_parallel_safe_stage(stage, visiting))
            }
            TypedEndpointKind::Variable(_)
            | TypedEndpointKind::Int(_)
            | TypedEndpointKind::Real(_)
            | TypedEndpointKind::Bool(_)
            | TypedEndpointKind::String(_)
            | TypedEndpointKind::Unit => true,
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

    pub(super) fn call_output_type(&self, name: &str, input: &Ty) -> Result<Ty, String> {
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

    pub(super) fn call_input_type_for_value(&self, name: &str, actual: &Ty) -> Result<Ty, String> {
        let signatures = self.call_signatures(name)?;
        let mut last_error = None;
        for signature in signatures {
            let mut vars = HashMap::new();
            match match_input_types(&signature.input, actual, &mut vars) {
                Ok(()) => {
                    let input = substitute_partial(&signature.input, &vars);
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

    pub(super) fn canonical_name(&self, name: &str) -> String {
        self.stdlib_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    pub(super) fn validate_gpu_host_callable(
        &self,
        _callable: &TypedCallable,
    ) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn gpu_repeat_accumulator(
        &self,
        node: &str,
        input_ty: &Ty,
    ) -> Option<GpuRepeatAccumulator> {
        if !self.gpu_enabled {
            return None;
        }
        let callable = self.callables.get(node)?;
        if callable.effect != Effect::Pure {
            return None;
        }
        if self.recognize_vector_score_accumulator(callable, input_ty) {
            return Some(GpuRepeatAccumulator {
                kind: GpuRepeatAccumulatorKind::VectorScore,
                wgsl: vector_score_accumulator_wgsl(),
            });
        }
        if self.recognize_matrix_score_accumulator(callable, input_ty) {
            return Some(GpuRepeatAccumulator {
                kind: GpuRepeatAccumulatorKind::MatrixScore,
                wgsl: matrix_score_accumulator_wgsl(),
            });
        }
        None
    }

    fn recognize_vector_score_accumulator(&self, callable: &TypedCallable, input_ty: &Ty) -> bool {
        if input_ty
            != &Ty::Tuple(vec![
                Ty::Seq(Box::new(Ty::Real)),
                Ty::Seq(Box::new(Ty::Real)),
                Ty::Real,
            ])
        {
            return false;
        }
        let [left_port, right_port, score_port] = callable.inputs.as_slice() else {
            return false;
        };
        let [out_left, out_right, out_score] = callable.outputs.as_slice() else {
            return false;
        };
        if left_port.ty != Ty::Seq(Box::new(Ty::Real))
            || right_port.ty != Ty::Seq(Box::new(Ty::Real))
            || score_port.ty != Ty::Real
            || out_left.ty != left_port.ty
            || out_right.ty != right_port.ty
            || out_score.ty != Ty::Real
        {
            return false;
        }

        let mut reductions: HashMap<String, ReductionTerm> = HashMap::new();
        let mut additions: HashMap<String, (String, String)> = HashMap::new();
        let mut left_passthrough = false;
        let mut right_passthrough = false;

        let chains = fuse_single_use_chains(callable);
        for chain in &chains {
            let Some(binding) = final_variable(chain) else {
                return false;
            };
            let Some(stages) = stages_binding_output(chain, binding) else {
                return false;
            };
            if stages.is_empty() {
                match (&chain.source.kind, binding) {
                    (TypedEndpointKind::Variable(name), out)
                        if name == &left_port.name && out == out_left.name =>
                    {
                        left_passthrough = true;
                        continue;
                    }
                    (TypedEndpointKind::Variable(name), out)
                        if name == &right_port.name && out == out_right.name =>
                    {
                        right_passthrough = true;
                        continue;
                    }
                    _ => return false,
                }
            }
            if let [stage] = stages
                && let TypedStageKind::Call { name, .. } = &stage.kind
            {
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
                if matches!(&chain.source.kind, TypedEndpointKind::Variable(name) if name == &left_port.name)
                    && self.fusion_for_name(name) == Some(Fusion::MapReduceAdd(MapOp::Square))
                {
                    reductions.insert(binding.to_string(), ReductionTerm::LeftSquare);
                    continue;
                }
            }
            if let [stage] = stages
                && let TypedStageKind::Call { name, .. } = &stage.kind
                && self.is_add(name)
            {
                let TypedEndpointKind::Tuple(items) = &chain.source.kind else {
                    return false;
                };
                let [left, right] = items.as_slice() else {
                    return false;
                };
                let (TypedEndpointKind::Variable(left), TypedEndpointKind::Variable(right)) =
                    (&left.kind, &right.kind)
                else {
                    return false;
                };
                additions.insert(binding.to_string(), (left.clone(), right.clone()));
                continue;
            }
            return false;
        }

        if !left_passthrough || !right_passthrough || reductions.is_empty() {
            return false;
        }
        let mut expected = reductions.keys().cloned().collect::<Vec<_>>();
        expected.push(score_port.name.clone());
        expected.sort();
        let mut actual = flatten_add_terms(&out_score.name, &additions);
        actual.sort();
        actual == expected
    }

    fn recognize_matrix_score_accumulator(&self, callable: &TypedCallable, input_ty: &Ty) -> bool {
        let matrix_ty = Ty::Seq(Box::new(Ty::Seq(Box::new(Ty::Real))));
        let vector_ty = Ty::Seq(Box::new(Ty::Real));
        if input_ty
            != &Ty::Tuple(vec![
                matrix_ty.clone(),
                matrix_ty.clone(),
                vector_ty.clone(),
                Ty::Real,
            ])
        {
            return false;
        }
        let [left_port, right_port, vector_port, score_port] = callable.inputs.as_slice() else {
            return false;
        };
        let [out_left, out_right, out_vector, out_score] = callable.outputs.as_slice() else {
            return false;
        };
        if left_port.ty != matrix_ty
            || right_port.ty != matrix_ty
            || vector_port.ty != vector_ty
            || score_port.ty != Ty::Real
            || out_left.ty != matrix_ty
            || out_right.ty != matrix_ty
            || out_vector.ty != vector_ty
            || out_score.ty != Ty::Real
        {
            return false;
        }

        let mut reductions: HashMap<String, MatrixReductionTerm> = HashMap::new();
        let mut additions: HashMap<String, (String, String)> = HashMap::new();
        let mut left_passthrough = false;
        let mut right_passthrough = false;
        let mut vector_passthrough = false;

        let chains = fuse_single_use_chains(callable);
        for chain in &chains {
            let Some(binding) = final_variable(chain) else {
                return false;
            };
            let Some(stages) = stages_binding_output(chain, binding) else {
                return false;
            };
            if stages.is_empty() {
                match (&chain.source.kind, binding) {
                    (TypedEndpointKind::Variable(name), out)
                        if name == &left_port.name && out == out_left.name =>
                    {
                        left_passthrough = true;
                        continue;
                    }
                    (TypedEndpointKind::Variable(name), out)
                        if name == &right_port.name && out == out_right.name =>
                    {
                        right_passthrough = true;
                        continue;
                    }
                    (TypedEndpointKind::Variable(name), out)
                        if name == &vector_port.name && out == out_vector.name =>
                    {
                        vector_passthrough = true;
                        continue;
                    }
                    _ => return false,
                }
            }
            if let [first, second] = stages {
                if let (
                    TypedStageKind::Call {
                        name: first_name, ..
                    },
                    TypedStageKind::Call {
                        name: second_name, ..
                    },
                ) = (&first.kind, &second.kind)
                {
                    if matches_pair_source(&chain.source, &left_port.name, &right_port.name)
                        && self.is_matmul_name(first_name)
                        && self.fusion_for_name(second_name) == Some(Fusion::NestedSum)
                    {
                        reductions.insert(binding.to_string(), MatrixReductionTerm::ProductSum);
                        continue;
                    }
                    if matches_pair_source(&chain.source, &left_port.name, &vector_port.name)
                        && self.is_matvec_name(first_name)
                        && self.fusion_for_name(second_name) == Some(Fusion::Sum)
                    {
                        reductions.insert(binding.to_string(), MatrixReductionTerm::MatvecSum);
                        continue;
                    }
                    if matches!(&chain.source.kind, TypedEndpointKind::Variable(name) if name == &left_port.name)
                        && self.is_map_sum_callable(first_name)
                        && self.fusion_for_name(second_name) == Some(Fusion::Sum)
                    {
                        reductions.insert(binding.to_string(), MatrixReductionTerm::RowSumTotal);
                        continue;
                    }
                }
            }
            if let [stage] = stages
                && let TypedStageKind::Call { name, .. } = &stage.kind
                && self.is_add(name)
            {
                let TypedEndpointKind::Tuple(items) = &chain.source.kind else {
                    return false;
                };
                let [left, right] = items.as_slice() else {
                    return false;
                };
                let (TypedEndpointKind::Variable(left), TypedEndpointKind::Variable(right)) =
                    (&left.kind, &right.kind)
                else {
                    return false;
                };
                additions.insert(binding.to_string(), (left.clone(), right.clone()));
                continue;
            }
            return false;
        }

        if !left_passthrough || !right_passthrough || !vector_passthrough || reductions.is_empty() {
            return false;
        }
        let mut expected = reductions.keys().cloned().collect::<Vec<_>>();
        expected.push(score_port.name.clone());
        expected.sort();
        let mut actual = flatten_add_terms(&out_score.name, &additions);
        actual.sort();
        actual == expected
    }

    pub(super) fn next_temp(&mut self) -> String {
        let tmp = format!("t{}", self.temp);
        self.temp += 1;
        tmp
    }
}

fn vector_score_accumulator_wgsl() -> String {
    r#"struct FaGpuProgramParams {
  work_items: u32,
  iterations: u32,
  slice0_len: u32,
  slice1_len: u32,
  slice2_len: u32,
  slice3_len: u32,
  matrix0_rows: u32,
  matrix0_cols: u32,
  matrix1_rows: u32,
  matrix1_cols: u32,
  matrix2_rows: u32,
  matrix2_cols: u32,
  matrix3_rows: u32,
  matrix3_cols: u32,
  scalar0: f32,
  scalar1: f32,
  scalar2: f32,
  scalar3: f32,
};
@group(0) @binding(0) var<storage, read_write> fa_output: array<f32>;
@group(0) @binding(1) var<uniform> fa_params: FaGpuProgramParams;
@group(0) @binding(2) var<storage, read> fa_left: array<f32>;
@group(0) @binding(3) var<storage, read> fa_right: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) fa_gid: vec3<u32>) {
  let i = fa_gid.x;
  if (i >= fa_params.work_items) { return; }
  let left = fa_left[i];
  let right = fa_right[i];
  let delta = left - right;
  fa_output[i] = left * right + delta * delta + left * left;
}
"#
    .to_string()
}

fn matrix_score_accumulator_wgsl() -> String {
    r#"struct FaGpuProgramParams {
  work_items: u32,
  iterations: u32,
  slice0_len: u32,
  slice1_len: u32,
  slice2_len: u32,
  slice3_len: u32,
  matrix0_rows: u32,
  matrix0_cols: u32,
  matrix1_rows: u32,
  matrix1_cols: u32,
  matrix2_rows: u32,
  matrix2_cols: u32,
  matrix3_rows: u32,
  matrix3_cols: u32,
  scalar0: f32,
  scalar1: f32,
  scalar2: f32,
  scalar3: f32,
};
@group(0) @binding(0) var<storage, read_write> fa_output: array<f32>;
@group(0) @binding(1) var<uniform> fa_params: FaGpuProgramParams;
@group(0) @binding(2) var<storage, read> fa_vector: array<f32>;
@group(0) @binding(3) var<storage, read> fa_left: array<f32>;
@group(0) @binding(4) var<storage, read> fa_right: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) fa_gid: vec3<u32>) {
  let i = fa_gid.x;
  if (i >= fa_params.work_items) { return; }
  let inner = fa_params.matrix0_cols;
  let k = i % inner;
  let left = fa_left[i];
  var right_sum = 0.0;
  for (var col = 0u; col < fa_params.matrix1_cols; col = col + 1u) {
    right_sum = right_sum + fa_right[k * fa_params.matrix1_cols + col];
  }
  fa_output[i] = left * right_sum + left * fa_vector[k] + left;
}
"#
    .to_string()
}

fn c_string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n\"\n\""),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_ascii_graphic() || ch == ' ' => out.push(ch),
            ch => out.push_str(&format!("\\x{:02x}", ch as u32)),
        }
    }
    out.push('"');
    out
}
