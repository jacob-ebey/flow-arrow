use super::{Ty, TypedCodegen, sanitize_symbol, user_fn_name};
use crate::ast::BindingTarget;
use crate::typecheck::{
    TypedCallable, TypedEndpoint, TypedEndpointKind, TypedMatchArm, TypedStageKind,
};
use std::collections::{BTreeSet, HashMap};

pub(super) fn emit_module(codegen: TypedCodegen<'_>) -> Result<String, String> {
    LlvmText::new(codegen).emit()
}

#[derive(Debug, Clone)]
struct TextValue {
    operand: String,
    ty: Ty,
}

struct LlvmText<'a> {
    codegen: TypedCodegen<'a>,
    temp: usize,
    declarations: BTreeSet<String>,
}

impl<'a> LlvmText<'a> {
    fn new(codegen: TypedCodegen<'a>) -> Self {
        Self {
            codegen,
            temp: 0,
            declarations: BTreeSet::new(),
        }
    }

    fn emit(mut self) -> Result<String, String> {
        let mut body = String::new();
        let mut callables = self.codegen.typed.callables.iter().collect::<Vec<_>>();
        callables.sort_by(|left, right| left.name.cmp(&right.name));

        for callable in callables {
            self.emit_callable(&mut body, callable)?;
        }

        let mut out = String::new();
        out.push_str("; FlowArrow LLVM IR preview\n");
        out.push_str(
            "; Constructed by the wasm-safe text emitter; object emission is not performed.\n",
        );
        out.push_str("source_filename = \"flowarrow-preview\"\n\n");
        for declaration in &self.declarations {
            out.push_str(declaration);
            out.push('\n');
        }
        if !self.declarations.is_empty() {
            out.push('\n');
        }
        out.push_str(&body);
        Ok(out)
    }

