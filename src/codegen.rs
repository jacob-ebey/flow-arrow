use crate::ast::*;
use crate::module_resolver;
use crate::stdlib::{self, RuntimeSupport};
use std::collections::{BTreeMap, HashMap, HashSet};

pub fn emit_module(module: &Module) -> Result<String, String> {
    let _ = module;
    Ok("declare i32 @flow_unboxed_main(i32, ptr)\n\n\
define i32 @main(i32 %argc, ptr %argv) {\n\
  %exit = call i32 @flow_unboxed_main(i32 %argc, ptr %argv)\n\
  ret i32 %exit\n\
}\n"
    .to_string())
}

pub fn emit_runtime_c(module: &Module) -> Result<String, String> {
    let expanded = module_resolver::expand_stdlib_sources(module)?;
    TypedCodegen::new(&expanded)?.emit()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Ty {
    Unit,
    Int,
    Real,
    Bool,
    Bytes,
    Args,
    Fault,
    Faultable(Box<Ty>),
    Seq(Box<Ty>),
    Tuple(Vec<Ty>),
    OneOf(Vec<Ty>),
    Var(String),
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

struct TypedCodegen<'a> {
    module: &'a Module,
    temp: usize,
    callables: HashMap<String, &'a Callable>,
    signatures: HashMap<String, Signature>,
    stdlib_names: HashMap<String, String>,
    types: TypeRegistry,
}

impl<'a> TypedCodegen<'a> {
    fn new(module: &'a Module) -> Result<Self, String> {
        let mut codegen = Self {
            module,
            temp: 0,
            callables: HashMap::new(),
            signatures: HashMap::new(),
            stdlib_names: HashMap::new(),
            types: TypeRegistry::default(),
        };
        codegen.collect_imports();
        codegen.collect_callables()?;
        Ok(codegen)
    }

    fn emit(mut self) -> Result<String, String> {
        let mut bodies = String::new();
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

        for decl in &self.module.declarations {
            match decl {
                Decl::Node(callable) => self.emit_callable(&mut bodies, callable, false)?,
                Decl::Program(callable) => self.emit_callable(&mut bodies, callable, true)?,
                Decl::Import(_) => {}
            }
        }

        let mut out = String::new();
        emit_preamble(&mut out);
        out.push_str(&self.types.emit_typedefs());
        out.push_str(&self.types.emit_helpers());
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
                        if symbol.kind == stdlib::SymbolKind::Node
                            && symbol.runtime != RuntimeSupport::Unsupported
                        {
                            self.stdlib_names.insert(
                                format!("{alias}.{}", symbol.name),
                                symbol.name.to_string(),
                            );
                        }
                    }
                }
                ImportClause::Items(items) => {
                    for item in items {
                        if let Some(symbol) = stdlib::find_export(module, &item.name) {
                            if symbol.kind == stdlib::SymbolKind::Node
                                && symbol.runtime != RuntimeSupport::Unsupported
                            {
                                self.stdlib_names.insert(
                                    item.alias.as_deref().unwrap_or(&item.name).to_string(),
                                    symbol.name.to_string(),
                                );
                            }
                        }
                    }
                }
            }
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
            .map(|port| parse_type(&port.ty))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match types.len() {
            0 => Ty::Unit,
            1 => types.remove(0),
            _ => Ty::Tuple(types),
        })
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
                let ty = parse_type(&port.ty)?;
                let c_ty = self.types.c_type(&ty);
                let var = c_ident(&port.name);
                out.push_str(&format!("  {c_ty} {var} = input;\n"));
                env.insert(port.name.clone(), Value { code: var, ty });
            }
            ports => {
                for (index, port) in ports.iter().enumerate() {
                    let ty = parse_type(&port.ty)?;
                    let c_ty = self.types.c_type(&ty);
                    let var = c_ident(&port.name);
                    out.push_str(&format!("  {c_ty} {var} = input.f{index};\n"));
                    env.insert(port.name.clone(), Value { code: var, ty });
                }
            }
        }

        for chain in &callable.chains {
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
            [output] => env
                .get(&output.name)
                .cloned()
                .ok_or_else(|| format!("output `{}` is never bound", output.name)),
            outputs => {
                let mut values = Vec::new();
                for output in outputs {
                    values.push(
                        env.get(&output.name)
                            .cloned()
                            .ok_or_else(|| format!("output `{}` is never bound", output.name))?,
                    );
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
        }
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
        if parse_type(&left_port.ty)? != Ty::Seq(Box::new(Ty::Real))
            || parse_type(&right_port.ty)? != Ty::Seq(Box::new(Ty::Real))
            || parse_type(&score_port.ty)? != Ty::Real
            || parse_type(&out_left.ty)? != Ty::Seq(Box::new(Ty::Real))
            || parse_type(&out_right.ty)? != Ty::Seq(Box::new(Ty::Real))
            || parse_type(&out_score.ty)? != Ty::Real
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
            if let [Stage::Endpoint(Endpoint::Name(name))] = stages {
                if self.is_add(name) {
                    let Endpoint::Tuple(items) = &chain.source else {
                        return Ok(false);
                    };
                    let [Endpoint::Variable(left), Endpoint::Variable(right)] = items.as_slice()
                    else {
                        return Ok(false);
                    };
                    additions.insert(binding.to_string(), (left.clone(), right.clone()));
                    continue;
                }
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
        let mut value = self.emit_endpoint(out, &chain.source, env)?;
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Endpoint(Endpoint::Variable(name)) if is_last => {
                    if env.insert(name.clone(), value.clone()).is_some() {
                        return Err(format!("value `{name}` is bound more than once"));
                    }
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value = self.emit_call(out, name, value.clone())?;
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
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
            }
        }
        Ok(())
    }

    fn emit_endpoint(
        &mut self,
        out: &mut String,
        endpoint: &Endpoint,
        env: &HashMap<String, Value>,
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
                let values = items
                    .iter()
                    .map(|item| self.emit_endpoint(out, item, env))
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
                    return Err("empty sequence literals need a type context".to_string());
                }
                let values = items
                    .iter()
                    .map(|item| self.emit_endpoint(out, item, env))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut item_ty = values[0].ty.clone();
                for value in values.iter().skip(1) {
                    item_ty = sequence_item_type(&item_ty, &value.ty)?;
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
            out.push_str(&format!("    {target}.is_fault = false;\n"));
            self.emit_assign_call_plain(
                out,
                &format!("{target}.value"),
                output_inner,
                name,
                &format!("{input}.value"),
                input_inner,
            )?;
            out.push_str("  }\n");
            return Ok(());
        }
        if let (Some(unwrapped_input), Ty::Faultable(output_inner)) =
            (unwrap_faultable_tuple(input_ty), output_ty)
        {
            let unwrapped_c_ty = self.types.c_type(&unwrapped_input);
            let unwrapped = self.next_temp();
            out.push_str(&format!("  {target}.is_fault = false;\n"));
            if let Ty::Tuple(items) = input_ty {
                for (index, item) in items.iter().enumerate() {
                    if matches!(item, Ty::Faultable(_)) {
                        out.push_str(&format!("  if (!{target}.is_fault && {input}.f{index}.is_fault) {{ {target}.is_fault = true; {target}.fault = {input}.f{index}.fault; }}\n"));
                    }
                }
                out.push_str(&format!("  if (!{target}.is_fault) {{\n"));
                out.push_str(&format!("    {unwrapped_c_ty} {unwrapped};\n"));
                for (index, item) in items.iter().enumerate() {
                    if matches!(item, Ty::Faultable(_)) {
                        out.push_str(&format!(
                            "    {unwrapped}.f{index} = {input}.f{index}.value;\n"
                        ));
                    } else {
                        out.push_str(&format!("    {unwrapped}.f{index} = {input}.f{index};\n"));
                    }
                }
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
            let Ty::Seq(_) = inner.as_ref() else {
                return Err(format!("`map {name}` expected Seq input"));
            };
            let inner_value = Value {
                code: format!("{}.value", input.code),
                ty: inner.as_ref().clone(),
            };
            let mapped_item_ty = match inner.as_ref() {
                Ty::Seq(item_ty) => self.call_output_type(name, item_ty)?,
                _ => unreachable!(),
            };
            let mapped_seq_ty = Ty::Seq(Box::new(mapped_item_ty));
            let output_ty = Ty::Faultable(Box::new(mapped_seq_ty));
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
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`map {name}` expected Seq input"));
        };
        let output_item_ty = self.call_output_type(name, &item_ty)?;
        let output_ty = Ty::Seq(Box::new(output_item_ty.clone()));
        let c_ty = self.types.c_type(&output_ty);
        let item_c_ty = self.types.c_type(&output_item_ty);
        let new_fn = self.types.seq_new_name(&output_ty)?;
        let tmp = self.next_temp();
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

    fn emit_filter(&mut self, out: &mut String, name: &str, input: Value) -> Result<Value, String> {
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
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`reduce {op}` expected Seq input"));
        };
        let canonical = self.canonical_name(op);
        if canonical == "add" {
            return self.emit_reduce_add(out, input, *item_ty, identity);
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
            "read_stdin" => out.push_str(&format!("  {target} = fa_read_stdin();\n")),
            "write_stdout" => {
                out.push_str(&format!("  {target} = fa_write_bytes(stdout, {input});\n"))
            }
            "write_stderr" => {
                out.push_str(&format!("  {target} = fa_write_bytes(stderr, {input});\n"))
            }
            "split_lines" => out.push_str(&format!("  {target} = fa_split_lines({input});\n")),
            "trim" => out.push_str(&format!("  {target} = fa_trim({input});\n")),
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
            "neg" | "abs" | "sqrt" => {
                out.push_str(&format!(
                    "  {target} = {};\n",
                    numeric_unary_expr(name, input, output_ty)
                ));
            }
            "eq" | "lt" | "gt" | "le" | "ge" => {
                out.push_str(&format!("  {target} = {};\n", compare_expr(name, input)));
            }
            "not_empty" => out.push_str(&format!("  {target} = {input}.len > 0;\n")),
            "is_empty" => out.push_str(&format!("  {target} = {input}.len == 0;\n")),
            "and" => out.push_str(&format!("  {target} = {input}.f0 && {input}.f1;\n")),
            "or" => out.push_str(&format!("  {target} = {input}.f0 || {input}.f1;\n")),
            "xor" => out.push_str(&format!("  {target} = {input}.f0 != {input}.f1;\n")),
            "not" => out.push_str(&format!("  {target} = !{input};\n")),
            "all" => self.emit_all_any(out, target, input, true),
            "any" => self.emit_all_any(out, target, input, false),
            "has_faults" => out.push_str(&format!("  {target} = {input}.count > 0;\n")),
            "format_faults" => out.push_str(&format!("  {target} = fa_format_faults({input});\n")),
            "select" => out.push_str(&format!(
                "  {target} = {input}.f0 ? {input}.f1 : {input}.f2;\n"
            )),
            "length" => out.push_str(&format!("  {target} = (int64_t){input}.count;\n")),
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
            "head" => self.emit_head(out, target, output_ty, input, input_ty)?,
            "range_step" => out.push_str(&format!(
                "  {target} = fa_range_step({input}.f0, {input}.f1, {input}.f2);\n"
            )),
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

    fn call_output_type(&self, name: &str, input: &Ty) -> Result<Ty, String> {
        if let Some(signature) = self.signatures.get(name) {
            if matches!(input, Ty::Faultable(_)) && !matches!(signature.output, Ty::Faultable(_)) {
                return Ok(Ty::Faultable(Box::new(signature.output.clone())));
            }
            return Ok(signature.output.clone());
        }
        let canonical = self.canonical_name(name);
        builtin_output_type(&canonical, input)
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

#[derive(Default)]
struct TypeRegistry {
    types: BTreeMap<String, Ty>,
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
        }
    }

    fn seq_new_name(&mut self, ty: &Ty) -> Result<String, String> {
        let Ty::Seq(_) = ty else {
            return Err(format!("expected sequence type, found `{ty}`"));
        };
        Ok(format!("{}_new", self.c_type(ty)))
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
        "read_stdin" => Ok(Ty::Bytes),
        "write_stdout" | "write_stderr" => Ok(Ty::Int),
        "split_lines" | "split_on" => Ok(Ty::Seq(Box::new(Ty::Bytes))),
        "trim" | "join_bytes" | "codes_to_bytes" | "format_faults" => Ok(Ty::Bytes),
        "concat_bytes" => match input {
            Ty::Seq(item) if matches!(item.as_ref(), Ty::Faultable(inner) if inner.as_ref() == &Ty::Bytes) => {
                Ok(Ty::Faultable(Box::new(Ty::Bytes)))
            }
            _ => Ok(Ty::Bytes),
        },
        "strip_prefix" | "strip_suffix" => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
        "bytes_to_codes" | "range_step" => Ok(Ty::Seq(Box::new(Ty::Int))),
        "byte_length" | "length" | "inner_length" | "bit_and" | "bit_or" | "bit_xor"
        | "bit_shl" | "bit_shr" => Ok(Ty::Int),
        "parse_int" => Ok(Ty::Faultable(Box::new(Ty::Int))),
        "parse_real" => Ok(Ty::Faultable(Box::new(Ty::Real))),
        "format_int" | "format_real" => match input {
            Ty::Faultable(_) => Ok(Ty::Faultable(Box::new(Ty::Bytes))),
            _ => Ok(Ty::Bytes),
        },
        "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => numeric_binary_output(input),
        "neg" | "abs" => Ok(input.clone()),
        "sqrt" => Ok(Ty::Real),
        "eq" | "lt" | "gt" | "le" | "ge" | "not_empty" | "is_empty" | "and" | "or" | "xor"
        | "not" | "all" | "any" | "has_faults" => Ok(Ty::Bool),
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
        "shift_right" => {
            let Ty::Tuple(items) = input else {
                return Err("shift_right expected tuple input".to_string());
            };
            items
                .first()
                .cloned()
                .ok_or_else(|| "shift_right expected sequence input".to_string())
        }
        "head" => {
            let Ty::Seq(item) = input else {
                return Err("head expected sequence input".to_string());
            };
            Ok(Ty::Faultable(item.clone()))
        }
        other => Err(format!("unsupported builtin `{other}`")),
    }
}

fn sequence_item_type(left: &Ty, right: &Ty) -> Result<Ty, String> {
    if left == right {
        return Ok(left.clone());
    }
    match (left, right) {
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
            other => other.clone(),
        })
        .collect::<Vec<_>>();
    saw_faultable.then_some(Ty::Tuple(unwrapped))
}

fn is_predefined_type_name(name: &str) -> bool {
    matches!(
        name,
        "FaSeq_Bytes"
            | "FaSeq_Int"
            | "FaSeq_Fault"
            | "FaFaultable_Int"
            | "FaFaultable_Real"
            | "FaFaultable_Bytes"
    )
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
        Stage::Endpoint(Endpoint::Variable(name)) if name == output => Some(stages),
        _ => None,
    }
}

