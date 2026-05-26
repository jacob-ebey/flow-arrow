use super::*;

use inkwell::AddressSpace;
use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::intrinsics::Intrinsic;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::{Linkage, Module as LlvmModule};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
};
use inkwell::types::{AnyType, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use std::collections::HashMap;

#[path = "llvm_seq.rs"]
mod llvm_seq;
#[path = "llvm_stdlib.rs"]
mod llvm_stdlib;

#[derive(Clone)]
#[cfg(not(target_arch = "wasm32"))]
struct LlvmValue<'ctx> {
    value: BasicValueEnum<'ctx>,
    ty: Ty,
}

#[cfg(not(target_arch = "wasm32"))]
struct ExtractedSeq<'ctx> {
    count: IntValue<'ctx>,
    items: PointerValue<'ctx>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) struct DirectLlvm<'ctx, 'a> {
    context: &'ctx Context,
    module: LlvmModule<'ctx>,
    builder: Builder<'ctx>,
    codegen: TypedCodegen<'a>,
    gpu_plan: gpu::GpuPlan,
    types: LlvmTypeRegistry<'ctx>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    options: DirectLlvmOptions,
    exports: Vec<String>,
    stream_helper: usize,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) struct DirectLlvmOptions {
    pub(super) target_triple: Option<String>,
    pub(super) emit_entrypoint: bool,
    pub(super) export_abi: Option<DirectExportAbi>,
    pub(super) emit_object: bool,
    pub(super) optimization: OptimizationLevel,
    pub(super) gpu: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(not(target_arch = "wasm32"))]