    fn emit_callable(&mut self, out: &mut String, callable: &TypedCallable) -> Result<(), String> {
        self.temp = 0;
        let signature = self
            .codegen
            .signatures
            .get(&callable.name)
            .cloned()
            .ok_or_else(|| format!("missing signature for `{}`", callable.name))?;
        let input_ty = llvm_ty(&signature.input);
        let output_ty = llvm_ty(&signature.output);
        out.push_str(&format!(
            "define {output_ty} @{}({input_ty} %input) {{\nentry:\n",
            user_fn_name(&callable.name)
        ));

        let mut env = HashMap::new();
        match callable.inputs.as_slice() {
            [] => {}
            [port] => {
                env.insert(
                    port.name.clone(),
                    TextValue {
                        operand: "%input".to_string(),
                        ty: signature.input.clone(),
                    },
                );
            }
            ports => {
                let Ty::Tuple(items) = &signature.input else {
                    return Err(format!("callable `{}` expected tuple input", callable.name));
                };
                for (index, (port, ty)) in ports.iter().zip(items.iter()).enumerate() {
                    let temp = self.next_temp();
                    out.push_str(&format!(
                        "  {temp} = extractvalue {} %input, {index}\n",
                        llvm_ty(&signature.input)
                    ));
                    env.insert(
                        port.name.clone(),
                        TextValue {
                            operand: temp,
                            ty: ty.clone(),
                        },
                    );
                }
            }
        }

        for chain in &callable.chains {
            let source_expected = if super::contains_empty_seq(&chain.source.ty) {
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
                self.emit_endpoint(out, &chain.source, &env, source_expected.as_ref())?;
            for stage in &chain.stages {
                match &stage.kind {
                    TypedStageKind::Bind { target } => {
                        self.bind_target(out, target, value.clone(), &mut env)?;
                    }
                    TypedStageKind::Call { name, .. } => {
                        value = self.emit_call(out, name, value)?;
                    }
                    TypedStageKind::Map { name, .. } => {
                        value = self.emit_map(out, name, value)?;
                    }
                    TypedStageKind::FaultMap {
                        node, ok, fault, ..
                    } => {
                        let partitioned = self.emit_fault_map(out, node, value.clone())?;
                        let [ok_ty, fault_ty] = tuple_items(&partitioned.ty)? else {
                            return Err("fault map helper expected tuple output".to_string());
                        };
                        let ok_operand = self.extract_tuple_field(out, &partitioned, 0)?;
                        let fault_operand = self.extract_tuple_field(out, &partitioned, 1)?;
                        env.insert(
                            ok.clone(),
                            TextValue {
                                operand: ok_operand,
                                ty: ok_ty.clone(),
                            },
                        );
                        env.insert(
                            fault.clone(),
                            TextValue {
                                operand: fault_operand,
                                ty: fault_ty.clone(),
                            },
                        );
                    }
                    TypedStageKind::Filter { name, .. } => {
                        value = self.emit_filter(out, name, value)?;
                    }
                    TypedStageKind::Field { .. } => {
                        return Err(
                            "LLVM text backend does not support struct field projection yet"
                                .to_string(),
                        );
                    }
                    TypedStageKind::Repeat { count, node, .. } => {
                        let count = self.emit_endpoint(out, count, &env, Some(&Ty::Int))?;
                        value = self.emit_repeat(out, node, value, count)?;
                    }
                    TypedStageKind::Reduce { op, identity, .. } => {
                        let identity = self.emit_endpoint(out, identity, &env, None)?;
                        value = self.emit_reduce(out, op, value, identity)?;
                    }
                    TypedStageKind::Scan { op, identity, .. } => {
                        let identity = self.emit_endpoint(out, identity, &env, None)?;
                        value = self.emit_scan(out, op, value, identity)?;
                    }
                    TypedStageKind::Match { arms } => {
                        value = self.emit_match(out, arms, stage.output.clone(), value)?;
                    }
                }
            }
        }

        let result = self.emit_outputs(out, callable, &env, &signature.output)?;
        out.push_str(&format!(
            "  ret {} {}\n}}\n\n",
            llvm_ty(&result.ty),
            result.operand
        ));
        Ok(())
    }

    fn emit_outputs(
        &mut self,
        out: &mut String,
        callable: &TypedCallable,
        env: &HashMap<String, TextValue>,
        expected_ty: &Ty,
    ) -> Result<TextValue, String> {
        match callable.outputs.as_slice() {
            [] => Ok(TextValue {
                operand: default_value(&Ty::Unit),
                ty: Ty::Unit,
            }),
            [port] => env
                .get(&port.name)
                .cloned()
                .ok_or_else(|| format!("output `{}` is never bound", port.name)),
            ports => {
                let Ty::Tuple(expected_items) = expected_ty else {
                    return Err(format!(
                        "callable `{}` has multiple outputs but signature output is `{expected_ty}`",
                        callable.name
                    ));
                };
                let mut current = "poison".to_string();
                for (index, (port, expected_item)) in
                    ports.iter().zip(expected_items.iter()).enumerate()
                {
                    let value = env
                        .get(&port.name)
                        .ok_or_else(|| format!("output `{}` is never bound", port.name))?;
                    let temp = self.next_temp();
                    out.push_str(&format!(
                        "  {temp} = insertvalue {} {current}, {} {}, {index}\n",
                        llvm_ty(expected_ty),
                        llvm_ty(expected_item),
                        value.operand
                    ));
                    current = temp;
                }
                Ok(TextValue {
                    operand: current,
                    ty: expected_ty.clone(),
                })
            }
        }
    }

    fn emit_endpoint(
        &mut self,
        out: &mut String,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, TextValue>,
        expected: Option<&Ty>,
    ) -> Result<TextValue, String> {
        match &endpoint.kind {
            TypedEndpointKind::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            TypedEndpointKind::NodeRef { name, .. } => {
                Err(format!("expected value, found node `{name}`"))
            }
            TypedEndpointKind::Int(value) => Ok(TextValue {
                operand: value.to_string(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Real(value) => Ok(TextValue {
                operand: format!("{value:.17e}"),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Bool(value) => Ok(TextValue {
                operand: if *value { "1" } else { "0" }.to_string(),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::String(value) => {
                out.push_str(&format!(
                    "  ; bytes literal {:?} is represented as a runtime value in native lowering\n",
                    value
                ));
                Ok(TextValue {
                    operand: default_value(&Ty::Bytes),
                    ty: endpoint.ty.clone(),
                })
            }
            TypedEndpointKind::Unit => Ok(TextValue {
                operand: default_value(&endpoint.ty),
                ty: endpoint.ty.clone(),
            }),
            TypedEndpointKind::Tuple(items) => {
                let expected_items = match expected {
                    Some(Ty::Tuple(expected_items)) if expected_items.len() == items.len() => {
                        Some(expected_items.as_slice())
                    }
                    _ => None,
                };
                let mut values = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    values.push(self.emit_endpoint(
                        out,
                        item,
                        env,
                        expected_items.and_then(|items| items.get(index)),
                    )?);
                }
                let mut current = "poison".to_string();
                for (index, value) in values.iter().enumerate() {
                    let temp = self.next_temp();
                    out.push_str(&format!(
                        "  {temp} = insertvalue {} {current}, {} {}, {index}\n",
                        llvm_ty(&endpoint.ty),
                        llvm_ty(&value.ty),
                        value.operand
                    ));
                    current = temp;
                }
                Ok(TextValue {
                    operand: current,
                    ty: endpoint.ty.clone(),
                })
            }
            TypedEndpointKind::Seq(items) => {
                out.push_str(&format!(
                    "  ; sequence literal with {} item(s) is represented as a runtime value in native lowering\n",
                    items.len()
                ));
                Ok(TextValue {
                    operand: default_value(&endpoint.ty),
                    ty: endpoint.ty.clone(),
                })
            }
            TypedEndpointKind::Struct { .. } => {
                Err("LLVM text backend does not support struct literals yet".to_string())
            }
            TypedEndpointKind::Eval { source, stages } => {
                let source_expected = if super::contains_empty_seq(&source.ty) {
                    match stages.first().map(|stage| &stage.kind) {
                        Some(TypedStageKind::Call { name, .. }) => {
                            Some(self.codegen.call_input_type_for_value(name, &source.ty)?)
                        }
                        Some(_) => stages.first().map(|stage| stage.input.clone()),
                        None => None,
                    }
                } else {
                    expected.cloned()
                };
                let mut value = self.emit_endpoint(out, source, env, source_expected.as_ref())?;
                for stage in stages {
                    match &stage.kind {
                        TypedStageKind::Call { name, .. } => {
                            value = self.emit_call(out, name, value)?;
                        }
                        TypedStageKind::Map { name, .. } => {
                            value = self.emit_map(out, name, value)?
                        }
                        TypedStageKind::Filter { name, .. } => {
                            value = self.emit_filter(out, name, value)?
                        }
                        TypedStageKind::Field { .. } => {
                            return Err(
                                "LLVM text backend does not support struct field projection yet"
                                    .to_string(),
                            );
                        }
                        TypedStageKind::Reduce { op, identity, .. } => {
                            let identity = self.emit_endpoint(out, identity, env, None)?;
                            value = self.emit_reduce(out, op, value, identity)?;
                        }
                        TypedStageKind::Scan { op, identity, .. } => {
                            let identity = self.emit_endpoint(out, identity, env, None)?;
                            value = self.emit_scan(out, op, value, identity)?;
                        }
                        TypedStageKind::Repeat { count, node, .. } => {
                            let count = self.emit_endpoint(out, count, env, Some(&Ty::Int))?;
                            value = self.emit_repeat(out, node, value, count)?;
                        }
                        TypedStageKind::Match { arms } => {
                            value = self.emit_match(out, arms, stage.output.clone(), value)?;
                        }
                        TypedStageKind::Bind { .. } | TypedStageKind::FaultMap { .. } => {
                            return Err("unsupported inline evaluation stage".to_string());
                        }
                    }
                }
                Ok(value)
            }
        }
    }

    fn emit_call(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
    ) -> Result<TextValue, String> {
        let output_ty = self.codegen.call_output_type(name, &input.ty)?;
        if self.codegen.callables.contains_key(name) {
            let temp = self.next_temp();
            out.push_str(&format!(
                "  {temp} = call {} @{}({} {})\n",
                llvm_ty(&output_ty),
                user_fn_name(name),
                llvm_ty(&input.ty),
                input.operand
            ));
            return Ok(TextValue {
                operand: temp,
                ty: output_ty,
            });
        }
        if let Some(binding) = self.codegen.foreign_c.get(name).cloned() {
            let temp = self.next_temp();
            let symbol = format!("@{}", binding.symbol);
            self.declare(&symbol, &output_ty, std::slice::from_ref(&input.ty));
            out.push_str(&format!(
                "  {temp} = call {} {symbol}({} {})\n",
                llvm_ty(&output_ty),
                llvm_ty(&input.ty),
                input.operand
            ));
            return Ok(TextValue {
                operand: temp,
                ty: output_ty,
            });
        }
        self.emit_builtin_call(out, name, input, output_ty)
    }

    fn emit_builtin_call(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
        output_ty: Ty,
    ) -> Result<TextValue, String> {
        let canonical = self.codegen.canonical_name(name);
        if let Some(value) = self.emit_numeric_builtin(out, &canonical, &input, &output_ty)? {
            return Ok(value);
        }
        let symbol = format!("@flow_builtin_{}", sanitize_symbol(&canonical));
        self.declare(&symbol, &output_ty, std::slice::from_ref(&input.ty));
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {})\n",
            llvm_ty(&output_ty),
            llvm_ty(&input.ty),
            input.operand
        ));
        Ok(TextValue {
            operand: temp,
            ty: output_ty,
        })
    }

    fn emit_numeric_builtin(
        &mut self,
        out: &mut String,
        name: &str,
        input: &TextValue,
        output_ty: &Ty,
    ) -> Result<Option<TextValue>, String> {
        let Ty::Tuple(items) = &input.ty else {
            return Ok(None);
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Ok(None);
        };
        if left_ty != right_ty || left_ty != output_ty {
            return Ok(None);
        }
        let Some(op) = numeric_instruction(name, left_ty) else {
            return Ok(None);
        };
        let left = self.extract_tuple_field(out, input, 0)?;
        let right = self.extract_tuple_field(out, input, 1)?;
        let temp = self.next_temp();
        match name {
            "min" | "max" => {
                let cmp = self.next_temp();
                let predicate = match (name, left_ty) {
                    ("min", Ty::Real) => "olt",
                    ("max", Ty::Real) => "ogt",
                    ("min", _) => "slt",
                    ("max", _) => "sgt",
                    _ => unreachable!(),
                };
                let cmp_inst = if matches!(left_ty, Ty::Real) {
                    "fcmp"
                } else {
                    "icmp"
                };
                out.push_str(&format!(
                    "  {cmp} = {cmp_inst} {predicate} {} {left}, {right}\n",
                    llvm_ty(left_ty)
                ));
                out.push_str(&format!(
                    "  {temp} = select i1 {cmp}, {} {left}, {} {right}\n",
                    llvm_ty(left_ty),
                    llvm_ty(right_ty)
                ));
            }
            _ => {
                out.push_str(&format!(
                    "  {temp} = {op} {} {left}, {right}\n",
                    llvm_ty(left_ty)
                ));
            }
        }
        Ok(Some(TextValue {
            operand: temp,
            ty: output_ty.clone(),
        }))
    }

    fn emit_map(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
    ) -> Result<TextValue, String> {
        let item_ty = match &input.ty {
            Ty::Seq(item) | Ty::Stream(item) => item.as_ref().clone(),
            other => {
                return Err(format!(
                    "`map {name}` expected Seq or Stream input, found `{other}`"
                ));
            }
        };
        let output_item_ty = self.codegen.call_output_type(name, &item_ty)?;
        let output_ty = match &input.ty {
            Ty::Stream(_) => Ty::Stream(Box::new(output_item_ty)),
            _ => Ty::Seq(Box::new(output_item_ty)),
        };
        let symbol = format!("@flow_map_{}", sanitize_symbol(name));
        self.declare(&symbol, &output_ty, std::slice::from_ref(&input.ty));
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {}) ; map {name}\n",
            llvm_ty(&output_ty),
            llvm_ty(&input.ty),
            input.operand
        ));
        Ok(TextValue {
            operand: temp,
            ty: output_ty,
        })
    }

    fn emit_fault_map(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
    ) -> Result<TextValue, String> {
        let Ty::Seq(item_ty) = &input.ty else {
            return Err(format!("`fault map {name}` expected Seq input"));
        };
        let output_item_ty = self.codegen.call_output_type(name, item_ty)?;
        let Ty::Faultable(ok_ty) = output_item_ty else {
            return Err(format!("`fault map {name}` node output must be Faultable"));
        };
        let output_ty = Ty::Tuple(vec![
            Ty::Seq(Box::new(ok_ty.as_ref().clone())),
            Ty::Seq(Box::new(Ty::Fault)),
        ]);
        let symbol = format!("@flow_fault_map_{}", sanitize_symbol(name));
        self.declare(&symbol, &output_ty, std::slice::from_ref(&input.ty));
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {}) ; fault map {name}\n",
            llvm_ty(&output_ty),
            llvm_ty(&input.ty),
            input.operand
        ));
        Ok(TextValue {
            operand: temp,
            ty: output_ty,
        })
    }

    fn emit_filter(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
    ) -> Result<TextValue, String> {
        let Ty::Seq(item_ty) = &input.ty else {
            return Err(format!("`filter {name}` expected Seq input"));
        };
        let predicate_ty = self.codegen.call_output_type(name, item_ty)?;
        if predicate_ty != Ty::Bool {
            return Err(format!(
                "`filter {name}` predicate expected `Bool`, found `{predicate_ty}`"
            ));
        }
        let symbol = format!("@flow_filter_{}", sanitize_symbol(name));
        self.declare(&symbol, &input.ty, std::slice::from_ref(&input.ty));
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {}) ; filter {name}\n",
            llvm_ty(&input.ty),
            llvm_ty(&input.ty),
            input.operand
        ));
        Ok(TextValue {
            operand: temp,
            ty: input.ty,
        })
    }

    fn emit_repeat(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
        count: TextValue,
    ) -> Result<TextValue, String> {
        let output_ty = self.codegen.call_output_type(name, &input.ty)?;
        let symbol = format!("@flow_repeat_{}", sanitize_symbol(name));
        self.declare(&symbol, &output_ty, &[input.ty.clone(), Ty::Int]);
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {}, i64 {}) ; repeat {name}\n",
            llvm_ty(&output_ty),
            llvm_ty(&input.ty),
            input.operand,
            count.operand
        ));
        Ok(TextValue {
            operand: temp,
            ty: output_ty,
        })
    }

    fn emit_reduce(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
        identity: TextValue,
    ) -> Result<TextValue, String> {
        let Ty::Seq(item_ty) = &input.ty else {
            return Err(format!("`reduce {name}` expected Seq input"));
        };
        if item_ty.as_ref() != &identity.ty {
            return Err(format!(
                "`reduce {name}` identity expected `{item_ty}`, found `{}`",
                identity.ty
            ));
        }
        let canonical = self.codegen.canonical_name(name);
        let output_ty = if matches!(canonical.as_str(), "add" | "min" | "max") {
            item_ty.as_ref().clone()
        } else {
            self.codegen.call_output_type(name, item_ty)?
        };
        let symbol = format!("@flow_reduce_{}", sanitize_symbol(name));
        self.declare(
            &symbol,
            &output_ty,
            &[input.ty.clone(), identity.ty.clone()],
        );
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {}, {} {}) ; reduce {name}\n",
            llvm_ty(&output_ty),
            llvm_ty(&input.ty),
            input.operand,
            llvm_ty(&identity.ty),
            identity.operand
        ));
        Ok(TextValue {
            operand: temp,
            ty: output_ty,
        })
    }

    fn emit_scan(
        &mut self,
        out: &mut String,
        name: &str,
        input: TextValue,
        identity: TextValue,
    ) -> Result<TextValue, String> {
        let Ty::Seq(item_ty) = &input.ty else {
            return Err(format!("`scan {name}` expected Seq input"));
        };
        if item_ty.as_ref() != &identity.ty {
            return Err(format!(
                "`scan {name}` identity expected `{item_ty}`, found `{}`",
                identity.ty
            ));
        }
        let canonical = self.codegen.canonical_name(name);
        let output_item_ty = if matches!(canonical.as_str(), "add" | "min" | "max") {
            item_ty.as_ref().clone()
        } else {
            self.codegen.call_output_type(name, item_ty)?
        };
        let output_ty = Ty::Seq(Box::new(output_item_ty));
        let symbol = format!("@flow_scan_{}", sanitize_symbol(name));
        self.declare(
            &symbol,
            &output_ty,
            &[input.ty.clone(), identity.ty.clone()],
        );
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {}, {} {}) ; scan {name}\n",
            llvm_ty(&output_ty),
            llvm_ty(&input.ty),
            input.operand,
            llvm_ty(&identity.ty),
            identity.operand
        ));
        Ok(TextValue {
            operand: temp,
            ty: output_ty,
        })
    }

    fn emit_match(
        &mut self,
        out: &mut String,
        arms: &[TypedMatchArm],
        output_ty: Ty,
        subject: TextValue,
    ) -> Result<TextValue, String> {
        let symbol = "@flow_match";
        self.declare(symbol, &output_ty, std::slice::from_ref(&subject.ty));
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = call {} {symbol}({} {}) ; match with {} arm(s)\n",
            llvm_ty(&output_ty),
            llvm_ty(&subject.ty),
            subject.operand,
            arms.len()
        ));
        Ok(TextValue {
            operand: temp,
            ty: output_ty,
        })
    }

    fn bind_target(
        &mut self,
        out: &mut String,
        target: &BindingTarget,
        value: TextValue,
        env: &mut HashMap<String, TextValue>,
    ) -> Result<(), String> {
        match target {
            BindingTarget::Discard => Ok(()),
            BindingTarget::Variable(name) => {
                if env.insert(name.clone(), value).is_some() {
                    return Err(format!("value `{name}` is bound more than once"));
                }
                Ok(())
            }
            BindingTarget::Tuple(items) => {
                let tuple_items = tuple_items(&value.ty)?;
                if tuple_items.len() != items.len() {
                    return Err(format!(
                        "tuple binding expected {} value(s), found {}",
                        items.len(),
                        tuple_items.len()
                    ));
                }
                for (index, target) in items.iter().enumerate() {
                    let operand = self.extract_tuple_field(out, &value, index)?;
                    self.bind_target(
                        out,
                        target,
                        TextValue {
                            operand,
                            ty: tuple_items[index].clone(),
                        },
                        env,
                    )?;
                }
                Ok(())
            }
        }
    }

    fn extract_tuple_field(
        &mut self,
        out: &mut String,
        value: &TextValue,
        index: usize,
    ) -> Result<String, String> {
        tuple_items(&value.ty)?
            .get(index)
            .ok_or_else(|| format!("tuple has no field {index}"))?;
        let temp = self.next_temp();
        out.push_str(&format!(
            "  {temp} = extractvalue {} {}, {index}\n",
            llvm_ty(&value.ty),
            value.operand
        ));
        Ok(temp)
    }

    fn declare(&mut self, symbol: &str, output_ty: &Ty, input_tys: &[Ty]) {
        let params = input_tys.iter().map(llvm_ty).collect::<Vec<_>>().join(", ");
        self.declarations
            .insert(format!("declare {} {symbol}({params})", llvm_ty(output_ty)));
    }

    fn next_temp(&mut self) -> String {
        let temp = format!("%t{}", self.temp);
        self.temp += 1;
        temp
    }
}

