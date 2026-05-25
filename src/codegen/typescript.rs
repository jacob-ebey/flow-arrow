use super::{
    Ty, TypedCodegen, assignable_output_ty, binding_target_is_discard, builtin_output_type_plain,
    contains_empty_seq, format_binding_target_for_error, gpu, sequence_item_type,
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
    fn requires_async(&self) -> bool {
        self.worker_concurrency || self.gpu
    }
}

#[derive(Debug, Clone)]
struct TsValue {
    code: String,
    ty: Ty,
    tuple_items: Option<Vec<TsValue>>,
}

struct TypeScriptCodegen<'a> {
    codegen: TypedCodegen<'a>,
    options: TypeScriptEmitOptions,
    temp: usize,
    used_idents: HashSet<String>,
    worker_mappers: Vec<WorkerMapper>,
    seen_worker_mapper_sources: HashMap<String, String>,
    gpu_plan: Option<gpu::GpuPlan>,
}

struct WorkerMapBatchItem {
    source: TypedEndpoint,
    source_key: String,
    target: String,
    output_ty: Ty,
    worker_fn: &'static str,
    mapper_id: String,
}

pub(super) struct TypeScriptEmitOutput {
    pub source: String,
    pub worker_mappers: Vec<WorkerMapper>,
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
        }
    }

    fn emit(self) -> Result<String, String> {
        Ok(self.emit_artifacts()?.source)
    }

    fn emit_artifacts(mut self) -> Result<TypeScriptEmitOutput, String> {
        let mut out = String::new();
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
        if self.options.requires_async() {
            self.emit_runtime_lifecycle_exports(&mut out);
        }

        if has_program_main {
            if self.options.requires_async() {
                out.push_str(
                    "\nconst __flowarrow_process = (globalThis as any).process;\n\
const __flowarrow_main_url = __flowarrow_process?.argv?.[1]\n  ? new URL(__flowarrow_process.argv[1], \"file:\").href\n  : \"\";\n\
if (import.meta.url === __flowarrow_main_url) {\n  (async () => {\n    await __flowarrow_setup_runtime();\n    let __flowarrow_exit = 1n;\n    try {\n      const __flowarrow_result = await main({ argv: __flowarrow_process.argv.slice(2) });\n      __flowarrow_exit = faExitCode(__flowarrow_result);\n    } finally {\n      await __flowarrow_teardown_runtime();\n    }\n    __flowarrow_process.exit(Number(__flowarrow_exit));\n  })();\n}\n",
                );
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
        if self.options.requires_async() {
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

        let fused_reductions = self
            .gpu_plan
            .as_ref()
            .map(|plan| plan.range_map_reductions(callable))
            .unwrap_or_default();
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
        while chain_index < callable.chains.len() {
            if let Some(reduction) = fused_by_reduce.get(&chain_index) {
                self.emit_gpu_range_map_reduction(out, reduction, &mut env, "  ")?;
                chain_index += 1;
                continue;
            }
            if fused_skip.contains(&chain_index) {
                chain_index += 1;
                continue;
            }
            let batch_len = self.worker_map_batch_len(&callable.chains[chain_index..], &env)?;
            let batch_crosses_fused_chain =
                (chain_index..chain_index + batch_len).any(|index| fused_skip.contains(&index));
            if batch_len > 1 && !batch_crosses_fused_chain {
                self.emit_worker_map_batch(
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

        let result = self.emit_outputs(callable, &env)?;
        let result = self.coerce_value(out, result, &signature.output, "  ")?;
        out.push_str(&format!("  return {};\n}}\n", result.code));
        Ok(())
    }

    fn emit_gpu_range_map_reduction(
        &mut self,
        out: &mut String,
        reduction: &gpu::GpuRangeMapReduction,
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let range_ty = Ty::Tuple(vec![Ty::Int, Ty::Int, Ty::Int]);
        let range = self.emit_endpoint_expected(
            out,
            &reduction.range_source,
            env,
            Some(&range_ty),
            indent,
        )?;
        let identity = self.emit_endpoint(out, &reduction.identity, env, indent)?;
        let tmp = self.next_temp_or_preferred(Some(&reduction.output_name));
        match reduction.map_kernel.scalar {
            gpu::GpuScalarKind::I32 => {
                if reduction.output_ty != Ty::Int {
                    return Err(format!(
                        "GPU range reduction expected Int output, found `{}`",
                        reduction.output_ty
                    ));
                }
                out.push_str(&format!(
                    "{indent}const {tmp}: bigint = await faGpuRangeMapReduceI32([{}], {}, {}, {}, {});\n",
                    call_args(&range, 3)?,
                    ts_string(&reduction.map_kernel.id),
                    ts_string(&reduction.map_kernel.map_expr),
                    ts_string(&reduction.op),
                    identity.code
                ));
            }
            gpu::GpuScalarKind::F32 => {
                return Err(
                    "GPU range reductions currently require Int range map kernels".to_string(),
                );
            }
        }
        if env
            .insert(
                reduction.output_name.clone(),
                ts_value(tmp, reduction.output_ty.clone()),
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
                        arms,
                        stage.output.clone(),
                        value,
                        env,
                        indent,
                        preferred,
                    )?;
                }
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
            TypedStageKind::Match { arms } => {
                self.emit_match(out, arms, stage.output.clone(), value, env, indent, None)
            }
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
            if self.options.requires_async() {
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
            "slice" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int, Ty::Int])) =>
            {
                format!(
                    "{}.slice(Number({}), Number({}))",
                    tuple_field(input, 0),
                    tuple_field(input, 1),
                    tuple_field(input, 2)
                )
            }
            "take" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) =>
            {
                format!(
                    "{}.slice(0, Number({}))",
                    tuple_field(input, 0),
                    tuple_field(input, 1)
                )
            }
            "drop" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) =>
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
            "format_int" | "format_real" => format!("{}.toString()", input.code),
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                ts_numeric_binary_expr(name, input, output_ty)
            }
            "neg" => format!("(-{})", input.code),
            "abs" => format!("({0} < 0 ? -{0} : {0})", input.code),
            "sqrt" => format!("Math.sqrt({})", input.code),
            "exp" => format!("Math.exp({})", input.code),
            "sin" => format!("Math.sin({})", input.code),
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
            out.push_str(&format!(
                "{indent}{tmp} = await {}({}, {}, {});\n",
                kernel.scalar.map_function(),
                source,
                ts_string(&kernel.id),
                ts_string(&kernel.wgsl)
            ));
            return Ok(ts_value(tmp, output_ty));
        }
        if !is_faultable
            && let Some((worker_fn, mapper_id)) =
                self.worker_map_call(name, item_ty.as_ref(), &output_item_ty)?
        {
            out.push_str(&format!(
                "{indent}{tmp} = await {worker_fn}({}, {});\n",
                source,
                ts_string(&mapper_id)
            ));
            return Ok(ts_value(tmp, output_ty));
        }
        let seq_tmp = self.next_temp();
        let item = self.next_temp();
        let body_indent = if is_faultable {
            format!("{indent}  ")
        } else {
            indent.to_string()
        };
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
        input: TsValue,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`filter {name}` expected Seq input"));
        };
        let tmp = self.next_temp_or_preferred(preferred);
        let item = self.next_temp();
        out.push_str(&format!(
            "{indent}const {tmp}: {} = [];\n",
            ts_type(&input.ty)
        ));
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
        let output_ty = if input_faultable || item_faultable {
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
        if self.options.gpu && !input_faultable && !item_faultable {
            let canonical = self.codegen.canonical_name(op);
            if matches!(canonical.as_str(), "add" | "min" | "max") {
                match plain_item_ty {
                    Ty::Int => {
                        out.push_str(&format!(
                            "{body_indent}{tmp} = await faGpuReduceI32({}, {}, {});\n",
                            source,
                            ts_string(&canonical),
                            identity.code
                        ));
                        return Ok(ts_value(tmp, output_ty));
                    }
                    Ty::Real => {
                        out.push_str(&format!(
                            "{body_indent}{tmp} = await faGpuReduceF32({}, {}, {});\n",
                            source,
                            ts_string(&canonical),
                            identity.code
                        ));
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
        if item_faultable {
            let fault = self.next_temp();
            out.push_str(&format!(
                "{body_indent}let {fault}: FaFault | null = null;\n"
            ));
            out.push_str(&format!("{body_indent}for (const {item} of {source}) {{\n"));
            out.push_str(&format!(
                "{body_indent}  if ({item}.is_fault === true) {{ {fault} = {item}.fault; break; }}\n"
            ));
            let pair_ty = Ty::Tuple(vec![plain_item_ty.clone(), plain_item_ty.clone()]);
            let pair = ts_tuple_value(
                vec![
                    ts_value(acc.clone(), plain_item_ty.clone()),
                    ts_value(format!("{item}.value"), plain_item_ty.clone()),
                ],
                pair_ty,
            );
            let reduced = self.emit_call(out, op, pair, &(body_indent.clone() + "  "))?;
            out.push_str(&format!("{body_indent}  {acc} = {};\n", reduced.code));
            out.push_str(&format!("{body_indent}}}\n"));
            out.push_str(&format!(
                "{body_indent}{tmp} = {fault} ? faFault({fault}) : faOk({acc});\n"
            ));
        } else {
            out.push_str(&format!("{body_indent}for (const {item} of {source}) {{\n"));
            let pair_ty = Ty::Tuple(vec![plain_item_ty.clone(), plain_item_ty.clone()]);
            let pair = ts_tuple_value(
                vec![
                    ts_value(acc.clone(), plain_item_ty.clone()),
                    ts_value(item.clone(), plain_item_ty.clone()),
                ],
                pair_ty,
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
        let tmp = self.next_temp_or_preferred(preferred);
        let acc = self.next_temp();
        let item = self.next_temp();
        let output_ty = input.ty.clone();
        out.push_str(&format!(
            "{indent}const {tmp}: {} = [];\n",
            ts_type(&output_ty)
        ));
        out.push_str(&format!(
            "{indent}let {acc}: {} = {};\n",
            ts_type(&item_ty),
            identity.code
        ));
        out.push_str(&format!(
            "{indent}for (const {item} of {}) {{\n",
            input.code
        ));
        let pair_ty = Ty::Tuple(vec![item_ty.as_ref().clone(), item_ty.as_ref().clone()]);
        let pair = ts_tuple_value(
            vec![
                ts_value(acc.clone(), item_ty.as_ref().clone()),
                ts_value(item.clone(), item_ty.as_ref().clone()),
            ],
            pair_ty,
        );
        let scanned = self.emit_call(out, op, pair, &(indent.to_string() + "  "))?;
        out.push_str(&format!("{indent}  {acc} = {};\n", scanned.code));
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
        arms: &[TypedMatchArm],
        output_ty: Ty,
        subject: TsValue,
        env: &HashMap<String, TsValue>,
        indent: &str,
        preferred: Option<&str>,
    ) -> Result<TsValue, String> {
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
            (Ty::Int, Ty::Int) => "faParallelMapBigInt",
            (Ty::Real, Ty::Real) => "faParallelMapNumber",
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
        let expr = match name {
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                ts_numeric_binary_expr(name, input, output_ty)
            }
            "neg" => format!("(-{})", input.code),
            "abs" => format!("({0} < 0 ? -{0} : {0})", input.code),
            "sqrt" => format!("Math.sqrt({})", input.code),
            "exp" => format!("Math.exp({})", input.code),
            "sin" => format!("Math.sin({})", input.code),
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
        Ty::Int => "bigint".to_string(),
        Ty::Real | Ty::OneOf(_) => "number".to_string(),
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
        Ty::Int => format!("BigInt({raw})"),
        Ty::Real => format!("Number({raw})"),
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
        "add" => format!("({left} + {right})"),
        "sub" => format!("({left} - {right})"),
        "mul" => format!("({left} * {right})"),
        "div" => format!("({left} / {right})"),
        "rem" => format!("({left} % {right})"),
        "min" => format!("({left} <= {right} ? {left} : {right})"),
        "max" => format!("({left} >= {right} ? {left} : {right})"),
        _ if matches!(output_ty, Ty::Real) => "Number.NaN".to_string(),
        _ => "0n".to_string(),
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
  return faOk(BigInt(text));
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

type FaScalarMapKind = "bigint" | "number" | "bool";
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

function faParallelMapBigInt(input: Array<bigint>, mapperId: string, workerCount?: number): Promise<Array<bigint>> {
  return faParallelMapScalar(input, mapperId, "bigint", "bigint", workerCount) as Promise<Array<bigint>>;
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
  return kind === "bigint" ? BigInt64Array.BYTES_PER_ELEMENT : kind === "number" ? Float64Array.BYTES_PER_ELEMENT : Uint8Array.BYTES_PER_ELEMENT;
}

function faWriteScalarBuffer<T>(buffer: SharedArrayBuffer, input: Array<T>, kind: FaScalarMapKind): void {
  if (kind === "bigint") {
    new BigInt64Array(buffer).set(input as Array<bigint>);
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
type FaGpuRuntimeModule = {
  default: (moduleOrPath?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module) => Promise<any>;
  fa_gpu_require_device: () => Promise<void>;
  fa_gpu_map_i32: (wgsl: string, input: Int32Array) => Promise<Int32Array>;
  fa_gpu_map_f64: (wgsl: string, input: Float64Array) => Promise<Float64Array>;
  fa_gpu_reduce_i32: (op: number, input: Int32Array, identity: number) => Promise<number>;
  fa_gpu_reduce_f64: (op: number, input: Float64Array, identity: number) => Promise<number>;
  fa_gpu_range_map_reduce_i32: (
    mapExpr: string,
    start: number,
    stop: number,
    step: number,
    op: number,
    identity: number,
  ) => Promise<number>;
};

let faGpuRuntimePromise: Promise<FaGpuRuntimeModule> | null = null;

async function faGpuRuntime(): Promise<FaGpuRuntimeModule> {
  if (faGpuRuntimePromise !== null) return faGpuRuntimePromise;
  faGpuRuntimePromise = (async () => {
    const runtime = await import("./flowarrow_gpu_runtime.mjs") as FaGpuRuntimeModule;
    await runtime.default(new URL("./flowarrow_gpu_runtime_bg.wasm", import.meta.url));
    await runtime.fa_gpu_require_device();
    return runtime;
  })();
  return faGpuRuntimePromise;
}

function faGpuAssertI32(value: bigint): number {
  if (value < -2147483648n || value > 2147483647n) {
    throw new Error("FlowArrow GPU Int currently requires signed 32-bit values");
  }
  return Number(value);
}

function faGpuReduceOp(op: string): number {
  if (op === "add") return 0;
  if (op === "min") return 1;
  if (op === "max") return 2;
  throw new Error(`unsupported GPU reduce op: ${op}`);
}

async function faGpuMapI32(input: bigint[], _kernelId: string, wgsl: string): Promise<bigint[]> {
  const runtime = await faGpuRuntime();
  const mapped = await runtime.fa_gpu_map_i32(wgsl, new Int32Array(input.map(faGpuAssertI32)));
  return Array.from(mapped, (value) => BigInt(value));
}

async function faGpuMapF32(input: number[], _kernelId: string, wgsl: string): Promise<number[]> {
  const runtime = await faGpuRuntime();
  const mapped = await runtime.fa_gpu_map_f64(wgsl, new Float64Array(input));
  return Array.from(mapped);
}

async function faGpuReduceI32(input: bigint[], op: string, identity: bigint): Promise<bigint> {
  const runtime = await faGpuRuntime();
  const reduced = await runtime.fa_gpu_reduce_i32(
    faGpuReduceOp(op),
    new Int32Array(input.map(faGpuAssertI32)),
    faGpuAssertI32(identity),
  );
  return BigInt(reduced);
}

async function faGpuReduceF32(input: number[], op: string, identity: number): Promise<number> {
  const runtime = await faGpuRuntime();
  return await runtime.fa_gpu_reduce_f64(faGpuReduceOp(op), new Float64Array(input), identity);
}

async function faGpuRangeMapReduceI32(
  range: [f0: bigint, f1: bigint, f2: bigint],
  _kernelId: string,
  mapExpr: string,
  op: string,
  identity: bigint,
): Promise<bigint> {
  const runtime = await faGpuRuntime();
  const reduced = await runtime.fa_gpu_range_map_reduce_i32(
    mapExpr,
    faGpuAssertI32(range[0]),
    faGpuAssertI32(range[1]),
    faGpuAssertI32(range[2]),
    faGpuReduceOp(op),
    faGpuAssertI32(identity),
  );
  return BigInt(reduced);
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
    : inputKind === "number"
      ? new Float64Array(inputBuffer)
      : new Uint8Array(inputBuffer);
  const output = outputKind === "bigint"
    ? new BigInt64Array(outputBuffer)
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