pub(super) enum DirectExportAbi {
    Wasm,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for DirectLlvmOptions {
    fn default() -> Self {
        Self {
            target_triple: None,
            emit_entrypoint: true,
            export_abi: None,
            emit_object: false,
            optimization: OptimizationLevel::Aggressive,
            gpu: false,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) struct DirectLlvmEmission {
    pub(super) llvm: String,
    pub(super) object: Option<Vec<u8>>,
    pub(super) symbol_exports: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<'a> DirectLlvm<'_, 'a> {
    #[cfg(test)]
    pub(super) fn emit(codegen: TypedCodegen<'a>) -> Result<String, String> {
        Ok(Self::emit_with_options(codegen, DirectLlvmOptions::default())?.llvm)
    }

    pub(super) fn emit_with_options(
        codegen: TypedCodegen<'a>,
        options: DirectLlvmOptions,
    ) -> Result<DirectLlvmEmission, String> {
        let context = Context::create();
        let module = context.create_module("flowarrow");
        if let Some(target_triple) = &options.target_triple {
            module.set_triple(&TargetTriple::create(target_triple));
        }
        let builder = context.create_builder();
        let types = LlvmTypeRegistry::new(&context);
        let gpu_plan = if options.gpu {
            gpu::GpuPlan::analyze(codegen.typed)
        } else {
            gpu::GpuPlan::empty()
        };
        let mut direct = DirectLlvm {
            context: &context,
            module,
            builder,
            codegen,
            gpu_plan,
            types,
            functions: HashMap::new(),
            options,
            exports: Vec::new(),
            stream_helper: 0,
        };
        direct.declare_callables()?;
        direct.emit_callables()?;
        if direct.options.export_abi == Some(DirectExportAbi::Wasm) {
            direct.emit_wasm_exports()?;
        }
        if direct.options.emit_entrypoint {
            direct.emit_entrypoint()?;
        }
        let object = if direct.options.emit_object {
            let target_triple = direct
                .options
                .target_triple
                .as_deref()
                .ok_or_else(|| "object emission requires a target triple".to_string())?;
            Some(emit_target_object(
                &direct.module,
                target_triple,
                direct.options.optimization,
            )?)
        } else {
            None
        };
        Ok(DirectLlvmEmission {
            llvm: direct.module.print_to_string().to_string(),
            object,
            symbol_exports: direct.exports,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn emit_target_object(
    module: &LlvmModule<'_>,
    target_triple: &str,
    optimization: OptimizationLevel,
) -> Result<Vec<u8>, String> {
    Target::initialize_webassembly(&InitializationConfig::default());
    let triple = TargetTriple::create(target_triple);
    let target = Target::from_triple(&triple)
        .map_err(|error| format!("failed to initialize target `{target_triple}`: {error}"))?;
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            optimization,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| format!("failed to create target machine for `{target_triple}`"))?;
    let object = machine
        .write_to_memory_buffer(module, FileType::Object)
        .map_err(|error| format!("failed to emit object for `{target_triple}`: {error}"))?;
    Ok(memory_buffer_without_nul(&object))
}

#[cfg(not(target_arch = "wasm32"))]
fn memory_buffer_without_nul(buffer: &MemoryBuffer<'_>) -> Vec<u8> {
    let bytes = buffer.as_slice();
    bytes[..bytes.len().saturating_sub(1)].to_vec()
}

fn gpu_reduce_op(op: &str) -> u32 {
    match op {
        "add" => 0,
        "min" => 1,
        "max" => 2,
        _ => unreachable!("unsupported GPU reduce op"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
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
        let foreign_names = self.codegen.foreign_c.keys().cloned().collect::<Vec<_>>();
        for name in foreign_names {
            let sig = self
                .codegen
                .signatures
                .get(&name)
                .ok_or_else(|| format!("missing signature for `{name}`"))?
                .clone();
            let output_ty = self.types.basic_type(&sig.output)?;
            let input_ty = self.types.basic_type(&sig.input)?;
            let function_ty = output_ty.fn_type(&[input_ty.into()], false);
            let symbol = self
                .codegen
                .foreign_c
                .get(&name)
                .ok_or_else(|| format!("missing foreign c binding for `{name}`"))?
                .symbol
                .clone();
            let function = self.module.add_function(&symbol, function_ty, None);
            self.functions.insert(name, function);
        }
        Ok(())
    }

    fn emit_callables(&mut self) -> Result<(), String> {
        let mut callables = self.codegen.typed.callables.iter().collect::<Vec<_>>();
        callables.sort_by(|left, right| left.name.cmp(&right.name));
        for callable in callables {
            self.emit_callable(callable)?;
        }
        Ok(())
    }

    fn emit_wasm_exports(&mut self) -> Result<(), String> {
        for name in self.exported_node_names() {
            let sig = self
                .codegen
                .signatures
                .get(&name)
                .ok_or_else(|| format!("missing signature for WASM export `{name}`"))?;
            if wasm_exportable_input(&sig.input) && wasm_exportable_output(&sig.output) {
                self.emit_c_abi_export(&name, DirectExportAbi::Wasm)?;
            }
        }
        Ok(())
    }

    fn exported_node_names(&self) -> Vec<String> {
        self.codegen
            .typed
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

    fn emit_c_abi_export(&mut self, name: &str, abi: DirectExportAbi) -> Result<(), String> {
        let export_name = sanitize_symbol(name);
        if export_name != name {
            let label = export_abi_label(abi);
            return Err(format!(
                "{label} export `{name}` cannot be represented as a stable symbol yet"
            ));
        }
        let sig = self
            .codegen
            .signatures
            .get(name)
            .ok_or_else(|| {
                format!(
                    "missing signature for {} export `{name}`",
                    export_abi_label(abi)
                )
            })?
            .clone();
        let output_ty = self.export_output_type(&sig.output, name, abi)?;
        let params = self.wasm_export_input_types(&sig.input, name)?;
        let param_types = params
            .iter()
            .map(|(_, ty)| (*ty).into())
            .collect::<Vec<_>>();
        let wrapper_ty = output_ty.fn_type(&param_types, false);
        let wrapper = self.module.add_function(&export_name, wrapper_ty, None);
        wrapper.set_linkage(Linkage::External);

        let block = self.context.append_basic_block(wrapper, "entry");
        self.builder.position_at_end(block);
        let input = self.build_scalar_export_input(wrapper, &sig.input, &params, name, abi)?;
        let internal = *self.functions.get(name).ok_or_else(|| {
            format!(
                "missing internal function for {} export `{name}`",
                export_abi_label(abi)
            )
        })?;
        let result = self
            .builder
            .build_call(internal, &[input.into()], "result")
            .map_err(|error| {
                format!(
                    "LLVM backend failed to call {} export `{name}`: {error}",
                    export_abi_label(abi)
                )
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| {
                format!(
                    "{} export `{name}` did not return a value",
                    export_abi_label(abi)
                )
            })?;
        self.builder.build_return(Some(&result)).map_err(|error| {
            format!(
                "LLVM backend failed to return {} export `{name}`: {error}",
                export_abi_label(abi)
            )
        })?;
        self.exports.push(export_name);
        Ok(())
    }

    fn export_input_types(
        &mut self,
        input_ty: &Ty,
        export_name: &str,
        abi: DirectExportAbi,
    ) -> Result<Vec<(Ty, BasicTypeEnum<'ctx>)>, String> {
        match input_ty {
            Ty::Unit => Ok(Vec::new()),
            Ty::Tuple(items) => items
                .iter()
                .map(|item| {
                    Ok((
                        item.clone(),
                        self.export_output_type(item, export_name, abi)?,
                    ))
                })
                .collect(),
            other => Ok(vec![(
                other.clone(),
                self.export_output_type(other, export_name, abi)?,
            )]),
        }
    }

    fn wasm_export_input_types(
        &mut self,
        input_ty: &Ty,
        export_name: &str,
    ) -> Result<Vec<(Ty, BasicTypeEnum<'ctx>)>, String> {
        self.export_input_types(input_ty, export_name, DirectExportAbi::Wasm)
    }

    fn export_output_type(
        &mut self,
        ty: &Ty,
        export_name: &str,
        abi: DirectExportAbi,
    ) -> Result<BasicTypeEnum<'ctx>, String> {
        match abi {
            DirectExportAbi::Wasm => match ty {
                Ty::I32 => Ok(self.context.i32_type().into()),
                Ty::I64 => Ok(self.context.i64_type().into()),
                Ty::F32 => Ok(self.context.f32_type().into()),
                Ty::F64 => Ok(self.context.f64_type().into()),
                other => Err(format!(
                    "WASM export `{export_name}` uses `{other}`; only i32, i64, f32, and f64 scalar inputs and outputs are supported"
                )),
            },
        }
    }

    fn build_scalar_export_input(
        &mut self,
        wrapper: FunctionValue<'ctx>,
        input_ty: &Ty,
        params: &[(Ty, BasicTypeEnum<'ctx>)],
        export_name: &str,
        abi: DirectExportAbi,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match input_ty {
            Ty::Unit => Ok(self.types.basic_type(&Ty::Unit)?.const_zero()),
            Ty::Tuple(items) => {
                let mut tuple = self
                    .types
                    .basic_type(input_ty)?
                    .into_struct_type()
                    .const_zero();
                for (index, item) in items.iter().enumerate() {
                    let param = wrapper.get_nth_param(index as u32).ok_or_else(|| {
                        format!(
                            "missing parameter {index} for {} export `{export_name}`",
                            export_abi_label(abi)
                        )
                    })?;
                    tuple = self
                        .builder
                        .build_insert_value(tuple, param, index as u32, "arg")
                        .map_err(|error| {
                            format!(
                                "LLVM backend failed to build tuple input for {} export `{export_name}`: {error}",
                                export_abi_label(abi)
                            )
                        })?
                        .into_struct_value();
                    if params.get(index).map(|(ty, _)| ty) != Some(item) {
                        return Err(format!(
                            "internal parameter mismatch for {} export `{export_name}`",
                            export_abi_label(abi)
                        ));
                    }
                }
                Ok(tuple.into())
            }
            _ => wrapper.get_nth_param(0).ok_or_else(|| {
                format!(
                    "missing parameter for {} export `{export_name}`",
                    export_abi_label(abi)
                )
            }),
        }
    }

    fn emit_callable(&mut self, callable: &TypedCallable) -> Result<(), String> {
        self.codegen.validate_gpu_host_callable(callable)?;
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
        callable: &TypedCallable,
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
        chain: &TypedChain,
        env: &mut HashMap<String, LlvmValue<'ctx>>,
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
            self.emit_endpoint_expected(&chain.source, env, source_expected.as_ref())?;
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match &stage.kind {
                TypedStageKind::Bind { target } if is_last => {
                    self.bind_target(target, value.clone(), env)?
                }
                TypedStageKind::Call { name, .. } => {
                    value = self.emit_call(name, value)?;
                }
                TypedStageKind::Map { name, .. } => {
                    value = self.emit_map(name, value)?;
                }
                TypedStageKind::Filter { name, .. } => {
                    value = self.emit_filter(name, value)?;
                }
                TypedStageKind::Field { name } => {
                    value = self.extract_struct_field(value, name)?;
                }
                TypedStageKind::Reduce { op, identity, .. } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_reduce(op, value, identity)?;
                }
                TypedStageKind::Scan { op, identity, .. } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_scan(op, value, identity)?;
                }
                TypedStageKind::Repeat { count, node, .. } => {
                    let count = self.emit_endpoint(count, env)?;
                    value = self.emit_repeat(node, value, count)?;
                }
                TypedStageKind::Match { arms } => {
                    value = self.emit_match(arms, stage.output.clone(), value, env)?;
                }
                TypedStageKind::FaultMap {
                    node, ok, fault, ..
                } => {
                    let (ok_value, fault_value) = self.emit_fault_map(node, value.clone())?;
                    if env.insert(ok.clone(), ok_value).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_value).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                TypedStageKind::Bind { .. } => {
                    return Err("binding targets may only appear as final stages".to_string());
                }
            }
        }
        Ok(())
    }

    fn emit_endpoint(
        &mut self,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
        self.emit_endpoint_expected(endpoint, env, None)
    }

    fn emit_endpoint_expected(
        &mut self,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, LlvmValue<'ctx>>,
        expected: Option<&Ty>,
    ) -> Result<LlvmValue<'ctx>, String> {
        match &endpoint.kind {
            TypedEndpointKind::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            TypedEndpointKind::NodeRef { name, .. } => {
                Err(format!("expected value, found node `{name}`"))
            }
            TypedEndpointKind::Int(value) => Ok(LlvmValue {
                value: self
                    .context
                    .i64_type()
                    .const_int(*value as u64, true)
                    .into(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Real(value) => Ok(LlvmValue {
                value: self.context.f64_type().const_float(*value).into(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Bool(value) => Ok(LlvmValue {
                value: self
                    .context
                    .i8_type()
                    .const_int(if *value { 1 } else { 0 }, false)
                    .into(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Unit => Ok(LlvmValue {
                value: self.types.basic_type(&Ty::Unit)?.const_zero(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Tuple(items) => {
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
                let ty = expected.cloned().unwrap_or_else(|| endpoint.ty.clone());
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
            TypedEndpointKind::String(_) | TypedEndpointKind::Seq(_) => {
                self.emit_literal_endpoint_expected(endpoint, env, expected)
            }
            TypedEndpointKind::Struct { name, fields, .. } => {
                self.emit_struct_endpoint(name, fields, env, expected)
            }
            TypedEndpointKind::Eval { source, stages } => {
                self.emit_inline_eval(source, stages, env)
            }
        }
    }

    fn emit_struct_endpoint(
        &mut self,
        name: &str,
        fields: &[(String, TypedEndpoint)],
        env: &HashMap<String, LlvmValue<'ctx>>,
        expected: Option<&Ty>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let ty = expected
            .cloned()
            .or_else(|| self.codegen.aliases.get(name).cloned())
            .ok_or_else(|| format!("unknown struct `{name}`"))?;
        let Ty::Struct {
            fields: expected_fields,
            ..
        } = &ty
        else {
            return Err(format!("struct literal expected struct type, found `{ty}`"));
        };
        let mut out = self.types.basic_type(&ty)?.into_struct_type().const_zero();
        for (index, (field, field_ty)) in expected_fields.iter().enumerate() {
            let (_, endpoint) = fields
                .iter()
                .find(|(candidate, _)| candidate == field)
                .ok_or_else(|| format!("struct `{name}` literal missing field `{field}`"))?;
            let value = self.emit_endpoint_expected(endpoint, env, Some(field_ty))?;
            let value = self.coerce_value_to_ty(value, field_ty)?;
            out = self
                .builder
                .build_insert_value(out, value.value, index as u32, "struct")
                .map_err(|error| {
                    format!("LLVM backend failed to assemble struct literal: {error}")
                })?
                .into_struct_value();
        }
        Ok(LlvmValue {
            value: out.into(),
            ty,
        })
    }

    fn emit_inline_eval(
        &mut self,
        source: &TypedEndpoint,
        stages: &[TypedStage],
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
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
        let mut value = self.emit_endpoint_expected(source, env, source_expected.as_ref())?;
        for stage in stages {
            match &stage.kind {
                TypedStageKind::Call { name, .. } => {
                    value = self.emit_call(name, value)?;
                }
                TypedStageKind::Bind { .. } => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                TypedStageKind::Map { name, .. } => value = self.emit_map(name, value)?,
                TypedStageKind::FaultMap { .. } => {
                    return Err("inline evaluations cannot use `fault map`".to_string());
                }
                TypedStageKind::Filter { name, .. } => value = self.emit_filter(name, value)?,
                TypedStageKind::Field { name } => value = self.extract_struct_field(value, name)?,
                TypedStageKind::Reduce { op, identity, .. } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_reduce(op, value, identity)?;
                }
                TypedStageKind::Scan { op, identity, .. } => {
                    let identity = self.emit_endpoint(identity, env)?;
                    value = self.emit_scan(op, value, identity)?;
                }
                TypedStageKind::Repeat { count, node, .. } => {
                    let count = self.emit_endpoint(count, env)?;
                    value = self.emit_repeat(node, value, count)?;
                }
                TypedStageKind::Match { arms } => {
                    value = self.emit_match(arms, stage.output.clone(), value, env)?;
                }
            }
        }
        Ok(value)
    }

    fn emit_literal_endpoint_expected(
        &mut self,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, LlvmValue<'ctx>>,
        expected: Option<&Ty>,
    ) -> Result<LlvmValue<'ctx>, String> {
        match &endpoint.kind {
            TypedEndpointKind::String(value) => {
                let global =
                    self.builder
                        .build_global_string_ptr(value, "str")
                        .map_err(|error| {
                            format!("LLVM backend failed to build string literal: {error}")
                        })?;
                let pair_ty = self.runtime_pair_type();
                let fn_value = self.runtime_function(
                    "fa_bytes_borrowed",
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
                        format!("LLVM backend failed to call fa_bytes_borrowed: {error}")
                    })?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "fa_bytes_borrowed did not return a value".to_string())?;
                let call = self.runtime_pair_to_value(call, &Ty::Bytes)?;
                Ok(LlvmValue {
                    value: call,
                    ty: Ty::Bytes,
                })
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

    fn emit_match(
        &mut self,
        arms: &[TypedMatchArm],
        output_ty: Ty,
        subject: LlvmValue<'ctx>,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
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
                TypedMatchGuard::Fallback => {
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
                TypedMatchGuard::Call { node, args, .. } => {
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

    fn emit_match_target(
        &mut self,
        target: &TypedMatchTarget,
        subject: LlvmValue<'ctx>,
        env: &HashMap<String, LlvmValue<'ctx>>,
    ) -> Result<LlvmValue<'ctx>, String> {
        match target {
            TypedMatchTarget::Node { name, .. } => self.emit_call(name, subject),
            TypedMatchTarget::Value(endpoint) => self.emit_endpoint(endpoint, env),
        }
    }

    fn emit_match_guard_input(
        &mut self,
        subject: LlvmValue<'ctx>,
        args: &[TypedEndpoint],
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
                "fa_calloc",
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
                .ok_or_else(|| "fa_calloc did not return a value".to_string())?
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

        if self.options.gpu
            && let Some(kernel) =
                self.gpu_plan
                    .kernel_for_map(name, item_ty.as_ref(), &output_item_ty)
        {
            let kernel_id = kernel.id.clone();
            let wgsl_source = kernel.wgsl.clone();
            let scalar = kernel.scalar;
            let wgsl = self
                .builder
                .build_global_string_ptr(&wgsl_source, &format!("{kernel_id}_wgsl"))
                .map_err(|error| format!("LLVM backend failed to build GPU WGSL string: {error}"))?
                .as_pointer_value();
            let input_items = self.seq_items(input.value)?;
            let output_items = self.seq_items(output.value)?;
            let map_function = match scalar {
                gpu::GpuScalarKind::I32 => self.gpu_map_i32_function(),
                gpu::GpuScalarKind::F32 => self.gpu_map_f32_function(),
                gpu::GpuScalarKind::F64 => self.gpu_map_f64_function(),
            };
            self.builder
                .build_call(
                    map_function,
                    &[
                        wgsl.into(),
                        input_items.into(),
                        output_items.into(),
                        count.into(),
                    ],
                    "gpu.map",
                )
                .map_err(|error| format!("LLVM backend failed to call native GPU map: {error}"))?;
            return Ok(output);
        }

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
        if let Some(plan) = self.codegen.gpu_repeat_accumulator(node, &input.ty) {
            return self.emit_gpu_repeat_accumulator(plan, input, count);
        }
        let Ty::I64 = count.ty else {
            return Err(format!(
                "`repeat {node}` count expected i64, found `{}`",
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

    fn emit_gpu_repeat_accumulator(
        &mut self,
        plan: GpuRepeatAccumulator,
        input: LlvmValue<'ctx>,
        count: LlvmValue<'ctx>,
    ) -> Result<LlvmValue<'ctx>, String> {
        let tuple = input.value.into_struct_value();
        let wgsl = self
            .builder
            .build_global_string_ptr(&plan.wgsl, "gpu.repeat.program")
            .map_err(|error| {
                format!("LLVM backend failed to build GPU repeat program literal: {error}")
            })?;
        let score_index = match plan.kind {
            GpuRepeatAccumulatorKind::VectorScore => 2,
            GpuRepeatAccumulatorKind::MatrixScore => 3,
        };
        let score = self
            .builder
            .build_extract_value(tuple, score_index, "gpu.repeat.score")
            .map_err(|error| format!("LLVM backend failed to extract GPU repeat score: {error}"))?;
        let next_score = match plan.kind {
            GpuRepeatAccumulatorKind::VectorScore => {
                let left = self.extract_tuple_seq(tuple, 0, "gpu.repeat.left")?;
                let right = self.extract_tuple_seq(tuple, 1, "gpu.repeat.right")?;
                let function = self.gpu_repeat_vector_accum_f64_function();
                self.builder
                    .build_call(
                        function,
                        &[
                            wgsl.as_pointer_value().into(),
                            left.items.into(),
                            left.count.into(),
                            right.items.into(),
                            right.count.into(),
                            score.into(),
                            count.value.into_int_value().into(),
                        ],
                        "gpu.repeat.vector",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to call GPU vector repeat program: {error}")
                    })?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "GPU vector repeat program did not return a value".to_string())?
            }
            GpuRepeatAccumulatorKind::MatrixScore => {
                let left = self.extract_tuple_seq(tuple, 0, "gpu.repeat.left_matrix")?;
                let right = self.extract_tuple_seq(tuple, 1, "gpu.repeat.right_matrix")?;
                let vector = self.extract_tuple_seq(tuple, 2, "gpu.repeat.vector")?;
                let function = self.gpu_repeat_matrix_accum_f64_function();
                self.builder
                    .build_call(
                        function,
                        &[
                            wgsl.as_pointer_value().into(),
                            left.items.into(),
                            left.count.into(),
                            right.items.into(),
                            right.count.into(),
                            vector.items.into(),
                            vector.count.into(),
                            score.into(),
                            count.value.into_int_value().into(),
                        ],
                        "gpu.repeat.matrix",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to call GPU matrix repeat program: {error}")
                    })?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "GPU matrix repeat program did not return a value".to_string())?
            }
        };
        let out = self
            .builder
            .build_insert_value(tuple, next_score, score_index, "gpu.repeat.out")
            .map_err(|error| format!("LLVM backend failed to update GPU repeat output: {error}"))?
            .into_struct_value();
        Ok(LlvmValue {
            value: out.into(),
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
        let input_item_ty = item_ty.as_ref().clone();
        let plain_item_ty = input_item_ty.inner_faultable();
        let pair_ty = Ty::Tuple(vec![plain_item_ty.clone(), plain_item_ty.clone()]);
        let operation_output_ty = self.codegen.call_output_type(op, &pair_ty)?;
        let operation_faultable = matches!(
            &operation_output_ty,
            Ty::Faultable(inner) if inner.as_ref() == &plain_item_ty
        );
        let item_faultable = matches!(input_item_ty, Ty::Faultable(_));
        let output_item_ty = if item_faultable || operation_faultable {
            Ty::Faultable(Box::new(plain_item_ty.clone()))
        } else {
            plain_item_ty.clone()
        };
        let output_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let count = self.seq_count(input.value)?;
        let output = self.emit_seq_new(&output_ty, count)?;
        let state_llvm_ty = self.types.basic_type(&output_item_ty)?;
        let state_ptr = self
            .builder
            .build_alloca(state_llvm_ty, "scan.state")
            .map_err(|error| format!("LLVM backend failed to allocate scan state: {error}"))?;
        let identity = self.coerce_value_to_ty(identity, &plain_item_ty)?;
        let initial = if matches!(output_item_ty, Ty::Faultable(_)) {
            self.faultable_value(&plain_item_ty, false, None, Some(identity.value))?
        } else {
            identity.value
        };
        self.builder
            .build_store(state_ptr, initial)
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
            .build_load(state_llvm_ty, state_ptr, "state")
            .map_err(|error| format!("LLVM backend failed to load scan state: {error}"))?;
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let next_state = if matches!(output_item_ty, Ty::Faultable(_)) {
            let state_fault = self.extract_faultable_is_fault(state)?;
            let item_fault = if item_faultable {
                self.extract_faultable_is_fault(item)?
            } else {
                self.context.bool_type().const_zero()
            };
            let existing_fault = self
                .builder
                .build_or(state_fault, item_fault, "scan.existing.fault")
                .map_err(|error| format!("LLVM backend failed to combine scan faults: {error}"))?;
            let state_value = self.extract_faultable_value(state)?;
            let item_value = if item_faultable {
                self.extract_faultable_value(item)?
            } else {
                item
            };
            let mut pair = self
                .types
                .basic_type(&pair_ty)?
                .into_struct_type()
                .const_zero();
            pair = self
                .builder
                .build_insert_value(pair, state_value, 0, "scan.pair")
                .map_err(|error| format!("LLVM backend failed to build scan pair: {error}"))?
                .into_struct_value();
            pair = self
                .builder
                .build_insert_value(pair, item_value, 1, "scan.pair")
                .map_err(|error| format!("LLVM backend failed to build scan pair: {error}"))?
                .into_struct_value();
            let scanned = self.emit_call(
                op,
                LlvmValue {
                    value: pair.into(),
                    ty: pair_ty.clone(),
                },
            )?;
            let scanned = if matches!(scanned.ty, Ty::Faultable(_)) {
                scanned.value
            } else {
                self.faultable_value(&plain_item_ty, false, None, Some(scanned.value))?
            };
            let scan_fault = self.extract_faultable_is_fault(scanned)?;
            let any_fault = self
                .builder
                .build_or(existing_fault, scan_fault, "scan.any.fault")
                .map_err(|error| {
                    format!("LLVM backend failed to combine scan op fault: {error}")
                })?;
            let state_fault_value = self.extract_faultable_fault(state)?;
            let item_fault_value = if item_faultable {
                self.extract_faultable_fault(item)?
            } else {
                self.extract_faultable_fault(scanned)?
            };
            let scan_fault_value = self.extract_faultable_fault(scanned)?;
            let selected_fault = self
                .builder
                .build_select(
                    item_fault,
                    item_fault_value,
                    state_fault_value,
                    "scan.fault",
                )
                .map_err(|error| format!("LLVM backend failed to select scan fault: {error}"))?;
            let no_existing_fault = self
                .builder
                .build_not(existing_fault, "scan.no.existing.fault")
                .map_err(|error| format!("LLVM backend failed to invert scan fault: {error}"))?;
            let new_scan_fault = self
                .builder
                .build_and(scan_fault, no_existing_fault, "scan.new.fault")
                .map_err(|error| {
                    format!("LLVM backend failed to combine scan new fault: {error}")
                })?;
            let selected_fault = self
                .builder
                .build_select(
                    new_scan_fault,
                    scan_fault_value,
                    selected_fault,
                    "scan.fault",
                )
                .map_err(|error| format!("LLVM backend failed to select scan op fault: {error}"))?;
            let scan_value = self.extract_faultable_value(scanned)?;
            let next =
                self.faultable_value(&plain_item_ty, true, Some(selected_fault), Some(scan_value))?;
            let any_fault_i8 = self
                .builder
                .build_int_z_extend(any_fault, self.context.i8_type(), "scan.fault.flag")
                .map_err(|error| {
                    format!("LLVM backend failed to extend scan fault flag: {error}")
                })?;
            self.builder
                .build_insert_value(next.into_struct_value(), any_fault_i8, 0, "scan.fault.flag")
                .map_err(|error| format!("LLVM backend failed to set scan fault flag: {error}"))?
                .as_basic_value_enum()
        } else {
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
            self.emit_call(
                op,
                LlvmValue {
                    value: pair.into(),
                    ty: pair_ty.clone(),
                },
            )?
            .value
        };
        self.builder
            .build_store(state_ptr, next_state)
            .map_err(|error| format!("LLVM backend failed to store scan state: {error}"))?;
        self.store_seq_item(output.value, &output_ty, i, next_state)?;
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
        if self.options.gpu && matches!(canonical.as_str(), "add" | "min" | "max") {
            match item_ty.as_ref() {
                Ty::I32 if canonical != "add" => {
                    let identity = self.coerce_value_to_ty(identity, &Ty::I32)?;
                    let input_items = self.seq_items(input.value)?;
                    let count = self.seq_count(input.value)?;
                    let reduce_fn = self.gpu_reduce_i32_function();
                    let reduced = self
                        .builder
                        .build_call(
                            reduce_fn,
                            &[
                                self.context
                                    .i32_type()
                                    .const_int(gpu_reduce_op(&canonical).into(), false)
                                    .into(),
                                input_items.into(),
                                count.into(),
                                identity.value.into(),
                            ],
                            "gpu.reduce.i32",
                        )
                        .map_err(|error| {
                            format!("LLVM backend failed to call native GPU reduce: {error}")
                        })?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| {
                            "native GPU i32 reduce did not return a value".to_string()
                        })?;
                    return Ok(LlvmValue {
                        value: reduced,
                        ty: Ty::I32,
                    });
                }
                Ty::F32 => {
                    let identity = self.coerce_value_to_ty(identity, &Ty::F32)?;
                    let input_items = self.seq_items(input.value)?;
                    let count = self.seq_count(input.value)?;
                    let reduce_fn = self.gpu_reduce_f32_function();
                    let reduced = self
                        .builder
                        .build_call(
                            reduce_fn,
                            &[
                                self.context
                                    .i32_type()
                                    .const_int(gpu_reduce_op(&canonical).into(), false)
                                    .into(),
                                input_items.into(),
                                count.into(),
                                identity.value.into(),
                            ],
                            "gpu.reduce.f32",
                        )
                        .map_err(|error| {
                            format!("LLVM backend failed to call native GPU reduce: {error}")
                        })?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| {
                            "native GPU f32 reduce did not return a value".to_string()
                        })?;
                    return Ok(LlvmValue {
                        value: reduced,
                        ty: Ty::F32,
                    });
                }
                Ty::F64 => {
                    let identity = self.coerce_value_to_ty(identity, &Ty::F64)?;
                    let input_items = self.seq_items(input.value)?;
                    let count = self.seq_count(input.value)?;
                    let reduce_fn = self.gpu_reduce_f64_function();
                    let reduced = self
                        .builder
                        .build_call(
                            reduce_fn,
                            &[
                                self.context
                                    .i32_type()
                                    .const_int(gpu_reduce_op(&canonical).into(), false)
                                    .into(),
                                input_items.into(),
                                count.into(),
                                identity.value.into(),
                            ],
                            "gpu.reduce.f64",
                        )
                        .map_err(|error| {
                            format!("LLVM backend failed to call native GPU reduce: {error}")
                        })?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| {
                            "native GPU f64 reduce did not return a value".to_string()
                        })?;
                    return Ok(LlvmValue {
                        value: reduced,
                        ty: Ty::F64,
                    });
                }
                _ => {}
            }
        }
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
        let (plain_ty, item_faultable) = match item_ty {
            Ty::Faultable(inner) => (*inner, true),
            other => (other, false),
        };
        let overflow_faultable = matches!(plain_ty, Ty::I32 | Ty::I64);
        let output_ty = if item_faultable || overflow_faultable {
            Ty::Faultable(Box::new(plain_ty.clone()))
        } else {
            plain_ty.clone()
        };
        let output_llvm_ty = self.types.basic_type(&output_ty)?;
        let state_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "reduce.state")
            .map_err(|error| format!("LLVM backend failed to allocate reduce state: {error}"))?;
        let initial = if item_faultable || overflow_faultable {
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
        if item_faultable || overflow_faultable {
            let state = self
                .builder
                .build_load(output_llvm_ty, state_ptr, "state")
                .map_err(|error| format!("LLVM backend failed to load reduce state: {error}"))?;
            let state_fault = self.extract_faultable_is_fault(state)?;
            let item_fault = if item_faultable {
                self.extract_faultable_is_fault(item)?
            } else {
                self.context.bool_type().const_zero()
            };
            let any_fault = self
                .builder
                .build_or(state_fault, item_fault, "any.fault")
                .map_err(|error| format!("LLVM backend failed to combine fault flags: {error}"))?;
            let existing_fault = any_fault;
            let state_value = self.extract_faultable_value(state)?;
            let item_value = if item_faultable {
                self.extract_faultable_value(item)?
            } else {
                item
            };
            let sum = if overflow_faultable {
                let add_fn = match plain_ty {
                    Ty::I32 => "fa_faultable_i32_add",
                    Ty::I64 => "fa_faultable_i64_add",
                    _ => unreachable!(),
                };
                let int_ty = self.types.basic_type(&plain_ty)?;
                self.emit_runtime_sret_call(
                    add_fn,
                    &output_ty,
                    &[int_ty, int_ty],
                    &[
                        state_value.into_int_value().into(),
                        item_value.into_int_value().into(),
                    ],
                )?
            } else {
                let sum_value = self.emit_add_values(state_value, item_value, &plain_ty)?;
                self.faultable_value(&plain_ty, false, None, Some(sum_value))?
            };
            let sum_fault = self.extract_faultable_is_fault(sum)?;
            let any_fault = self
                .builder
                .build_or(any_fault, sum_fault, "any.fault")
                .map_err(|error| format!("LLVM backend failed to combine sum fault: {error}"))?;
            let item_fault_value = if item_faultable {
                self.extract_faultable_fault(item)?
            } else {
                self.extract_faultable_fault(sum)?
            };
            let state_fault_value = self.extract_faultable_fault(state)?;
            let sum_fault_value = self.extract_faultable_fault(sum)?;
            let selected_fault = self
                .builder
                .build_select(item_fault, item_fault_value, state_fault_value, "fault")
                .map_err(|error| format!("LLVM backend failed to select reduce fault: {error}"))?;
            let no_existing_fault = self
                .builder
                .build_not(existing_fault, "no.existing.fault")
                .map_err(|error| format!("LLVM backend failed to invert reduce fault: {error}"))?;
            let new_sum_fault = self
                .builder
                .build_and(sum_fault, no_existing_fault, "new.sum.fault")
                .map_err(|error| {
                    format!("LLVM backend failed to combine reduce sum fault: {error}")
                })?;
            let selected_fault = self
                .builder
                .build_select(new_sum_fault, sum_fault_value, selected_fault, "fault")
                .map_err(|error| {
                    format!("LLVM backend failed to select reduce sum fault: {error}")
                })?;
            let sum_value = self.extract_faultable_value(sum)?;
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
        if left_ty != right_ty {
            return Err(format!(
                "`{name}` expected matching operand types, found `{left_ty}` and `{right_ty}`"
            ));
        }
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        if matches!(left_ty, Ty::F32 | Ty::F64) {
            let left = left.into_float_value();
            let right = right.into_float_value();
            let float_ty = self.types.basic_type(left_ty)?.into_float_type();
            let result = match name {
                "add" => self.builder.build_float_add(left, right, "add"),
                "sub" => self.builder.build_float_sub(left, right, "sub"),
                "mul" => self.builder.build_float_mul(left, right, "mul"),
                "div" | "rem" => {
                    let function_name = match (name, left_ty) {
                        ("div", Ty::F32) => "fa_checked_f32_div",
                        ("div", Ty::F64) => "fa_checked_f64_div",
                        ("rem", Ty::F32) => "fa_checked_f32_rem",
                        ("rem", Ty::F64) => "fa_checked_f64_rem",
                        _ => unreachable!(),
                    };
                    let function = self.runtime_function(
                        function_name,
                        Some(float_ty.into()),
                        &[float_ty.into(), float_ty.into()],
                    )?;
                    return self
                        .builder
                        .build_call(function, &[left.into(), right.into()], name)
                        .map_err(|error| {
                            format!("LLVM backend failed to call `{function_name}`: {error}")
                        })?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| {
                            format!("runtime function `{function_name}` did not return a value")
                        });
                }
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
            let int_ty = self.types.basic_type(left_ty)?.into_int_type();
            match name {
                "add" | "sub" | "mul" | "div" | "rem" => {
                    let function_name = match (name, left_ty) {
                        ("add", Ty::I32) => "fa_checked_i32_add",
                        ("add", Ty::I64) => "fa_checked_i64_add",
                        ("sub", Ty::I32) => "fa_checked_i32_sub",
                        ("sub", Ty::I64) => "fa_checked_i64_sub",
                        ("mul", Ty::I32) => "fa_checked_i32_mul",
                        ("mul", Ty::I64) => "fa_checked_i64_mul",
                        ("div", Ty::I32) => "fa_checked_i32_div",
                        ("div", Ty::I64) => "fa_checked_i64_div",
                        ("rem", Ty::I32) => "fa_checked_i32_rem",
                        ("rem", Ty::I64) => "fa_checked_i64_rem",
                        _ => unreachable!(),
                    };
                    let function = self.runtime_function(
                        function_name,
                        Some(int_ty.into()),
                        &[int_ty.into(), int_ty.into()],
                    )?;
                    self.builder
                        .build_call(function, &[left.into(), right.into()], name)
                        .map_err(|error| {
                            format!("LLVM backend failed to call `{function_name}`: {error}")
                        })?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| {
                            format!("runtime function `{function_name}` did not return a value")
                        })
                }
                "min" => {
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::SLT, left, right, "min")
                        .map_err(|error| format!("LLVM backend failed to compare min: {error}"))?;
                    self.builder
                        .build_select(cmp, left, right, "min")
                        .map_err(|error| format!("LLVM backend failed to select min: {error}"))
                }
                "max" => {
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::SGT, left, right, "max")
                        .map_err(|error| format!("LLVM backend failed to compare max: {error}"))?;
                    self.builder
                        .build_select(cmp, left, right, "max")
                        .map_err(|error| format!("LLVM backend failed to select max: {error}"))
                }
                _ => unreachable!(),
            }
        }
    }

    fn emit_from_int(&mut self, input: LlvmValue<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        if input.ty != Ty::I64 {
            return Err(format!("from_int expected i64, found `{}`", input.ty));
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
                Ty::I32 => {
                    self.emit_checked_int_unary("fa_checked_i32_neg", "neg", input.value, &Ty::I32)
                }
                Ty::I64 => {
                    self.emit_checked_int_unary("fa_checked_i64_neg", "neg", input.value, &Ty::I64)
                }
                Ty::F32 | Ty::F64 => Ok(self
                    .builder
                    .build_float_neg(input.value.into_float_value(), "neg")
                    .map_err(|error| format!("LLVM backend failed to build neg: {error}"))?
                    .into()),
                ref other => Err(format!("neg expected numeric input, found `{other}`")),
            },
            "abs" => match input.ty {
                Ty::I32 => {
                    self.emit_checked_int_unary("fa_checked_i32_abs", "abs", input.value, &Ty::I32)
                }
                Ty::I64 => {
                    self.emit_checked_int_unary("fa_checked_i64_abs", "abs", input.value, &Ty::I64)
                }
                Ty::F32 | Ty::F64 => {
                    let value = input.value.into_float_value();
                    let negative = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OLT,
                            value,
                            self.types
                                .basic_type(&input.ty)?
                                .into_float_type()
                                .const_float(0.0),
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
                if !matches!(output_ty, Ty::F32 | Ty::F64) {
                    return Err(format!(
                        "{name} expected f32 or f64 output, found `{output_ty}`"
                    ));
                }
                if input.ty != *output_ty {
                    return Err(format!(
                        "{name} expected `{output_ty}` input, found `{}`",
                        input.ty
                    ));
                }
                let runtime_name = match (name, output_ty) {
                    ("sqrt", Ty::F32) => "fa_checked_sqrtf",
                    ("sqrt", Ty::F64) => "fa_checked_sqrt",
                    ("exp", Ty::F32) => "expf",
                    ("exp", Ty::F64) => "exp",
                    ("sin", Ty::F32) => "sinf",
                    ("sin", Ty::F64) => "sin",
                    ("cos", Ty::F32) => "cosf",
                    ("cos", Ty::F64) => "cos",
                    _ => unreachable!(),
                };
                let float_ty = self.types.basic_type(output_ty)?.into_float_type();
                let fn_value =
                    self.runtime_function(runtime_name, Some(float_ty.into()), &[float_ty.into()])?;
                self.builder
                    .build_call(fn_value, &[input.value.into_float_value().into()], name)
                    .map_err(|error| format!("LLVM backend failed to call `{name}`: {error}"))?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| {
                        format!("runtime function `{runtime_name}` did not return a value")
                    })
            }
            _ => unreachable!(),
        }
    }

    fn emit_checked_int_unary(
        &mut self,
        function_name: &str,
        label: &str,
        input: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let int_ty = self.types.basic_type(ty)?.into_int_type();
        let function =
            self.runtime_function(function_name, Some(int_ty.into()), &[int_ty.into()])?;
        self.builder
            .build_call(function, &[input.into_int_value().into()], label)
            .map_err(|error| format!("LLVM backend failed to call `{function_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("runtime function `{function_name}` did not return a value"))
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
        if left_ty != &Ty::I64 || right_ty != &Ty::I64 {
            return Err(format!("`{name}` expected i64 operands"));
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
        if left_ty != right_ty {
            return Err(format!(
                "`{name}` expected matching operand types, found `{left_ty}` and `{right_ty}`"
            ));
        }
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        let bit = if matches!(left_ty, Ty::F32 | Ty::F64) {
            let left = left.into_float_value();
            let right = right.into_float_value();
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
            Ty::F32 | Ty::F64 => Ok(self
                .builder
                .build_float_add(left.into_float_value(), right.into_float_value(), "add")
                .map_err(|error| format!("LLVM backend failed to build real add: {error}"))?
                .into()),
            Ty::I32 | Ty::I64 => Ok(self
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
            Ty::F32 | Ty::F64 => {
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
            Ty::I32 | Ty::I64 => {
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
            (Ty::Faultable(inner), actual)
                if unwrap_faultable_tuple(&actual)
                    .as_ref()
                    .is_some_and(|unwrapped| unwrapped == inner.as_ref()) =>
            {
                self.coerce_faultable_tuple_to_faultable(value, inner)
            }
            (Ty::Faultable(inner), actual) if inner.as_ref() == &actual => {
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

    fn collect_nested_fault_state(
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

    fn strip_nested_faultable_value(
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

    fn extract_struct_field(
        &mut self,
        input: LlvmValue<'ctx>,
        field: &str,
    ) -> Result<LlvmValue<'ctx>, String> {
        let Ty::Struct { name, fields } = input.ty.clone() else {
            return Err(format!(
                "field `{field}` expected struct input, found `{}`",
                input.ty
            ));
        };
        let Some((index, (_, ty))) = fields
            .iter()
            .enumerate()
            .find(|(_, (candidate, _))| candidate == field)
        else {
            return Err(format!("struct `{name}` has no field `{field}`"));
        };
        let value = self
            .builder
            .build_extract_value(input.value.into_struct_value(), index as u32, "field")
            .map_err(|error| format!("LLVM backend failed to extract struct field: {error}"))?;
        Ok(LlvmValue {
            value,
            ty: ty.clone(),
        })
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
        if self.options.export_abi == Some(DirectExportAbi::Wasm) {
            match name {
                "fa_checked_i32_add" => {
                    return self.define_wasm_checked_int_overflow_function(
                        name,
                        "llvm.sadd.with.overflow.i32",
                        &Ty::I32,
                    );
                }
                "fa_checked_i32_sub" => {
                    return self.define_wasm_checked_int_overflow_function(
                        name,
                        "llvm.ssub.with.overflow.i32",
                        &Ty::I32,
                    );
                }
                "fa_checked_i32_mul" => {
                    return self.define_wasm_checked_int_overflow_function(
                        name,
                        "llvm.smul.with.overflow.i32",
                        &Ty::I32,
                    );
                }
                "fa_checked_i64_add" => {
                    return self.define_wasm_checked_int_overflow_function(
                        name,
                        "llvm.sadd.with.overflow.i64",
                        &Ty::I64,
                    );
                }
                "fa_checked_i64_sub" => {
                    return self.define_wasm_checked_int_overflow_function(
                        name,
                        "llvm.ssub.with.overflow.i64",
                        &Ty::I64,
                    );
                }
                "fa_checked_i64_mul" => {
                    return self.define_wasm_checked_int_overflow_function(
                        name,
                        "llvm.smul.with.overflow.i64",
                        &Ty::I64,
                    );
                }
                "fa_faultable_i32_add" => {
                    return self.define_wasm_faultable_int_overflow_function(
                        name,
                        "llvm.sadd.with.overflow.i32",
                        &Ty::I32,
                    );
                }
                "fa_faultable_i32_sub" => {
                    return self.define_wasm_faultable_int_overflow_function(
                        name,
                        "llvm.ssub.with.overflow.i32",
                        &Ty::I32,
                    );
                }
                "fa_faultable_i32_mul" => {
                    return self.define_wasm_faultable_int_overflow_function(
                        name,
                        "llvm.smul.with.overflow.i32",
                        &Ty::I32,
                    );
                }
                "fa_faultable_i64_add" => {
                    return self.define_wasm_faultable_int_overflow_function(
                        name,
                        "llvm.sadd.with.overflow.i64",
                        &Ty::I64,
                    );
                }
                "fa_faultable_i64_sub" => {
                    return self.define_wasm_faultable_int_overflow_function(
                        name,
                        "llvm.ssub.with.overflow.i64",
                        &Ty::I64,
                    );
                }
                "fa_faultable_i64_mul" => {
                    return self.define_wasm_faultable_int_overflow_function(
                        name,
                        "llvm.smul.with.overflow.i64",
                        &Ty::I64,
                    );
                }
                "fa_faultable_i32_div" => {
                    return self.define_wasm_faultable_int_div_rem(name, "div", &Ty::I32);
                }
                "fa_faultable_i32_rem" => {
                    return self.define_wasm_faultable_int_div_rem(name, "rem", &Ty::I32);
                }
                "fa_faultable_i64_div" => {
                    return self.define_wasm_faultable_int_div_rem(name, "div", &Ty::I64);
                }
                "fa_faultable_i64_rem" => {
                    return self.define_wasm_faultable_int_div_rem(name, "rem", &Ty::I64);
                }
                "fa_faultable_i32_neg" => {
                    return self.define_wasm_faultable_int_unary(name, "neg", &Ty::I32);
                }
                "fa_faultable_i32_abs" => {
                    return self.define_wasm_faultable_int_unary(name, "abs", &Ty::I32);
                }
                "fa_faultable_i64_neg" => {
                    return self.define_wasm_faultable_int_unary(name, "neg", &Ty::I64);
                }
                "fa_faultable_i64_abs" => {
                    return self.define_wasm_faultable_int_unary(name, "abs", &Ty::I64);
                }
                "fa_checked_i32_div" => {
                    return self.define_wasm_checked_int_div_rem(name, "div", &Ty::I32);
                }
                "fa_checked_i32_rem" => {
                    return self.define_wasm_checked_int_div_rem(name, "rem", &Ty::I32);
                }
                "fa_checked_i64_div" => {
                    return self.define_wasm_checked_int_div_rem(name, "div", &Ty::I64);
                }
                "fa_checked_i64_rem" => {
                    return self.define_wasm_checked_int_div_rem(name, "rem", &Ty::I64);
                }
                "fa_checked_i32_neg" => {
                    return self.define_wasm_checked_int_unary(name, "neg", &Ty::I32);
                }
                "fa_checked_i32_abs" => {
                    return self.define_wasm_checked_int_unary(name, "abs", &Ty::I32);
                }
                "fa_checked_i64_neg" => {
                    return self.define_wasm_checked_int_unary(name, "neg", &Ty::I64);
                }
                "fa_checked_i64_abs" => {
                    return self.define_wasm_checked_int_unary(name, "abs", &Ty::I64);
                }
                "fa_faultable_f32_div" => {
                    return self.define_wasm_faultable_float_div_rem(name, "div", &Ty::F32);
                }
                "fa_faultable_f32_rem" => {
                    return self.define_wasm_faultable_float_div_rem(name, "rem", &Ty::F32);
                }
                "fa_faultable_f64_div" => {
                    return self.define_wasm_faultable_float_div_rem(name, "div", &Ty::F64);
                }
                "fa_faultable_f64_rem" => {
                    return self.define_wasm_faultable_float_div_rem(name, "rem", &Ty::F64);
                }
                "fa_checked_f32_div" => {
                    return self.define_wasm_checked_float_div_rem(name, "div", &Ty::F32);
                }
                "fa_checked_f32_rem" => {
                    return self.define_wasm_checked_float_div_rem(name, "rem", &Ty::F32);
                }
                "fa_checked_f64_div" => {
                    return self.define_wasm_checked_float_div_rem(name, "div", &Ty::F64);
                }
                "fa_checked_f64_rem" => {
                    return self.define_wasm_checked_float_div_rem(name, "rem", &Ty::F64);
                }
                "fa_checked_sqrtf" => return self.define_wasm_checked_sqrt(name, &Ty::F32),
                "fa_checked_sqrt" => return self.define_wasm_checked_sqrt(name, &Ty::F64),
                "fa_faultable_sqrtf" => return self.define_wasm_faultable_sqrt(name, &Ty::F32),
                "fa_faultable_sqrt" => return self.define_wasm_faultable_sqrt(name, &Ty::F64),
                "fa_exit_fault" => return self.define_wasm_exit_fault(),
                _ => {}
            }
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

    fn define_wasm_checked_int_overflow_function(
        &mut self,
        name: &str,
        intrinsic_name: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let int_ty = self.types.basic_type(ty)?.into_int_type();
        let function = self.module.add_function(
            name,
            int_ty.fn_type(&[int_ty.into(), int_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let left = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing left parameter"))?
            .into_int_value();
        let right = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing right parameter"))?
            .into_int_value();
        let intrinsic = Intrinsic::find(intrinsic_name)
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[int_ty.into()]))
            .ok_or_else(|| format!("failed to declare `{intrinsic_name}`"))?;
        let pair = self
            .builder
            .build_call(intrinsic, &[left.into(), right.into()], name)
            .map_err(|error| format!("LLVM backend failed to call `{intrinsic_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{intrinsic_name}` did not return a value"))?
            .into_struct_value();
        let result = self
            .builder
            .build_extract_value(pair, 0, "result")
            .map_err(|error| format!("LLVM backend failed to extract `{name}` result: {error}"))?
            .into_int_value();
        let overflow = self
            .builder
            .build_extract_value(pair, 1, "overflow")
            .map_err(|error| format!("LLVM backend failed to extract `{name}` overflow: {error}"))?
            .into_int_value();
        self.build_trap_if(overflow, name)?;
        self.builder
            .build_return(Some(&result))
            .map_err(|error| format!("LLVM backend failed to return `{name}`: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_faultable_int_overflow_function(
        &mut self,
        name: &str,
        intrinsic_name: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let int_ty = self.types.basic_type(ty)?.into_int_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            name,
            self.context
                .void_type()
                .fn_type(&[ptr_ty.into(), int_ty.into(), int_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        let fault_block = self.context.append_basic_block(function, "fault");
        let ok_block = self.context.append_basic_block(function, "ok");
        self.builder.position_at_end(entry);
        let out_ptr = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing output parameter"))?
            .into_pointer_value();
        let left = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing left parameter"))?
            .into_int_value();
        let right = function
            .get_nth_param(2)
            .ok_or_else(|| format!("`{name}` missing right parameter"))?
            .into_int_value();
        let intrinsic = Intrinsic::find(intrinsic_name)
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[int_ty.into()]))
            .ok_or_else(|| format!("failed to declare `{intrinsic_name}`"))?;
        let pair = self
            .builder
            .build_call(intrinsic, &[left.into(), right.into()], name)
            .map_err(|error| format!("LLVM backend failed to call `{intrinsic_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{intrinsic_name}` did not return a value"))?
            .into_struct_value();
        let result = self
            .builder
            .build_extract_value(pair, 0, "result")
            .map_err(|error| format!("LLVM backend failed to extract `{name}` result: {error}"))?;
        let overflow = self
            .builder
            .build_extract_value(pair, 1, "overflow")
            .map_err(|error| format!("LLVM backend failed to extract `{name}` overflow: {error}"))?
            .into_int_value();
        self.builder
            .build_conditional_branch(overflow, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in `{name}`: {error}"))?;

        self.builder.position_at_end(fault_block);
        let faulted = self.faultable_value(ty, true, None, Some(result))?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store `{name}` fault: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` fault: {error}"))?;

        self.builder.position_at_end(ok_block);
        let ok = self.faultable_value(ty, false, None, Some(result))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store `{name}` ok: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` ok: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_checked_int_div_rem(
        &mut self,
        name: &str,
        op: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let int_ty = self.types.basic_type(ty)?.into_int_type();
        let function = self.module.add_function(
            name,
            int_ty.fn_type(&[int_ty.into(), int_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let left = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing left parameter"))?
            .into_int_value();
        let right = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing right parameter"))?
            .into_int_value();
        let is_zero = self
            .builder
            .build_int_compare(IntPredicate::EQ, right, int_ty.const_zero(), "is_zero")
            .map_err(|error| format!("LLVM backend failed to check `{name}` zero: {error}"))?;
        let is_min = self
            .builder
            .build_int_compare(IntPredicate::EQ, left, int_min_value(int_ty, ty), "is_min")
            .map_err(|error| format!("LLVM backend failed to check `{name}` min: {error}"))?;
        let is_neg_one = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                right,
                int_ty.const_int((-1_i64) as u64, true),
                "is_neg_one",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}` -1: {error}"))?;
        let overflow = self
            .builder
            .build_and(is_min, is_neg_one, "overflow")
            .map_err(|error| {
                format!("LLVM backend failed to combine `{name}` overflow: {error}")
            })?;
        let invalid = self
            .builder
            .build_or(is_zero, overflow, "invalid")
            .map_err(|error| format!("LLVM backend failed to combine `{name}` checks: {error}"))?;
        self.build_trap_if(invalid, name)?;
        let result = match op {
            "div" => self.builder.build_int_signed_div(left, right, "div"),
            "rem" => self.builder.build_int_signed_rem(left, right, "rem"),
            _ => unreachable!(),
        }
        .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?;
        self.builder
            .build_return(Some(&result))
            .map_err(|error| format!("LLVM backend failed to return `{name}`: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_checked_int_unary(
        &mut self,
        name: &str,
        op: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let int_ty = self.types.basic_type(ty)?.into_int_type();
        let function =
            self.module
                .add_function(name, int_ty.fn_type(&[int_ty.into()], false), None);
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let value = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing value parameter"))?
            .into_int_value();
        let overflow = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                value,
                int_min_value(int_ty, ty),
                "overflow",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}`: {error}"))?;
        self.build_trap_if(overflow, name)?;
        let result = self.emit_int_unary_value(op, int_ty, value, name)?;
        self.builder
            .build_return(Some(&result))
            .map_err(|error| format!("LLVM backend failed to return `{name}`: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_faultable_int_div_rem(
        &mut self,
        name: &str,
        op: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let int_ty = self.types.basic_type(ty)?.into_int_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            name,
            self.context
                .void_type()
                .fn_type(&[ptr_ty.into(), int_ty.into(), int_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        let fault_block = self.context.append_basic_block(function, "fault");
        let ok_block = self.context.append_basic_block(function, "ok");
        self.builder.position_at_end(entry);
        let out_ptr = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing output parameter"))?
            .into_pointer_value();
        let left = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing left parameter"))?
            .into_int_value();
        let right = function
            .get_nth_param(2)
            .ok_or_else(|| format!("`{name}` missing right parameter"))?
            .into_int_value();
        let is_zero = self
            .builder
            .build_int_compare(IntPredicate::EQ, right, int_ty.const_zero(), "is_zero")
            .map_err(|error| format!("LLVM backend failed to check `{name}` zero: {error}"))?;
        let is_min = self
            .builder
            .build_int_compare(IntPredicate::EQ, left, int_min_value(int_ty, ty), "is_min")
            .map_err(|error| format!("LLVM backend failed to check `{name}` min: {error}"))?;
        let is_neg_one = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                right,
                int_ty.const_int((-1_i64) as u64, true),
                "is_neg_one",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}` -1: {error}"))?;
        let overflow = self
            .builder
            .build_and(is_min, is_neg_one, "overflow")
            .map_err(|error| {
                format!("LLVM backend failed to combine `{name}` overflow: {error}")
            })?;
        let invalid = self
            .builder
            .build_or(is_zero, overflow, "invalid")
            .map_err(|error| format!("LLVM backend failed to combine `{name}` checks: {error}"))?;
        self.builder
            .build_conditional_branch(invalid, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in `{name}`: {error}"))?;
        self.builder.position_at_end(fault_block);
        let faulted = self.faultable_value(ty, true, None, Some(left.into()))?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store `{name}` fault: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` fault: {error}"))?;

        self.builder.position_at_end(ok_block);
        let result = match op {
            "div" => self.builder.build_int_signed_div(left, right, "div"),
            "rem" => self.builder.build_int_signed_rem(left, right, "rem"),
            _ => unreachable!(),
        }
        .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))?;
        let ok = self.faultable_value(ty, false, None, Some(result.into()))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store `{name}` ok: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` ok: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_faultable_int_unary(
        &mut self,
        name: &str,
        op: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let int_ty = self.types.basic_type(ty)?.into_int_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            name,
            self.context
                .void_type()
                .fn_type(&[ptr_ty.into(), int_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        let fault_block = self.context.append_basic_block(function, "fault");
        let ok_block = self.context.append_basic_block(function, "ok");
        self.builder.position_at_end(entry);
        let out_ptr = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing output parameter"))?
            .into_pointer_value();
        let value = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing value parameter"))?
            .into_int_value();
        let overflow = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                value,
                int_min_value(int_ty, ty),
                "overflow",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}`: {error}"))?;
        let result = self.emit_int_unary_value(op, int_ty, value, name)?;
        self.builder
            .build_conditional_branch(overflow, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in `{name}`: {error}"))?;
        self.builder.position_at_end(fault_block);
        let faulted = self.faultable_value(ty, true, None, Some(value.into()))?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store `{name}` fault: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` fault: {error}"))?;
        self.builder.position_at_end(ok_block);
        let ok = self.faultable_value(ty, false, None, Some(result.into()))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store `{name}` ok: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` ok: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn emit_int_unary_value(
        &self,
        op: &str,
        int_ty: inkwell::types::IntType<'ctx>,
        value: inkwell::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        match op {
            "neg" => self
                .builder
                .build_int_neg(value, "neg")
                .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}")),
            "abs" => {
                let is_negative = self
                    .builder
                    .build_int_compare(IntPredicate::SLT, value, int_ty.const_zero(), "is_negative")
                    .map_err(|error| format!("LLVM backend failed to compare `{name}`: {error}"))?;
                let negated = self
                    .builder
                    .build_int_neg(value, "neg")
                    .map_err(|error| format!("LLVM backend failed to negate `{name}`: {error}"))?;
                self.builder
                    .build_select(is_negative, negated, value, "abs")
                    .map_err(|error| format!("LLVM backend failed to select `{name}`: {error}"))
                    .map(|value| value.into_int_value())
            }
            _ => unreachable!(),
        }
    }

    fn define_wasm_checked_float_div_rem(
        &mut self,
        name: &str,
        op: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let float_ty = self.types.basic_type(ty)?.into_float_type();
        let function = self.module.add_function(
            name,
            float_ty.fn_type(&[float_ty.into(), float_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let left = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing left parameter"))?
            .into_float_value();
        let right = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing right parameter"))?
            .into_float_value();
        let is_zero = self
            .builder
            .build_float_compare(
                FloatPredicate::OEQ,
                right,
                float_ty.const_float(0.0),
                "is_zero",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}` zero: {error}"))?;
        self.build_trap_if(is_zero, name)?;
        let result = self.emit_wasm_float_div_rem_result(op, ty, float_ty, left, right, name)?;
        self.builder
            .build_return(Some(&result))
            .map_err(|error| format!("LLVM backend failed to return `{name}`: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_faultable_float_div_rem(
        &mut self,
        name: &str,
        op: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let float_ty = self.types.basic_type(ty)?.into_float_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            name,
            self.context
                .void_type()
                .fn_type(&[ptr_ty.into(), float_ty.into(), float_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        let fault_block = self.context.append_basic_block(function, "fault");
        let ok_block = self.context.append_basic_block(function, "ok");
        self.builder.position_at_end(entry);
        let out_ptr = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing output parameter"))?
            .into_pointer_value();
        let left = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing left parameter"))?
            .into_float_value();
        let right = function
            .get_nth_param(2)
            .ok_or_else(|| format!("`{name}` missing right parameter"))?
            .into_float_value();
        let is_zero = self
            .builder
            .build_float_compare(
                FloatPredicate::OEQ,
                right,
                float_ty.const_float(0.0),
                "is_zero",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}` zero: {error}"))?;
        self.builder
            .build_conditional_branch(is_zero, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in `{name}`: {error}"))?;
        self.builder.position_at_end(fault_block);
        let faulted = self.faultable_value(ty, true, None, Some(left.into()))?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store `{name}` fault: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` fault: {error}"))?;

        self.builder.position_at_end(ok_block);
        let result = self.emit_wasm_float_div_rem_result(op, ty, float_ty, left, right, name)?;
        let ok = self.faultable_value(ty, false, None, Some(result.into()))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store `{name}` ok: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` ok: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn emit_wasm_float_div_rem_result(
        &mut self,
        op: &str,
        ty: &Ty,
        float_ty: inkwell::types::FloatType<'ctx>,
        left: inkwell::values::FloatValue<'ctx>,
        right: inkwell::values::FloatValue<'ctx>,
        name: &str,
    ) -> Result<inkwell::values::FloatValue<'ctx>, String> {
        match op {
            "div" => self
                .builder
                .build_float_div(left, right, "div")
                .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}")),
            "rem" => {
                let quotient = self
                    .builder
                    .build_float_div(left, right, "rem.quotient")
                    .map_err(|error| {
                        format!("LLVM backend failed to build `{name}` quotient: {error}")
                    })?;
                let intrinsic_name = match ty {
                    Ty::F32 => "llvm.trunc.f32",
                    Ty::F64 => "llvm.trunc.f64",
                    _ => unreachable!(),
                };
                let trunc = Intrinsic::find(intrinsic_name)
                    .and_then(|intrinsic| {
                        intrinsic.get_declaration(&self.module, &[float_ty.into()])
                    })
                    .ok_or_else(|| format!("failed to declare `{intrinsic_name}`"))?;
                let quotient = self
                    .builder
                    .build_call(trunc, &[quotient.into()], "rem.trunc")
                    .map_err(|error| {
                        format!("LLVM backend failed to call `{intrinsic_name}`: {error}")
                    })?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("`{intrinsic_name}` did not return a value"))?
                    .into_float_value();
                let product = self
                    .builder
                    .build_float_mul(quotient, right, "rem.product")
                    .map_err(|error| {
                        format!("LLVM backend failed to build `{name}` product: {error}")
                    })?;
                self.builder
                    .build_float_sub(left, product, "rem")
                    .map_err(|error| format!("LLVM backend failed to build `{name}`: {error}"))
            }
            _ => unreachable!(),
        }
    }

    fn define_wasm_checked_sqrt(
        &mut self,
        name: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let float_ty = self.types.basic_type(ty)?.into_float_type();
        let function =
            self.module
                .add_function(name, float_ty.fn_type(&[float_ty.into()], false), None);
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let value = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing value parameter"))?
            .into_float_value();
        let is_negative = self
            .builder
            .build_float_compare(
                FloatPredicate::OLT,
                value,
                float_ty.const_float(0.0),
                "is_negative",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}`: {error}"))?;
        self.build_trap_if(is_negative, name)?;
        let intrinsic_name = match ty {
            Ty::F32 => "llvm.sqrt.f32",
            Ty::F64 => "llvm.sqrt.f64",
            _ => unreachable!(),
        };
        let sqrt = Intrinsic::find(intrinsic_name)
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[float_ty.into()]))
            .ok_or_else(|| format!("failed to declare `{intrinsic_name}`"))?;
        let result = self
            .builder
            .build_call(sqrt, &[value.into()], "sqrt")
            .map_err(|error| format!("LLVM backend failed to call `{intrinsic_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{intrinsic_name}` did not return a value"))?
            .into_float_value();
        self.builder
            .build_return(Some(&result))
            .map_err(|error| format!("LLVM backend failed to return `{name}`: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_faultable_sqrt(
        &mut self,
        name: &str,
        ty: &Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let float_ty = self.types.basic_type(ty)?.into_float_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            name,
            self.context
                .void_type()
                .fn_type(&[ptr_ty.into(), float_ty.into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        let fault_block = self.context.append_basic_block(function, "fault");
        let ok_block = self.context.append_basic_block(function, "ok");
        self.builder.position_at_end(entry);
        let out_ptr = function
            .get_nth_param(0)
            .ok_or_else(|| format!("`{name}` missing output parameter"))?
            .into_pointer_value();
        let value = function
            .get_nth_param(1)
            .ok_or_else(|| format!("`{name}` missing value parameter"))?
            .into_float_value();
        let is_negative = self
            .builder
            .build_float_compare(
                FloatPredicate::OLT,
                value,
                float_ty.const_float(0.0),
                "is_negative",
            )
            .map_err(|error| format!("LLVM backend failed to check `{name}`: {error}"))?;
        self.builder
            .build_conditional_branch(is_negative, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in `{name}`: {error}"))?;

        self.builder.position_at_end(fault_block);
        let faulted = self.faultable_value(ty, true, None, Some(value.into()))?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store `{name}` fault: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` fault: {error}"))?;

        self.builder.position_at_end(ok_block);
        let intrinsic_name = match ty {
            Ty::F32 => "llvm.sqrt.f32",
            Ty::F64 => "llvm.sqrt.f64",
            _ => unreachable!(),
        };
        let sqrt = Intrinsic::find(intrinsic_name)
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[float_ty.into()]))
            .ok_or_else(|| format!("failed to declare `{intrinsic_name}`"))?;
        let result = self
            .builder
            .build_call(sqrt, &[value.into()], "sqrt")
            .map_err(|error| format!("LLVM backend failed to call `{intrinsic_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{intrinsic_name}` did not return a value"))?;
        let ok = self.faultable_value(ty, false, None, Some(result))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store `{name}` ok: {error}"))?;
        self.builder
            .build_return(None)
            .map_err(|error| format!("LLVM backend failed to return `{name}` ok: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn define_wasm_exit_fault(&mut self) -> Result<FunctionValue<'ctx>, String> {
        let saved_block = self.builder.get_insert_block();
        let function = self.module.add_function(
            "fa_exit_fault",
            self.context
                .void_type()
                .fn_type(&[self.runtime_pair_type().into()], false),
            None,
        );
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        self.build_trap_if(
            self.context.bool_type().const_int(1, false),
            "fa_exit_fault",
        )?;
        self.builder
            .build_unreachable()
            .map_err(|error| format!("LLVM backend failed to build fa_exit_fault trap: {error}"))?;
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }
        Ok(function)
    }

    fn build_trap_if(&mut self, condition: IntValue<'ctx>, label: &str) -> Result<(), String> {
        let function = self.current_function()?;
        let trap_block = self
            .context
            .append_basic_block(function, &format!("{label}.trap"));
        let ok_block = self
            .context
            .append_basic_block(function, &format!("{label}.ok"));
        self.builder
            .build_conditional_branch(condition, trap_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch for `{label}` trap: {error}")
            })?;
        self.builder.position_at_end(trap_block);
        let trap = Intrinsic::find("llvm.trap")
            .and_then(|intrinsic| intrinsic.get_declaration(&self.module, &[]))
            .ok_or_else(|| "failed to declare `llvm.trap`".to_string())?;
        self.builder
            .build_call(trap, &[], "trap")
            .map_err(|error| format!("LLVM backend failed to call `llvm.trap`: {error}"))?;
        self.builder.build_unreachable().map_err(|error| {
            format!("LLVM backend failed to build `llvm.trap` terminator: {error}")
        })?;
        self.builder.position_at_end(ok_block);
        Ok(())
    }

    fn calloc_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_calloc") {
            return function;
        }
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        self.module.add_function(
            "fa_calloc",
            ptr_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false),
            None,
        )
    }

    fn gpu_require_device_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_require_device") {
            return function;
        }
        self.module.add_function(
            "fa_gpu_require_device",
            self.context.void_type().fn_type(&[], false),
            None,
        )
    }

    fn gpu_map_f32_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_map_f32") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        self.module.add_function(
            "fa_gpu_map_f32",
            self.context.void_type().fn_type(
                &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into(), i64_ty.into()],
                false,
            ),
            None,
        )
    }

    fn gpu_map_f64_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_map_f64") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        self.module.add_function(
            "fa_gpu_map_f64",
            self.context.void_type().fn_type(
                &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into(), i64_ty.into()],
                false,
            ),
            None,
        )
    }

    fn gpu_map_i32_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_map_i32") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        self.module.add_function(
            "fa_gpu_map_i32",
            self.context.void_type().fn_type(
                &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into(), i64_ty.into()],
                false,
            ),
            None,
        )
    }

    fn gpu_reduce_i32_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_reduce_i32") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        self.module.add_function(
            "fa_gpu_reduce_i32",
            i32_ty.fn_type(
                &[i32_ty.into(), ptr_ty.into(), i64_ty.into(), i32_ty.into()],
                false,
            ),
            None,
        )
    }

    fn gpu_reduce_f32_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_reduce_f32") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let f32_ty = self.context.f32_type();
        self.module.add_function(
            "fa_gpu_reduce_f32",
            f32_ty.fn_type(
                &[i32_ty.into(), ptr_ty.into(), i64_ty.into(), f32_ty.into()],
                false,
            ),
            None,
        )
    }

    fn gpu_reduce_f64_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_reduce_f64") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let f64_ty = self.context.f64_type();
        self.module.add_function(
            "fa_gpu_reduce_f64",
            f64_ty.fn_type(
                &[i32_ty.into(), ptr_ty.into(), i64_ty.into(), f64_ty.into()],
                false,
            ),
            None,
        )
    }

    fn gpu_repeat_vector_accum_f64_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_repeat_vector_accum_f64") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let f64_ty = self.context.f64_type();
        self.module.add_function(
            "fa_gpu_repeat_vector_accum_f64",
            f64_ty.fn_type(
                &[
                    ptr_ty.into(),
                    ptr_ty.into(),
                    i64_ty.into(),
                    ptr_ty.into(),
                    i64_ty.into(),
                    f64_ty.into(),
                    i64_ty.into(),
                ],
                false,
            ),
            None,
        )
    }

    fn gpu_repeat_matrix_accum_f64_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function("fa_gpu_repeat_matrix_accum_f64") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i64_ty = self.context.i64_type();
        let f64_ty = self.context.f64_type();
        self.module.add_function(
            "fa_gpu_repeat_matrix_accum_f64",
            f64_ty.fn_type(
                &[
                    ptr_ty.into(),
                    ptr_ty.into(),
                    i64_ty.into(),
                    ptr_ty.into(),
                    i64_ty.into(),
                    ptr_ty.into(),
                    i64_ty.into(),
                    f64_ty.into(),
                    i64_ty.into(),
                ],
                false,
            ),
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

    fn extract_tuple_seq(
        &mut self,
        tuple: inkwell::values::StructValue<'ctx>,
        index: u32,
        label: &str,
    ) -> Result<ExtractedSeq<'ctx>, String> {
        let seq = self
            .builder
            .build_extract_value(tuple, index, label)
            .map_err(|error| format!("LLVM backend failed to extract {label}: {error}"))?
            .into_struct_value();
        let count = self
            .builder
            .build_extract_value(seq, 0, &format!("{label}.count"))
            .map_err(|error| format!("LLVM backend failed to extract {label} count: {error}"))?
            .into_int_value();
        let items = self
            .builder
            .build_extract_value(seq, 1, &format!("{label}.items"))
            .map_err(|error| format!("LLVM backend failed to extract {label} items: {error}"))?
            .into_pointer_value();
        Ok(ExtractedSeq { count, items })
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
        if self.options.gpu {
            let require_device = self.gpu_require_device_function();
            self.builder
                .build_call(require_device, &[], "gpu.require")
                .map_err(|error| {
                    format!("LLVM backend failed to require native GPU device: {error}")
                })?;
        }
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
            Some(Ty::I64) => self
                .builder
                .build_int_truncate(result.into_int_value(), i32_ty, "exit")
                .map_err(|error| format!("LLVM backend failed to truncate exit code: {error}"))?,
            Some(Ty::Faultable(inner)) if inner.as_ref() == &Ty::I64 => {
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

#[cfg(not(target_arch = "wasm32"))]
fn int_min_value<'ctx>(
    int_ty: inkwell::types::IntType<'ctx>,
    ty: &Ty,
) -> inkwell::values::IntValue<'ctx> {
    match ty {
        Ty::I32 => int_ty.const_int(i32::MIN as u64, true),
        Ty::I64 => int_ty.const_int(i64::MIN as u64, true),
        _ => unreachable!(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct LlvmTypeRegistry<'ctx> {
    context: &'ctx Context,
    structs: HashMap<String, StructType<'ctx>>,
}

#[cfg(not(target_arch = "wasm32"))]
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
            Ty::I32 => Ok(self.context.i32_type().into()),
            Ty::I64 => Ok(self.context.i64_type().into()),
            Ty::F32 => Ok(self.context.f32_type().into()),
            Ty::F64 => Ok(self.context.f64_type().into()),
            Ty::OneOf(_) => Err(format!("union type `{ty}` is not runtime-represented")),
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
            Ty::Struct { fields, .. } => {
                let fields = fields
                    .iter()
                    .map(|(_, item)| self.basic_type(item))
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