fn tuple_items(ty: &Ty) -> Result<&[Ty], String> {
    let Ty::Tuple(items) = ty else {
        return Err(format!("expected tuple, found `{ty}`"));
    };
    Ok(items)
}

fn numeric_instruction(name: &str, ty: &Ty) -> Option<&'static str> {
    match (name, ty) {
        ("add", Ty::Int) => Some("add"),
        ("add", Ty::Real) => Some("fadd"),
        ("sub", Ty::Int) => Some("sub"),
        ("sub", Ty::Real) => Some("fsub"),
        ("mul", Ty::Int) => Some("mul"),
        ("mul", Ty::Real) => Some("fmul"),
        ("div", Ty::Int) => Some("sdiv"),
        ("div", Ty::Real) => Some("fdiv"),
        ("rem", Ty::Int) => Some("srem"),
        ("min" | "max", Ty::Int | Ty::Real) => Some("select"),
        _ => None,
    }
}

fn llvm_ty(ty: &Ty) -> String {
    match ty {
        Ty::Unit => "{ i8 }".to_string(),
        Ty::Int => "i64".to_string(),
        Ty::Real => "double".to_string(),
        Ty::Bool => "i1".to_string(),
        Ty::Bytes | Ty::Args | Ty::Fault => "{ i64, ptr }".to_string(),
        Ty::HttpServerConfig
        | Ty::HttpListener
        | Ty::HttpRequest
        | Ty::HttpResponse
        | Ty::SqliteConnection
        | Ty::SqliteRow
        | Ty::SqliteValue
        | Ty::Stream(_) => "ptr".to_string(),
        Ty::Faultable(inner) => format!("{{ i1, {}, {} }}", llvm_ty(&Ty::Fault), llvm_ty(inner)),
        Ty::Seq(_) | Ty::EmptySeq => "{ i64, ptr }".to_string(),
        Ty::Tuple(items) => format!(
            "{{ {} }}",
            items.iter().map(llvm_ty).collect::<Vec<_>>().join(", ")
        ),
        Ty::Struct { fields, .. } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(_, ty)| llvm_ty(ty))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Ty::OneOf(items) => format!(
            "{{ i64, {} }}",
            items
                .first()
                .map(llvm_ty)
                .unwrap_or_else(|| "{ i8 }".to_string())
        ),
        Ty::Var(name) => format!("%{}", sanitize_symbol(name)),
    }
}

fn default_value(ty: &Ty) -> String {
    match ty {
        Ty::Int => "0".to_string(),
        Ty::Real => "0.000000e+00".to_string(),
        Ty::Bool => "0".to_string(),
        Ty::HttpServerConfig
        | Ty::HttpListener
        | Ty::HttpRequest
        | Ty::HttpResponse
        | Ty::SqliteConnection
        | Ty::SqliteRow
        | Ty::SqliteValue
        | Ty::Stream(_) => "null".to_string(),
        _ => "zeroinitializer".to_string(),
    }
}
