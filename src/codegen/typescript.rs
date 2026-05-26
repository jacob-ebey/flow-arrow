use super::{
    GpuRepeatAccumulator, GpuRepeatAccumulatorKind, Ty, TypedCodegen, assignable_output_ty,
    binding_target_is_discard, builtin_output_type_plain, contains_empty_seq,
    format_binding_target_for_error, gpu, sequence_item_type,
};
use crate::ast::{BindingTarget, Decl, ForeignSource};
use crate::typecheck::{
    TypedCallable, TypedChain, TypedEndpoint, TypedEndpointKind, TypedMatchArm, TypedMatchGuard,
    TypedMatchTarget, TypedStageKind,
};
use std::collections::{HashMap, HashSet};

pub(super) fn emit_module(codegen: TypedCodegen<'_>) -> Result<String, String> {
    TypeScriptCodegen::new(codegen, TypeScriptEmitOptions::default()).emit()
}

pub(super) fn emit_module_with_options(
    codegen: TypedCodegen<'_>,
    options: TypeScriptEmitOptions,
) -> Result<String, String> {
    TypeScriptCodegen::new(codegen, options).emit()
}

pub(super) fn emit_module_artifacts_with_options(
    codegen: TypedCodegen<'_>,
    options: TypeScriptEmitOptions,
) -> Result<TypeScriptEmitOutput, String> {
    TypeScriptCodegen::new(codegen, options).emit_artifacts()
}

pub(super) fn scalar_worker_module_source(mappers: &[WorkerMapper]) -> String {
    scalar_worker_module_source_from_mappers(mappers)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TypeScriptEmitOptions {
    pub worker_concurrency: bool,
    pub worker_module_specifier: Option<String>,
    pub gpu: bool,
}

impl TypeScriptEmitOptions {
    fn has_runtime_lifecycle(&self) -> bool {
        self.worker_concurrency
    }
}

#[derive(Debug, Clone)]
struct TsValue {
    code: String,
    ty: Ty,
    tuple_items: Option<Vec<TsValue>>,
}

struct TsMatchParams<'a> {
    arms: &'a [TypedMatchArm],
    output_ty: Ty,
    subject: TsValue,
    env: &'a HashMap<String, TsValue>,
    indent: &'a str,
    preferred: Option<&'a str>,
}

struct TypeScriptCodegen<'a> {
    codegen: TypedCodegen<'a>,
    options: TypeScriptEmitOptions,
    temp: usize,
    used_idents: HashSet<String>,
    worker_mappers: Vec<WorkerMapper>,
    seen_worker_mapper_sources: HashMap<String, String>,
    gpu_plan: Option<gpu::GpuPlan>,
    async_callables: HashSet<String>,
}

struct WorkerMapBatchItem {
    source: TypedEndpoint,
    source_key: String,
    target: String,
    output_ty: Ty,
    worker_fn: &'static str,
    mapper_id: String,
}

struct SyncMapBatchItem {
    source: TypedEndpoint,
    source_key: String,
    target: String,
    mapper: String,
    item_ty: Ty,
    output_ty: Ty,
}

struct ReduceBatchItem {
    source: TypedEndpoint,
    source_key: String,
    target: String,
    op: String,
    identity: TypedEndpoint,
    item_ty: Ty,
    output_ty: Ty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncMapOutputStorage {
    Array,
    Float32Array,
    Float64Array,
    Int32Array,
}

struct RangeSyncMapBatch {
    range_source: TypedEndpoint,
    items: Vec<SyncMapBatchItem>,
}

enum AsyncChainBatchItem {
    Call {
        source: TypedEndpoint,
        target: String,
        output_ty: Ty,
        callee: String,
    },
    Map {
        source: TypedEndpoint,
        target: String,
        output_ty: Ty,
        function: &'static str,
        kernel_id: String,
        wgsl: String,
    },
    Reduce {
        source: TypedEndpoint,
        target: String,
        output_ty: Ty,
        function: &'static str,
        op: String,
        identity: TypedEndpoint,
    },
}

impl SyncMapBatchItem {
    fn storage(&self) -> SyncMapOutputStorage {
        sync_map_output_storage(&self.output_ty)
    }
}

impl ReduceBatchItem {
    fn references_any(&self, names: &HashSet<String>) -> bool {
        endpoint_references_any(&self.source, names)
            || endpoint_references_any(&self.identity, names)
    }
}

impl AsyncChainBatchItem {
    fn target(&self) -> &str {
        match self {
            Self::Call { target, .. } | Self::Map { target, .. } | Self::Reduce { target, .. } => {
                target
            }
        }
    }

    fn output_ty(&self) -> &Ty {
        match self {
            Self::Call { output_ty, .. }
            | Self::Map { output_ty, .. }
            | Self::Reduce { output_ty, .. } => output_ty,
        }
    }