fn final_variable(chain: &Chain) -> Option<&str> {
    match chain.stages.last()? {
        Stage::Endpoint(Endpoint::Variable(name)) => Some(name),
        _ => None,
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
            if !self.eat(')') {
                loop {
                    items.push(self.parse()?);
                    if self.eat(',') {
                        continue;
                    }
                    self.expect(')')?;
                    break;
                }
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
        Ok(match name.as_str() {
            "Unit" | "void" => Ty::Unit,
            "Int" | "i8" | "i16" | "i32" | "i64" => Ty::Int,
            "Real" | "f16" | "float" | "double" => Ty::Real,
            "Bool" | "i1" => Ty::Bool,
            "Bytes" | "ptr" => Ty::Bytes,
            "Args" => Ty::Args,
            "Fault" => Ty::Fault,
            other => Ty::Var(other.to_string()),
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

fn type_suffix(ty: &Ty) -> String {
    match ty {
        Ty::Unit => "Unit".to_string(),
        Ty::Int => "Int".to_string(),
        Ty::Real => "Real".to_string(),
        Ty::Bool => "Bool".to_string(),
        Ty::Bytes => "Bytes".to_string(),
        Ty::Args => "Args".to_string(),
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
    }
}

fn type_depth(ty: &Ty) -> usize {
    match ty {
        Ty::Seq(item) | Ty::Faultable(item) => 1 + type_depth(item),
        Ty::Tuple(items) | Ty::OneOf(items) => 1 + items.iter().map(type_depth).max().unwrap_or(0),
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

        assert_eq!(
            llvm,
            "declare i32 @flow_unboxed_main(i32, ptr)\n\n\
define i32 @main(i32 %argc, ptr %argv) {\n\
  %exit = call i32 @flow_unboxed_main(i32 %argc, ptr %argv)\n\
  ret i32 %exit\n\
}\n"
        );
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
}
