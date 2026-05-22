use super::{
    Ty, TypedCodegen, assignable_output_ty, binding_target_is_discard, builtin_output_type_plain,
    common_assignable_output_ty, endpoint_contains_empty_seq, format_binding_target_for_error,
    format_match_target, sequence_item_type,
};
use crate::ast::{
    BindingTarget, Callable, Chain, Decl, Endpoint, MatchArm, MatchGuard, MatchTarget, Stage,
};
use std::collections::HashMap;

pub(super) fn emit_module(codegen: TypedCodegen<'_>) -> Result<String, String> {
    TypeScriptCodegen::new(codegen).emit()
}

#[derive(Debug, Clone)]
struct TsValue {
    code: String,
    ty: Ty,
}

struct TypeScriptCodegen<'a> {
    codegen: TypedCodegen<'a>,
    temp: usize,
}

impl<'a> TypeScriptCodegen<'a> {
    fn new(codegen: TypedCodegen<'a>) -> Self {
        Self { codegen, temp: 0 }
    }

    fn emit(mut self) -> Result<String, String> {
        let mut out = String::new();
        out.push_str(TS_PRELUDE);

        let callables = self
            .codegen
            .module
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                Decl::Node(callable) => Some((callable.clone(), false)),
                Decl::Program(callable) => Some((callable.clone(), true)),
                Decl::TypeAlias(_) | Decl::Import(_) => None,
            })
            .collect::<Vec<_>>();
        let has_program_main = callables
            .iter()
            .any(|(callable, is_program)| *is_program && callable.name == "main");

        for (callable, is_program) in &callables {
            self.emit_callable(&mut out, callable, *is_program)?;
        }

        if has_program_main {
            out.push_str(
                "\nconst __flowarrow_main_url = typeof process !== \"undefined\" && process.argv?.[1]\n  ? new URL(process.argv[1], \"file:\").href\n  : \"\";\n\
if (import.meta.url === __flowarrow_main_url) {\n  const __flowarrow_result = main({ argv: process.argv.slice(2) });\n  const __flowarrow_exit = faExitCode(__flowarrow_result);\n  process.exit(Number(__flowarrow_exit));\n}\n",
            );
        }

        Ok(out)
    }

    fn emit_callable(
        &mut self,
        out: &mut String,
        callable: &Callable,
        is_program: bool,
    ) -> Result<(), String> {
        self.temp = 0;
        let signature = self
            .codegen
            .signatures
            .get(&callable.name)
            .cloned()
            .ok_or_else(|| format!("missing signature for `{}`", callable.name))?;
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
        let return_ty = ts_type(&signature.output);

        match callable.inputs.as_slice() {
            [] => out.push_str(&format!("\n{export}function {fn_name}(): {return_ty} {{\n")),
            [port] => out.push_str(&format!(
                "\n{export}function {fn_name}({}: {}): {return_ty} {{\n",
                ts_ident(&port.name),
                ts_type(&self.codegen.parse_declared_type(&port.ty)?)
            )),
            _ => out.push_str(&format!(
                "\n{export}function {fn_name}(input: {}): {return_ty} {{\n",
                ts_type(&signature.input)
            )),
        }

        let mut env = HashMap::new();
        match callable.inputs.as_slice() {
            [] => {}
            [port] => {
                let ty = self.codegen.parse_declared_type(&port.ty)?;
                env.insert(
                    port.name.clone(),
                    TsValue {
                        code: ts_ident(&port.name),
                        ty,
                    },
                );
            }
            ports => {
                for (index, port) in ports.iter().enumerate() {
                    let ty = self.codegen.parse_declared_type(&port.ty)?;
                    env.insert(
                        port.name.clone(),
                        TsValue {
                            code: format!("input.f{index}"),
                            ty,
                        },
                    );
                }
            }
        }

        for chain in &callable.chains {
            self.emit_chain(out, chain, &mut env, "  ")?;
        }

        let result = self.emit_outputs(out, callable, &env, "  ")?;
        let result = self.coerce_value(out, result, &signature.output, "  ")?;
        out.push_str(&format!("  return {};\n}}\n", result.code));
        Ok(())
    }

    fn emit_outputs(
        &mut self,
        out: &mut String,
        callable: &Callable,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        match callable.outputs.as_slice() {
            [] => Ok(TsValue {
                code: "undefined".to_string(),
                ty: Ty::Unit,
            }),
            [output] => env
                .get(&output.name)
                .cloned()
                .ok_or_else(|| format!("output `{}` is never bound", output.name)),
            outputs => {
                let fields = outputs
                    .iter()
                    .enumerate()
                    .map(|(index, output)| {
                        env.get(&output.name)
                            .map(|value| format!("f{index}: {}", value.code))
                            .ok_or_else(|| format!("output `{}` is never bound", output.name))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let ty = Ty::Tuple(
                    outputs
                        .iter()
                        .map(|output| self.codegen.parse_declared_type(&output.ty))
                        .collect::<Result<Vec<_>, _>>()?,
                );
                let tmp = self.next_temp();
                out.push_str(&format!(
                    "{indent}const {tmp}: {} = {{ {} }};\n",
                    ts_type(&ty),
                    fields.join(", ")
                ));
                Ok(TsValue { code: tmp, ty })
            }
        }
    }

    fn emit_chain(
        &mut self,
        out: &mut String,
        chain: &Chain,
        env: &mut HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<(), String> {
        let mut value = if endpoint_contains_empty_seq(&chain.source) {
            if let Some(Stage::Endpoint(Endpoint::Name(name))) = chain.stages.first() {
                let actual = self.endpoint_type(&chain.source, env)?;
                let expected = self.codegen.call_input_type_for_value(name, &actual)?;
                self.emit_endpoint_expected(out, &chain.source, env, Some(&expected), indent)?
            } else {
                self.emit_endpoint(out, &chain.source, env, indent)?
            }
        } else {
            self.emit_endpoint(out, &chain.source, env, indent)?
        };

        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Bind(target) if is_last => {
                    self.bind_target(out, target, value.clone(), env, indent)?;
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value = self.emit_call(out, name, value, indent)?;
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
                }
                Stage::Bind(_) => {
                    return Err("binding targets may only appear as final stages".to_string());
                }
                Stage::Map(name) => {
                    value = self.emit_map(out, name, value, indent)?;
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let (ok_value, fault_value) =
                        self.emit_fault_map(out, node, value.clone(), indent)?;
                    if env.insert(ok.clone(), ok_value).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_value).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                Stage::Filter(name) => {
                    value = self.emit_filter(out, name, value, indent)?;
                }
                Stage::Repeat { count, node } => {
                    let count = self.emit_endpoint(out, count, env, indent)?;
                    value = self.emit_repeat(out, node, value, count, indent)?;
                }
                Stage::Reduce { op, identity } => {
                    let identity = self.emit_endpoint(out, identity, env, indent)?;
                    value = self.emit_reduce(out, op, value, identity, indent)?;
                }
                Stage::Scan { op, identity } => {
                    let identity = self.emit_endpoint(out, identity, env, indent)?;
                    value = self.emit_scan(out, op, value, identity, indent)?;
                }
                Stage::Match { arms } => {
                    value = self.emit_match(out, arms, value, env, indent)?;
                }
            }
        }
        Ok(())
    }

    fn emit_endpoint(
        &mut self,
        out: &mut String,
        endpoint: &Endpoint,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        self.emit_endpoint_expected(out, endpoint, env, None, indent)
    }

    fn emit_endpoint_expected(
        &mut self,
        out: &mut String,
        endpoint: &Endpoint,
        env: &HashMap<String, TsValue>,
        expected: Option<&Ty>,
        indent: &str,
    ) -> Result<TsValue, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Int(value) => Ok(TsValue {
                code: format!("{value}n"),
                ty: Ty::Int,
            }),
            Endpoint::Real(value) => Ok(TsValue {
                code: format!("{value:.17e}"),
                ty: Ty::Real,
            }),
            Endpoint::Bool(value) => Ok(TsValue {
                code: value.to_string(),
                ty: Ty::Bool,
            }),
            Endpoint::String(value) => Ok(TsValue {
                code: ts_string(value),
                ty: Ty::Bytes,
            }),
            Endpoint::Unit => Ok(TsValue {
                code: "undefined".to_string(),
                ty: Ty::Unit,
            }),
            Endpoint::Tuple(items) => {
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
                let ty = expected.cloned().unwrap_or_else(|| {
                    Ty::Tuple(values.iter().map(|value| value.ty.clone()).collect())
                });
                let tmp = self.next_temp();
                let fields = values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| format!("f{index}: {}", value.code))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "{indent}const {tmp}: {} = {{ {fields} }};\n",
                    ts_type(&ty)
                ));
                Ok(TsValue { code: tmp, ty })
            }
            Endpoint::Seq(items) => {
                if items.is_empty() {
                    let Some(seq_ty @ Ty::Seq(_)) = expected else {
                        return Err("empty sequence literals need a type context".to_string());
                    };
                    return Ok(TsValue {
                        code: "[]".to_string(),
                        ty: seq_ty.clone(),
                    });
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
                Ok(TsValue { code, ty })
            }
            Endpoint::Eval { source, stages } => {
                let mut value = self.emit_endpoint(out, source, env, indent)?;
                for stage in stages {
                    value = self.emit_inline_stage(out, stage, value, env, indent)?;
                }
                Ok(value)
            }
        }
    }

    fn emit_inline_stage(
        &mut self,
        out: &mut String,
        stage: &Stage,
        value: TsValue,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        match stage {
            Stage::Endpoint(Endpoint::Name(name)) => self.emit_call(out, name, value, indent),
            Stage::Endpoint(Endpoint::Variable(_)) | Stage::Bind(_) => {
                Err("inline evaluations cannot bind values".to_string())
            }
            Stage::Endpoint(_) => {
                Err("non-name endpoints may only appear as inline evaluation sources".to_string())
            }
            Stage::Map(name) => self.emit_map(out, name, value, indent),
            Stage::FaultMap { .. } => Err("inline evaluations cannot use `fault map`".to_string()),
            Stage::Filter(name) => self.emit_filter(out, name, value, indent),
            Stage::Repeat { count, node } => {
                let count = self.emit_endpoint(out, count, env, indent)?;
                self.emit_repeat(out, node, value, count, indent)
            }
            Stage::Reduce { op, identity } => {
                let identity = self.emit_endpoint(out, identity, env, indent)?;
                self.emit_reduce(out, op, value, identity, indent)
            }
            Stage::Scan { op, identity } => {
                let identity = self.emit_endpoint(out, identity, env, indent)?;
                self.emit_scan(out, op, value, identity, indent)
            }
            Stage::Match { arms } => self.emit_match(out, arms, value, env, indent),
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
                if env.insert(name.clone(), value).is_some() {
                    return Err(format!("value `{name}` is bound more than once"));
                }
                Ok(())
            }
            BindingTarget::Tuple(targets) => match value.ty.clone() {
                Ty::Tuple(items) if items.len() == targets.len() => {
                    for (index, (target, ty)) in targets.iter().zip(items.iter()).enumerate() {
                        if binding_target_is_discard(target) {
                            continue;
                        }
                        self.bind_target(
                            out,
                            target,
                            TsValue {
                                code: format!("{}.f{index}", value.code),
                                ty: ty.clone(),
                            },
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
                            "{indent}const {tmp}: {} = {}.is_fault ? faFault({}.fault) : faOk({}.value.f{index});\n",
                            ts_type(&projected_ty),
                            value.code,
                            value.code,
                            value.code
                        ));
                        self.bind_target(
                            out,
                            target,
                            TsValue {
                                code: tmp,
                                ty: projected_ty,
                            },
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
            },
        }
    }

    fn emit_call(
        &mut self,
        out: &mut String,
        name: &str,
        mut input: TsValue,
        indent: &str,
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
            let tmp = self.next_temp();
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
            let plain_input = TsValue {
                code: format!("{}.value", input.code),
                ty: input_inner.as_ref().clone(),
            };
            let called = self.emit_plain_call(
                out,
                name,
                plain_input,
                &plain_output,
                &(indent.to_string() + "  "),
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
            return Ok(TsValue {
                code: tmp,
                ty: output_ty,
            });
        }

        if let Ty::Faultable(output_inner) = &output_ty {
            let plain_output = self.plain_output_type(name, &input.ty)?;
            let called = self.emit_plain_call(out, name, input.clone(), &plain_output, indent)?;
            if plain_output == output_ty {
                return Ok(called);
            }
            if &plain_output == output_inner.as_ref() {
                return Ok(TsValue {
                    code: format!("faOk({})", called.code),
                    ty: output_ty,
                });
            }
        }

        self.emit_plain_call(out, name, input, &output_ty, indent)
    }

    fn emit_plain_call(
        &mut self,
        out: &mut String,
        name: &str,
        input: TsValue,
        output_ty: &Ty,
        indent: &str,
    ) -> Result<TsValue, String> {
        let expr = if self.codegen.callables.contains_key(name) {
            format!("{}({})", ts_ident(name), input.code)
        } else {
            self.emit_builtin_expr(&self.codegen.canonical_name(name), &input, output_ty)?
        };
        if expression_is_simple(&expr) {
            Ok(TsValue {
                code: expr,
                ty: output_ty.clone(),
            })
        } else {
            let tmp = self.next_temp();
            out.push_str(&format!(
                "{indent}const {tmp}: {} = {expr};\n",
                ts_type(output_ty)
            ));
            Ok(TsValue {
                code: tmp,
                ty: output_ty.clone(),
            })
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
            "contains" => format!("{}.f0.includes({}.f1)", input.code, input.code),
            "starts_with" => format!("{}.f0.startsWith({}.f1)", input.code, input.code),
            "ends_with" => format!("{}.f0.endsWith({}.f1)", input.code, input.code),
            "index_of" => format!("BigInt({}.f0.indexOf({}.f1))", input.code, input.code),
            "last_index_of" => format!("BigInt({}.f0.lastIndexOf({}.f1))", input.code, input.code),
            "slice" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int, Ty::Int])) =>
            {
                format!(
                    "{}.f0.slice(Number({}.f1), Number({}.f2))",
                    input.code, input.code, input.code
                )
            }
            "take" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) =>
            {
                format!("{}.f0.slice(0, Number({}.f1))", input.code, input.code)
            }
            "drop" if matches!(&input.ty, Ty::Tuple(items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) =>
            {
                format!("{}.f0.slice(Number({}.f1))", input.code, input.code)
            }
            "replace" => format!(
                "{}.f0.split({}.f1).join({}.f2)",
                input.code, input.code, input.code
            ),
            "repeat_bytes" => format!("{}.f0.repeat(Number({}.f1))", input.code, input.code),
            "ascii_lower" => format!("{}.toLowerCase()", input.code),
            "ascii_upper" => format!("{}.toUpperCase()", input.code),
            "split_on" => format!("{}.f0.split({}.f1)", input.code, input.code),
            "strip_prefix" => format!("faStripPrefix({})", input.code),
            "strip_suffix" => format!("faStripSuffix({})", input.code),
            "bytes_to_codes" => {
                format!("Array.from({}, ch => BigInt(ch.charCodeAt(0)))", input.code)
            }
            "codes_to_bytes" => format!("String.fromCharCode(...{}.map(Number))", input.code),
            "byte_length" => format!("BigInt({}.length)", input.code),
            "concat_bytes" => format!("faConcatBytes({})", input.code),
            "join_bytes" => format!("{}.f1.join({}.f0)", input.code, input.code),
            "parse_int" => format!("faParseInt({})", input.code),
            "parse_real" => format!("faParseReal({})", input.code),
            "from_int" => format!("Number({})", input.code),
            "format_int" | "format_real" => format!("{}.toString()", input.code),
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                ts_numeric_binary_expr(name, &input.code, output_ty)
            }
            "neg" => format!("(-{})", input.code),
            "abs" => format!("({0} < 0 ? -{0} : {0})", input.code),
            "sqrt" => format!("Math.sqrt({})", input.code),
            "exp" => format!("Math.exp({})", input.code),
            "sin" => format!("Math.sin({})", input.code),
            "cos" => format!("Math.cos({})", input.code),
            "eq" => format!("({}.f0 === {}.f1)", input.code, input.code),
            "lt" => format!("({}.f0 < {}.f1)", input.code, input.code),
            "gt" => format!("({}.f0 > {}.f1)", input.code, input.code),
            "le" => format!("({}.f0 <= {}.f1)", input.code, input.code),
            "ge" => format!("({}.f0 >= {}.f1)", input.code, input.code),
            "not_empty" => format!("({}.length > 0)", input.code),
            "is_empty" => match input.ty {
                Ty::Bytes => format!("({}.length === 0)", input.code),
                Ty::Seq(_) => format!("({}.length === 0)", input.code),
                _ => return Err("is_empty expected Bytes or Seq input".to_string()),
            },
            "and" => format!("({}.f0 && {}.f1)", input.code, input.code),
            "or" => format!("({}.f0 || {}.f1)", input.code, input.code),
            "xor" => format!("({}.f0 !== {}.f1)", input.code, input.code),
            "not" => format!("(!{})", input.code),
            "all" => format!("{}.every(Boolean)", input.code),
            "any" => format!("{}.some(Boolean)", input.code),
            "has_faults" => format!("({}.length > 0)", input.code),
            "format_faults" => format!("{}.map(f => f.message).join(\"\\n\")", input.code),
            "expect" => format!("faExpect({})", input.code),
            "collect" => format!("faCollect({})", input.code),
            "select" => format!(
                "({}.f0 ? {}.f1 : {}.f2)",
                input.code, input.code, input.code
            ),
            "length" => format!("BigInt({}.length)", input.code),
            "inner_length" => format!("BigInt({}[0]?.length ?? 0)", input.code),
            "first" => format!("{}.f0", input.code),
            "second" => format!("{}.f1", input.code),
            "swap" => format!("{{ f0: {}.f1, f1: {}.f0 }}", input.code, input.code),
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
            "take" => format!("{}.f0.slice(0, Number({}.f1))", input.code, input.code),
            "drop" => format!("{}.f0.slice(Number({}.f1))", input.code, input.code),
            "fill" => format!("faFill({})", input.code),
            "slice" => format!(
                "{}.f0.slice(Number({}.f1), Number({}.f2))",
                input.code, input.code, input.code
            ),
            "last" => format!("faLast({})", input.code),
            "get" => format!("faGet({})", input.code),
            "get_or" => format!("faGetOr({})", input.code),
            "at" => format!("faAt({})", input.code),
            "append" => format!("[...{}.f0, {}.f1]", input.code, input.code),
            "set" => format!("faSet({})", input.code),
            "concat" => format!("[...{}.f0, ...{}.f1]", input.code, input.code),
            "range_step" => format!("faRangeStep({})", input.code),
            "bit_and" => format!("({}.f0 & {}.f1)", input.code, input.code),
            "bit_or" => format!("({}.f0 | {}.f1)", input.code, input.code),
            "bit_xor" => format!("({}.f0 ^ {}.f1)", input.code, input.code),
            "bit_shl" => format!("({}.f0 << {}.f1)", input.code, input.code),
            "bit_shr" => format!("({}.f0 >> {}.f1)", input.code, input.code),
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
        let tmp = self.next_temp();
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
            TsValue {
                code: item.clone(),
                ty: item_ty.as_ref().clone(),
            },
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
        Ok(TsValue {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_filter(
        &mut self,
        out: &mut String,
        name: &str,
        input: TsValue,
        indent: &str,
    ) -> Result<TsValue, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`filter {name}` expected Seq input"));
        };
        let tmp = self.next_temp();
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
            TsValue {
                code: item.clone(),
                ty: item_ty.as_ref().clone(),
            },
            &(indent.to_string() + "  "),
        )?;
        out.push_str(&format!(
            "{indent}  if ({}) {tmp}.push({item});\n",
            keep.code
        ));
        out.push_str(&format!("{indent}}}\n"));
        Ok(TsValue {
            code: tmp,
            ty: input.ty,
        })
    }

    fn emit_reduce(
        &mut self,
        out: &mut String,
        op: &str,
        input: TsValue,
        identity: TsValue,
        indent: &str,
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
        let tmp = self.next_temp();
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
            let pair = TsValue {
                code: format!("{{ f0: {acc}, f1: {item}.value }}"),
                ty: pair_ty,
            };
            let reduced = self.emit_call(out, op, pair, &(body_indent.clone() + "  "))?;
            out.push_str(&format!("{body_indent}  {acc} = {};\n", reduced.code));
            out.push_str(&format!("{body_indent}}}\n"));
            out.push_str(&format!(
                "{body_indent}{tmp} = {fault} ? faFault({fault}) : faOk({acc});\n"
            ));
        } else {
            out.push_str(&format!("{body_indent}for (const {item} of {source}) {{\n"));
            let pair_ty = Ty::Tuple(vec![plain_item_ty.clone(), plain_item_ty.clone()]);
            let pair = TsValue {
                code: format!("{{ f0: {acc}, f1: {item} }}"),
                ty: pair_ty,
            };
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
        Ok(TsValue {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_scan(
        &mut self,
        out: &mut String,
        op: &str,
        input: TsValue,
        identity: TsValue,
        indent: &str,
    ) -> Result<TsValue, String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`scan {op}` expected Seq input"));
        };
        let tmp = self.next_temp();
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
        let pair = TsValue {
            code: format!("{{ f0: {acc}, f1: {item} }}"),
            ty: Ty::Tuple(vec![item_ty.as_ref().clone(), item_ty.as_ref().clone()]),
        };
        let scanned = self.emit_call(out, op, pair, &(indent.to_string() + "  "))?;
        out.push_str(&format!("{indent}  {acc} = {};\n", scanned.code));
        out.push_str(&format!("{indent}  {tmp}.push({acc});\n"));
        out.push_str(&format!("{indent}}}\n"));
        Ok(TsValue {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_repeat(
        &mut self,
        out: &mut String,
        node: &str,
        input: TsValue,
        count: TsValue,
        indent: &str,
    ) -> Result<TsValue, String> {
        let tmp = self.next_temp();
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
            TsValue {
                code: tmp.clone(),
                ty: input.ty.clone(),
            },
            &(indent.to_string() + "  "),
        )?;
        out.push_str(&format!("{indent}  {tmp} = {};\n", next.code));
        out.push_str(&format!("{indent}}}\n"));
        Ok(TsValue {
            code: tmp,
            ty: input.ty,
        })
    }

    fn emit_match(
        &mut self,
        out: &mut String,
        arms: &[MatchArm],
        subject: TsValue,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        let output_ty = self.match_output_type(arms, &subject.ty, env)?;
        let tmp = self.next_temp();
        out.push_str(&format!("{indent}let {tmp}: {};\n", ts_type(&output_ty)));
        for (index, arm) in arms.iter().enumerate() {
            match &arm.guard {
                MatchGuard::Fallback => {
                    if index + 1 != arms.len() {
                        return Err("`match` fallback arm must be last".to_string());
                    }
                    if index == 0 {
                        out.push_str(&format!("{indent}{{\n"));
                    } else {
                        out.push_str(&format!("{indent}else {{\n"));
                    }
                }
                MatchGuard::Call { node, args } => {
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
                MatchGuard::Fallback => format!("{indent}  "),
                MatchGuard::Call { .. } => format!("{indent}    "),
            };
            let value =
                self.emit_match_target(out, &arm.target, subject.clone(), env, &arm_indent)?;
            let value = self.coerce_value(out, value, &output_ty, &arm_indent)?;
            out.push_str(&format!("{arm_indent}{tmp} = {};\n", value.code));
            match &arm.guard {
                MatchGuard::Fallback => out.push_str(&format!("{indent}}}\n")),
                MatchGuard::Call { .. } => out.push_str(&format!("{indent}  }}\n")),
            }
        }
        for _ in arms
            .iter()
            .filter(|arm| !matches!(arm.guard, MatchGuard::Fallback))
        {
            out.push_str(&format!("{indent}}}\n"));
        }
        Ok(TsValue {
            code: tmp,
            ty: output_ty,
        })
    }

    fn emit_match_target(
        &mut self,
        out: &mut String,
        target: &MatchTarget,
        subject: TsValue,
        env: &HashMap<String, TsValue>,
        indent: &str,
    ) -> Result<TsValue, String> {
        match target {
            MatchTarget::Node(node) => self.emit_call(out, node, subject, indent),
            MatchTarget::Value(endpoint) => self.emit_endpoint(out, endpoint, env, indent),
        }
    }

    fn emit_match_guard_input(
        &mut self,
        out: &mut String,
        subject: TsValue,
        args: &[Endpoint],
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
        let code = format!(
            "{{ {} }}",
            values
                .iter()
                .enumerate()
                .map(|(index, value)| format!("f{index}: {}", value.code))
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(TsValue { code, ty })
    }

    fn emit_fault_map(
        &mut self,
        out: &mut String,
        node: &str,
        input: TsValue,
        indent: &str,
    ) -> Result<(TsValue, TsValue), String> {
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("`fault map {node}` expected Seq input"));
        };
        let Ty::Faultable(ok_ty) = item_ty.as_ref() else {
            return Err(format!(
                "`fault map {node}` expected Seq[Faultable[V]] input"
            ));
        };
        let ok_tmp = self.next_temp();
        let fault_tmp = self.next_temp();
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
            TsValue {
                code: format!("{item}.value"),
                ty: ok_ty.as_ref().clone(),
            },
            &(indent.to_string() + "    "),
        )?;
        out.push_str(&format!("{indent}    {ok_tmp}.push({});\n", mapped.code));
        out.push_str(&format!("{indent}  }}\n"));
        out.push_str(&format!("{indent}}}\n"));
        Ok((
            TsValue {
                code: ok_tmp,
                ty: ok_seq_ty,
            },
            TsValue {
                code: fault_tmp,
                ty: fault_seq_ty,
            },
        ))
    }

    fn match_output_type(
        &self,
        arms: &[MatchArm],
        subject_ty: &Ty,
        env: &HashMap<String, TsValue>,
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

    fn endpoint_type(
        &self,
        endpoint: &Endpoint,
        env: &HashMap<String, TsValue>,
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
            Endpoint::Eval { source, stages } => {
                let mut value_ty = self.endpoint_type(source, env)?;
                for stage in stages {
                    value_ty = match stage {
                        Stage::Endpoint(Endpoint::Name(name)) => {
                            self.codegen.call_output_type(name, &value_ty)?
                        }
                        Stage::Map(name) => match value_ty {
                            Ty::Seq(item) => {
                                Ty::Seq(Box::new(self.codegen.call_output_type(name, &item)?))
                            }
                            _ => return Err(format!("`map {name}` expected Seq input")),
                        },
                        Stage::Filter(_) => value_ty,
                        Stage::Repeat { node, .. } => {
                            self.codegen.call_output_type(node, &value_ty)?
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
                            self.codegen.call_output_type(op, item_ty)?
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
                            value_ty
                        }
                        Stage::Match { arms } => self.match_output_type(arms, &value_ty, env)?,
                        Stage::Endpoint(_) | Stage::Bind(_) | Stage::FaultMap { .. } => {
                            return Err("unsupported inline evaluation stage".to_string());
                        }
                    };
                }
                Ok(value_ty)
            }
        }
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
            return Ok(TsValue {
                code: format!("faOk({})", value.code),
                ty: expected.clone(),
            });
        }
        let tmp = self.next_temp();
        out.push_str(&format!(
            "{indent}const {tmp}: {} = {};\n",
            ts_type(expected),
            value.code
        ));
        Ok(TsValue {
            code: tmp,
            ty: expected.clone(),
        })
    }

    fn plain_output_type(&self, name: &str, input_ty: &Ty) -> Result<Ty, String> {
        if let Some(signature) = self.codegen.signatures.get(name) {
            Ok(signature.output.clone())
        } else {
            builtin_output_type_plain(&self.codegen.canonical_name(name), input_ty)
        }
    }

    fn next_temp(&mut self) -> String {
        let tmp = format!("t{}", self.temp);
        self.temp += 1;
        tmp
    }
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
                "{}".to_string()
            } else {
                format!(
                    "{{ {} }}",
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| format!("f{index}: {}", ts_type(item)))
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            }
        }
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