    fn references_any(&self, names: &HashSet<String>) -> bool {
        match self {
            Self::Call { source, .. } => endpoint_references_any(source, names),
            Self::Map { source, .. } => endpoint_references_any(source, names),
            Self::Reduce {
                source, identity, ..
            } => endpoint_references_any(source, names) || endpoint_references_any(identity, names),
        }
    }
}

pub(super) struct TypeScriptEmitOutput {
    pub source: String,
    pub worker_mappers: Vec<WorkerMapper>,
}

trait AnyResultExt: Iterator + Sized {
    fn any_result<E, F>(self, predicate: F) -> Result<bool, E>
    where
        F: FnMut(Self::Item) -> Result<bool, E>;
}

impl<I> AnyResultExt for I
where
    I: Iterator + Sized,
{
    fn any_result<E, F>(self, mut predicate: F) -> Result<bool, E>
    where
        F: FnMut(Self::Item) -> Result<bool, E>,
    {
        for item in self {
            if predicate(item)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkerMapper {
    pub id: String,
    pub source: String,
}

impl<'a> TypeScriptCodegen<'a> {
    fn new(mut codegen: TypedCodegen<'a>, options: TypeScriptEmitOptions) -> Self {
        codegen.gpu_enabled = options.gpu;
        let gpu_plan = options.gpu.then(|| gpu::GpuPlan::analyze(codegen.typed));
        codegen.gpu_plan = gpu_plan.clone().unwrap_or_else(gpu::GpuPlan::empty);
        Self {
            codegen,
            options,
            temp: 0,
            used_idents: HashSet::new(),
            worker_mappers: Vec::new(),
            seen_worker_mapper_sources: HashMap::new(),
            gpu_plan,
            async_callables: HashSet::new(),
        }
    }

    fn emit(self) -> Result<String, String> {
        Ok(self.emit_artifacts()?.source)
    }

    fn emit_artifacts(mut self) -> Result<TypeScriptEmitOutput, String> {
        self.analyze_async_callables()?;
        let mut out = String::new();
        if self.options.gpu {
            out.push_str("import * as faGpuRuntimeModule from \"./flowarrow_gpu_runtime.mjs\";\n");
        }
        self.emit_foreign_js_imports(&mut out);
        out.push_str(TS_PRELUDE);
        if self.options.gpu {
            out.push_str(TS_GPU_WASM_PRELUDE);
        }
        self.emit_foreign_js_wrappers(&mut out)?;

        let callables = self.codegen.typed.callables.clone();
        let has_program_main = callables.iter().any(|callable| {
            matches!(callable.kind, crate::typecheck::TypedCallableKind::Program)
                && callable.name == "main"
        });

        for callable in &callables {
            self.emit_callable(&mut out, callable)?;
        }

        if self.options.worker_concurrency {
            self.emit_worker_lifecycle_exports(&mut out);
        }
        let main_is_async = self.async_callables.contains("main");
        if self.options.has_runtime_lifecycle() || (has_program_main && main_is_async) {
            self.emit_runtime_lifecycle_exports(&mut out);
        }

        if has_program_main {
            if self.options.has_runtime_lifecycle() || main_is_async {
                let main_call = if main_is_async {
                    "await main({ argv: __flowarrow_process.argv.slice(2) })"
                } else {
                    "main({ argv: __flowarrow_process.argv.slice(2) })"
                };
                let bootstrap =
                    "\nconst __flowarrow_process = (globalThis as any).process;\n\
const __flowarrow_main_url = __flowarrow_process?.argv?.[1]\n  ? new URL(__flowarrow_process.argv[1], \"file:\").href\n  : \"\";\n\
if (import.meta.url === __flowarrow_main_url) {\n  (async () => {\n    await __flowarrow_setup_runtime();\n    let __flowarrow_exit = 1n;\n    try {\n      const __flowarrow_result = __FLOWARROW_MAIN_CALL__;\n      __flowarrow_exit = faExitCode(__flowarrow_result);\n    } finally {\n      await __flowarrow_teardown_runtime();\n    }\n    __flowarrow_process.exit(Number(__flowarrow_exit));\n  })();\n}\n"
                    .replace("__FLOWARROW_MAIN_CALL__", main_call);
                out.push_str(&bootstrap);
            } else {
                out.push_str(
                    "\nconst __flowarrow_process = (globalThis as any).process;\n\
const __flowarrow_main_url = __flowarrow_process?.argv?.[1]\n  ? new URL(__flowarrow_process.argv[1], \"file:\").href\n  : \"\";\n\
if (import.meta.url === __flowarrow_main_url) {\n  const __flowarrow_result = main({ argv: __flowarrow_process.argv.slice(2) });\n  const __flowarrow_exit = faExitCode(__flowarrow_result);\n  __flowarrow_process.exit(Number(__flowarrow_exit));\n}\n",
                );
            }
        }

        Ok(TypeScriptEmitOutput {
            source: out,
            worker_mappers: self.worker_mappers,
        })
    }

    fn emit_worker_lifecycle_exports(&self, out: &mut String) {
        let mapper_ids = self
            .worker_mappers
            .iter()
            .map(|mapper| ts_string(&mapper.id))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "\nconst __flowarrow_worker_mapper_ids = [{mapper_ids}];\n\
faUseSharedNumericSequences = true;\n\
{}\n\
export async function __flowarrow_setup_workers(): Promise<void> {{\n\
  await faSetupScalarWorkerPools(__flowarrow_worker_mapper_ids);\n\
}}\n\
\n\
export async function __flowarrow_teardown_workers(): Promise<void> {{\n\
  await faTeardownScalarWorkerPools();\n\
}}\n",
            self.options
                .worker_module_specifier
                .as_ref()
                .map(|specifier| format!(
                    "faScalarWorkerModuleUrl = new URL({}, import.meta.url).href;",
                    ts_string(specifier)
                ))
                .unwrap_or_default()
        ));
    }

    fn emit_runtime_lifecycle_exports(&self, out: &mut String) {
        out.push_str("\nasync function __flowarrow_setup_runtime(): Promise<void> {\n");
        if self.options.worker_concurrency {
            out.push_str("  await __flowarrow_setup_workers();\n");
        }
        out.push_str("}\n");
        out.push_str("\nasync function __flowarrow_teardown_runtime(): Promise<void> {\n");
        if self.options.worker_concurrency {
            out.push_str("  await __flowarrow_teardown_workers();\n");
        }
        out.push_str("}\n");
    }

    fn emit_foreign_js_imports(&self, out: &mut String) {
        let mut module_sources = self
            .codegen
            .module
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                Decl::Foreign(foreign) => match &foreign.source {
                    ForeignSource::Module(specifier) => Some(specifier.as_str()),
                    ForeignSource::Global(_) => None,
                    ForeignSource::CHeader { .. } => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        module_sources.sort();
        module_sources.dedup();
        for specifier in module_sources {
            out.push_str(&format!(
                "import * as {} from {};\n",
                foreign_module_alias(specifier),
                ts_string(specifier)
            ));
        }
        if !out.is_empty() {
            out.push('\n');
        }
    }

    fn emit_foreign_js_wrappers(&self, out: &mut String) -> Result<(), String> {
        for decl in &self.codegen.module.declarations {
            let Decl::Foreign(foreign) = decl else {
                continue;
            };
            for node in &foreign.nodes {
                let signature =
                    self.codegen.signatures.get(&node.name).ok_or_else(|| {
                        format!("missing signature for foreign node `{}`", node.name)
                    })?;
                let params = node
                    .inputs
                    .iter()
                    .map(|port| {
                        Ok(format!(
                            "{}: {}",
                            ts_ident(&port.name),
                            ts_type(&self.codegen.parse_declared_type(&port.ty)?)
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .join(", ");
                out.push_str(&format!(
                    "\nfunction {}({params}): {} {{\n",
                    ts_ident(&node.name),
                    ts_type(&signature.output)
                ));
                let args = node
                    .inputs
                    .iter()
                    .map(|port| ts_ident(&port.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let callee = match &foreign.source {
                    ForeignSource::Module(specifier) => {
                        format!("{}.{}", foreign_module_alias(specifier), node.symbol)
                    }
                    ForeignSource::Global(name) => format!("{name}.{}", node.symbol),
                    ForeignSource::CHeader { .. } => {
                        return Err(
                            "foreign c declarations are supported only by native LLVM builds"
                                .to_string(),
                        );
                    }
                };
                let call = format!("{callee}({args})");
                if matches!(signature.output, Ty::Unit) {
                    out.push_str(&format!("  {call};\n  return undefined;\n}}\n"));
                } else {
                    out.push_str(&format!(
                        "  const __fa_result = {call};\n  return {};\n}}\n",
                        foreign_result_expr("__fa_result", &signature.output)
                    ));
                }
            }
        }
        Ok(())
    }

    fn analyze_async_callables(&mut self) -> Result<(), String> {
        let mut async_callables = HashSet::new();
        loop {
            let mut changed = false;
            for callable in &self.codegen.typed.callables {
                if async_callables.contains(&callable.name) {
                    continue;
                }
                if self.callable_needs_async(callable, &async_callables)? {
                    async_callables.insert(callable.name.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.async_callables = async_callables;
        Ok(())
    }

    fn callable_needs_async(
        &self,
        callable: &TypedCallable,
        async_callables: &HashSet<String>,
    ) -> Result<bool, String> {
        callable.chains.iter().any_result(|chain| {
            Ok(self.endpoint_needs_async(&chain.source, async_callables)?
                || chain
                    .stages
                    .iter()
                    .any_result(|stage| self.stage_needs_async(stage, async_callables))?)
        })
    }

    fn endpoint_needs_async(
        &self,
        endpoint: &TypedEndpoint,
        async_callables: &HashSet<String>,
    ) -> Result<bool, String> {
        match &endpoint.kind {
            TypedEndpointKind::Tuple(items) | TypedEndpointKind::Seq(items) => items
                .iter()
                .any_result(|item| self.endpoint_needs_async(item, async_callables)),
            TypedEndpointKind::Struct { fields, .. } => fields
                .iter()
                .any_result(|(_, item)| self.endpoint_needs_async(item, async_callables)),
            TypedEndpointKind::Eval { source, stages } => Ok(self
                .endpoint_needs_async(source, async_callables)?
                || stages
                    .iter()
                    .any_result(|stage| self.stage_needs_async(stage, async_callables))?),
            TypedEndpointKind::Variable(_)
            | TypedEndpointKind::NodeRef { .. }
            | TypedEndpointKind::Int(_)
            | TypedEndpointKind::Real(_)
            | TypedEndpointKind::Bool(_)
            | TypedEndpointKind::String(_)
            | TypedEndpointKind::Unit => Ok(false),
        }
    }

    fn stage_needs_async(
        &self,
        stage: &crate::typecheck::TypedStage,
        async_callables: &HashSet<String>,
    ) -> Result<bool, String> {
        match &stage.kind {
            TypedStageKind::Bind { .. } | TypedStageKind::Field { .. } => Ok(false),
            TypedStageKind::Call { name, .. } => Ok(async_callables.contains(name)),
            TypedStageKind::Map { name, .. } => {
                self.map_stage_needs_async(name, &stage.input, &stage.output, async_callables)
            }
            TypedStageKind::FaultMap { node, .. } => Ok(async_callables.contains(node)),
            TypedStageKind::Filter { name, .. } => Ok(async_callables.contains(name)),
            TypedStageKind::Repeat { count, node, .. } => Ok(self
                .endpoint_needs_async(count, async_callables)?
                || async_callables.contains(node)),
            TypedStageKind::Reduce { op, identity, .. } => Ok(self
                .endpoint_needs_async(identity, async_callables)?
                || self.reduce_stage_uses_gpu(&stage.input, op)
                || async_callables.contains(op)),
            TypedStageKind::Scan { op, identity, .. } => Ok(self
                .endpoint_needs_async(identity, async_callables)?
                || async_callables.contains(op)),
            TypedStageKind::Match { arms } => arms.iter().any_result(|arm| {
                let guard_async = match &arm.guard {
                    TypedMatchGuard::Call { node, args, .. } => {
                        async_callables.contains(node)
                            || args
                                .iter()
                                .any_result(|arg| self.endpoint_needs_async(arg, async_callables))?
                    }
                    TypedMatchGuard::Fallback => false,
                };
                let target_async = match &arm.target {
                    TypedMatchTarget::Node { name, .. } => async_callables.contains(name),
                    TypedMatchTarget::Value(endpoint) => {
                        self.endpoint_needs_async(endpoint, async_callables)?
                    }
                };
                Ok(guard_async || target_async)
            }),
        }
    }

    fn map_stage_needs_async(
        &self,
        name: &str,
        input_ty: &Ty,
        output_ty: &Ty,
        async_callables: &HashSet<String>,
    ) -> Result<bool, String> {
        if async_callables.contains(name) {
            return Ok(true);
        }
        let (is_faultable, seq_ty) = match input_ty {
            Ty::Faultable(inner) => (true, inner.as_ref()),
            other => (false, other),
        };
        let Ty::Seq(item_ty) = seq_ty else {
            return Ok(false);
        };
        let Ty::Seq(output_item_ty) = output_ty else {
            return Ok(false);
        };
        if !is_faultable
            && self.options.gpu
            && self
                .gpu_plan
                .as_ref()
                .and_then(|plan| plan.kernel_for_map(name, item_ty, output_item_ty))
                .is_some()
        {
            return Ok(true);
        }
        if !is_faultable
            && self.options.worker_concurrency
            && self
                .worker_mapper_source(name, item_ty, output_item_ty)?
                .is_some()
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn reduce_stage_uses_gpu(&self, input_ty: &Ty, op: &str) -> bool {
        if !self.options.gpu {
            return false;
        }
        let seq_ty = match input_ty {
            Ty::Faultable(_) => return false,
            other => other,
        };
        let Ty::Seq(item_ty) = seq_ty else {
            return false;
        };
        if matches!(item_ty.as_ref(), Ty::Faultable(_)) {
            return false;
        }
        matches!(
            self.codegen.canonical_name(op).as_str(),
            "add" | "min" | "max"
        ) && matches!(item_ty.as_ref(), Ty::I32 | Ty::F32 | Ty::F64)
    }

    fn emit_callable(&mut self, out: &mut String, callable: &TypedCallable) -> Result<(), String> {
        self.codegen.validate_gpu_host_callable(callable)?;
        self.temp = 0;
        self.used_idents.clear();
        self.reserve_internal_idents();
        let callable_idents = self
            .codegen
            .callables
            .keys()
            .chain(self.codegen.foreign_js.iter())
            .map(|name| ts_ident(name))
            .collect::<Vec<_>>();
        for ident in callable_idents {
            self.reserve_ident(&ident);
        }
        let signature = self
            .codegen
            .signatures
            .get(&callable.name)
            .cloned()
            .ok_or_else(|| format!("missing signature for `{}`", callable.name))?;
        let is_program = matches!(callable.kind, crate::typecheck::TypedCallableKind::Program);
        let export = if is_program || (callable.is_extern && !callable.name.starts_with("__flow_"))
        {
            "export "
        } else {
            ""
        };
        let fn_name = if is_program && callable.name == "main" {
            "main".to_string()
        } else {
            ts_ident(&callable.name)
        };
        self.reserve_ident(&fn_name);
        let return_ty = ts_type(&signature.output);

        let params = callable
            .inputs
            .iter()
            .map(|port| {
                let name = ts_ident(&port.name);
                self.reserve_ident(&name);
                Ok(format!("{name}: {}", ts_type(&port.ty)))
            })
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        if self.async_callables.contains(&callable.name) {
            out.push_str(&format!(
                "\n{export}async function {fn_name}({params}): Promise<{return_ty}> {{\n"
            ));
        } else {
            out.push_str(&format!(
                "\n{export}function {fn_name}({params}): {return_ty} {{\n"
            ));
        }

        let mut env = HashMap::new();
        for port in &callable.inputs {
            env.insert(
                port.name.clone(),
                ts_value(ts_ident(&port.name), port.ty.clone()),
            );
        }

        let mut chain_index = 0;
        while chain_index < callable.chains.len() {
            if let Some(batch) = self.range_sync_map_batch(callable, chain_index)? {
                let consumed = 1 + batch.items.len();
                self.emit_range_sync_map_batch(out, batch, &mut env, "  ")?;
                chain_index += consumed;
                continue;
            }
            let batch_len = self.async_chain_batch_len(&callable.chains[chain_index..])?;
            if batch_len > 0 {
                self.emit_async_chain_batch(
                    out,
                    &callable.chains[chain_index..chain_index + batch_len],
                    &mut env,
                    "  ",
                )?;
                chain_index += batch_len;
                continue;
            }
            let batch_len = self.reduce_batch_len(&callable.chains[chain_index..])?;
            if batch_len > 1 {
                self.emit_reduce_batch(
                    out,
                    &callable.chains[chain_index..chain_index + batch_len],
                    &mut env,
                    "  ",
                )?;
                chain_index += batch_len;
                continue;
            }
            let batch_len = self.worker_map_batch_len(&callable.chains[chain_index..], &env)?;
            if batch_len > 1 {
                self.emit_worker_map_batch(
                    out,
                    &callable.chains[chain_index..chain_index + batch_len],
                    &mut env,
                    "  ",
                )?;
                chain_index += batch_len;
            } else {
                let batch_len = self.sync_map_batch_len(&callable.chains[chain_index..])?;
                if batch_len > 1 {
                    self.emit_sync_map_batch(
                        out,
                        &callable.chains[chain_index..chain_index + batch_len],
                        &mut env,
                        "  ",
                    )?;
                    chain_index += batch_len;
                } else {
                    self.emit_chain(out, &callable.chains[chain_index], &mut env, "  ")?;
                    chain_index += 1;
                }
            }
        }

        let result = self.emit_outputs(callable, &env)?;
        let result = self.coerce_value(out, result, &signature.output, "  ")?;
        out.push_str(&format!("  return {};\n}}\n", result.code));
        Ok(())
    }

    fn emit_outputs(
        &mut self,
        callable: &TypedCallable,
        env: &HashMap<String, TsValue>,
    ) -> Result<TsValue, String> {
        match callable.outputs.as_slice() {
            [] => Ok(ts_value("undefined", Ty::Unit)),
            [output] => env
                .get(&output.name)
                .cloned()
                .ok_or_else(|| format!("output `{}` is never bound", output.name)),
            outputs => {
                let values = outputs
                    .iter()
                    .map(|output| {
                        env.get(&output.name)
                            .cloned()
                            .ok_or_else(|| format!("output `{}` is never bound", output.name))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let ty = Ty::Tuple(
                    outputs
                        .iter()
                        .map(|output| output.ty.clone())
                        .collect::<Vec<_>>(),
                );
                Ok(ts_tuple_value(values, ty))
            }
        }
    }

    fn async_chain_batch_len(&self, chains: &[TypedChain]) -> Result<usize, String> {
        let Some(first) = chains.first() else {
            return Ok(0);
        };
        if self.async_chain_batch_item(first)?.is_none() {
            return Ok(0);
        }
        let mut produced = HashSet::new();
        let mut len = 0;
        for chain in chains {
            let Some(item) = self.async_chain_batch_item(chain)? else {
                break;
            };
            if item.references_any(&produced) {
                break;
            }
            produced.insert(item.target().to_string());
            len += 1;
        }
        Ok(len)
    }

    fn async_chain_batch_item(
        &self,
        chain: &TypedChain,
    ) -> Result<Option<AsyncChainBatchItem>, String> {
        let [stage, bind] = chain.stages.as_slice() else {
            return Ok(None);
        };
        let TypedStageKind::Bind {
            target: BindingTarget::Variable(target),
        } = &bind.kind
        else {
            return Ok(None);
        };
        match &stage.kind {
            TypedStageKind::Call { name, .. } => {
                if !self.async_callables.contains(name) {
                    return Ok(None);
                }
                Ok(Some(AsyncChainBatchItem::Call {
                    source: chain.source.clone(),
                    target: target.clone(),
                    output_ty: stage.output.clone(),
                    callee: name.clone(),
                }))
            }
            TypedStageKind::Map { name, .. } => {
                let Ty::Seq(item_ty) = &stage.input else {
                    return Ok(None);
                };
                let Ty::Seq(output_item_ty) = &stage.output else {
                    return Ok(None);
                };
                let Some(kernel) = self
                    .gpu_plan
                    .as_ref()
                    .and_then(|plan| plan.kernel_for_map(name, item_ty, output_item_ty))
                    .cloned()
                else {
                    return Ok(None);
                };
                Ok(Some(AsyncChainBatchItem::Map {
                    source: chain.source.clone(),
                    target: target.clone(),
                    output_ty: stage.output.clone(),
                    function: kernel.scalar.map_function(),
                    kernel_id: kernel.id,
                    wgsl: kernel.wgsl,
                }))
            }
            TypedStageKind::Reduce { op, identity, .. } => {
                if !self.options.gpu {
                    return Ok(None);
                }
                let Ty::Seq(item_ty) = &stage.input else {
                    return Ok(None);
                };
                if matches!(item_ty.as_ref(), Ty::Faultable(_)) {
                    return Ok(None);
                }
                let canonical = self.codegen.canonical_name(op);
                if !matches!(canonical.as_str(), "add" | "min" | "max") {
                    return Ok(None);
                }
                let function = match item_ty.as_ref() {
                    Ty::I32 => "faGpuReduceI32",
                    Ty::F32 => "faGpuReduceF32",
                    Ty::F64 => "faGpuReduceF64",
                    _ => return Ok(None),
                };
                Ok(Some(AsyncChainBatchItem::Reduce {
                    source: chain.source.clone(),
                    target: target.clone(),
                    output_ty: stage.output.clone(),
                    function,
                    op: canonical,
                    identity: identity.clone(),
                }))
            }
            _ => Ok(None),
        }
    }

    fn emit_async_chain_batch(
        &mut self,
        out: &mut String,
        chains: &[TypedChain],
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let items = chains
            .iter()
            .map(|chain| {
                self.async_chain_batch_item(chain)?
                    .ok_or_else(|| "expected async chain batch item".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut targets = Vec::with_capacity(items.len());
        let mut output_types = Vec::with_capacity(items.len());
        let mut calls = Vec::with_capacity(items.len());
        for item in &items {
            let target = ts_ident(item.target());
            if !self.try_reserve_ident(&target) {
                return Err(format!("value `{}` is bound more than once", item.target()));
            }
            targets.push(target);
            output_types.push(item.output_ty().clone());
            calls.push(self.async_chain_batch_call(out, item, env, indent)?);
        }
        let tuple_ty = format!(
            "[{}]",
            output_types
                .iter()
                .map(ts_type)
                .collect::<Vec<_>>()
                .join(", ")
        );
        out.push_str(&format!(
            "{indent}const [{}]: {tuple_ty} = await Promise.all([{}]);\n",
            targets.join(", "),
            calls.join(", ")
        ));
        for ((item, target), output_ty) in items.iter().zip(targets).zip(output_types) {
            if env
                .insert(item.target().to_string(), ts_value(target, output_ty))
                .is_some()
            {
                return Err(format!("value `{}` is bound more than once", item.target()));
            }
        }
        Ok(())
    }

    fn async_chain_batch_call(
        &mut self,
        out: &mut String,
        item: &AsyncChainBatchItem,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<String, String> {
        match item {
            AsyncChainBatchItem::Call { source, callee, .. } => {
                let source = self.emit_endpoint(out, source, env, indent)?;
                let arity = self
                    .codegen
                    .callables
                    .get(callee)
                    .map(|callable| callable.inputs.len())
                    .ok_or_else(|| format!("missing callable `{callee}`"))?;
                Ok(format!(
                    "{}({})",
                    ts_ident(callee),
                    call_args(&source, arity)?
                ))
            }
            AsyncChainBatchItem::Map {
                source,
                function,
                kernel_id,
                wgsl,
                ..
            } => {
                let source = self.emit_endpoint(out, source, env, indent)?;
                Ok(format!(
                    "{}({}, {}, {})",
                    function,
                    source.code,
                    ts_string(kernel_id),
                    ts_string(wgsl)
                ))
            }
            AsyncChainBatchItem::Reduce {
                source,
                function,
                op,
                identity,
                ..
            } => {
                let source = self.emit_endpoint(out, source, env, indent)?;
                let identity = self.emit_endpoint(out, identity, env, indent)?;
                Ok(format!(
                    "{}({}, {}, {})",
                    function,
                    source.code,
                    ts_string(op),
                    identity.code
                ))
            }
        }
    }

    fn emit_chain(
        &mut self,
        out: &mut String,
        chain: &TypedChain,
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let source_expected = if contains_empty_seq(&chain.source.ty) {
            match chain.stages.first().map(|stage| &stage.kind) {
                Some(TypedStageKind::Call { name, .. }) => Some(
                    self.codegen
                        .call_input_type_for_value(name, &chain.source.ty)?,
                ),
                Some(_) => chain.stages.first().map(|stage| stage.input.clone()),
                None => None,
            }
        } else {
            None
        };
        let mut value =
            self.emit_endpoint_expected(out, &chain.source, env, source_expected.as_ref(), indent)?;

        for (index, stage) in chain.stages.iter().enumerate() {
            match &stage.kind {
                TypedStageKind::Bind { target } => {
                    self.bind_target(out, target, value.clone(), env, indent)?;
                }
                TypedStageKind::Call { name, .. } => {
                    let preferred = typed_final_bind_target_for_stage(chain, index)
                        .and_then(binding_target_name);
                    value = self.emit_call_preferred(out, name, value, indent, preferred)?;
                }
                TypedStageKind::Map { name, .. } => {
                    let preferred = typed_final_bind_target_for_stage(chain, index)
                        .and_then(binding_target_name);
                    value = self.emit_map(out, name, value, indent, preferred)?;
                }
                TypedStageKind::FaultMap {
                    node, ok, fault, ..
                } => {
                    let (ok_value, fault_value) =
                        self.emit_fault_map(out, node, value.clone(), indent, ok, fault)?;
                    if env.insert(ok.clone(), ok_value).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_value).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                TypedStageKind::Filter { name, .. } => {
                    let preferred = typed_final_bind_target_for_stage(chain, index)
                        .and_then(binding_target_name);
                    value = self.emit_filter(out, name, value, indent, preferred)?;
                }
                TypedStageKind::Field { name } => {
                    value = self.emit_field(name, value)?;
                }
                TypedStageKind::Repeat { count, node, .. } => {
                    let count = self.emit_endpoint(out, count, env, indent)?;
                    let preferred_target =
                        typed_final_bind_target_for_stage(chain, index).map(|target| target as &_);
                    value = self.emit_repeat(out, node, value, count, indent, preferred_target)?;
                }
                TypedStageKind::Reduce { op, identity, .. } => {
                    let identity = self.emit_endpoint(out, identity, env, indent)?;
                    let preferred = typed_final_bind_target_for_stage(chain, index)
                        .and_then(binding_target_name);
                    value = self.emit_reduce(out, op, value, identity, indent, preferred)?;
                }
                TypedStageKind::Scan { op, identity, .. } => {
                    let identity = self.emit_endpoint(out, identity, env, indent)?;
                    let preferred = typed_final_bind_target_for_stage(chain, index)
                        .and_then(binding_target_name);
                    value = self.emit_scan(out, op, value, identity, indent, preferred)?;
                }
                TypedStageKind::Match { arms } => {
                    let preferred = typed_final_bind_target_for_stage(chain, index)
                        .and_then(binding_target_name);
                    value = self.emit_match(
                        out,
                        TsMatchParams {
                            arms,
                            output_ty: stage.output.clone(),
                            subject: value,
                            env,
                            indent,
                            preferred,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn reduce_batch_len(&self, chains: &[TypedChain]) -> Result<usize, String> {
        let Some(first_chain) = chains.first() else {
            return Ok(0);
        };
        let Some(first) = self.reduce_batch_item(first_chain)? else {
            return Ok(0);
        };
        let source_key = first.source_key;
        let mut produced = HashSet::new();
        let mut len = 0;
        for chain in chains {
            let Some(item) = self.reduce_batch_item(chain)? else {
                break;
            };
            if item.source_key != source_key || item.references_any(&produced) {
                break;
            }
            produced.insert(item.target.clone());
            len += 1;
        }
        Ok(len)
    }

    fn reduce_batch_item(&self, chain: &TypedChain) -> Result<Option<ReduceBatchItem>, String> {
        let [reduce, bind] = chain.stages.as_slice() else {
            return Ok(None);
        };
        let TypedStageKind::Reduce { op, identity, .. } = &reduce.kind else {
            return Ok(None);
        };
        let TypedStageKind::Bind {
            target: BindingTarget::Variable(target),
        } = &bind.kind
        else {
            return Ok(None);
        };
        if self.reduce_stage_uses_gpu(&reduce.input, op) || self.async_callables.contains(op) {
            return Ok(None);
        }
        let Ty::Seq(item_ty) = &reduce.input else {
            return Ok(None);
        };
        if matches!(item_ty.as_ref(), Ty::Faultable(_))
            || matches!(&reduce.output, Ty::Faultable(_))
            || item_ty.as_ref() != &reduce.output
        {
            return Ok(None);
        }
        Ok(Some(ReduceBatchItem {
            source: chain.source.clone(),
            source_key: chain.source.label.clone(),
            target: target.clone(),
            op: op.clone(),
            identity: identity.clone(),
            item_ty: item_ty.as_ref().clone(),
            output_ty: reduce.output.clone(),
        }))
    }

    fn emit_reduce_batch(
        &mut self,
        out: &mut String,
        chains: &[TypedChain],
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let items = chains
            .iter()
            .map(|chain| {
                self.reduce_batch_item(chain)?
                    .ok_or_else(|| "expected reduce batch item".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut targets = Vec::with_capacity(items.len());
        for item in &items {
            let target = ts_ident(&item.target);
            if !self.try_reserve_ident(&target) {
                return Err(format!("value `{}` is bound more than once", item.target));
            }
            targets.push(target);
        }

        let mut source = self.emit_endpoint(out, &items[0].source, env, indent)?;
        if !ts_value_code_is_stable(&source.code) {
            let source_tmp = self.next_temp();
            out.push_str(&format!(
                "{indent}const {source_tmp}: {} = {};\n",
                ts_type(&source.ty),
                source.code
            ));
            source.code = source_tmp;
        }
        for (item, target) in items.iter().zip(targets.iter()) {
            let identity = self.emit_endpoint(out, &item.identity, env, indent)?;
            out.push_str(&format!(
                "{indent}let {target}: {} = {};\n",
                ts_type(&item.output_ty),
                identity.code
            ));
        }
        let item_name = self.next_temp();
        out.push_str(&format!(
            "{indent}for (const {item_name} of {}) {{\n",
            source.code
        ));
        for (item, target) in items.iter().zip(targets.iter()) {
            let pair_ty = Ty::Tuple(vec![item.output_ty.clone(), item.item_ty.clone()]);
            let pair = ts_tuple_value(
                vec![
                    ts_value(target.clone(), item.output_ty.clone()),
                    ts_value(item_name.clone(), item.item_ty.clone()),
                ],
                pair_ty,
            );
            let reduced = self.emit_call(out, &item.op, pair, &(indent.to_string() + "  "))?;
            out.push_str(&format!("{indent}  {target} = {};\n", reduced.code));
        }
        out.push_str(&format!("{indent}}}\n"));

        for (item, target) in items.iter().zip(targets) {
            if env
                .insert(
                    item.target.clone(),
                    ts_value(target, item.output_ty.clone()),
                )
                .is_some()
            {
                return Err(format!("value `{}` is bound more than once", item.target));
            }
        }
        Ok(())
    }

    fn worker_map_batch_len(
        &mut self,
        chains: &[TypedChain],
        env: &HashMap<String, TsValue>,
    ) -> Result<usize, String> {
        let Some(first) = self.worker_map_batch_item(&chains[0], env)? else {
            return Ok(0);
        };
        let source_key = first.source_key;
        let mut len = 1;
        for chain in chains.iter().skip(1) {
            let Some(item) = self.worker_map_batch_item(chain, env)? else {
                break;
            };
            if item.source_key != source_key {
                break;
            }
            len += 1;
        }
        Ok(len)
    }

    fn worker_map_batch_item(
        &mut self,
        chain: &TypedChain,
        _env: &HashMap<String, TsValue>,
    ) -> Result<Option<WorkerMapBatchItem>, String> {
        let [first, second] = chain.stages.as_slice() else {
            return Ok(None);
        };
        let TypedStageKind::Map { name, .. } = &first.kind else {
            return Ok(None);
        };
        let TypedStageKind::Bind {
            target: BindingTarget::Variable(target),
        } = &second.kind
        else {
            return Ok(None);
        };
        let input = chain.source.ty.clone();
        let Ty::Seq(item_ty) = input else {
            return Ok(None);
        };
        let output_item_ty = self.codegen.call_output_type(name, &item_ty)?;
        let output_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let Some((worker_fn, mapper_id)) =
            self.worker_map_call(name, item_ty.as_ref(), &output_item_ty)?
        else {
            return Ok(None);
        };
        Ok(Some(WorkerMapBatchItem {
            source: chain.source.clone(),
            source_key: chain.source.label.clone(),
            target: target.clone(),
            output_ty,
            worker_fn,
            mapper_id,
        }))
    }

    fn emit_worker_map_batch(
        &mut self,
        out: &mut String,
        chains: &[TypedChain],
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let items = chains
            .iter()
            .map(|chain| {
                self.worker_map_batch_item(chain, env)?
                    .ok_or_else(|| "expected worker map batch item".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut source = self.emit_endpoint(out, &items[0].source, env, indent)?;
        if !ts_value_code_is_stable(&source.code) {
            let source_tmp = self.next_temp();
            out.push_str(&format!(
                "{indent}const {source_tmp}: {} = {};\n",
                ts_type(&source.ty),
                source.code
            ));
            source.code = source_tmp;
        }
        let temp_names = items
            .iter()
            .map(|item| self.next_temp_or_preferred(Some(&item.target)))
            .collect::<Vec<_>>();
        for (item, temp) in items.iter().zip(temp_names.iter()) {
            out.push_str(&format!(
                "{indent}let {temp}: {};\n",
                ts_type(&item.output_ty)
            ));
        }
        let worker_count = self.next_temp();
        let per_map_worker_count = self.next_temp();
        out.push_str(&format!(
            "{indent}const {worker_count} = faDefaultScalarWorkerCount({}.length);\n",
            source.code
        ));
        out.push_str(&format!(
            "{indent}const {per_map_worker_count} = Math.max(1, Math.floor({worker_count} / {}));\n",
            items.len()
        ));
        let batch = self.next_temp();
        let calls = items
            .iter()
            .map(|item| {
                format!(
                    "{}({}, {}, {})",
                    item.worker_fn,
                    source.code,
                    ts_string(&item.mapper_id),
                    per_map_worker_count
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "{indent}const {batch} = await Promise.all([{calls}]);\n"
        ));
        for (index, ((item, temp), chain)) in items
            .iter()
            .zip(temp_names.iter())
            .zip(chains.iter())
            .enumerate()
        {
            out.push_str(&format!("{indent}{temp} = {batch}[{index}];\n"));
            let value = ts_value(temp.clone(), item.output_ty.clone());
            let Some(last) = chain.stages.last() else {
                return Err("expected worker map batch binding".to_string());
            };
            let TypedStageKind::Bind { target } = &last.kind else {
                return Err("expected worker map batch binding".to_string());
            };
            self.bind_target(out, target, value, env, indent)?;
        }
        Ok(())
    }

    fn range_sync_map_batch(
        &self,
        callable: &TypedCallable,
        range_index: usize,
    ) -> Result<Option<RangeSyncMapBatch>, String> {
        let Some(range_chain) = callable.chains.get(range_index) else {
            return Ok(None);
        };
        let Some((range_source, range_name)) = self.range_step_binding(range_chain) else {
            return Ok(None);
        };
        if callable
            .outputs
            .iter()
            .any(|output| output.name == range_name)
        {
            return Ok(None);
        }

        let mut items = Vec::new();
        for chain in callable.chains.iter().skip(range_index + 1) {
            let Some(item) = self.sync_map_batch_item(chain)? else {
                break;
            };
            if item.source_key != range_name {
                break;
            }
            items.push(item);
        }
        if items.is_empty() {
            return Ok(None);
        }

        let expected_uses = (range_index + 1..range_index + 1 + items.len()).collect::<Vec<_>>();
        let actual_uses = callable
            .chains
            .iter()
            .enumerate()
            .filter_map(|(index, chain)| {
                (chain_source_variable(chain) == Some(range_name)).then_some(index)
            })
            .collect::<Vec<_>>();
        if actual_uses != expected_uses {
            return Ok(None);
        }

        Ok(Some(RangeSyncMapBatch {
            range_source: range_source.clone(),
            items,
        }))
    }

    fn range_step_binding<'b>(
        &self,
        chain: &'b TypedChain,
    ) -> Option<(&'b TypedEndpoint, &'b str)> {
        let [call, bind] = chain.stages.as_slice() else {
            return None;
        };
        let TypedStageKind::Call { name, .. } = &call.kind else {
            return None;
        };
        if self.codegen.canonical_name(name) != "range_step" {
            return None;
        }
        if call.output != Ty::Seq(Box::new(Ty::I64)) {
            return None;
        }
        let TypedStageKind::Bind { target } = &bind.kind else {
            return None;
        };
        binding_target_name(target).map(|name| (&chain.source, name))
    }

    fn sync_map_batch_len(&self, chains: &[TypedChain]) -> Result<usize, String> {
        let Some(first_chain) = chains.first() else {
            return Ok(0);
        };
        let Some(first) = self.sync_map_batch_item(first_chain)? else {
            return Ok(0);
        };
        let source_key = first.source_key;
        let mut len = 1;
        for chain in chains.iter().skip(1) {
            let Some(item) = self.sync_map_batch_item(chain)? else {
                break;
            };
            if item.source_key != source_key {
                break;
            }
            len += 1;
        }
        Ok(len)
    }

    fn sync_map_batch_item(&self, chain: &TypedChain) -> Result<Option<SyncMapBatchItem>, String> {
        let Some(source_key) = chain_source_variable(chain) else {
            return Ok(None);
        };
        let [map, bind] = chain.stages.as_slice() else {
            return Ok(None);
        };
        let TypedStageKind::Map { name, .. } = &map.kind else {
            return Ok(None);
        };
        let TypedStageKind::Bind {
            target: BindingTarget::Variable(target),
        } = &bind.kind
        else {
            return Ok(None);
        };
        let Ty::Seq(item_ty) = &map.input else {
            return Ok(None);
        };
        let Ty::Seq(output_item_ty) = &map.output else {
            return Ok(None);
        };
        if !self.map_can_emit_sync_loop(name, &map.input, output_item_ty)? {
            return Ok(None);
        }
        Ok(Some(SyncMapBatchItem {
            source: chain.source.clone(),
            source_key: source_key.to_string(),
            target: target.clone(),
            mapper: name.clone(),
            item_ty: item_ty.as_ref().clone(),
            output_ty: map.output.clone(),
        }))
    }

    fn map_can_emit_sync_loop(
        &self,
        name: &str,
        input_ty: &Ty,
        output_item_ty: &Ty,
    ) -> Result<bool, String> {
        if self.async_callables.contains(name) {
            return Ok(false);
        }
        let Ty::Seq(item_ty) = input_ty else {
            return Ok(false);
        };
        if self.options.gpu
            && self
                .gpu_plan
                .as_ref()
                .and_then(|plan| plan.kernel_for_map(name, item_ty, output_item_ty))
                .is_some()
        {
            return Ok(false);
        }
        if self.options.worker_concurrency
            && self
                .worker_mapper_source(name, item_ty, output_item_ty)?
                .is_some()
        {
            return Ok(false);
        }
        Ok(true)
    }

    fn emit_range_sync_map_batch(
        &mut self,
        out: &mut String,
        batch: RangeSyncMapBatch,
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let range_ty = Ty::Tuple(vec![Ty::I64, Ty::I64, Ty::I64]);
        let typed_outputs = batch
            .items
            .iter()
            .any(|item| item.storage() != SyncMapOutputStorage::Array);
        let item = self.next_temp();

        if let Some((start, stop, step)) = const_int_range_endpoint(&batch.range_source) {
            if step == 0 {
                self.emit_sync_map_batch_outputs(out, &batch.items, env, indent, Some("0"))?;
                out.push_str(&format!(
                    "{indent}throw new Error(\"range_step: step cannot be zero\");\n"
                ));
            } else {
                let cmp = if step > 0 { "<" } else { ">" };
                let count = const_range_len(start, stop, step);
                self.emit_sync_map_batch_outputs(out, &batch.items, env, indent, Some(&count))?;
                let index = typed_outputs.then(|| self.next_temp());
                if let Some(index) = &index {
                    out.push_str(&format!("{indent}let {index} = 0;\n"));
                }
                out.push_str(&format!(
                    "{indent}for (let {item} = {start}n; {item} {cmp} {stop}n; {item} += {step}n) {{\n"
                ));
                self.emit_sync_map_batch_loop_body(
                    out,
                    &batch.items,
                    &item,
                    index.as_deref(),
                    &(indent.to_string() + "  "),
                )?;
                out.push_str(&format!("{indent}}}\n"));
            }
            self.bind_sync_map_batch_outputs(&batch.items, env)?;
            return Ok(());
        }

        let range =
            self.emit_endpoint_expected(out, &batch.range_source, env, Some(&range_ty), indent)?;
        let range_tmp = self.next_temp();
        out.push_str(&format!(
            "{indent}const {range_tmp}: {} = {};\n",
            ts_type(&range_ty),
            range.code
        ));
        out.push_str(&format!("{indent}if ({range_tmp}[2] === 0n) {{\n"));
        out.push_str(&format!(
            "{indent}  throw new Error(\"range_step: step cannot be zero\");\n"
        ));
        out.push_str(&format!("{indent}}}\n"));
        let count = self.next_temp();
        out.push_str(&format!("{indent}const {count} = {range_tmp}[2] > 0n\n"));
        out.push_str(&format!(
            "{indent}  ? {range_tmp}[0] >= {range_tmp}[1] ? 0 : Number((({range_tmp}[1] - {range_tmp}[0] - 1n) / {range_tmp}[2]) + 1n)\n"
        ));
        out.push_str(&format!(
            "{indent}  : {range_tmp}[0] <= {range_tmp}[1] ? 0 : Number((({range_tmp}[0] - {range_tmp}[1] - 1n) / -{range_tmp}[2]) + 1n);\n"
        ));
        self.emit_sync_map_batch_outputs(out, &batch.items, env, indent, Some(&count))?;
        let index = typed_outputs.then(|| self.next_temp());
        if let Some(index) = &index {
            out.push_str(&format!("{indent}let {index} = 0;\n"));
        }
        out.push_str(&format!("{indent}if ({range_tmp}[2] > 0n) {{\n"));
        out.push_str(&format!(
            "{indent}  for (let {item} = {range_tmp}[0]; {item} < {range_tmp}[1]; {item} += {range_tmp}[2]) {{\n"
        ));
        self.emit_sync_map_batch_loop_body(
            out,
            &batch.items,
            &item,
            index.as_deref(),
            &(indent.to_string() + "    "),
        )?;
        out.push_str(&format!("{indent}  }}\n"));
        out.push_str(&format!("{indent}}} else {{\n"));
        out.push_str(&format!(
            "{indent}  for (let {item} = {range_tmp}[0]; {item} > {range_tmp}[1]; {item} += {range_tmp}[2]) {{\n"
        ));
        self.emit_sync_map_batch_loop_body(
            out,
            &batch.items,
            &item,
            index.as_deref(),
            &(indent.to_string() + "    "),
        )?;
        out.push_str(&format!("{indent}  }}\n"));
        out.push_str(&format!("{indent}}}\n"));
        self.bind_sync_map_batch_outputs(&batch.items, env)?;
        Ok(())
    }

    fn emit_sync_map_batch(
        &mut self,
        out: &mut String,
        chains: &[TypedChain],
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let items = chains
            .iter()
            .map(|chain| {
                self.sync_map_batch_item(chain)?
                    .ok_or_else(|| "expected synchronous map batch item".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut source = self.emit_endpoint(out, &items[0].source, env, indent)?;
        if !ts_value_code_is_stable(&source.code) {
            let source_tmp = self.next_temp();
            out.push_str(&format!(
                "{indent}const {source_tmp}: {} = {};\n",
                ts_type(&source.ty),
                source.code
            ));
            source.code = source_tmp;
        }
        self.emit_sync_map_batch_outputs(
            out,
            &items,
            env,
            indent,
            Some(&format!("{}.length", source.code)),
        )?;
        let item = self.next_temp();
        let typed_outputs = items
            .iter()
            .any(|item| item.storage() != SyncMapOutputStorage::Array);
        let index = typed_outputs.then(|| self.next_temp());
        if let Some(index) = &index {
            out.push_str(&format!("{indent}let {index} = 0;\n"));
        }
        out.push_str(&format!(
            "{indent}for (const {item} of {}) {{\n",
            source.code
        ));
        self.emit_sync_map_batch_loop_body(
            out,
            &items,
            &item,
            index.as_deref(),
            &(indent.to_string() + "  "),
        )?;
        out.push_str(&format!("{indent}}}\n"));
        self.bind_sync_map_batch_outputs(&items, env)?;
        Ok(())
    }

    fn emit_sync_map_batch_outputs(
        &mut self,
        out: &mut String,
        items: &[SyncMapBatchItem],
        _env: &HashMap<String, TsValue>,
        indent: &str,
        length: Option<&str>,
    ) -> Result<(), String> {
        for item in items {
            let target = ts_ident(&item.target);
            if !self.try_reserve_ident(&target) {
                return Err(format!("value `{}` is bound more than once", item.target));
            }
            let initializer = match (item.storage(), length) {
                (SyncMapOutputStorage::Int32Array, Some(length)) => {
                    format!(
                        "new Int32Array({length}) as unknown as {}",
                        ts_type(&item.output_ty)
                    )
                }
                (SyncMapOutputStorage::Float32Array, Some(length)) => {
                    format!(
                        "new Float32Array({length}) as unknown as {}",
                        ts_type(&item.output_ty)
                    )
                }
                (SyncMapOutputStorage::Float64Array, Some(length)) => {
                    format!(
                        "new Float64Array({length}) as unknown as {}",
                        ts_type(&item.output_ty)
                    )
                }
                (SyncMapOutputStorage::Int32Array, None)
                | (SyncMapOutputStorage::Float32Array, None)
                | (SyncMapOutputStorage::Float64Array, None)
                | (SyncMapOutputStorage::Array, _) => "[]".to_string(),
            };
            out.push_str(&format!(
                "{indent}const {target}: {} = {initializer};\n",
                ts_type(&item.output_ty),
            ));
        }
        Ok(())
    }

    fn emit_sync_map_batch_loop_body(
        &mut self,
        out: &mut String,
        items: &[SyncMapBatchItem],
        item_name: &str,
        index_name: Option<&str>,
        indent: &str,
    ) -> Result<(), String> {
        for item in items {
            let mapped = self.emit_call(
                out,
                &item.mapper,
                ts_value(item_name.to_string(), item.item_ty.clone()),
                indent,
            )?;
            match item.storage() {
                SyncMapOutputStorage::Array => {
                    out.push_str(&format!(
                        "{indent}{}.push({});\n",
                        ts_ident(&item.target),
                        mapped.code
                    ));
                }
                SyncMapOutputStorage::Int32Array
                | SyncMapOutputStorage::Float32Array
                | SyncMapOutputStorage::Float64Array => {
                    let index = index_name.ok_or_else(|| {
                        "typed synchronous map output requires an index".to_string()
                    })?;
                    out.push_str(&format!(
                        "{indent}{}[{index}] = {};\n",
                        ts_ident(&item.target),
                        mapped.code
                    ));
                }
            }
        }
        if let Some(index) = index_name {
            out.push_str(&format!("{indent}{index}++;\n"));
        }
        Ok(())
    }

    fn bind_sync_map_batch_outputs(
        &self,
        items: &[SyncMapBatchItem],
        env: &mut HashMap<String, TsValue>,
    ) -> Result<(), String> {
        for item in items {
            let target = ts_ident(&item.target);
            if env
                .insert(
                    item.target.clone(),
                    ts_value(target, item.output_ty.clone()),
                )
                .is_some()
            {
                return Err(format!("value `{}` is bound more than once", item.target));
            }
        }
        Ok(())
    }

    fn emit_endpoint(
        &mut self,
        out: &mut String,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        self.emit_endpoint_expected(out, endpoint, env, None, indent)
    }

    fn emit_endpoint_expected(
        &mut self,
        out: &mut String,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, TsValue>,
        expected: Option<&Ty>,
        indent: &str,
    ) -> Result<TsValue, String> {
        match &endpoint.kind {
            TypedEndpointKind::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            TypedEndpointKind::NodeRef { name, .. } => {
                Err(format!("expected value, found node `{name}`"))
            }
            TypedEndpointKind::Int(value) => Ok(ts_value(format!("{value}n"), endpoint.ty.clone())),
            TypedEndpointKind::Real(value) => {
                Ok(ts_value(format!("{value:.17e}"), endpoint.ty.clone()))
            }
            TypedEndpointKind::Bool(value) => Ok(ts_value(value.to_string(), endpoint.ty.clone())),
            TypedEndpointKind::String(value) => Ok(ts_value(ts_string(value), endpoint.ty.clone())),
            TypedEndpointKind::Unit => Ok(ts_value("undefined", endpoint.ty.clone())),
            TypedEndpointKind::Tuple(items) => {
                let expected_items = match expected {
                    Some(Ty::Tuple(expected_items)) if expected_items.len() == items.len() => {
                        Some(expected_items.as_slice())
                    }
                    _ => None,
                };
                let mut values = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    values.push(self.emit_endpoint_expected(
                        out,
                        item,
                        env,
                        expected_items.and_then(|items| items.get(index)),
                        indent,
                    )?);
                }
                let ty = expected.cloned().unwrap_or_else(|| endpoint.ty.clone());
                Ok(ts_tuple_value(values, ty))
            }
            TypedEndpointKind::Seq(items) => {
                if items.is_empty() {
                    let seq_ty = match expected {
                        Some(seq_ty @ Ty::Seq(_)) => seq_ty.clone(),
                        Some(other) => {
                            return Err(format!(
                                "empty sequence literal expected Seq context, found `{other}`"
                            ));
                        }
                        None if matches!(endpoint.ty, Ty::Seq(_)) => endpoint.ty.clone(),
                        None => {
                            return Err("empty sequence literals need a type context".to_string());
                        }
                    };
                    return Ok(ts_value("[]", seq_ty));
                }
                let expected_item = match expected {
                    Some(Ty::Seq(item)) => Some(item.as_ref()),
                    _ => None,
                };
                let mut values = items
                    .iter()
                    .map(|item| self.emit_endpoint_expected(out, item, env, expected_item, indent))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut item_ty = values[0].ty.clone();
                for value in values.iter().skip(1) {
                    item_ty = sequence_item_type(&item_ty, &value.ty)?;
                }
                if let Some(expected_item) = expected_item {
                    for value in &mut values {
                        if let Ty::Faultable(inner) = expected_item
                            && inner.as_ref() == &value.ty
                        {
                            value.code = format!("faOk({})", value.code);
                            value.ty = expected_item.clone();
                        }
                    }
                    item_ty = expected_item.clone();
                }
                if let Ty::Faultable(inner) = &item_ty {
                    for value in &mut values {
                        if inner.as_ref() == &value.ty {
                            value.code = format!("faOk({})", value.code);
                            value.ty = item_ty.clone();
                        }
                    }
                }
                let ty = Ty::Seq(Box::new(item_ty));
                let code = format!(
                    "[{}]",
                    values
                        .iter()
                        .map(|value| value.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                Ok(ts_value(code, ty))
            }
            TypedEndpointKind::Struct { name, fields, .. } => {
                self.emit_struct_endpoint(out, name, fields, env, indent)
            }
            TypedEndpointKind::Eval { source, stages } => {
                let source_expected = if contains_empty_seq(&source.ty) {
                    match stages.first().map(|stage| &stage.kind) {
                        Some(TypedStageKind::Call { name, .. }) => {
                            Some(self.codegen.call_input_type_for_value(name, &source.ty)?)
                        }
                        Some(_) => stages.first().map(|stage| stage.input.clone()),
                        None => None,
                    }
                } else {
                    None
                };
                let mut value = self.emit_endpoint_expected(
                    out,
                    source,
                    env,
                    source_expected.as_ref(),
                    indent,
                )?;
                for stage in stages {
                    value = self.emit_inline_stage(out, stage, value, env, indent)?;
                }
                Ok(value)
            }
        }
    }

    fn emit_struct_endpoint(
        &mut self,
        out: &mut String,
        name: &str,
        fields: &[(String, TypedEndpoint)],
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        let ty = self
            .codegen
            .aliases
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown struct `{name}`"))?;
        let Ty::Struct {
            fields: expected_fields,
            ..
        } = &ty
        else {
            return Err(format!("`{name}` is not a struct"));
        };
        let mut parts = Vec::with_capacity(expected_fields.len());
        for (field, field_ty) in expected_fields {
            let (_, endpoint) = fields
                .iter()
                .find(|(candidate, _)| candidate == field)
                .ok_or_else(|| format!("struct `{name}` literal missing field `{field}`"))?;
            let value = self.emit_endpoint_expected(out, endpoint, env, Some(field_ty), indent)?;
            parts.push(format!("{}: {}", ts_object_key(field), value.code));
        }
        Ok(ts_value(format!("{{ {} }}", parts.join(", ")), ty))
    }

    fn emit_inline_stage(
        &mut self,
        out: &mut String,
        stage: &crate::typecheck::TypedStage,
        value: TsValue,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        match &stage.kind {
            TypedStageKind::Call { name, .. } => self.emit_call(out, name, value, indent),
            TypedStageKind::Bind { .. } => Err("inline evaluations cannot bind values".to_string()),
            TypedStageKind::Map { name, .. } => self.emit_map(out, name, value, indent, None),
            TypedStageKind::FaultMap { .. } => {
                Err("inline evaluations cannot use `fault map`".to_string())
            }
            TypedStageKind::Filter { name, .. } => self.emit_filter(out, name, value, indent, None),
            TypedStageKind::Field { name } => self.emit_field(name, value),
            TypedStageKind::Repeat { count, node, .. } => {
                let count = self.emit_endpoint(out, count, env, indent)?;
                self.emit_repeat(out, node, value, count, indent, None)
            }
            TypedStageKind::Reduce { op, identity, .. } => {
                let identity = self.emit_endpoint(out, identity, env, indent)?;
                self.emit_reduce(out, op, value, identity, indent, None)
            }
            TypedStageKind::Scan { op, identity, .. } => {
                let identity = self.emit_endpoint(out, identity, env, indent)?;
                self.emit_scan(out, op, value, identity, indent, None)
            }
            TypedStageKind::Match { arms } => self.emit_match(
                out,
                TsMatchParams {
                    arms,
                    output_ty: stage.output.clone(),
                    subject: value,
                    env,
                    indent,
                    preferred: None,
                },
            ),
        }
    }

    fn bind_target(
        &mut self,
        out: &mut String,
        target: &BindingTarget,
        value: TsValue,
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        match target {
            BindingTarget::Discard => Ok(()),
            BindingTarget::Variable(name) => {
                let ident = ts_ident(name);
                let value = if value.code == ident {
                    value
                } else if self.try_reserve_ident(&ident) {
                    out.push_str(&format!(
                        "{indent}const {ident}: {} = {};\n",
                        ts_type(&value.ty),
                        value.code
                    ));
                    ts_value(ident, value.ty)
                } else {
                    value
                };
                if env.insert(name.clone(), value).is_some() {
                    return Err(format!("value `{name}` is bound more than once"));
                }
                Ok(())
            }
            BindingTarget::Tuple(targets) => {
                let value = self.stabilize_tuple_value(out, value, indent)?;
                match value.ty.clone() {
                    Ty::Tuple(items) if items.len() == targets.len() => {
                        for (index, (target, ty)) in targets.iter().zip(items.iter()).enumerate() {
                            if binding_target_is_discard(target) {
                                continue;
                            }
                            self.bind_target(
                                out,
                                target,
                                ts_value(tuple_field(&value, index), ty.clone()),
                                env,
                                indent,
                            )?;
                        }
                        Ok(())
                    }
                    Ty::Faultable(inner) => {
                        let Ty::Tuple(items) = inner.as_ref() else {
                            return Err("tuple binding expected tuple value".to_string());
                        };
                        for (index, (target, ty)) in targets.iter().zip(items.iter()).enumerate() {
                            if binding_target_is_discard(target) {
                                continue;
                            }
                            let projected_ty = Ty::Faultable(Box::new(ty.clone()));
                            let tmp = self.next_temp();
                            out.push_str(&format!(
                                "{indent}const {tmp}: {} = {}.is_fault ? faFault({}.fault) : faOk({});\n",
                                ts_type(&projected_ty),
                                value.code,
                                value.code,
                                ts_index(&format!("{}.value", value.code), index)
                            ));
                            self.bind_target(
                                out,
                                target,
                                ts_value(tmp, projected_ty),
                                env,
                                indent,
                            )?;
                        }
                        Ok(())
                    }
                    Ty::Tuple(items) => Err(format!(
                        "binding target `{}` expected {} tuple fields, found {}",
                        format_binding_target_for_error(target),
                        targets.len(),
                        items.len()
                    )),
                    other => Err(format!(
                        "binding target `{}` expected tuple input, found `{other}`",
                        format_binding_target_for_error(target)
                    )),
                }
            }
        }
    }

    fn stabilize_tuple_value(
        &mut self,
        out: &mut String,
        value: TsValue,
        indent: &str,
    ) -> Result<TsValue, String> {
        if value.tuple_items.is_some() || ts_value_code_is_stable(&value.code) {
            return Ok(value);
        }
        if matches!(value.ty, Ty::Tuple(_) | Ty::Faultable(_)) {
            let tmp = self.next_temp();
            out.push_str(&format!(
                "{indent}const {tmp}: {} = {};\n",
                ts_type(&value.ty),
                value.code
            ));
            Ok(ts_value(tmp, value.ty))
        } else {
            Ok(value)
        }
    }

    fn emit_call(
        &mut self,
        out: &mut String,
        name: &str,
        input: TsValue,
        indent: &str,
    ) -> Result<TsValue, String> {
        self.emit_call_preferred(out, name, input, indent, None)
    }

    fn emit_call_preferred(
        &mut self,
        out: &mut String,
        name: &str,
        mut input: TsValue,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
        if matches!(input.ty, Ty::Faultable(_)) && !ts_value_code_is_stable(&input.code) {
            let input_tmp = self.next_temp();
            out.push_str(&format!(
                "{indent}const {input_tmp}: {} = {};\n",
                ts_type(&input.ty),
                input.code
            ));
            input.code = input_tmp;
        }
        let output_ty = self.codegen.call_output_type(name, &input.ty)?;

        if let (Ty::Faultable(input_inner), Ty::Faultable(output_inner)) = (&input.ty, &output_ty) {
            let plain_output = self.plain_output_type(name, input_inner)?;
            let tmp = self.next_temp_or_preferred(preferred);
            out.push_str(&format!("{indent}let {tmp}: {};\n", ts_type(&output_ty)));
            out.push_str(&format!(
                "{indent}if ({}.is_fault === true) {{\n",
                input.code
            ));
            out.push_str(&format!(
                "{indent}  {tmp} = faFault({}.fault);\n",
                input.code
            ));
            out.push_str(&format!("{indent}}} else {{\n"));
            let plain_input = ts_value(
                format!("{}.value", input.code),
                input_inner.as_ref().clone(),
            );
            let called = self.emit_plain_call(
                out,
                name,
                plain_input,
                &plain_output,
                &(indent.to_string() + "  "),
                None,
            )?;
            if plain_output == output_ty {
                out.push_str(&format!("{indent}  {tmp} = {};\n", called.code));
            } else if &plain_output == output_inner.as_ref() {
                out.push_str(&format!("{indent}  {tmp} = faOk({});\n", called.code));
            } else {
                return Err(format!(
                    "TypeScript backend cannot wrap `{plain_output}` as `{output_ty}`"
                ));
            }
            out.push_str(&format!("{indent}}}\n"));
            return Ok(ts_value(tmp, output_ty));
        }

        if let Ty::Faultable(output_inner) = &output_ty {
            let plain_output = self.plain_output_type(name, &input.ty)?;
            let called =
                self.emit_plain_call(out, name, input.clone(), &plain_output, indent, preferred)?;
            if plain_output == output_ty {
                return Ok(called);
            }
            if &plain_output == output_inner.as_ref() {
                return Ok(ts_value(format!("faOk({})", called.code), output_ty));
            }
        }

        self.emit_plain_call(out, name, input, &output_ty, indent, preferred)
    }

    fn emit_plain_call(
        &mut self,
        out: &mut String,
        name: &str,
        input: TsValue,
        output_ty: &Ty,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
        let expr = if self.codegen.foreign_js.contains(name) {
            let arity = match &input.ty {
                Ty::Unit => 0,
                Ty::Tuple(items) => items.len(),
                _ => 1,
            };
            format!("{}({})", ts_ident(name), call_args(&input, arity)?)
        } else if self.codegen.callables.contains_key(name) {
            let arity = self
                .codegen
                .callables
                .get(name)
                .map(|callable| callable.inputs.len())
                .ok_or_else(|| format!("missing callable `{name}`"))?;
            let call = format!("{}({})", ts_ident(name), call_args(&input, arity)?);
            if self.async_callables.contains(name) {
                format!("await {call}")
            } else {
                call
            }
        } else {
            self.emit_builtin_expr(&self.codegen.canonical_name(name), &input, output_ty)?
        };
        if expression_is_simple(&expr) {
            Ok(ts_value(expr, output_ty.clone()))
        } else {
            let tmp = self.next_temp_or_preferred(preferred);
            out.push_str(&format!(
                "{indent}const {tmp}: {} = {expr};\n",
                ts_type(output_ty)
            ));
            Ok(ts_value(tmp, output_ty.clone()))
        }
    }

    fn emit_builtin_expr(
        &self,
        name: &str,
        input: &TsValue,
        output_ty: &Ty,
    ) -> Result<String, String> {
        let expr = match name {
            "argv" => format!("{}.argv", input.code),
            "flag_present" => format!("faFlagPresent({})", input.code),
            "flag_value" => format!("faFlagValue({})", input.code),
            "read_stdin" => "faReadStdin()".to_string(),
            "write_stdout" => format!("faWriteStdout({})", input.code),
            "write_stderr" => format!("faWriteStderr({})", input.code),
            "split_lines" => format!("faSplitLines({})", input.code),
            "trim" => format!("{}.trim()", input.code),
            "contains" => format!(
                "{}.includes({})",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "starts_with" => format!(
                "{}.startsWith({})",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "ends_with" => format!(
                "{}.endsWith({})",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "index_of" => format!(
                "BigInt({}.indexOf({}))",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "last_index_of" => format!(
                "BigInt({}.lastIndexOf({}))",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "slice" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::I64, Ty::I64])) =>
            {
                format!(
                    "{}.slice(Number({}), Number({}))",
                    tuple_field(input, 0),
                    tuple_field(input, 1),
                    tuple_field(input, 2)
                )
            }
            "take" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::I64])) =>
            {
                format!(
                    "{}.slice(0, Number({}))",
                    tuple_field(input, 0),
                    tuple_field(input, 1)
                )
            }
            "drop" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::I64])) =>
            {
                format!(
                    "{}.slice(Number({}))",
                    tuple_field(input, 0),
                    tuple_field(input, 1)
                )
            }
            "replace" => format!(
                "{}.split({}).join({})",
                tuple_field(input, 0),
                tuple_field(input, 1),
                tuple_field(input, 2)
            ),
            "repeat_bytes" => format!(
                "{}.repeat(Number({}))",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "ascii_lower" => format!("{}.toLowerCase()", input.code),
            "ascii_upper" => format!("{}.toUpperCase()", input.code),
            "split_on" => format!("{}.split({})", tuple_field(input, 0), tuple_field(input, 1)),
            "strip_prefix" => format!("faStripPrefix({})", input.code),
            "strip_suffix" => format!("faStripSuffix({})", input.code),
            "bytes_to_codes" => {
                format!("Array.from({}, ch => BigInt(ch.charCodeAt(0)))", input.code)
            }
            "codes_to_bytes" => format!("String.fromCharCode(...{}.map(Number))", input.code),
            "byte_length" => format!("BigInt({}.length)", input.code),
            "concat_bytes" => format!("faConcatBytes({})", input.code),
            "join_bytes" => format!("{}.join({})", tuple_field(input, 1), tuple_field(input, 0)),
            "parse_int" => format!("faParseInt({})", input.code),
            "parse_real" => format!("faParseReal({})", input.code),
            "from_int" => format!("Number({})", input.code),
            "from_int_f32" => format!("Math.fround(Number({}))", input.code),
            "format_int" | "format_real" | "format_real_f32" => {
                format!("{}.toString()", input.code)
            }
            "add" | "sub" | "mul" | "div" | "rem" if matches!(output_ty, Ty::Faultable(_)) => {
                ts_faultable_numeric_binary_expr(name, input, output_ty)
            }
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                ts_numeric_binary_expr(name, input, output_ty)
            }
            "neg" if matches!(output_ty, Ty::Faultable(_)) => match output_ty {
                Ty::Faultable(inner) if inner.as_ref() == &Ty::I32 => {
                    format!("faFaultableI32Neg({})", input.code)
                }
                Ty::Faultable(inner) if inner.as_ref() == &Ty::I64 => {
                    format!("faFaultableI64Neg({})", input.code)
                }
                _ => unreachable!(),
            },
            "neg" if output_ty == &Ty::I32 => format!("faCheckedI32Neg({})", input.code),
            "neg" if output_ty == &Ty::I64 => format!("faCheckedI64Neg({})", input.code),
            "neg" if output_ty == &Ty::F32 => format!("Math.fround(-{})", input.code),
            "neg" => format!("(-{})", input.code),
            "abs" if matches!(output_ty, Ty::Faultable(_)) => match output_ty {
                Ty::Faultable(inner) if inner.as_ref() == &Ty::I32 => {
                    format!("faFaultableI32Abs({})", input.code)
                }
                Ty::Faultable(inner) if inner.as_ref() == &Ty::I64 => {
                    format!("faFaultableI64Abs({})", input.code)
                }
                _ => unreachable!(),
            },
            "abs" if output_ty == &Ty::I32 => format!("faCheckedI32Abs({})", input.code),
            "abs" if output_ty == &Ty::I64 => format!("faCheckedI64Abs({})", input.code),
            "abs" if output_ty == &Ty::F32 => format!("Math.fround(Math.abs({}))", input.code),
            "abs" => format!("Math.abs({})", input.code),
            "sqrt" if matches!(output_ty, Ty::Faultable(_)) => match output_ty {
                Ty::Faultable(inner) if inner.as_ref() == &Ty::F32 => {
                    format!("faFaultableSqrtF32({})", input.code)
                }
                Ty::Faultable(inner) if inner.as_ref() == &Ty::F64 => {
                    format!("faFaultableSqrt({})", input.code)
                }
                _ => unreachable!(),
            },
            "sqrt" if output_ty == &Ty::F32 => format!("faCheckedSqrtF32({})", input.code),
            "sqrt" => format!("faCheckedSqrt({})", input.code),
            "exp" if output_ty == &Ty::F32 => format!("Math.fround(Math.exp({}))", input.code),
            "exp" => format!("Math.exp({})", input.code),
            "sin" if output_ty == &Ty::F32 => format!("Math.fround(Math.sin({}))", input.code),
            "sin" => format!("Math.sin({})", input.code),
            "cos" if output_ty == &Ty::F32 => format!("Math.fround(Math.cos({}))", input.code),
            "cos" => format!("Math.cos({})", input.code),
            "eq" => format!("({} === {})", tuple_field(input, 0), tuple_field(input, 1)),
            "lt" => format!("({} < {})", tuple_field(input, 0), tuple_field(input, 1)),
            "gt" => format!("({} > {})", tuple_field(input, 0), tuple_field(input, 1)),
            "le" => format!("({} <= {})", tuple_field(input, 0), tuple_field(input, 1)),
            "ge" => format!("({} >= {})", tuple_field(input, 0), tuple_field(input, 1)),
            "not_empty" => format!("({}.length > 0)", input.code),
            "is_empty" => match input.ty {
                Ty::Bytes => format!("({}.length === 0)", input.code),
                Ty::Seq(_) => format!("({}.length === 0)", input.code),
                _ => return Err("is_empty expected Bytes or Seq input".to_string()),
            },
            "and" => format!("({} && {})", tuple_field(input, 0), tuple_field(input, 1)),
            "or" => format!("({} || {})", tuple_field(input, 0), tuple_field(input, 1)),
            "xor" => format!("({} !== {})", tuple_field(input, 0), tuple_field(input, 1)),
            "not" => format!("(!{})", input.code),
            "all" => format!("{}.every(Boolean)", input.code),
            "any" => format!("{}.some(Boolean)", input.code),
            "has_faults" => format!("({}.length > 0)", input.code),
            "format_faults" => format!("{}.map(f => f.message).join(\"\\n\")", input.code),
            "expect" => format!("faExpect({})", input.code),
            "collect" => format!("faCollect({})", input.code),
            "select" => format!(
                "({} ? {} : {})",
                tuple_field(input, 0),
                tuple_field(input, 1),
                tuple_field(input, 2)
            ),
            "length" => format!("BigInt({}.length)", input.code),
            "inner_length" => format!("BigInt({}[0]?.length ?? 0)", input.code),
            "first" => tuple_field(input, 0),
            "second" => tuple_field(input, 1),
            "swap" => format!("[{}, {}]", tuple_field(input, 1), tuple_field(input, 0)),
            "zip" => format!("faZip({})", input.code),
            "broadcast_left" => format!("faBroadcastLeft({})", input.code),
            "broadcast_right" => format!("faBroadcastRight({})", input.code),
            "transpose" => format!("faTranspose({})", input.code),
            "flatten" => format!("{}.flat()", input.code),
            "group_by_id" => format!("faGroupById({})", input.code),
            "shift_right" => format!("faShiftRight({})", input.code),
            "shift_left" => format!("faShiftLeft({})", input.code),
            "head" => format!("faHead({})", input.code),
            "tail" => format!("{}.slice(1)", input.code),
            "reverse" => format!("[...{}].reverse()", input.code),
            "take" => format!(
                "{}.slice(0, Number({}))",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "drop" => format!(
                "{}.slice(Number({}))",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "fill" => format!("faFill({})", input.code),
            "slice" => format!(
                "{}.slice(Number({}), Number({}))",
                tuple_field(input, 0),
                tuple_field(input, 1),
                tuple_field(input, 2)
            ),
            "last" => format!("faLast({})", input.code),
            "get" => format!("faGet({})", input.code),
            "get_or" => format!("faGetOr({})", input.code),
            "at" => format!("faAt({})", input.code),
            "append" => format!("[...{}, {}]", tuple_field(input, 0), tuple_field(input, 1)),
            "set" => format!("faSet({})", input.code),
            "concat" => format!(
                "[...{}, ...{}]",
                tuple_field(input, 0),
                tuple_field(input, 1)
            ),
            "range_step" => format!("faRangeStep({})", input.code),
            "bit_and" => format!("({} & {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_or" => format!("({} | {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_xor" => format!("({} ^ {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_shl" => format!("({} << {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_shr" => format!("({} >> {})", tuple_field(input, 0), tuple_field(input, 1)),
            "read_file" | "write_file" | "exists" | "is_file" | "is_dir" | "file_size"
            | "list_dir" | "walk_files" | "read_files" | "open_file" | "size" | "read_at"
            | "copy_to_file" | "close" | "to_seq" | "drain" => {
                return Err(format!(
                    "TypeScript backend does not support stdlib builtin `{name}` yet"
                ));
            }
            other => {
                return Err(format!(
                    "TypeScript backend does not support stdlib builtin `{other}` yet"
                ));
            }
        };
        Ok(expr)
    }

    fn emit_map(
        &mut self,
        out: &mut String,
        name: &str,
        input: TsValue,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
        let (is_faultable, seq_ty) = match input.ty.clone() {
            Ty::Faultable(inner) => (true, inner.as_ref().clone()),
            other => (false, other),
        };
        let Ty::Seq(item_ty) = seq_ty else {
            return Err(format!("`map {name}` expected Seq input"));
        };
        let output_item_ty = self.codegen.call_output_type(name, &item_ty)?;
        let output_seq_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let output_ty = if is_faultable {
            Ty::Faultable(Box::new(output_seq_ty.clone()))
        } else {
            output_seq_ty.clone()
        };
        let tmp = self.next_temp_or_preferred(preferred);
        out.push_str(&format!("{indent}let {tmp}: {};\n", ts_type(&output_ty)));
        if is_faultable {
            out.push_str(&format!(
                "{indent}if ({}.is_fault === true) {{\n",
                input.code
            ));
            out.push_str(&format!(
                "{indent}  {tmp} = faFault({}.fault);\n",
                input.code
            ));
            out.push_str(&format!("{indent}}} else {{\n"));
        }
        let source = if is_faultable {
            format!("{}.value", input.code)
        } else {
            input.code.clone()
        };
        if !is_faultable
            && let Some(kernel) = self
                .gpu_plan
                .as_ref()
                .and_then(|plan| plan.kernel_for_map(name, item_ty.as_ref(), &output_item_ty))
                .cloned()
        {
            let batch = self.next_temp();
            out.push_str(&format!(
                "{indent}const {batch}: [{}] = await Promise.all([{}({}, {}, {})]);\n",
                ts_type(&output_ty),
                kernel.scalar.map_function(),
                source,
                ts_string(&kernel.id),
                ts_string(&kernel.wgsl)
            ));
            out.push_str(&format!("{indent}{tmp} = {batch}[0];\n"));
            return Ok(ts_value(tmp, output_ty));
        }
        if !is_faultable
            && let Some((worker_fn, mapper_id)) =
                self.worker_map_call(name, item_ty.as_ref(), &output_item_ty)?
        {
            let batch = self.next_temp();
            out.push_str(&format!(
                "{indent}const {batch}: [{}] = await Promise.all([{worker_fn}({}, {})]);\n",
                ts_type(&output_ty),
                source,
                ts_string(&mapper_id)
            ));
            out.push_str(&format!("{indent}{tmp} = {batch}[0];\n"));
            return Ok(ts_value(tmp, output_ty));
        }
        let body_indent = if is_faultable {
            format!("{indent}  ")
        } else {
            indent.to_string()
        };
        if self.async_callables.contains(name) {
            let item = self.next_temp();
            let mapped = format!("{}({item})", ts_ident(name));
            if is_faultable {
                out.push_str(&format!(
                    "{body_indent}{tmp} = faOk(await Promise.all(Array.from({source}, ({item}) => {mapped})));\n"
                ));
                out.push_str(&format!("{indent}}}\n"));
            } else {
                out.push_str(&format!(
                    "{body_indent}{tmp} = await Promise.all(Array.from({source}, ({item}) => {mapped}));\n"
                ));
            }
            return Ok(ts_value(tmp, output_ty));
        }
        let seq_tmp = self.next_temp();
        let item = self.next_temp();
        out.push_str(&format!(
            "{body_indent}const {seq_tmp}: {} = [];\n",
            ts_type(&output_seq_ty)
        ));
        out.push_str(&format!("{body_indent}for (const {item} of {source}) {{\n"));
        let mapped = self.emit_call(
            out,
            name,
            ts_value(item.clone(), item_ty.as_ref().clone()),
            &(body_indent.clone() + "  "),
        )?;
        out.push_str(&format!(
            "{body_indent}  {seq_tmp}.push({});\n",
            mapped.code
        ));
        out.push_str(&format!("{body_indent}}}\n"));
        if is_faultable {
            out.push_str(&format!("{body_indent}{tmp} = faOk({seq_tmp});\n"));
            out.push_str(&format!("{indent}}}\n"));
        } else {
            out.push_str(&format!("{body_indent}{tmp} = {seq_tmp};\n"));
        }
        Ok(ts_value(tmp, output_ty))
    }

    fn emit_filter(
        &mut self,
        out: &mut String,
        name: &str,
        mut input: TsValue,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`filter {name}` expected Seq input"));
        };
        if !ts_value_code_is_stable(&input.code) {
            let input_tmp = self.next_temp();
            out.push_str(&format!(
                "{indent}const {input_tmp}: {} = {};\n",
                ts_type(&input.ty),
                input.code
            ));
            input.code = input_tmp;
        }
        let tmp = self.next_temp_or_preferred(preferred);
        let item = self.next_temp();
        out.push_str(&format!(
            "{indent}const {tmp}: {} = [];\n",
            ts_type(&input.ty)
        ));
        if self.async_callables.contains(name) {
            let decisions = self.next_temp();
            let index = self.next_temp();
            out.push_str(&format!(
                "{indent}const {decisions}: Array<boolean> = await Promise.all(Array.from({}, ({item}) => {}({item})));\n",
                input.code,
                ts_ident(name)
            ));
            out.push_str(&format!("{indent}let {index} = 0;\n"));
            out.push_str(&format!(
                "{indent}for (const {item} of {}) {{\n",
                input.code
            ));
            out.push_str(&format!(
                "{indent}  if ({decisions}[{index}++]) {tmp}.push({item});\n"
            ));
            out.push_str(&format!("{indent}}}\n"));
            return Ok(ts_value(tmp, input.ty));
        }
        out.push_str(&format!(
            "{indent}for (const {item} of {}) {{\n",
            input.code
        ));
        let keep = self.emit_call(
            out,
            name,
            ts_value(item.clone(), item_ty.as_ref().clone()),
            &(indent.to_string() + "  "),
        )?;
        out.push_str(&format!(
            "{indent}  if ({}) {tmp}.push({item});\n",
            keep.code
        ));
        out.push_str(&format!("{indent}}}\n"));
        Ok(ts_value(tmp, input.ty))
    }

    fn emit_field(&self, field: &str, input: TsValue) -> Result<TsValue, String> {
        let Ty::Struct { name, fields } = input.ty.clone() else {
            return Err(format!(
                "field `{field}` expected struct input, found `{}`",
                input.ty
            ));
        };
        let (_, ty) = fields
            .iter()
            .find(|(candidate, _)| candidate == field)
            .ok_or_else(|| format!("struct `{name}` has no field `{field}`"))?;
        Ok(ts_value(
            format!("{}.{}", input.code, ts_property(field)),
            ty.clone(),
        ))
    }

    fn emit_reduce(
        &mut self,
        out: &mut String,
        op: &str,
        input: TsValue,
        identity: TsValue,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
        let input_faultable = matches!(input.ty, Ty::Faultable(_));
        let seq_ty = match input.ty.clone() {
            Ty::Faultable(inner) => inner.as_ref().clone(),
            other => other,
        };
        let Ty::Seq(item_ty) = seq_ty else {
            return Err(format!("`reduce {op}` expected Seq input"));
        };
        let item_faultable = matches!(item_ty.as_ref(), Ty::Faultable(_));
        let plain_item_ty = match item_ty.as_ref() {
            Ty::Faultable(inner) => inner.as_ref().clone(),
            other => other.clone(),
        };
        let pair_ty = Ty::Tuple(vec![plain_item_ty.clone(), plain_item_ty.clone()]);
        let operation_output_ty = self.codegen.call_output_type(op, &pair_ty)?;
        let operation_faultable = matches!(
            &operation_output_ty,
            Ty::Faultable(inner) if inner.as_ref() == &plain_item_ty
        );
        let output_ty = if input_faultable || item_faultable || operation_faultable {
            Ty::Faultable(Box::new(plain_item_ty.clone()))
        } else {
            plain_item_ty.clone()
        };
        let tmp = self.next_temp_or_preferred(preferred);
        out.push_str(&format!("{indent}let {tmp}: {};\n", ts_type(&output_ty)));
        if input_faultable {
            out.push_str(&format!(
                "{indent}if ({}.is_fault === true) {{\n",
                input.code
            ));
            out.push_str(&format!(
                "{indent}  {tmp} = faFault({}.fault);\n",
                input.code
            ));
            out.push_str(&format!("{indent}}} else {{\n"));
        }
        let body_indent = if input_faultable {
            format!("{indent}  ")
        } else {
            indent.to_string()
        };
        let source = if input_faultable {
            format!("{}.value", input.code)
        } else {
            input.code.clone()
        };
        if self.options.gpu && !input_faultable && !item_faultable && !operation_faultable {
            let canonical = self.codegen.canonical_name(op);
            if matches!(canonical.as_str(), "add" | "min" | "max") {
                match plain_item_ty {
                    Ty::I32 => {
                        let batch = self.next_temp();
                        out.push_str(&format!(
                            "{body_indent}const {batch}: [{}] = await Promise.all([faGpuReduceI32({}, {}, {})]);\n",
                            ts_type(&output_ty),
                            source,
                            ts_string(&canonical),
                            identity.code
                        ));
                        out.push_str(&format!("{body_indent}{tmp} = {batch}[0];\n"));
                        return Ok(ts_value(tmp, output_ty));
                    }
                    Ty::F32 => {
                        let batch = self.next_temp();
                        out.push_str(&format!(
                            "{body_indent}const {batch}: [{}] = await Promise.all([faGpuReduceF32({}, {}, {})]);\n",
                            ts_type(&output_ty),
                            source,
                            ts_string(&canonical),
                            identity.code
                        ));
                        out.push_str(&format!("{body_indent}{tmp} = {batch}[0];\n"));
                        return Ok(ts_value(tmp, output_ty));
                    }
                    Ty::F64 => {
                        let batch = self.next_temp();
                        out.push_str(&format!(
                            "{body_indent}const {batch}: [{}] = await Promise.all([faGpuReduceF64({}, {}, {})]);\n",
                            ts_type(&output_ty),
                            source,
                            ts_string(&canonical),
                            identity.code
                        ));
                        out.push_str(&format!("{body_indent}{tmp} = {batch}[0];\n"));
                        return Ok(ts_value(tmp, output_ty));
                    }
                    _ => {}
                }
            }
        }
        let acc = self.next_temp();
        let item = self.next_temp();
        out.push_str(&format!(
            "{body_indent}let {acc}: {} = {};\n",
            ts_type(&plain_item_ty),
            identity.code
        ));
        if item_faultable || operation_faultable {
            let fault = self.next_temp();
            out.push_str(&format!(
                "{body_indent}let {fault}: FaFault | null = null;\n"
            ));
            out.push_str(&format!("{body_indent}for (const {item} of {source}) {{\n"));
            out.push_str(&format!("{body_indent}  if ({fault}) break;\n"));
            if item_faultable {
                out.push_str(&format!(
                    "{body_indent}  if ({item}.is_fault === true) {{ {fault} = {item}.fault; break; }}\n"
                ));
            }
            let pair = ts_tuple_value(
                vec![
                    ts_value(acc.clone(), plain_item_ty.clone()),
                    ts_value(
                        if item_faultable {
                            format!("{item}.value")
                        } else {
                            item.clone()
                        },
                        plain_item_ty.clone(),
                    ),
                ],
                pair_ty.clone(),
            );
            let reduced = self.emit_call(out, op, pair, &(body_indent.clone() + "  "))?;
            let reduced_code = if operation_faultable && !ts_value_code_is_stable(&reduced.code) {
                let reduced_tmp = self.next_temp();
                out.push_str(&format!(
                    "{body_indent}  const {reduced_tmp}: {} = {};\n",
                    ts_type(&reduced.ty),
                    reduced.code
                ));
                reduced_tmp
            } else {
                reduced.code
            };
            if operation_faultable {
                out.push_str(&format!(
                    "{body_indent}  if ({}.is_fault === true) {{ {fault} = {}.fault; }} else {{ {acc} = {}.value; }}\n",
                    reduced_code, reduced_code, reduced_code
                ));
            } else {
                out.push_str(&format!("{body_indent}  {acc} = {};\n", reduced_code));
            }
            out.push_str(&format!("{body_indent}}}\n"));
            out.push_str(&format!(
                "{body_indent}{tmp} = {fault} ? faFault({fault}) : faOk({acc});\n"
            ));
        } else {
            out.push_str(&format!("{body_indent}for (const {item} of {source}) {{\n"));
            let pair = ts_tuple_value(
                vec![
                    ts_value(acc.clone(), plain_item_ty.clone()),
                    ts_value(item.clone(), plain_item_ty.clone()),
                ],
                pair_ty.clone(),
            );
            let reduced = self.emit_call(out, op, pair, &(body_indent.clone() + "  "))?;
            out.push_str(&format!("{body_indent}  {acc} = {};\n", reduced.code));
            out.push_str(&format!("{body_indent}}}\n"));
            if matches!(output_ty, Ty::Faultable(_)) {
                out.push_str(&format!("{body_indent}{tmp} = faOk({acc});\n"));
            } else {
                out.push_str(&format!("{body_indent}{tmp} = {acc};\n"));
            }
        }
        if input_faultable {
            out.push_str(&format!("{indent}}}\n"));
        }
        Ok(ts_value(tmp, output_ty))
    }

    fn emit_scan(
        &mut self,
        out: &mut String,
        op: &str,
        input: TsValue,
        identity: TsValue,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`scan {op}` expected Seq input"));
        };
        let plain_item_ty = item_ty.inner_faultable();
        let pair_ty = Ty::Tuple(vec![plain_item_ty.clone(), plain_item_ty.clone()]);
        let operation_output_ty = self.codegen.call_output_type(op, &pair_ty)?;
        let operation_faultable = matches!(
            &operation_output_ty,
            Ty::Faultable(inner) if inner.as_ref() == &plain_item_ty
        );
        let item_faultable = matches!(item_ty.as_ref(), Ty::Faultable(_));
        let output_item_ty = if item_faultable || operation_faultable {
            Ty::Faultable(Box::new(plain_item_ty.clone()))
        } else {
            plain_item_ty.clone()
        };
        let output_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let tmp = self.next_temp_or_preferred(preferred);
        let acc = self.next_temp();
        let item = self.next_temp();
        out.push_str(&format!(
            "{indent}const {tmp}: {} = [];\n",
            ts_type(&output_ty)
        ));
        if matches!(output_item_ty, Ty::Faultable(_)) {
            out.push_str(&format!(
                "{indent}let {acc}: {} = faOk({});\n",
                ts_type(&output_item_ty),
                identity.code
            ));
        } else {
            out.push_str(&format!(
                "{indent}let {acc}: {} = {};\n",
                ts_type(&output_item_ty),
                identity.code
            ));
        }
        out.push_str(&format!(
            "{indent}for (const {item} of {}) {{\n",
            input.code
        ));
        if matches!(output_item_ty, Ty::Faultable(_)) {
            out.push_str(&format!("{indent}  if ({acc}.is_fault !== true) {{\n"));
            if item_faultable {
                out.push_str(&format!(
                    "{indent}    if ({item}.is_fault === true) {{ {acc} = faFault({item}.fault); }} else {{\n"
                ));
            }
            let nested_indent = if item_faultable {
                format!("{indent}      ")
            } else {
                format!("{indent}    ")
            };
            let pair = ts_tuple_value(
                vec![
                    ts_value(format!("{acc}.value"), plain_item_ty.clone()),
                    ts_value(
                        if item_faultable {
                            format!("{item}.value")
                        } else {
                            item.clone()
                        },
                        plain_item_ty.clone(),
                    ),
                ],
                pair_ty,
            );
            let scanned = self.emit_call(out, op, pair, &nested_indent)?;
            if operation_faultable {
                out.push_str(&format!("{nested_indent}{acc} = {};\n", scanned.code));
            } else {
                out.push_str(&format!("{nested_indent}{acc} = faOk({});\n", scanned.code));
            }
            if item_faultable {
                out.push_str(&format!("{indent}    }}\n"));
            }
            out.push_str(&format!("{indent}  }}\n"));
        } else {
            let pair = ts_tuple_value(
                vec![
                    ts_value(acc.clone(), plain_item_ty.clone()),
                    ts_value(item.clone(), plain_item_ty.clone()),
                ],
                pair_ty,
            );
            let scanned = self.emit_call(out, op, pair, &(indent.to_string() + "  "))?;
            out.push_str(&format!("{indent}  {acc} = {};\n", scanned.code));
        }
        out.push_str(&format!("{indent}  {tmp}.push({acc});\n"));
        out.push_str(&format!("{indent}}}\n"));
        Ok(ts_value(tmp, output_ty))
    }

    fn emit_repeat(
        &mut self,
        out: &mut String,
        node: &str,
        input: TsValue,
        count: TsValue,
        indent: &str,
        preferred_target: Option<&BindingTarget>,
    ) -> Result<TsValue, String> {
        if let Some(plan) = self.codegen.gpu_repeat_accumulator(node, &input.ty) {
            return self.emit_gpu_repeat_accumulator(
                out,
                plan,
                input,
                count,
                indent,
                preferred_target,
            );
        }

        if let Some(target @ BindingTarget::Tuple(_)) = preferred_target
            && matches!(&input.ty, Ty::Tuple(_))
        {
            return self.emit_repeat_tuple_state(out, node, input, count, indent, target);
        }

        let tmp = self.next_temp_or_preferred(preferred_target.and_then(binding_target_name));
        let i = self.next_temp();
        out.push_str(&format!(
            "{indent}let {tmp}: {} = {};\n",
            ts_type(&input.ty),
            input.code
        ));
        out.push_str(&format!(
            "{indent}for (let {i} = 0n; {i} < {}; {i}++) {{\n",
            count.code
        ));
        let next = self.emit_call(
            out,
            node,
            ts_value(tmp.clone(), input.ty.clone()),
            &(indent.to_string() + "  "),
        )?;
        out.push_str(&format!("{indent}  {tmp} = {};\n", next.code));
        out.push_str(&format!("{indent}}}\n"));
        Ok(ts_value(tmp, input.ty))
    }

    fn emit_gpu_repeat_accumulator(
        &mut self,
        out: &mut String,
        plan: GpuRepeatAccumulator,
        input: TsValue,
        count: TsValue,
        indent: &str,
        preferred_target: Option<&BindingTarget>,
    ) -> Result<TsValue, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("GPU repeat accumulator expected tuple state".to_string());
        };
        let target_items = match preferred_target {
            Some(BindingTarget::Tuple(targets)) => {
                if targets.len() != items.len() {
                    return Err(format!(
                        "GPU repeat accumulator expected {} tuple fields, found {}",
                        items.len(),
                        targets.len()
                    ));
                }
                Some(targets.as_slice())
            }
            _ => None,
        };

        let mut state = Vec::with_capacity(items.len());
        for (index, ty) in items.iter().enumerate() {
            let preferred = target_items
                .and_then(|targets| targets.get(index))
                .and_then(binding_target_name);
            let state_name = self.next_temp_or_preferred(preferred);
            out.push_str(&format!(
                "{indent}let {state_name}: {} = {};\n",
                ts_type(ty),
                tuple_field(&input, index)
            ));
            state.push(ts_value(state_name, ty.clone()));
        }

        let iter = self.next_temp();
        out.push_str(&format!("{indent}const {iter}: bigint = {};\n", count.code));
        out.push_str(&format!("{indent}if ({iter} > 0n) {{\n"));
        match plan.kind {
            GpuRepeatAccumulatorKind::VectorScore => {
                if state.len() != 3 {
                    return Err("GPU vector accumulator expected three tuple fields".to_string());
                }
                let helper = match plan.scalar {
                    gpu::GpuScalarKind::F32 => "faGpuRepeatVectorAccumF32",
                    gpu::GpuScalarKind::F64 => "faGpuRepeatVectorAccumF64",
                    gpu::GpuScalarKind::I32 => {
                        return Err("GPU vector accumulator expected f32 or f64 state".to_string());
                    }
                };
                let batch = self.next_temp();
                out.push_str(&format!(
                    "{indent}  const {batch}: [number] = await Promise.all([{helper}({}, {}, {}, {}, {iter})]);\n",
                    ts_string(&plan.wgsl),
                    state[0].code,
                    state[1].code,
                    state[2].code,
                ));
                out.push_str(&format!("{indent}  {} = {batch}[0];\n", state[2].code));
            }
            GpuRepeatAccumulatorKind::MatrixScore => {
                if state.len() != 4 {
                    return Err("GPU matrix accumulator expected four tuple fields".to_string());
                }
                let batch = self.next_temp();
                out.push_str(&format!(
                    "{indent}  const {batch}: [number] = await Promise.all([faGpuRepeatMatrixAccumF64({}, {}, {}, {}, {}, {iter})]);\n",
                    ts_string(&plan.wgsl),
                    state[0].code,
                    state[1].code,
                    state[2].code,
                    state[3].code,
                ));
                out.push_str(&format!("{indent}  {} = {batch}[0];\n", state[3].code));
            }
        }
        out.push_str(&format!("{indent}}}\n"));

        Ok(ts_tuple_value(state, Ty::Tuple(items)))
    }

    fn emit_repeat_tuple_state(
        &mut self,
        out: &mut String,
        node: &str,
        input: TsValue,
        count: TsValue,
        indent: &str,
        target: &BindingTarget,
    ) -> Result<TsValue, String> {
        let BindingTarget::Tuple(targets) = target else {
            return Err("repeat tuple state expected tuple binding target".to_string());
        };
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("repeat tuple state expected tuple input".to_string());
        };
        if items.len() != targets.len() {
            return Err(format!(
                "repeat tuple state expected {} tuple fields, found {}",
                targets.len(),
                items.len()
            ));
        }

        let mut state = Vec::with_capacity(items.len());
        for (index, (target, ty)) in targets.iter().zip(items.iter()).enumerate() {
            let state_name = self.next_temp_or_preferred(binding_target_name(target));
            out.push_str(&format!(
                "{indent}let {state_name}: {} = {};\n",
                ts_type(ty),
                tuple_field(&input, index)
            ));
            state.push(ts_value(state_name, ty.clone()));
        }

        let i = self.next_temp();
        let state_ty = Ty::Tuple(items);
        out.push_str(&format!(
            "{indent}for (let {i} = 0n; {i} < {}; {i}++) {{\n",
            count.code
        ));
        let next = self.emit_call(
            out,
            node,
            ts_tuple_value(state.clone(), state_ty.clone()),
            &(indent.to_string() + "  "),
        )?;
        out.push_str(&format!(
            "{indent}  [{}] = {};\n",
            state
                .iter()
                .map(|value| value.code.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            next.code
        ));
        out.push_str(&format!("{indent}}}\n"));
        Ok(ts_tuple_value(state, state_ty))
    }

    fn emit_match(
        &mut self,
        out: &mut String,
        params: TsMatchParams<'_>,
    ) -> Result<TsValue, String> {
        let TsMatchParams {
            arms,
            output_ty,
            subject,
            env,
            indent,
            preferred,
        } = params;
        let tmp = self.next_temp_or_preferred(preferred);
        out.push_str(&format!("{indent}let {tmp}: {};\n", ts_type(&output_ty)));
        for (index, arm) in arms.iter().enumerate() {
            match &arm.guard {
                TypedMatchGuard::Fallback => {
                    if index + 1 != arms.len() {
                        return Err("`match` fallback arm must be last".to_string());
                    }
                    if index == 0 {
                        out.push_str(&format!("{indent}{{\n"));
                    } else {
                        out.push_str(&format!("{indent}else {{\n"));
                    }
                }
                TypedMatchGuard::Call { node, args, .. } => {
                    if index == 0 {
                        out.push_str(&format!("{indent}{{\n"));
                    } else {
                        out.push_str(&format!("{indent}else {{\n"));
                    }
                    let guard_input = self.emit_match_guard_input(
                        out,
                        subject.clone(),
                        args,
                        env,
                        &(indent.to_string() + "  "),
                    )?;
                    let guard =
                        self.emit_call(out, node, guard_input, &(indent.to_string() + "  "))?;
                    out.push_str(&format!("{indent}  if ({}) {{\n", guard.code));
                }
            }
            let arm_indent = match &arm.guard {
                TypedMatchGuard::Fallback => format!("{indent}  "),
                TypedMatchGuard::Call { .. } => format!("{indent}    "),
            };
            let value =
                self.emit_match_target(out, &arm.target, subject.clone(), env, &arm_indent)?;
            let value = self.coerce_value(out, value, &output_ty, &arm_indent)?;
            out.push_str(&format!("{arm_indent}{tmp} = {};\n", value.code));
            match &arm.guard {
                TypedMatchGuard::Fallback => out.push_str(&format!("{indent}}}\n")),
                TypedMatchGuard::Call { .. } => out.push_str(&format!("{indent}  }}\n")),
            }
        }
        for _ in arms
            .iter()
            .filter(|arm| !matches!(arm.guard, TypedMatchGuard::Fallback))
        {
            out.push_str(&format!("{indent}}}\n"));
        }
        Ok(ts_value(tmp, output_ty))
    }

    fn emit_match_target(
        &mut self,
        out: &mut String,
        target: &TypedMatchTarget,
        subject: TsValue,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        match target {
            TypedMatchTarget::Node { name, .. } => self.emit_call(out, name, subject, indent),
            TypedMatchTarget::Value(endpoint) => self.emit_endpoint(out, endpoint, env, indent),
        }
    }

    fn emit_match_guard_input(
        &mut self,
        out: &mut String,
        subject: TsValue,
        args: &[TypedEndpoint],
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        if args.is_empty() {
            return Ok(subject);
        }
        let mut values = Vec::with_capacity(args.len() + 1);
        values.push(subject);
        for arg in args {
            values.push(self.emit_endpoint(out, arg, env, indent)?);
        }
        let ty = Ty::Tuple(values.iter().map(|value| value.ty.clone()).collect());
        Ok(ts_tuple_value(values, ty))
    }

    fn emit_fault_map(
        &mut self,
        out: &mut String,
        node: &str,
        input: TsValue,
        indent: &str,
        ok: &str,
        fault: &str,
    ) -> Result<(TsValue, TsValue), String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`fault map {node}` expected Seq input"));
        };
        let Ty::Faultable(ok_ty) = item_ty.as_ref() else {
            return Err(format!(
                "`fault map {node}` expected Seq[Faultable[V]] input"
            ));
        };
        let ok_tmp = self.next_temp_or_preferred(Some(ok));
        let fault_tmp = self.next_temp_or_preferred(Some(fault));
        let item = self.next_temp();
        let ok_seq_ty = Ty::Seq(ok_ty.clone());
        let fault_seq_ty = Ty::Seq(Box::new(Ty::Fault));
        out.push_str(&format!(
            "{indent}const {ok_tmp}: {} = [];\n",
            ts_type(&ok_seq_ty)
        ));
        out.push_str(&format!(
            "{indent}const {fault_tmp}: {} = [];\n",
            ts_type(&fault_seq_ty)
        ));
        out.push_str(&format!(
            "{indent}for (const {item} of {}) {{\n",
            input.code
        ));
        out.push_str(&format!(
            "{indent}  if ({item}.is_fault === true) {{ {fault_tmp}.push({item}.fault); }} else {{\n"
        ));
        let mapped = self.emit_call(
            out,
            node,
            ts_value(format!("{item}.value"), ok_ty.as_ref().clone()),
            &(indent.to_string() + "    "),
        )?;
        out.push_str(&format!("{indent}    {ok_tmp}.push({});\n", mapped.code));
        out.push_str(&format!("{indent}  }}\n"));
        out.push_str(&format!("{indent}}}\n"));
        Ok((
            ts_value(ok_tmp, ok_seq_ty),
            ts_value(fault_tmp, fault_seq_ty),
        ))
    }

    fn coerce_value(
        &mut self,
        out: &mut String,
        value: TsValue,
        expected: &Ty,
        indent: &str,
    ) -> Result<TsValue, String> {
        if &value.ty == expected {
            return Ok(value);
        }
        if !assignable_output_ty(expected, &value.ty) {
            return Err(format!("expected `{expected}`, found `{}`", value.ty));
        }
        if let Ty::Faultable(inner) = expected
            && inner.as_ref() == &value.ty
        {
            return Ok(ts_value(format!("faOk({})", value.code), expected.clone()));
        }
        let tmp = self.next_temp();
        out.push_str(&format!(
            "{indent}const {tmp}: {} = {};\n",
            ts_type(expected),
            value.code
        ));
        Ok(ts_value(tmp, expected.clone()))
    }

    fn plain_output_type(&self, name: &str, input_ty: &Ty) -> Result<Ty, String> {
        if let Some(signature) = self.codegen.signatures.get(name) {
            Ok(signature.output.clone())
        } else {
            builtin_output_type_plain(&self.codegen.canonical_name(name), input_ty)
        }
    }

    fn worker_map_call(
        &mut self,
        name: &str,
        input_ty: &Ty,
        output_ty: &Ty,
    ) -> Result<Option<(&'static str, String)>, String> {
        if !self.options.worker_concurrency {
            return Ok(None);
        }
        let worker_fn = match (input_ty, output_ty) {
            (Ty::I32, Ty::I32) => "faParallelMapI32",
            (Ty::I64, Ty::I64) => "faParallelMapBigInt",
            (Ty::F32, Ty::F32) => "faParallelMapF32",
            (Ty::F64, Ty::F64) => "faParallelMapNumber",
            (Ty::Bool, Ty::Bool) => "faParallelMapBool",
            _ => return Ok(None),
        };
        let Some(mapper) = self.worker_mapper_source(name, input_ty, output_ty)? else {
            return Ok(None);
        };
        let mapper_id = self.worker_mapper_id(&mapper);
        Ok(Some((worker_fn, mapper_id)))
    }

    fn worker_mapper_id(&mut self, source: &str) -> String {
        if let Some(id) = self.seen_worker_mapper_sources.get(source) {
            return id.clone();
        }
        let id = format!("m{}", self.worker_mappers.len());
        self.worker_mappers.push(WorkerMapper {
            id: id.clone(),
            source: source.to_string(),
        });
        self.seen_worker_mapper_sources
            .insert(source.to_string(), id.clone());
        id
    }

    fn worker_mapper_source(
        &self,
        name: &str,
        input_ty: &Ty,
        output_ty: &Ty,
    ) -> Result<Option<String>, String> {
        if let Some(expr) = self.worker_builtin_expr(
            &self.codegen.canonical_name(name),
            &ts_value("input", input_ty.clone()),
            output_ty,
        )? {
            return Ok(Some(format!("function(input) {{ return {expr}; }}")));
        }
        let Some(callable) = self
            .codegen
            .typed
            .callables
            .iter()
            .find(|callable| callable.name == name)
        else {
            return Ok(None);
        };
        if callable.inputs.len() != 1 || callable.outputs.len() != 1 {
            return Ok(None);
        }
        if !self
            .codegen
            .is_parallel_safe_name(name, &mut HashSet::new())
        {
            return Ok(None);
        }

        let mut lines = vec!["function(input) {".to_string()];
        let mut env = HashMap::new();
        let input_name = callable.inputs[0].name.clone();
        env.insert(input_name, ts_value("input", input_ty.clone()));

        for chain in &callable.chains {
            let mut value = match self.worker_endpoint_expr(&chain.source, &env)? {
                Some(value) => value,
                None => return Ok(None),
            };
            for (index, stage) in chain.stages.iter().enumerate() {
                let is_last = index + 1 == chain.stages.len();
                match &stage.kind {
                    TypedStageKind::Call { name, .. } => {
                        let output_ty = self.codegen.call_output_type(name, &value.ty)?;
                        value = match self.worker_call_expr(name, &value, &output_ty)? {
                            Some(value) => value,
                            None => return Ok(None),
                        };
                    }
                    TypedStageKind::Bind { target } if is_last => {
                        if !self.worker_bind_target(&mut lines, target, value.clone(), &mut env)? {
                            return Ok(None);
                        }
                    }
                    _ => return Ok(None),
                }
            }
        }

        let output = callable.outputs[0].name.clone();
        let Some(value) = env.get(&output) else {
            return Ok(None);
        };
        if &value.ty != output_ty {
            return Ok(None);
        }
        lines.push(format!("  return {};", value.code));
        lines.push("}".to_string());
        Ok(Some(lines.join("\n")))
    }

    fn worker_endpoint_expr(
        &self,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, TsValue>,
    ) -> Result<Option<TsValue>, String> {
        match &endpoint.kind {
            TypedEndpointKind::Variable(name) => Ok(env.get(name).cloned()),
            TypedEndpointKind::Int(value) => {
                Ok(Some(ts_value(format!("{value}n"), endpoint.ty.clone())))
            }
            TypedEndpointKind::Real(value) => {
                Ok(Some(ts_value(format!("{value:.17e}"), endpoint.ty.clone())))
            }
            TypedEndpointKind::Bool(value) => {
                Ok(Some(ts_value(value.to_string(), endpoint.ty.clone())))
            }
            TypedEndpointKind::Tuple(items) => {
                let values = items
                    .iter()
                    .map(|item| self.worker_endpoint_expr(item, env))
                    .collect::<Result<Option<Vec<_>>, _>>()?;
                Ok(values.map(|values| {
                    let ty = Ty::Tuple(values.iter().map(|value| value.ty.clone()).collect());
                    ts_tuple_value(values, ty)
                }))
            }
            TypedEndpointKind::NodeRef { .. }
            | TypedEndpointKind::String(_)
            | TypedEndpointKind::Unit
            | TypedEndpointKind::Seq(_)
            | TypedEndpointKind::Struct { .. }
            | TypedEndpointKind::Eval { .. } => Ok(None),
        }
    }

    fn worker_call_expr(
        &self,
        name: &str,
        input: &TsValue,
        output_ty: &Ty,
    ) -> Result<Option<TsValue>, String> {
        let Some(expr) =
            self.worker_builtin_expr(&self.codegen.canonical_name(name), input, output_ty)?
        else {
            return Ok(None);
        };
        Ok(Some(ts_value(expr, output_ty.clone())))
    }

    fn worker_builtin_expr(
        &self,
        name: &str,
        input: &TsValue,
        output_ty: &Ty,
    ) -> Result<Option<String>, String> {
        if matches!(output_ty, Ty::Faultable(_)) {
            return match name {
                "add" | "sub" | "mul" => Ok(Some(ts_faultable_numeric_binary_expr(
                    name, input, output_ty,
                ))),
                _ => Ok(None),
            };
        }
        let expr = match name {
            "expect" => match &input.ty {
                Ty::Faultable(_) => format!("faExpect({})", input.code),
                _ => input.code.clone(),
            },
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                ts_numeric_binary_expr(name, input, output_ty)
            }
            "neg" if output_ty == &Ty::I32 => format!("faCheckedI32Neg({})", input.code),
            "neg" if output_ty == &Ty::I64 => format!("faCheckedI64Neg({})", input.code),
            "neg" if output_ty == &Ty::F32 => format!("Math.fround(-{})", input.code),
            "neg" => format!("(-{})", input.code),
            "abs" if output_ty == &Ty::I32 => format!("faCheckedI32Abs({})", input.code),
            "abs" if output_ty == &Ty::I64 => format!("faCheckedI64Abs({})", input.code),
            "abs" if output_ty == &Ty::F32 => format!("Math.fround(Math.abs({}))", input.code),
            "abs" => format!("Math.abs({})", input.code),
            "sqrt" if output_ty == &Ty::F32 => format!("faCheckedSqrtF32({})", input.code),
            "sqrt" => format!("faCheckedSqrt({})", input.code),
            "exp" if output_ty == &Ty::F32 => format!("Math.fround(Math.exp({}))", input.code),
            "exp" => format!("Math.exp({})", input.code),
            "sin" if output_ty == &Ty::F32 => format!("Math.fround(Math.sin({}))", input.code),
            "sin" => format!("Math.sin({})", input.code),
            "cos" if output_ty == &Ty::F32 => format!("Math.fround(Math.cos({}))", input.code),
            "cos" => format!("Math.cos({})", input.code),
            "eq" => format!("({} === {})", tuple_field(input, 0), tuple_field(input, 1)),
            "lt" => format!("({} < {})", tuple_field(input, 0), tuple_field(input, 1)),
            "gt" => format!("({} > {})", tuple_field(input, 0), tuple_field(input, 1)),
            "le" => format!("({} <= {})", tuple_field(input, 0), tuple_field(input, 1)),
            "ge" => format!("({} >= {})", tuple_field(input, 0), tuple_field(input, 1)),
            "and" => format!("({} && {})", tuple_field(input, 0), tuple_field(input, 1)),
            "or" => format!("({} || {})", tuple_field(input, 0), tuple_field(input, 1)),
            "xor" => format!("({} !== {})", tuple_field(input, 0), tuple_field(input, 1)),
            "not" => format!("(!{})", input.code),
            "bit_and" => format!("({} & {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_or" => format!("({} | {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_xor" => format!("({} ^ {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_shl" => format!("({} << {})", tuple_field(input, 0), tuple_field(input, 1)),
            "bit_shr" => format!("({} >> {})", tuple_field(input, 0), tuple_field(input, 1)),
            _ => return Ok(None),
        };
        Ok(Some(expr))
    }

    fn worker_bind_target(
        &self,
        lines: &mut Vec<String>,
        target: &BindingTarget,
        value: TsValue,
        env: &mut HashMap<String, TsValue>,
    ) -> Result<bool, String> {
        match target {
            BindingTarget::Discard => Ok(true),
            BindingTarget::Variable(name) => {
                let ident = ts_ident(name);
                lines.push(format!("  const {ident} = {};", value.code));
                env.insert(name.clone(), ts_value(ident, value.ty));
                Ok(true)
            }
            BindingTarget::Tuple(targets) => {
                let Ty::Tuple(items) = value.ty.clone() else {
                    return Ok(false);
                };
                if items.len() != targets.len() {
                    return Ok(false);
                }
                for (index, (target, ty)) in targets.iter().zip(items.iter()).enumerate() {
                    if binding_target_is_discard(target) {
                        continue;
                    }
                    let item = ts_value(tuple_field(&value, index), ty.clone());
                    if !self.worker_bind_target(lines, target, item, env)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    fn next_temp(&mut self) -> String {
        loop {
            let tmp = format!("t{}", self.temp);
            self.temp += 1;
            if self.try_reserve_ident(&tmp) {
                return tmp;
            }
        }
    }

    fn next_temp_or_preferred(&mut self, preferred: Option<&str>) -> String {
        if let Some(preferred) = preferred {
            let ident = ts_ident(preferred);
            if self.try_reserve_ident(&ident) {
                return ident;
            }
        }
        self.next_temp()
    }

    fn reserve_ident(&mut self, ident: &str) {
        self.used_idents.insert(ident.to_string());
    }

    fn try_reserve_ident(&mut self, ident: &str) -> bool {
        self.used_idents.insert(ident.to_string())
    }

    fn reserve_internal_idents(&mut self) {
        for ident in TS_INTERNAL_VALUE_IDENTS {
            self.reserve_ident(ident);
        }
    }
}

fn typed_final_bind_target_for_stage(chain: &TypedChain, index: usize) -> Option<&BindingTarget> {
    if index + 2 != chain.stages.len() {
        return None;
    }
    match chain.stages.last() {
        Some(stage) => match &stage.kind {
            TypedStageKind::Bind { target } => Some(target),
            _ => None,
        },
        None => None,
    }
}

fn binding_target_name(target: &BindingTarget) -> Option<&str> {
    match target {
        BindingTarget::Variable(name) => Some(name),
        BindingTarget::Discard | BindingTarget::Tuple(_) => None,
    }
}

fn chain_source_variable(chain: &TypedChain) -> Option<&str> {
    match &chain.source.kind {
        TypedEndpointKind::Variable(name) => Some(name),
        _ => None,
    }
}

fn endpoint_references_any(endpoint: &TypedEndpoint, names: &HashSet<String>) -> bool {
    match &endpoint.kind {
        TypedEndpointKind::Variable(name) | TypedEndpointKind::NodeRef { name, .. } => {
            names.contains(name)
        }
        TypedEndpointKind::Tuple(items) | TypedEndpointKind::Seq(items) => items
            .iter()
            .any(|item| endpoint_references_any(item, names)),
        TypedEndpointKind::Struct { fields, .. } => fields
            .iter()
            .any(|(_, item)| endpoint_references_any(item, names)),
        TypedEndpointKind::Eval { source, stages } => {
            endpoint_references_any(source, names)
                || stages
                    .iter()
                    .any(|stage| stage_references_any(stage, names))
        }
        TypedEndpointKind::Int(_)
        | TypedEndpointKind::Real(_)
        | TypedEndpointKind::Bool(_)
        | TypedEndpointKind::String(_)
        | TypedEndpointKind::Unit => false,
    }
}

fn stage_references_any(stage: &crate::typecheck::TypedStage, names: &HashSet<String>) -> bool {
    match &stage.kind {
        TypedStageKind::Repeat { count, .. } => endpoint_references_any(count, names),
        TypedStageKind::Reduce { identity, .. } | TypedStageKind::Scan { identity, .. } => {
            endpoint_references_any(identity, names)
        }
        TypedStageKind::Match { arms } => arms.iter().any(|arm| {
            let guard_refs = match &arm.guard {
                TypedMatchGuard::Call { args, .. } => {
                    args.iter().any(|arg| endpoint_references_any(arg, names))
                }
                TypedMatchGuard::Fallback => false,
            };
            let target_refs = match &arm.target {
                TypedMatchTarget::Value(endpoint) => endpoint_references_any(endpoint, names),
                TypedMatchTarget::Node { name, .. } => names.contains(name),
            };
            guard_refs || target_refs
        }),
        TypedStageKind::Call { name, .. }
        | TypedStageKind::Map { name, .. }
        | TypedStageKind::FaultMap { node: name, .. }
        | TypedStageKind::Filter { name, .. } => names.contains(name),
        TypedStageKind::Bind { target } => binding_target_references_any(target, names),
        TypedStageKind::Field { .. } => false,
    }
}

fn binding_target_references_any(target: &BindingTarget, names: &HashSet<String>) -> bool {
    match target {
        BindingTarget::Variable(name) => names.contains(name),
        BindingTarget::Tuple(targets) => targets
            .iter()
            .any(|target| binding_target_references_any(target, names)),
        BindingTarget::Discard => false,
    }
}

fn const_int_range_endpoint(endpoint: &TypedEndpoint) -> Option<(i64, i64, i64)> {
    let TypedEndpointKind::Tuple(items) = &endpoint.kind else {
        return None;
    };
    let [start, stop, step] = items.as_slice() else {
        return None;
    };
    Some((
        const_int_endpoint(start)?,
        const_int_endpoint(stop)?,
        const_int_endpoint(step)?,
    ))
}

fn const_int_endpoint(endpoint: &TypedEndpoint) -> Option<i64> {
    match &endpoint.kind {
        TypedEndpointKind::Int(value) => Some(*value),
        _ => None,
    }
}

fn const_range_len(start: i64, stop: i64, step: i64) -> String {
    if step == 0 {
        return "0".to_string();
    }
    let len = if step > 0 {
        if start >= stop {
            0
        } else {
            ((stop - start - 1) / step) + 1
        }
    } else if start <= stop {
        0
    } else {
        ((start - stop - 1) / -step) + 1
    };
    len.to_string()
}

fn sync_map_output_storage(ty: &Ty) -> SyncMapOutputStorage {
    match ty {
        Ty::Seq(item) if matches!(item.as_ref(), Ty::I32) => SyncMapOutputStorage::Int32Array,
        Ty::Seq(item) if matches!(item.as_ref(), Ty::F32) => SyncMapOutputStorage::Float32Array,
        Ty::Seq(item) if matches!(item.as_ref(), Ty::F64) => SyncMapOutputStorage::Float64Array,
        _ => SyncMapOutputStorage::Array,
    }
}

fn ts_value(code: impl Into<String>, ty: Ty) -> TsValue {
    TsValue {
        code: code.into(),
        ty,
        tuple_items: None,
    }
}

fn ts_tuple_value(values: Vec<TsValue>, ty: Ty) -> TsValue {
    let code = format!(
        "[{}]",
        values
            .iter()
            .map(|value| value.code.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    TsValue {
        code,
        ty,
        tuple_items: Some(values),
    }
}

fn call_args(input: &TsValue, arity: usize) -> Result<String, String> {
    match arity {
        0 => Ok(String::new()),
        1 => Ok(input.code.clone()),
        _ => {
            let Ty::Tuple(items) = &input.ty else {
                return Err("multi-input call expected tuple input".to_string());
            };
            if items.len() != arity {
                return Err(format!(
                    "multi-input call expected {arity} tuple fields, found {}",
                    items.len()
                ));
            }
            if let Some(values) = &input.tuple_items {
                Ok(values
                    .iter()
                    .map(|value| value.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", "))
            } else {
                Ok((0..arity)
                    .map(|index| ts_index(&input.code, index))
                    .collect::<Vec<_>>()
                    .join(", "))
            }
        }
    }
}

fn tuple_field(input: &TsValue, index: usize) -> String {
    input
        .tuple_items
        .as_ref()
        .and_then(|items| items.get(index))
        .map(|value| value.code.clone())
        .unwrap_or_else(|| ts_index(&input.code, index))
}

fn ts_index(code: &str, index: usize) -> String {
    format!("{code}[{index}]")
}

fn ts_type(ty: &Ty) -> String {
    match ty {
        Ty::Unit => "undefined".to_string(),
        Ty::I32 => "number".to_string(),
        Ty::I64 => "bigint".to_string(),
        Ty::F32 => "number".to_string(),
        Ty::F64 => "number".to_string(),
        Ty::OneOf(_) => "never".to_string(),
        Ty::Bool => "boolean".to_string(),
        Ty::Bytes => "string".to_string(),
        Ty::Args => "FaArgs".to_string(),
        Ty::HttpServerConfig => "FaHttpServerConfig".to_string(),
        Ty::HttpListener => "FaHttpListener".to_string(),
        Ty::HttpRequest => "FaHttpRequest".to_string(),
        Ty::HttpResponse => "FaHttpResponse".to_string(),
        Ty::SqliteConnection => "FaSqliteConnection".to_string(),
        Ty::SqliteRow => "FaSqliteRow".to_string(),
        Ty::SqliteValue => "FaSqliteValue".to_string(),
        Ty::Stream(item) => format!("FaStream<{}>", ts_type(item)),
        Ty::Fault => "FaFault".to_string(),
        Ty::Faultable(inner) => format!("FaFaultable<{}>", ts_type(inner)),
        Ty::Seq(item) => format!("Array<{}>", ts_type(item)),
        Ty::Tuple(items) => {
            if items.is_empty() {
                "[]".to_string()
            } else {
                format!(
                    "[{}]",
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| format!("f{index}: {}", ts_type(item)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        Ty::Struct { fields, .. } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(field, ty)| format!("{}: {}", ts_object_key(field), ts_type(ty)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        Ty::Var(_) | Ty::EmptySeq => "unknown".to_string(),
    }
}

fn ts_ident(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    if TS_RESERVED.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

fn foreign_module_alias(specifier: &str) -> String {
    format!("__fa_foreign_{}", ts_ident(specifier))
}

fn foreign_result_expr(raw: &str, ty: &Ty) -> String {
    match ty {
        Ty::Unit => "undefined".to_string(),
        Ty::I32 => format!("faAssertI32(Number({raw}), \"foreign i32 result\")"),
        Ty::I64 => format!("BigInt({raw})"),
        Ty::F32 => format!("Math.fround(Number({raw}))"),
        Ty::F64 => format!("Number({raw})"),
        Ty::Bool => format!("Boolean({raw})"),
        Ty::Bytes => format!("String({raw})"),
        _ => format!("{raw} as {}", ts_type(ty)),
    }
}

fn ts_string(value: &str) -> String {
    format!("{value:?}")
}

fn ts_object_key(name: &str) -> String {
    if is_valid_ts_identifier(name) && !TS_RESERVED.contains(&name) {
        name.to_string()
    } else {
        ts_string(name)
    }
}

fn ts_property(name: &str) -> String {
    if is_valid_ts_identifier(name) && !TS_RESERVED.contains(&name) {
        name.to_string()
    } else {
        format!("[{}]", ts_string(name))
    }
}

fn is_valid_ts_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn expression_is_simple(expr: &str) -> bool {
    !expr.contains('\n') && expr.len() < 96
}

fn ts_value_code_is_stable(code: &str) -> bool {
    code.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn ts_numeric_binary_expr(name: &str, input: &TsValue, output_ty: &Ty) -> String {
    let left = tuple_field(input, 0);
    let right = tuple_field(input, 1);
    match name {
        "add" if output_ty == &Ty::I32 => format!("faCheckedI32Add({left}, {right})"),
        "add" if output_ty == &Ty::I64 => format!("faCheckedI64Add({left}, {right})"),
        "add" if output_ty == &Ty::F32 => format!("Math.fround({left} + {right})"),
        "add" => format!("({left} + {right})"),
        "sub" if output_ty == &Ty::I32 => format!("faCheckedI32Sub({left}, {right})"),
        "sub" if output_ty == &Ty::I64 => format!("faCheckedI64Sub({left}, {right})"),
        "sub" if output_ty == &Ty::F32 => format!("Math.fround({left} - {right})"),
        "sub" => format!("({left} - {right})"),
        "mul" if output_ty == &Ty::I32 => format!("faCheckedI32Mul({left}, {right})"),
        "mul" if output_ty == &Ty::I64 => format!("faCheckedI64Mul({left}, {right})"),
        "mul" if output_ty == &Ty::F32 => format!("Math.fround({left} * {right})"),
        "mul" => format!("({left} * {right})"),
        "div" if output_ty == &Ty::I32 => format!("faCheckedI32Div({left}, {right})"),
        "div" if output_ty == &Ty::I64 => format!("faCheckedI64Div({left}, {right})"),
        "div" if output_ty == &Ty::F32 => format!("faCheckedF32Div({left}, {right})"),
        "div" => format!("faCheckedRealDiv({left}, {right})"),
        "rem" if output_ty == &Ty::I32 => format!("faCheckedI32Rem({left}, {right})"),
        "rem" if output_ty == &Ty::I64 => format!("faCheckedI64Rem({left}, {right})"),
        "rem" if output_ty == &Ty::F32 => format!("faCheckedF32Rem({left}, {right})"),
        "rem" => format!("faCheckedRealRem({left}, {right})"),
        "min" => format!("({left} <= {right} ? {left} : {right})"),
        "max" => format!("({left} >= {right} ? {left} : {right})"),
        _ if matches!(output_ty, Ty::F64) => "Number.NaN".to_string(),
        _ => "0n".to_string(),
    }
}

fn ts_faultable_numeric_binary_expr(name: &str, input: &TsValue, output_ty: &Ty) -> String {
    let Ty::Faultable(inner) = output_ty else {
        unreachable!("faultable numeric binary op expected faultable output")
    };
    let left = tuple_field(input, 0);
    let right = tuple_field(input, 1);
    match (name, inner.as_ref()) {
        ("div", Ty::I32) => format!("faFaultableI32Div({left}, {right})"),
        ("div", Ty::I64) => format!("faFaultableI64Div({left}, {right})"),
        ("div", Ty::F32) => format!("faFaultableF32Div({left}, {right})"),
        ("div", Ty::F64) => format!("faFaultableRealDiv({left}, {right})"),
        ("rem", Ty::I32) => format!("faFaultableI32Rem({left}, {right})"),
        ("rem", Ty::I64) => format!("faFaultableI64Rem({left}, {right})"),
        ("rem", Ty::F32) => format!("faFaultableF32Rem({left}, {right})"),
        ("rem", Ty::F64) => format!("faFaultableRealRem({left}, {right})"),
        ("add", Ty::I32) => format!("faFaultableI32Add({left}, {right})"),
        ("add", Ty::I64) => format!("faFaultableI64Add({left}, {right})"),
        ("sub", Ty::I32) => format!("faFaultableI32Sub({left}, {right})"),
        ("sub", Ty::I64) => format!("faFaultableI64Sub({left}, {right})"),
        ("mul", Ty::I32) => format!("faFaultableI32Mul({left}, {right})"),
        ("mul", Ty::I64) => format!("faFaultableI64Mul({left}, {right})"),
        _ => unreachable!(),
    }
}

const TS_RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

const TS_INTERNAL_VALUE_IDENTS: &[&str] = &[
    "faAt",
    "faBroadcastLeft",
    "faBroadcastRight",
    "faCollect",
    "faConcatBytes",
    "faExitCode",
    "faExpect",
    "faFault",
    "faFaultMessage",
    "faFill",
    "faFlagPresent",
    "faFlagValue",
    "faGet",
    "faGetOr",
    "faGroupById",
    "faHead",
    "faLast",
    "faOk",
    "faParallelMapBigInt",
    "faParallelMapBool",
    "faParallelMapNumber",
    "faParallelMapScalar",
    "faParseInt",
    "faParseReal",
    "faRangeStep",
    "faReadStdin",
    "faSet",
    "faShiftLeft",
    "faShiftRight",
    "__flowarrow_setup_workers",
    "__flowarrow_teardown_workers",
    "__flowarrow_worker_mapper_ids",
    "faCreateScalarPooledWorker",
    "faCreateScalarWorkerPool",
    "faDefaultScalarWorkerCount",
    "faDispatchScalarWorkerPool",
    "faGrowScalarWorkerPool",
    "faLoadNodeWorkerThreads",
    "faGpuRuntimeModule",
    "faRejectScalarWorker",
    "faRetireScalarWorkerPool",
    "faRunScalarWorker",
    "faScalarInputBuffer",
    "faScalarWorkerPool",
    "faScalarWorkerPools",
    "faSetupScalarWorkerPools",
    "faTeardownScalarWorkerPools",
    "faUseSharedNumericSequences",
    "faSplitLines",
    "faStripPrefix",
    "faStripSuffix",
    "faTranspose",
    "faWriteStderr",
    "faWriteStdout",
    "faZip",
    "faReadScalarBuffer",
    "faScalarBytes",
    "faScalarWorkerRuntime",
    "faWriteScalarBuffer",
];

const TS_PRELUDE: &str = r#"// Generated by FlowArrow. Do not edit by hand.
declare const process: any;

type FaArgs = { argv: string[] };
type FaFault = { message: string };
type FaFaultable<T> = { is_fault: true; fault: FaFault } | { is_fault: false; value: T };
type FaStream<T> = unknown;
type FaHttpServerConfig = unknown;
type FaHttpListener = unknown;
type FaHttpRequest = unknown;
type FaHttpResponse = unknown;
type FaSqliteConnection = unknown;
type FaSqliteRow = unknown;
type FaSqliteValue = unknown;

function faOk<T>(value: T): FaFaultable<T> {
  return { is_fault: false, value };
}

function faFault<T = never>(fault: FaFault): FaFaultable<T> {
  return { is_fault: true, fault };
}

function faFaultMessage<T = never>(message: string): FaFaultable<T> {
  return faFault({ message });
}

function faExpect<T>(value: FaFaultable<T> | T): T {
  if (typeof value === "object" && value !== null && "is_fault" in value) {
    const faultable = value as FaFaultable<T>;
    if (faultable.is_fault === true) throw new Error(faultable.fault.message);
    return faultable.value;
  }
  return value as T;
}

function faExitCode(value: bigint | FaFaultable<bigint>): bigint {
  if (typeof value === "object" && value !== null && "is_fault" in value) {
    const faultable = value as FaFaultable<bigint>;
    if (faultable.is_fault === true) {
      console.error(faultable.fault.message);
      return 1n;
    }
    return faultable.value;
  }
  return value as bigint;
}

function faReadStdin(): string {
  const fs = process.getBuiltinModule("node:fs");
  return fs.readFileSync(0, "utf8");
}

function faWriteStdout(bytes: string): bigint {
  process.stdout.write(bytes);
  return 0n;
}

function faWriteStderr(bytes: string): bigint {
  process.stderr.write(bytes);
  return 0n;
}

function faSplitLines(bytes: string): string[] {
  if (bytes.length === 0) return [];
  return bytes.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n").filter((line, index, lines) => index + 1 < lines.length || line.length > 0);
}

function faConcatBytes(items: Array<string>): string;
function faConcatBytes(items: Array<FaFaultable<string>>): FaFaultable<string>;
function faConcatBytes(items: Array<string | FaFaultable<string>>): string | FaFaultable<string> {
  let out = "";
  let sawFaultable = false;
  for (const item of items) {
    if (typeof item === "object" && item !== null && "is_fault" in item) {
      sawFaultable = true;
      const faultable = item as FaFaultable<string>;
      if (faultable.is_fault === true) return faFault(faultable.fault);
      out += faultable.value;
    } else {
      out += item;
    }
  }
  return sawFaultable ? faOk(out) : out;
}

function faParseInt(bytes: string): FaFaultable<bigint> {
  const text = bytes.trim();
  if (!/^[+-]?\d+$/.test(text)) return faFaultMessage(`parse_int: invalid integer '${bytes}'`);
  const value = BigInt(text);
  if (value < FA_I64_MIN || value > FA_I64_MAX) return faFaultMessage(`parse_int: integer out of range '${bytes}'`);
  return faOk(value);
}

const FA_I64_MIN = -(1n << 63n);
const FA_I64_MAX = (1n << 63n) - 1n;
const FA_I32_MIN = -(2 ** 31);
const FA_I32_MAX = (2 ** 31) - 1;

function faAssertI32(value: number, label: string): number {
  if (!Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX) throw new Error(`${label}: integer overflow`);
  return value | 0;
}

function faAssertI64(value: bigint, label: string): bigint {
  if (value < FA_I64_MIN || value > FA_I64_MAX) throw new Error(`${label}: integer overflow`);
  return value;
}

function faCheckedI32Add(left: number, right: number): number {
  return faAssertI32(left + right, "add");
}

function faCheckedI32Sub(left: number, right: number): number {
  return faAssertI32(left - right, "sub");
}

function faCheckedI32Mul(left: number, right: number): number {
  return faAssertI32(left * right, "mul");
}

function faCheckedI32Div(left: number, right: number): number {
  if (right === 0) throw new Error("div: division by zero");
  if (left === FA_I32_MIN && right === -1) throw new Error("div: integer overflow");
  return Math.trunc(left / right);
}

function faCheckedI32Rem(left: number, right: number): number {
  if (right === 0) throw new Error("rem: remainder by zero");
  if (left === FA_I32_MIN && right === -1) throw new Error("rem: integer overflow");
  return left % right;
}

function faCheckedI32Neg(value: number): number {
  if (value === FA_I32_MIN) throw new Error("neg: integer overflow");
  return -value;
}

function faCheckedI32Abs(value: number): number {
  if (value === FA_I32_MIN) throw new Error("abs: integer overflow");
  return value < 0 ? -value : value;
}

function faCheckedI64Add(left: bigint, right: bigint): bigint {
  return faAssertI64(left + right, "add");
}

function faCheckedI64Sub(left: bigint, right: bigint): bigint {
  return faAssertI64(left - right, "sub");
}

function faCheckedI64Mul(left: bigint, right: bigint): bigint {
  return faAssertI64(left * right, "mul");
}

function faCheckedI64Div(left: bigint, right: bigint): bigint {
  if (right === 0n) throw new Error("div: division by zero");
  if (left === FA_I64_MIN && right === -1n) throw new Error("div: integer overflow");
  return left / right;
}

function faCheckedI64Rem(left: bigint, right: bigint): bigint {
  if (right === 0n) throw new Error("rem: remainder by zero");
  if (left === FA_I64_MIN && right === -1n) throw new Error("rem: integer overflow");
  return left % right;
}

function faCheckedI64Neg(value: bigint): bigint {
  if (value === FA_I64_MIN) throw new Error("neg: integer overflow");
  return -value;
}

function faCheckedI64Abs(value: bigint): bigint {
  if (value === FA_I64_MIN) throw new Error("abs: integer overflow");
  return value < 0n ? -value : value;
}

function faFaultableI32Add(left: number, right: number): FaFaultable<number> {
  const value = left + right;
  if (!Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX) return faFaultMessage("add: integer overflow");
  return faOk(value | 0);
}

function faFaultableI32Sub(left: number, right: number): FaFaultable<number> {
  const value = left - right;
  if (!Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX) return faFaultMessage("sub: integer overflow");
  return faOk(value | 0);
}

function faFaultableI32Mul(left: number, right: number): FaFaultable<number> {
  const value = left * right;
  if (!Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX) return faFaultMessage("mul: integer overflow");
  return faOk(value | 0);
}

function faFaultableI32Neg(value: number): FaFaultable<number> {
  if (value === FA_I32_MIN) return faFaultMessage("neg: integer overflow");
  return faOk(-value);
}

function faFaultableI32Abs(value: number): FaFaultable<number> {
  if (value === FA_I32_MIN) return faFaultMessage("abs: integer overflow");
  return faOk(value < 0 ? -value : value);
}

function faFaultableI64Add(left: bigint, right: bigint): FaFaultable<bigint> {
  const value = left + right;
  if (value < FA_I64_MIN || value > FA_I64_MAX) return faFaultMessage("add: integer overflow");
  return faOk(value);
}

function faFaultableI64Sub(left: bigint, right: bigint): FaFaultable<bigint> {
  const value = left - right;
  if (value < FA_I64_MIN || value > FA_I64_MAX) return faFaultMessage("sub: integer overflow");
  return faOk(value);
}

function faFaultableI64Mul(left: bigint, right: bigint): FaFaultable<bigint> {
  const value = left * right;
  if (value < FA_I64_MIN || value > FA_I64_MAX) return faFaultMessage("mul: integer overflow");
  return faOk(value);
}

function faFaultableI64Neg(value: bigint): FaFaultable<bigint> {
  if (value === FA_I64_MIN) return faFaultMessage("neg: integer overflow");
  return faOk(-value);
}

function faFaultableI64Abs(value: bigint): FaFaultable<bigint> {
  if (value === FA_I64_MIN) return faFaultMessage("abs: integer overflow");
  return faOk(value < 0n ? -value : value);
}

function faCheckedF32Div(left: number, right: number): number {
  if (right === 0) throw new Error("div: division by zero");
  return Math.fround(left / right);
}

function faCheckedF32Rem(left: number, right: number): number {
  if (right === 0) throw new Error("rem: remainder by zero");
  return Math.fround(left % right);
}

function faCheckedRealDiv(left: number, right: number): number {
  if (right === 0) throw new Error("div: division by zero");
  return left / right;
}

function faCheckedRealRem(left: number, right: number): number {
  if (right === 0) throw new Error("rem: remainder by zero");
  return left % right;
}

function faCheckedSqrt(value: number): number {
  if (value < 0) throw new Error("sqrt: negative input");
  return Math.sqrt(value);
}

function faCheckedSqrtF32(value: number): number {
  if (value < 0) throw new Error("sqrt: negative input");
  return Math.fround(Math.sqrt(value));
}

function faFaultableI32Div(left: number, right: number): FaFaultable<number> {
  if (right === 0) return faFaultMessage("div: division by zero");
  if (left === FA_I32_MIN && right === -1) return faFaultMessage("div: integer overflow");
  return faOk(Math.trunc(left / right));
}

function faFaultableI32Rem(left: number, right: number): FaFaultable<number> {
  if (right === 0) return faFaultMessage("rem: remainder by zero");
  if (left === FA_I32_MIN && right === -1) return faFaultMessage("rem: integer overflow");
  return faOk(left % right);
}

function faFaultableI64Div(left: bigint, right: bigint): FaFaultable<bigint> {
  if (right === 0n) return faFaultMessage("div: division by zero");
  if (left === FA_I64_MIN && right === -1n) return faFaultMessage("div: integer overflow");
  return faOk(left / right);
}

function faFaultableI64Rem(left: bigint, right: bigint): FaFaultable<bigint> {
  if (right === 0n) return faFaultMessage("rem: remainder by zero");
  if (left === FA_I64_MIN && right === -1n) return faFaultMessage("rem: integer overflow");
  return faOk(left % right);
}

function faFaultableF32Div(left: number, right: number): FaFaultable<number> {
  if (right === 0) return faFaultMessage("div: division by zero");
  return faOk(Math.fround(left / right));
}

function faFaultableF32Rem(left: number, right: number): FaFaultable<number> {
  if (right === 0) return faFaultMessage("rem: remainder by zero");
  return faOk(Math.fround(left % right));
}

function faFaultableRealDiv(left: number, right: number): FaFaultable<number> {
  if (right === 0) return faFaultMessage("div: division by zero");
  return faOk(left / right);
}

function faFaultableRealRem(left: number, right: number): FaFaultable<number> {
  if (right === 0) return faFaultMessage("rem: remainder by zero");
  return faOk(left % right);
}

function faFaultableSqrt(value: number): FaFaultable<number> {
  if (value < 0) return faFaultMessage("sqrt: negative input");
  return faOk(Math.sqrt(value));
}

function faFaultableSqrtF32(value: number): FaFaultable<number> {
  if (value < 0) return faFaultMessage("sqrt: negative input");
  return faOk(Math.fround(Math.sqrt(value)));
}

function faParseReal(bytes: string): FaFaultable<number> {
  const text = bytes.trim();
  const value = Number(text);
  if (text.length === 0 || Number.isNaN(value)) return faFaultMessage(`parse_real: invalid real '${bytes}'`);
  return faOk(value);
}

function faFlagPresent(input: [f0: string[], f1: string]): boolean {
  return input[0].includes(input[1]);
}

function faFlagValue(input: [f0: string[], f1: string]): FaFaultable<string> {
  const index = input[0].indexOf(input[1]);
  if (index < 0 || index + 1 >= input[0].length) return faFaultMessage(`flag_value: missing value for ${input[1]}`);
  return faOk(input[0][index + 1]);
}

function faStripPrefix(input: [f0: string, f1: string]): FaFaultable<string> {
  return input[0].startsWith(input[1]) ? faOk(input[0].slice(input[1].length)) : faFaultMessage("strip_prefix: prefix not found");
}

function faStripSuffix(input: [f0: string, f1: string]): FaFaultable<string> {
  return input[0].endsWith(input[1]) ? faOk(input[0].slice(0, -input[1].length)) : faFaultMessage("strip_suffix: suffix not found");
}

function faCollect<T>(items: Array<FaFaultable<T>>): FaFaultable<T[]> {
  const out: T[] = [];
  for (const item of items) {
    if (item.is_fault === true) return faFault(item.fault);
    out.push(item.value);
  }
  return faOk(out);
}

type FaScalarMapKind = "bigint" | "i32" | "f32" | "number" | "bool";
type FaScalarWorker = {
  postMessage(message: unknown): void;
  terminate(): void | Promise<unknown>;
  onmessage?: ((event: { data: unknown }) => void) | null;
  onerror?: ((event: { error?: unknown; message?: string }) => void) | null;
  on?: (event: string, listener: (...args: Array<any>) => void) => void;
};
type FaScalarWorkerRuntime =
  | {
      kind: "browser";
      workerUrl: string;
      Worker: new (url: string, options: { type: "module" }) => FaScalarWorker;
      revokeObjectURL: (url: string) => void;
    }
  | {
      kind: "node";
      workerUrl: string;
      Worker: new (url: URL, options: { type: "module"; execArgv: Array<string> }) => FaScalarWorker;
    };
type FaScalarPooledWorker = {
  worker: FaScalarWorker;
  alive: boolean;
  ready: Promise<void>;
  resolve?: () => void;
  reject?: (error: unknown) => void;
};
type FaScalarWorkerPool = {
  runtime: FaScalarWorkerRuntime;
  workers: FaScalarPooledWorker[];
  queue: Promise<void>;
  retired: boolean;
};
let faUseSharedNumericSequences = false;
let faScalarWorkerModuleUrl: string | null = null;
const faScalarWorkerPools = new Map<string, Promise<FaScalarWorkerPool | null>>();

function faParallelMapI32(input: Array<number>, mapperId: string, workerCount?: number): Promise<Array<number>> {
  return faParallelMapScalar(input, mapperId, "i32", "i32", workerCount) as Promise<Array<number>>;
}

function faParallelMapBigInt(input: Array<bigint>, mapperId: string, workerCount?: number): Promise<Array<bigint>> {
  return faParallelMapScalar(input, mapperId, "bigint", "bigint", workerCount) as Promise<Array<bigint>>;
}

function faParallelMapF32(input: Array<number>, mapperId: string, workerCount?: number): Promise<Array<number>> {
  return faParallelMapScalar(input, mapperId, "f32", "f32", workerCount) as Promise<Array<number>>;
}

function faParallelMapNumber(input: Array<number>, mapperId: string, workerCount?: number): Promise<Array<number>> {
  return faParallelMapScalar(input, mapperId, "number", "number", workerCount) as Promise<Array<number>>;
}

function faParallelMapBool(input: Array<boolean>, mapperId: string, workerCount?: number): Promise<Array<boolean>> {
  return faParallelMapScalar(input, mapperId, "bool", "bool", workerCount) as Promise<Array<boolean>>;
}

async function faParallelMapScalar<I, O>(
  input: Array<I>,
  mapperId: string,
  inputKind: FaScalarMapKind,
  outputKind: FaScalarMapKind,
  requestedWorkerCount?: number,
): Promise<Array<O>> {
  if (typeof SharedArrayBuffer !== "function") throw new Error("worker concurrency requires SharedArrayBuffer");
  if (input.length === 0) return [];

  const workerCount = requestedWorkerCount === undefined
    ? faDefaultScalarWorkerCount(input.length)
    : Math.max(1, Math.min(input.length, requestedWorkerCount));
  if (workerCount <= 0) throw new Error("worker concurrency requires at least one worker");

  const inputBuffer = faScalarInputBuffer(input, inputKind);
  const outputBuffer = new SharedArrayBuffer(input.length * faScalarBytes(outputKind));
  const pool = await faScalarWorkerPool(mapperId, workerCount);
  await faDispatchScalarWorkerPool(pool, workerCount, input.length, { inputBuffer, outputBuffer, inputKind, outputKind });
  return faReadScalarBuffer<O>(outputBuffer, input.length, outputKind);
}

function faDefaultScalarWorkerCount(inputLength?: number): number {
  const hardwareConcurrency = (globalThis as typeof globalThis & { navigator?: { hardwareConcurrency?: number } }).navigator?.hardwareConcurrency ?? 4;
  const workerCount = Math.max(1, Math.min(8, hardwareConcurrency));
  return inputLength === undefined ? workerCount : Math.min(inputLength, workerCount);
}

async function faSetupScalarWorkerPools(mapperIds: string[]): Promise<void> {
  if (typeof SharedArrayBuffer !== "function") throw new Error("worker concurrency requires SharedArrayBuffer");
  const workerCount = faDefaultScalarWorkerCount();
  const perMapperWorkerCount = Math.max(1, Math.floor(workerCount / Math.max(1, mapperIds.length)));
  await Promise.all(mapperIds.map((mapperId) => faScalarWorkerPool(mapperId, perMapperWorkerCount)));
}

async function faTeardownScalarWorkerPools(): Promise<void> {
  const entries = Array.from(faScalarWorkerPools.entries());
  faScalarWorkerPools.clear();
  await Promise.all(entries.map(async ([mapperId, poolPromise]) => {
    const pool = await poolPromise;
    if (pool !== null) await faRetireScalarWorkerPool(mapperId, pool);
  }));
}

async function faScalarWorkerPool(mapperId: string, workerCount: number): Promise<FaScalarWorkerPool> {
  let poolPromise = faScalarWorkerPools.get(mapperId);
  if (poolPromise === undefined) {
    poolPromise = faCreateScalarWorkerPool(mapperId);
    faScalarWorkerPools.set(mapperId, poolPromise);
  }

  const pool = await poolPromise;
  if (pool === null) throw new Error("worker concurrency is unavailable");
  if (pool.retired) throw new Error("worker pool is retired");

  try {
    faGrowScalarWorkerPool(pool, mapperId, workerCount);
    return pool;
  } catch {
    await faRetireScalarWorkerPool(mapperId, pool);
    throw new Error(`failed to create worker pool for ${mapperId}`);
  }
}

async function faCreateScalarWorkerPool(mapperId: string): Promise<FaScalarWorkerPool | null> {
  const runtime = await faScalarWorkerRuntime();
  if (runtime === null) throw new Error("worker concurrency requires module worker support");
  return { runtime, workers: [], queue: Promise.resolve(), retired: false };
}

async function faScalarWorkerRuntime(): Promise<FaScalarWorkerRuntime | null> {
  if (faScalarWorkerModuleUrl !== null) {
    const workerGlobals = globalThis as typeof globalThis & {
      Worker?: new (url: string, options: { type: "module" }) => FaScalarWorker;
    };
    if (typeof workerGlobals.Worker === "function") {
      return {
        kind: "browser",
        workerUrl: faScalarWorkerModuleUrl,
        Worker: workerGlobals.Worker,
        revokeObjectURL: () => undefined,
      };
    }

    const processLike = (globalThis as typeof globalThis & {
      process?: {
        versions?: { node?: string };
        getBuiltinModule?: (name: string) => any;
      };
    }).process;
    if (typeof processLike?.versions?.node !== "string") return null;
    try {
      const workerThreads = faLoadNodeWorkerThreads(processLike);
      return typeof workerThreads?.Worker === "function"
        ? { kind: "node", workerUrl: faScalarWorkerModuleUrl, Worker: workerThreads.Worker }
        : null;
    } catch {
      return null;
    }
  }

  return null;
}

function faLoadNodeWorkerThreads(processLike: { getBuiltinModule?: (name: string) => any }): any {
  return typeof processLike.getBuiltinModule === "function"
    ? processLike.getBuiltinModule("node:worker_threads")
    : null;
}

function faGrowScalarWorkerPool(pool: FaScalarWorkerPool, mapperId: string, workerCount: number): void {
  while (pool.workers.length < workerCount) {
    pool.workers.push(faCreateScalarPooledWorker(pool.runtime, mapperId));
  }
}

function faDispatchScalarWorkerPool(
  pool: FaScalarWorkerPool,
  workerCount: number,
  inputLength: number,
  message: {
    inputBuffer: SharedArrayBuffer;
    outputBuffer: SharedArrayBuffer;
    inputKind: FaScalarMapKind;
    outputKind: FaScalarMapKind;
  },
): Promise<void> {
  const dispatch = async () => {
    if (pool.retired) throw new Error("Worker pool is retired");
    const jobs = pool.workers.slice(0, workerCount).map((worker, index) => {
      const start = Math.floor((inputLength * index) / workerCount);
      const end = Math.floor((inputLength * (index + 1)) / workerCount);
      return faRunScalarWorker(worker, { ...message, start, end });
    });
    await Promise.all(jobs);
  };
  pool.queue = pool.queue.then(dispatch, dispatch);
  return pool.queue;
}

function faCreateScalarPooledWorker(runtime: FaScalarWorkerRuntime, mapperId: string): FaScalarPooledWorker {
  let worker: FaScalarWorker;
  try {
    worker = runtime.kind === "browser"
      ? new runtime.Worker(runtime.workerUrl, { type: "module" })
      : new runtime.Worker(new URL(runtime.workerUrl), { type: "module", execArgv: [] });
  } catch (e) {
    if (runtime.kind === "browser") console.error(`Cross-origin worker module failed to load: ${e}`);
    throw e;
  }

  let readyResolve: () => void = () => undefined;
  let readyReject: (error: unknown) => void = () => undefined;
  const ready = new Promise<void>((resolve, reject) => {
    readyResolve = resolve;
    readyReject = reject;
  });
  const pooled: FaScalarPooledWorker = { worker, alive: true, ready };
  if (runtime.kind === "browser") {
    worker.onmessage = (event) => {
      if (event.data && (event.data as { type?: string }).type === "ready") {
        readyResolve();
        return;
      }
      const resolve = pooled.resolve;
      pooled.resolve = undefined;
      pooled.reject = undefined;
      if (resolve) resolve();
    };
    worker.onerror = (event) => {
      pooled.alive = false;
      readyReject(event.error ?? new Error(event.message));
      faRejectScalarWorker(pooled, event.error ?? new Error(event.message));
    };
  } else {
    if (worker.on) worker.on("message", (data: unknown) => {
      if (data && (data as { type?: string }).type === "ready") {
        readyResolve();
        return;
      }
      const resolve = pooled.resolve;
      pooled.resolve = undefined;
      pooled.reject = undefined;
      if (resolve) resolve();
    });
    if (worker.on) worker.on("error", (error: unknown) => {
      pooled.alive = false;
      readyReject(error);
      faRejectScalarWorker(pooled, error);
    });
    if (worker.on) worker.on("exit", (code: number) => {
      const wasAlive = pooled.alive;
      pooled.alive = false;
      if (code !== 0 && wasAlive) {
        const error = new Error(`Worker stopped with exit code ${code}`);
        readyReject(error);
        faRejectScalarWorker(pooled, error);
      }
    });
  }
  worker.postMessage({ type: "init", mapperId });
  return pooled;
}

async function faRunScalarWorker(
  pooled: FaScalarPooledWorker,
  message: {
    inputBuffer: SharedArrayBuffer;
    outputBuffer: SharedArrayBuffer;
    start: number;
    end: number;
    inputKind: FaScalarMapKind;
    outputKind: FaScalarMapKind;
  },
): Promise<void> {
  await pooled.ready;
  return new Promise<void>((resolve, reject) => {
    if (!pooled.alive) {
      reject(new Error("Worker is no longer available"));
      return;
    }
    if (pooled.resolve !== undefined || pooled.reject !== undefined) {
      reject(new Error("Worker is already busy"));
      return;
    }

    pooled.resolve = resolve;
    pooled.reject = reject;
    try {
      pooled.worker.postMessage(message);
    } catch (e) {
      pooled.resolve = undefined;
      pooled.reject = undefined;
      reject(e);
    }
  });
}

function faRejectScalarWorker(pooled: FaScalarPooledWorker, error: unknown): void {
  const reject = pooled.reject;
  pooled.resolve = undefined;
  pooled.reject = undefined;
  if (reject) reject(error);
}

async function faRetireScalarWorkerPool(mapperId: string, pool: FaScalarWorkerPool): Promise<void> {
  faScalarWorkerPools.delete(mapperId);
  pool.retired = true;
  await pool.queue.catch(() => undefined);
  const terminations: Array<Promise<unknown>> = [];
  for (const pooled of pool.workers) {
    pooled.alive = false;
    try {
      const done = pooled.worker.terminate();
      if (done && typeof (done as Promise<unknown>).catch === "function") {
        terminations.push(done.catch(() => undefined));
      }
    } catch {
      // Ignore cleanup failures after a worker pool has already failed.
    }
  }
  await Promise.all(terminations);
  if (pool.runtime.kind === "browser") pool.runtime.revokeObjectURL(pool.runtime.workerUrl);
}

function faScalarBytes(kind: FaScalarMapKind): number {
  if (kind === "bigint") return BigInt64Array.BYTES_PER_ELEMENT;
  if (kind === "i32") return Int32Array.BYTES_PER_ELEMENT;
  if (kind === "f32") return Float32Array.BYTES_PER_ELEMENT;
  if (kind === "number") return Float64Array.BYTES_PER_ELEMENT;
  return Uint8Array.BYTES_PER_ELEMENT;
}

function faWriteScalarBuffer<T>(buffer: SharedArrayBuffer, input: Array<T>, kind: FaScalarMapKind): void {
  if (kind === "bigint") {
    new BigInt64Array(buffer).set(input as Array<bigint>);
  } else if (kind === "i32") {
    new Int32Array(buffer).set((input as Array<number>).map((value) => faAssertI32(value, "worker i32")));
  } else if (kind === "f32") {
    new Float32Array(buffer).set((input as Array<number>).map(Math.fround));
  } else if (kind === "number") {
    new Float64Array(buffer).set(input as Array<number>);
  } else {
    const view = new Uint8Array(buffer);
    input.forEach((value, index) => {
      view[index] = value ? 1 : 0;
    });
  }
}

function faScalarInputBuffer<T>(input: Array<T>, kind: FaScalarMapKind): SharedArrayBuffer {
  const byteLength = input.length * faScalarBytes(kind);
  if (
    kind === "bigint" &&
    input instanceof BigInt64Array &&
    input.buffer instanceof SharedArrayBuffer &&
    input.byteOffset === 0 &&
    input.byteLength === byteLength
  ) {
    return input.buffer;
  }
  if (
    kind === "i32" &&
    input instanceof Int32Array &&
    input.buffer instanceof SharedArrayBuffer &&
    input.byteOffset === 0 &&
    input.byteLength === byteLength
  ) {
    return input.buffer;
  }
  if (
    kind === "f32" &&
    input instanceof Float32Array &&
    input.buffer instanceof SharedArrayBuffer &&
    input.byteOffset === 0 &&
    input.byteLength === byteLength
  ) {
    return input.buffer;
  }
  if (
    kind === "number" &&
    input instanceof Float64Array &&
    input.buffer instanceof SharedArrayBuffer &&
    input.byteOffset === 0 &&
    input.byteLength === byteLength
  ) {
    return input.buffer;
  }
  const buffer = new SharedArrayBuffer(byteLength);
  faWriteScalarBuffer(buffer, input, kind);
  return buffer;
}

function faReadScalarBuffer<T>(buffer: SharedArrayBuffer, length: number, kind: FaScalarMapKind): Array<T> {
  if (kind === "bigint") return new BigInt64Array(buffer, 0, length) as unknown as Array<T>;
  if (kind === "i32") return new Int32Array(buffer, 0, length) as unknown as Array<T>;
  if (kind === "f32") return new Float32Array(buffer, 0, length) as unknown as Array<T>;
  if (kind === "number") return new Float64Array(buffer, 0, length) as unknown as Array<T>;
  return Array.from(new Uint8Array(buffer, 0, length), (value) => value !== 0) as Array<T>;
}

function faZip<A, B>(input: [f0: A[], f1: B[]]): Array<[f0: A, f1: B]> {
  if (input[0].length !== input[1].length) throw new Error("zip: sequences must have the same length");
  return input[0].map((left, index) => [left, input[1][index]]);
}

function faBroadcastLeft<A, B>(input: [f0: A, f1: B[]]): Array<[f0: A, f1: B]> {
  return input[1].map((item) => [input[0], item]);
}

function faBroadcastRight<A, B>(input: [f0: A[], f1: B]): Array<[f0: A, f1: B]> {
  return input[0].map((item) => [item, input[1]]);
}

function faTranspose<T>(rows: T[][]): T[][] {
  if (rows.length === 0) return [];
  const width = rows[0].length;
  if (!rows.every((row) => row.length === width)) throw new Error("transpose: rows must have the same length");
  return Array.from({ length: width }, (_, column) => rows.map((row) => row[column]));
}

function faGroupById<T>(items: Array<[f0: bigint, f1: T]>): T[][] {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const key = item[0].toString();
    const group = groups.get(key) ?? [];
    group.push(item[1]);
    groups.set(key, group);
  }
  return [...groups.keys()].sort((a, b) => Number(BigInt(a) - BigInt(b))).map((key) => groups.get(key)!);
}

function faShiftRight<T>(input: [f0: T[], f1: T]): T[] {
  return [input[1], ...input[0].slice(0, Math.max(0, input[0].length - 1))];
}

function faShiftLeft<T>(input: [f0: T[], f1: T]): T[] {
  return [...input[0].slice(1), input[1]];
}

function faHead<T>(items: T[]): FaFaultable<T> {
  return items.length === 0 ? faFaultMessage("head: empty sequence") : faOk(items[0]);
}

function faLast<T>(items: T[]): FaFaultable<T> {
  return items.length === 0 ? faFaultMessage("last: empty sequence") : faOk(items[items.length - 1]);
}

function faGet<T>(input: [f0: T[], f1: bigint]): FaFaultable<T> {
  const index = Number(input[1]);
  return index < 0 || index >= input[0].length ? faFaultMessage("get: index out of range") : faOk(input[0][index]);
}

function faGetOr<T>(input: [f0: T[], f1: bigint, f2: T]): T {
  const index = Number(input[1]);
  return index < 0 || index >= input[0].length ? input[2] : input[0][index];
}

function faAt<T>(input: [f0: T[], f1: bigint]): T {
  const index = Number(input[1]);
  if (index < 0 || index >= input[0].length) throw new Error("at: index out of range");
  return input[0][index];
}

function faFill<T>(input: [f0: bigint, f1: T]): T[] {
  return Array.from({ length: Number(input[0]) }, () => input[1]);
}

function faSet<T>(input: [f0: T[], f1: bigint, f2: T]): FaFaultable<T[]> {
  const index = Number(input[1]);
  if (index < 0 || index >= input[0].length) return faFaultMessage("set: index out of range");
  const out = [...input[0]];
  out[index] = input[2];
  return faOk(out);
}

function faRangeStep(input: [f0: bigint, f1: bigint, f2: bigint]): bigint[] {
  if (input[2] === 0n) throw new Error("range_step: step cannot be zero");
  if (faUseSharedNumericSequences && typeof SharedArrayBuffer === "function") {
    const count = input[2] > 0n
      ? input[0] >= input[1] ? 0 : Number(((input[1] - input[0] - 1n) / input[2]) + 1n)
      : input[0] <= input[1] ? 0 : Number(((input[0] - input[1] - 1n) / -input[2]) + 1n);
    const out = new BigInt64Array(new SharedArrayBuffer(count * BigInt64Array.BYTES_PER_ELEMENT));
    let index = 0;
    if (input[2] > 0n) {
      for (let value = input[0]; value < input[1]; value += input[2]) out[index++] = value;
    } else {
      for (let value = input[0]; value > input[1]; value += input[2]) out[index++] = value;
    }
    return out as unknown as bigint[];
  }
  const out: bigint[] = [];
  if (input[2] > 0n) {
    for (let value = input[0]; value < input[1]; value += input[2]) out.push(value);
  } else {
    for (let value = input[0]; value > input[1]; value += input[2]) out.push(value);
  }
  return out;
}

"#;

const TS_GPU_WASM_PRELUDE: &str = r#"
type FaGpuRuntimeModule = typeof faGpuRuntimeModule;

let faGpuRuntimePromise: Promise<FaGpuRuntimeModule> | null = null;

function faGpuRuntimeWasmModule(): URL | Uint8Array {
  const wasmUrl = new URL("./flowarrow_gpu_runtime_bg.wasm", import.meta.url);
  const processLike = (globalThis as {
    process?: {
      versions?: { node?: string };
      getBuiltinModule?: (name: string) => any;
    };
  }).process;
  if (processLike?.versions?.node && typeof processLike.getBuiltinModule === "function") {
    const fs = processLike.getBuiltinModule("node:fs");
    const url = processLike.getBuiltinModule("node:url");
    return fs.readFileSync(url.fileURLToPath(wasmUrl));
  }
  return wasmUrl;
}

function faGpuRuntime(): Promise<FaGpuRuntimeModule> {
  if (faGpuRuntimePromise !== null) return faGpuRuntimePromise;
  const runtime = faGpuRuntimeModule;
  faGpuRuntimePromise = runtime
    .default({ module_or_path: faGpuRuntimeWasmModule() })
    .then(() => runtime.fa_gpu_require_device())
    .then(() => runtime);
  return faGpuRuntimePromise;
}

function faGpuReduceOp(op: string): number {
  if (op === "add") return 0;
  if (op === "min") return 1;
  if (op === "max") return 2;
  throw new Error(`unsupported GPU reduce op: ${op}`);
}

function faGpuMapI32(input: number[], _kernelId: string, wgsl: string): Promise<number[]> {
  const packed = new Int32Array(input.map((value) => faAssertI32(value, "FlowArrow GPU i32")));
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_map_i32(wgsl, packed))
    .then((mapped) => Array.from(mapped));
}

function faGpuMapF32(input: number[], _kernelId: string, wgsl: string): Promise<number[]> {
  const packed = faGpuFloat32Input(input);
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_map_f32(wgsl, packed))
    .then((mapped) => mapped as unknown as number[]);
}

function faGpuMapF64(input: number[], _kernelId: string, wgsl: string): Promise<number[]> {
  const packed = faGpuFloat64Input(input);
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_map_f64(wgsl, packed))
    .then((mapped) => mapped as unknown as number[]);
}

function faGpuReduceI32(input: number[], op: string, identity: number): Promise<number> {
  const reduceOp = faGpuReduceOp(op);
  const packed = new Int32Array(input.map((value) => faAssertI32(value, "FlowArrow GPU i32")));
  const packedIdentity = faAssertI32(identity, "FlowArrow GPU i32 identity");
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_reduce_i32(reduceOp, packed, packedIdentity))
    .then((reduced) => Number(reduced));
}

function faGpuReduceF32(input: number[], op: string, identity: number): Promise<number> {
  const reduceOp = faGpuReduceOp(op);
  const packed = faGpuFloat32Input(input);
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_reduce_f32(reduceOp, packed, Math.fround(identity)));
}

function faGpuReduceF64(input: number[], op: string, identity: number): Promise<number> {
  const reduceOp = faGpuReduceOp(op);
  const packed = faGpuFloat64Input(input);
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_reduce_f64(reduceOp, packed, identity));
}

function faGpuFloat32Input(input: number[] | Float32Array): Float32Array {
  return input instanceof Float32Array ? input : new Float32Array(input.map(Math.fround));
}

function faGpuRepeatVectorAccumF32(
  wgsl: string,
  left: number[] | Float32Array,
  right: number[] | Float32Array,
  score: number,
  iterations: bigint,
): Promise<number> {
  const leftPacked = faGpuFloat32Input(left);
  const rightPacked = faGpuFloat32Input(right);
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_repeat_vector_accum_f32(
      wgsl,
      leftPacked,
      rightPacked,
      Math.fround(score),
      iterations,
    ));
}

function faGpuRepeatVectorAccumF64(
  wgsl: string,
  left: number[],
  right: number[],
  score: number,
  iterations: bigint,
): Promise<number> {
  const leftPacked = faGpuFloat64Input(left);
  const rightPacked = faGpuFloat64Input(right);
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_repeat_vector_accum_f64(
      wgsl,
      leftPacked,
      rightPacked,
      score,
      iterations,
    ));
}

function faGpuRepeatMatrixAccumF64(
  wgsl: string,
  left: number[][],
  right: number[][],
  vector: number[],
  score: number,
  iterations: bigint,
): Promise<number> {
  const leftFlat = faGpuFlattenMatrix(left, "left");
  const rightFlat = faGpuFlattenMatrix(right, "right");
  const vectorPacked = faGpuFloat64Input(vector);
  return faGpuRuntime()
    .then((runtime) => runtime.fa_gpu_repeat_matrix_accum_f64(
      wgsl,
      leftFlat.values,
      leftFlat.rows,
      leftFlat.cols,
      rightFlat.values,
      rightFlat.rows,
      rightFlat.cols,
      vectorPacked,
      score,
      iterations,
    ));
}

function faGpuFlattenMatrix(input: number[][], label: string): { values: Float64Array; rows: number; cols: number } {
  const rows = input.length;
  const cols = rows === 0 ? 0 : input[0].length;
  const values = new Float64Array(rows * cols);
  for (let row = 0; row < rows; row++) {
    if (input[row].length !== cols) {
      throw new Error(`FlowArrow GPU ${label} matrix must be rectangular`);
    }
    values.set(input[row], row * cols);
  }
  return { values, rows, cols };
}

function faGpuFloat64Input(input: number[] | Float64Array): Float64Array {
  return input instanceof Float64Array ? input : new Float64Array(input);
}

"#;

fn scalar_worker_module_source_from_mappers(mappers: &[WorkerMapper]) -> String {
    let mapper_entries = mappers
        .iter()
        .map(|mapper| format!("  [{}, {}]", ts_string(&mapper.id), mapper.source))
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        r#"let nodeParentPort = null;
try {{
  if (typeof process !== "undefined" && process.versions && process.versions.node) {{
    nodeParentPort = (await import("node:worker_threads")).parentPort;
  }}
}} catch {{
  nodeParentPort = null;
}}
const faScalarWorkerMappers = new Map([
{mapper_entries}
]);
const FA_I64_MIN = -(1n << 63n);
const FA_I64_MAX = (1n << 63n) - 1n;
const FA_I32_MIN = -(2 ** 31);
const FA_I32_MAX = (2 ** 31) - 1;
function faAssertI32(value, label) {{
  if (!Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX) throw new Error(`${{label}}: integer overflow`);
  return value | 0;
}}
function faAssertI64(value, label) {{
  if (value < FA_I64_MIN || value > FA_I64_MAX) throw new Error(`${{label}}: integer overflow`);
  return value;
}}
function faOk(value) {{ return {{ is_fault: false, value }}; }}
function faFaultMessage(message) {{ return {{ is_fault: true, fault: {{ message }} }}; }}
function faExpect(value) {{
  if (value && value.is_fault === true) throw new Error(value.fault.message);
  return value && value.is_fault === false ? value.value : value;
}}
function faCheckedI32Add(left, right) {{ return faAssertI32(left + right, "add"); }}
function faCheckedI32Sub(left, right) {{ return faAssertI32(left - right, "sub"); }}
function faCheckedI32Mul(left, right) {{ return faAssertI32(left * right, "mul"); }}
function faFaultableI32Add(left, right) {{
  const value = left + right;
  return !Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX ? faFaultMessage("add: integer overflow") : faOk(value | 0);
}}
function faFaultableI32Sub(left, right) {{
  const value = left - right;
  return !Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX ? faFaultMessage("sub: integer overflow") : faOk(value | 0);
}}
function faFaultableI32Mul(left, right) {{
  const value = left * right;
  return !Number.isInteger(value) || value < FA_I32_MIN || value > FA_I32_MAX ? faFaultMessage("mul: integer overflow") : faOk(value | 0);
}}
function faFaultableI32Div(left, right) {{
  if (right === 0) return faFaultMessage("div: division by zero");
  if (left === FA_I32_MIN && right === -1) return faFaultMessage("div: integer overflow");
  return faOk(Math.trunc(left / right));
}}
function faFaultableI32Rem(left, right) {{
  if (right === 0) return faFaultMessage("rem: remainder by zero");
  if (left === FA_I32_MIN && right === -1) return faFaultMessage("rem: integer overflow");
  return faOk(left % right);
}}
function faFaultableI32Neg(value) {{
  if (value === FA_I32_MIN) return faFaultMessage("neg: integer overflow");
  return faOk(-value);
}}
function faFaultableI32Abs(value) {{
  if (value === FA_I32_MIN) return faFaultMessage("abs: integer overflow");
  return faOk(value < 0 ? -value : value);
}}
function faCheckedI64Add(left, right) {{ return faAssertI64(left + right, "add"); }}
function faCheckedI64Sub(left, right) {{ return faAssertI64(left - right, "sub"); }}
function faCheckedI64Mul(left, right) {{ return faAssertI64(left * right, "mul"); }}
function faFaultableI64Add(left, right) {{
  const value = left + right;
  return value < FA_I64_MIN || value > FA_I64_MAX ? faFaultMessage("add: integer overflow") : faOk(value);
}}
function faFaultableI64Sub(left, right) {{
  const value = left - right;
  return value < FA_I64_MIN || value > FA_I64_MAX ? faFaultMessage("sub: integer overflow") : faOk(value);
}}
function faFaultableI64Mul(left, right) {{
  const value = left * right;
  return value < FA_I64_MIN || value > FA_I64_MAX ? faFaultMessage("mul: integer overflow") : faOk(value);
}}
function faFaultableI64Div(left, right) {{
  if (right === 0n) return faFaultMessage("div: division by zero");
  if (left === FA_I64_MIN && right === -1n) return faFaultMessage("div: integer overflow");
  return faOk(left / right);
}}
function faFaultableI64Rem(left, right) {{
  if (right === 0n) return faFaultMessage("rem: remainder by zero");
  if (left === FA_I64_MIN && right === -1n) return faFaultMessage("rem: integer overflow");
  return faOk(left % right);
}}
function faFaultableI64Neg(value) {{
  if (value === FA_I64_MIN) return faFaultMessage("neg: integer overflow");
  return faOk(-value);
}}
function faFaultableI64Abs(value) {{
  if (value === FA_I64_MIN) return faFaultMessage("abs: integer overflow");
  return faOk(value < 0n ? -value : value);
}}
function faCheckedI64Div(left, right) {{
  if (right === 0n) throw new Error("div: division by zero");
  if (left === FA_I64_MIN && right === -1n) throw new Error("div: integer overflow");
  return left / right;
}}
function faCheckedI64Rem(left, right) {{
  if (right === 0n) throw new Error("rem: remainder by zero");
  if (left === FA_I64_MIN && right === -1n) throw new Error("rem: integer overflow");
  return left % right;
}}
function faCheckedI64Neg(value) {{
  if (value === FA_I64_MIN) throw new Error("neg: integer overflow");
  return -value;
}}
function faCheckedI64Abs(value) {{
  if (value === FA_I64_MIN) throw new Error("abs: integer overflow");
  return value < 0n ? -value : value;
}}
function faCheckedI32Div(left, right) {{
  if (right === 0) throw new Error("div: division by zero");
  if (left === FA_I32_MIN && right === -1) throw new Error("div: integer overflow");
  return Math.trunc(left / right);
}}
function faCheckedI32Rem(left, right) {{
  if (right === 0) throw new Error("rem: remainder by zero");
  if (left === FA_I32_MIN && right === -1) throw new Error("rem: integer overflow");
  return left % right;
}}
function faCheckedI32Neg(value) {{
  if (value === FA_I32_MIN) throw new Error("neg: integer overflow");
  return -value;
}}
function faCheckedI32Abs(value) {{
  if (value === FA_I32_MIN) throw new Error("abs: integer overflow");
  return value < 0 ? -value : value;
}}
function faCheckedRealDiv(left, right) {{
  if (right === 0) throw new Error("div: division by zero");
  return left / right;
}}
function faCheckedF32Div(left, right) {{
  if (right === 0) throw new Error("div: division by zero");
  return Math.fround(left / right);
}}
function faCheckedRealRem(left, right) {{
  if (right === 0) throw new Error("rem: remainder by zero");
  return left % right;
}}
function faCheckedF32Rem(left, right) {{
  if (right === 0) throw new Error("rem: remainder by zero");
  return Math.fround(left % right);
}}
function faFaultableF32Div(left, right) {{
  if (right === 0) return faFaultMessage("div: division by zero");
  return faOk(Math.fround(left / right));
}}
function faFaultableF32Rem(left, right) {{
  if (right === 0) return faFaultMessage("rem: remainder by zero");
  return faOk(Math.fround(left % right));
}}
function faFaultableRealDiv(left, right) {{
  if (right === 0) return faFaultMessage("div: division by zero");
  return faOk(left / right);
}}
function faFaultableRealRem(left, right) {{
  if (right === 0) return faFaultMessage("rem: remainder by zero");
  return faOk(left % right);
}}
function faCheckedSqrt(value) {{
  if (value < 0) throw new Error("sqrt: negative input");
  return Math.sqrt(value);
}}
function faCheckedSqrtF32(value) {{
  if (value < 0) throw new Error("sqrt: negative input");
  return Math.fround(Math.sqrt(value));
}}
function faFaultableSqrt(value) {{
  if (value < 0) return faFaultMessage("sqrt: negative input");
  return faOk(Math.sqrt(value));
}}
function faFaultableSqrtF32(value) {{
  if (value < 0) return faFaultMessage("sqrt: negative input");
  return faOk(Math.fround(Math.sqrt(value)));
}}
let mapper = null;
const postDone = () => {{
  if (nodeParentPort) {{
    nodeParentPort.postMessage({{ done: true }});
  }} else {{
    self.postMessage({{ done: true }});
  }}
}};
const postReady = () => {{
  if (nodeParentPort) {{
    nodeParentPort.postMessage({{ type: "ready" }});
  }} else {{
    self.postMessage({{ type: "ready" }});
  }}
}};
const handleMessage = (message) => {{
  if (message.type === "init") {{
    mapper = faScalarWorkerMappers.get(message.mapperId) ?? null;
    if (mapper === null) throw new Error(`worker mapper not found: ${{message.mapperId}}`);
    postReady();
    return;
  }}
  if (mapper === null) throw new Error("worker mapper has not been initialized");
  const {{ inputBuffer, outputBuffer, start, end, inputKind, outputKind }} = message;
  const input = inputKind === "bigint"
    ? new BigInt64Array(inputBuffer)
    : inputKind === "i32"
      ? new Int32Array(inputBuffer)
      : inputKind === "f32"
        ? new Float32Array(inputBuffer)
        : inputKind === "number"
          ? new Float64Array(inputBuffer)
          : new Uint8Array(inputBuffer);
  const output = outputKind === "bigint"
    ? new BigInt64Array(outputBuffer)
    : outputKind === "i32"
      ? new Int32Array(outputBuffer)
      : outputKind === "f32"
        ? new Float32Array(outputBuffer)
        : outputKind === "number"
          ? new Float64Array(outputBuffer)
          : new Uint8Array(outputBuffer);
  for (let index = start; index < end; index++) {{
    const value = inputKind === "bool" ? input[index] !== 0 : input[index];
    const mapped = mapper(value);
    output[index] = outputKind === "bool" ? (mapped ? 1 : 0) : mapped;
  }}
  postDone();
}};
if (nodeParentPort) {{
  nodeParentPort.on("message", handleMessage);
}} else {{
  self.onmessage = (event) => handleMessage(event.data);
}}
"#
    )
}