fn ts_string(value: &str) -> String {
    format!("{value:?}")
}

fn expression_is_simple(expr: &str) -> bool {
    !expr.contains('\n') && expr.len() < 96
}

fn ts_value_code_is_stable(code: &str) -> bool {
    code.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn ts_numeric_binary_expr(name: &str, input: &str, output_ty: &Ty) -> String {
    match name {
        "add" => format!("({input}.f0 + {input}.f1)"),
        "sub" => format!("({input}.f0 - {input}.f1)"),
        "mul" => format!("({input}.f0 * {input}.f1)"),
        "div" => format!("({input}.f0 / {input}.f1)"),
        "rem" => format!("({input}.f0 % {input}.f1)"),
        "min" => format!("({input}.f0 <= {input}.f1 ? {input}.f0 : {input}.f1)"),
        "max" => format!("({input}.f0 >= {input}.f1 ? {input}.f0 : {input}.f1)"),
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

const TS_PRELUDE: &str = r#"// Generated by FlowArrow. Do not edit by hand.
declare const process: any;

export type FaArgs = { argv: string[] };
export type FaFault = { message: string };
export type FaFaultable<T> = { is_fault: true; fault: FaFault } | { is_fault: false; value: T };
export type FaStream<T> = unknown;
export type FaHttpServerConfig = unknown;
export type FaHttpListener = unknown;
export type FaHttpRequest = unknown;
export type FaHttpResponse = unknown;
export type FaSqliteConnection = unknown;
export type FaSqliteRow = unknown;
export type FaSqliteValue = unknown;

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

function faFlagPresent(input: { f0: string[]; f1: string }): boolean {
  return input.f0.includes(input.f1);
}

function faFlagValue(input: { f0: string[]; f1: string }): FaFaultable<string> {
  const index = input.f0.indexOf(input.f1);
  if (index < 0 || index + 1 >= input.f0.length) return faFaultMessage(`flag_value: missing value for ${input.f1}`);
  return faOk(input.f0[index + 1]);
}

function faStripPrefix(input: { f0: string; f1: string }): FaFaultable<string> {
  return input.f0.startsWith(input.f1) ? faOk(input.f0.slice(input.f1.length)) : faFaultMessage("strip_prefix: prefix not found");
}

function faStripSuffix(input: { f0: string; f1: string }): FaFaultable<string> {
  return input.f0.endsWith(input.f1) ? faOk(input.f0.slice(0, -input.f1.length)) : faFaultMessage("strip_suffix: suffix not found");
}

function faCollect<T>(items: Array<FaFaultable<T>>): FaFaultable<T[]> {
  const out: T[] = [];
  for (const item of items) {
    if (item.is_fault === true) return faFault(item.fault);
    out.push(item.value);
  }
  return faOk(out);
}

function faZip<A, B>(input: { f0: A[]; f1: B[] }): Array<{ f0: A; f1: B }> {
  if (input.f0.length !== input.f1.length) throw new Error("zip: sequences must have the same length");
  return input.f0.map((left, index) => ({ f0: left, f1: input.f1[index] }));
}

function faBroadcastLeft<A, B>(input: { f0: A; f1: B[] }): Array<{ f0: A; f1: B }> {
  return input.f1.map((item) => ({ f0: input.f0, f1: item }));
}

function faBroadcastRight<A, B>(input: { f0: A[]; f1: B }): Array<{ f0: A; f1: B }> {
  return input.f0.map((item) => ({ f0: item, f1: input.f1 }));
}

function faTranspose<T>(rows: T[][]): T[][] {
  if (rows.length === 0) return [];
  const width = rows[0].length;
  if (!rows.every((row) => row.length === width)) throw new Error("transpose: rows must have the same length");
  return Array.from({ length: width }, (_, column) => rows.map((row) => row[column]));
}

function faGroupById<T>(items: Array<{ f0: bigint; f1: T }>): T[][] {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const key = item.f0.toString();
    const group = groups.get(key) ?? [];
    group.push(item.f1);
    groups.set(key, group);
  }
  return [...groups.keys()].sort((a, b) => Number(BigInt(a) - BigInt(b))).map((key) => groups.get(key)!);
}

function faShiftRight<T>(input: { f0: T[]; f1: T }): T[] {
  return [input.f1, ...input.f0.slice(0, Math.max(0, input.f0.length - 1))];
}

function faShiftLeft<T>(input: { f0: T[]; f1: T }): T[] {
  return [...input.f0.slice(1), input.f1];
}

function faHead<T>(items: T[]): FaFaultable<T> {
  return items.length === 0 ? faFaultMessage("head: empty sequence") : faOk(items[0]);
}

function faLast<T>(items: T[]): FaFaultable<T> {
  return items.length === 0 ? faFaultMessage("last: empty sequence") : faOk(items[items.length - 1]);
}

function faGet<T>(input: { f0: T[]; f1: bigint }): FaFaultable<T> {
  const index = Number(input.f1);
  return index < 0 || index >= input.f0.length ? faFaultMessage("get: index out of range") : faOk(input.f0[index]);
}

function faGetOr<T>(input: { f0: T[]; f1: bigint; f2: T }): T {
  const index = Number(input.f1);
  return index < 0 || index >= input.f0.length ? input.f2 : input.f0[index];
}

function faAt<T>(input: { f0: T[]; f1: bigint }): T {
  const index = Number(input.f1);
  if (index < 0 || index >= input.f0.length) throw new Error("at: index out of range");
  return input.f0[index];
}

function faFill<T>(input: { f0: bigint; f1: T }): T[] {
  return Array.from({ length: Number(input.f0) }, () => input.f1);
}

function faSet<T>(input: { f0: T[]; f1: bigint; f2: T }): FaFaultable<T[]> {
  const index = Number(input.f1);
  if (index < 0 || index >= input.f0.length) return faFaultMessage("set: index out of range");
  const out = [...input.f0];
  out[index] = input.f2;
  return faOk(out);
}

function faRangeStep(input: { f0: bigint; f1: bigint; f2: bigint }): bigint[] {
  if (input.f2 === 0n) throw new Error("range_step: step cannot be zero");
  const out: bigint[] = [];
  if (input.f2 > 0n) {
    for (let value = input.f0; value < input.f1; value += input.f2) out.push(value);
  } else {
    for (let value = input.f0; value > input.f1; value += input.f2) out.push(value);
  }
  return out;
}

"#;
