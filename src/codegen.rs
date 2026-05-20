use crate::ast::*;
use crate::stdlib::{self, RuntimeSupport};
mod runtime;
use std::collections::{BTreeSet, HashMap};

pub fn emit_module(module: &Module) -> Result<String, String> {
    Codegen::new(module)?.emit_llvm_entry()
}

pub fn emit_runtime_c(module: &Module) -> Result<String, String> {
    Codegen::new(module)?.emit_runtime_c()
}

struct Codegen<'a> {
    module: &'a Module,
    temp: usize,
    callables: HashMap<String, &'a Callable>,
    stdlib_names: HashMap<String, String>,
}

impl<'a> Codegen<'a> {
    fn new(module: &'a Module) -> Result<Self, String> {
        let mut generator = Self {
            module,
            temp: 0,
            callables: HashMap::new(),
            stdlib_names: HashMap::new(),
        };
        generator.collect_imports()?;
        generator.collect_callables()?;
        Ok(generator)
    }

    fn emit_llvm_entry(&self) -> Result<String, String> {
        Ok("declare i32 @flow_unboxed_main(i32, ptr)\n\n\
define i32 @main(i32 %argc, ptr %argv) {\n\
  %exit = call i32 @flow_unboxed_main(i32 %argc, ptr %argv)\n\
  ret i32 %exit\n\
}\n"
        .to_string())
    }

    fn emit_runtime_c(mut self) -> Result<String, String> {
        let mut out = String::new();
        runtime::emit_preamble(&mut out);
        self.emit_builtin_forwarders(&mut out);

        let names = self
            .callables
            .keys()
            .map(|name| name.as_str())
            .collect::<BTreeSet<_>>();
        for name in &names {
            out.push_str(&format!(
                "static FaValue {}(FaValue input);\n",
                user_fn_name(name)
            ));
        }
        out.push('\n');

        for decl in &self.module.declarations {
            match decl {
                Decl::Node(callable) => self.emit_callable(&mut out, callable, false)?,
                Decl::Program(callable) => self.emit_callable(&mut out, callable, true)?,
                Decl::Import(_) => {}
            }
        }

        out.push_str(
            "int flow_unboxed_main(int argc, char **argv) {\n\
  FaValue args = fa_args(argc, argv);\n\
  FaValue result = flow_program_main(args);\n\
  return fa_value_to_exit_code(result);\n\
}\n",
        );
        Ok(out)
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
                        if symbol.kind != stdlib::SymbolKind::Node
                            || symbol.runtime == RuntimeSupport::Unsupported
                        {
                            continue;
                        }
                        self.stdlib_names
                            .insert(format!("{alias}.{}", symbol.name), symbol.name.to_string());
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
        Ok(())
    }

    fn collect_callables(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            match decl {
                Decl::Node(callable) | Decl::Program(callable) => {
                    if self
                        .callables
                        .insert(callable.name.clone(), callable)
                        .is_some()
                    {
                        return Err(format!("duplicate declaration `{}`", callable.name));
                    }
                }
                Decl::Import(_) => {}
            }
        }
        if !self.callables.contains_key("main") {
            return Err("missing `program main`".to_string());
        }
        Ok(())
    }

    fn emit_builtin_forwarders(&self, out: &mut String) {
        for name in [
            "argv",
            "read_stdin",
            "split_lines",
            "range_step",
            "format_int",
            "parse_int",
            "parse_real",
            "format_real",
            "concat_bytes",
            "join_bytes",
            "add",
            "sub",
            "mul",
            "div",
            "rem",
            "eq",
            "lt",
            "gt",
            "le",
            "ge",
            "max",
            "not_empty",
            "is_empty",
            "has_faults",
            "format_faults",
            "write_stdout",
            "write_stderr",
            "select",
            "and",
            "or",
            "xor",
            "not",
            "all",
            "any",
        ] {
            out.push_str(&format!(
                "#define {} fa_builtin_{}\n",
                builtin_fn_name(name),
                name
            ));
        }
        out.push('\n');
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
        out.push_str(&format!("static FaValue {symbol}(FaValue input) {{\n"));
        let mut env = HashMap::new();
        match callable.inputs.as_slice() {
            [] => {
                out.push_str("  (void)input;\n");
            }
            [port] => {
                let var = c_ident(&port.name);
                out.push_str(&format!("  FaValue {var} = input;\n"));
                env.insert(port.name.clone(), var);
            }
            ports => {
                out.push_str("  FaValue __inputs = fa_expect_seq(input, \"node input\");\n");
                for (index, port) in ports.iter().enumerate() {
                    let var = c_ident(&port.name);
                    out.push_str(&format!(
                        "  FaValue {var} = fa_seq_get(__inputs, {index});\n"
                    ));
                    env.insert(port.name.clone(), var);
                }
            }
        }
        for chain in &callable.chains {
            self.emit_chain(out, chain, &mut env)?;
        }
        let result = self.emit_outputs(out, callable, &env)?;
        out.push_str(&format!("  return {result};\n"));
        out.push_str("}\n\n");
        Ok(())
    }

    fn emit_outputs(
        &mut self,
        out: &mut String,
        callable: &Callable,
        env: &HashMap<String, String>,
    ) -> Result<String, String> {
        match callable.outputs.as_slice() {
            [] => Err(format!("`{}` must declare an output", callable.name)),
            [output] => env
                .get(&output.name)
                .cloned()
                .ok_or_else(|| format!("output `{}` is never bound", output.name)),
            outputs => {
                let seq = self.next_temp();
                out.push_str(&format!(
                    "  FaValue {seq} = fa_seq_new({});\n",
                    outputs.len()
                ));
                for (index, output) in outputs.iter().enumerate() {
                    let value = env
                        .get(&output.name)
                        .ok_or_else(|| format!("output `{}` is never bound", output.name))?;
                    out.push_str(&format!("  fa_seq_set(&{seq}, {index}, {value});\n"));
                }
                Ok(seq)
            }
        }
    }

    fn emit_chain(
        &mut self,
        out: &mut String,
        chain: &Chain,
        env: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        let mut value = self.emit_endpoint_value(out, &chain.source, env)?;
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Endpoint(Endpoint::Name(name)) if is_last => {
                    let canonical_name = self.canonical_name(name);
                    if self.callables.contains_key(name) || is_builtin(&canonical_name) {
                        value = self.emit_call(out, name, &value);
                    } else if env.insert(name.clone(), value.clone()).is_some() {
                        return Err(format!("value `{name}` is bound more than once"));
                    }
                }
                Stage::Endpoint(endpoint) => match endpoint {
                    Endpoint::Name(name) => value = self.emit_call(out, name, &value),
                    _ => return Err("non-name endpoints may only appear as values".to_string()),
                },
                Stage::Map(node) => {
                    let tmp = self.next_temp();
                    let function = self.function_for(node)?;
                    out.push_str(&format!("  FaValue {tmp} = fa_map({value}, {function});\n"));
                    value = tmp;
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let function = self.function_for(node)?;
                    let tmp = self.next_temp();
                    out.push_str(&format!(
                        "  FaFaultMapResult {tmp} = fa_fault_map({value}, {function});\n"
                    ));
                    if env.insert(ok.clone(), format!("{tmp}.ok")).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), format!("{tmp}.faults")).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                Stage::Filter(predicate) => {
                    let tmp = self.next_temp();
                    let function = self.function_for(predicate)?;
                    out.push_str(&format!(
                        "  FaValue {tmp} = fa_filter({value}, {function});\n"
                    ));
                    value = tmp;
                }
                Stage::Repeat { count, node } => {
                    let count_value = self.emit_endpoint_value(out, count, env)?;
                    let tmp = self.next_temp();
                    let function = self.function_for(node)?;
                    out.push_str(&format!(
                        "  FaValue {tmp} = fa_repeat({value}, {count_value}, {function});\n"
                    ));
                    value = tmp;
                }
                Stage::Reduce { op, identity } => {
                    let identity_value = self.emit_endpoint_value(out, identity, env)?;
                    let tmp = self.next_temp();
                    let op_name = self.canonical_name(op);
                    let reducer = match op_name.as_str() {
                        "add" => "FA_REDUCE_ADD",
                        "concat_bytes" => "FA_REDUCE_CONCAT_BYTES",
                        other => return Err(format!("unsupported reduce op `{other}`")),
                    };
                    out.push_str(&format!(
                        "  FaValue {tmp} = fa_reduce({value}, {reducer}, {identity_value});\n"
                    ));
                    value = tmp;
                }
            }
        }
        Ok(())
    }

    fn emit_call(&mut self, out: &mut String, name: &str, input: &str) -> String {
        let tmp = self.next_temp();
        let function = if self.callables.contains_key(name) {
            user_fn_name(name)
        } else {
            builtin_fn_name(&self.canonical_name(name))
        };
        out.push_str(&format!("  FaValue {tmp} = {function}({input});\n"));
        tmp
    }

    fn emit_endpoint_value(
        &mut self,
        out: &mut String,
        endpoint: &Endpoint,
        env: &HashMap<String, String>,
    ) -> Result<String, String> {
        match endpoint {
            Endpoint::Name(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Int(value) => {
                let tmp = self.next_temp();
                out.push_str(&format!("  FaValue {tmp} = fa_int({value});\n"));
                Ok(tmp)
            }
            Endpoint::Real(value) => {
                let tmp = self.next_temp();
                out.push_str(&format!("  FaValue {tmp} = fa_real({value:.17e});\n"));
                Ok(tmp)
            }
            Endpoint::Bool(value) => {
                let tmp = self.next_temp();
                let bit = if *value { "true" } else { "false" };
                out.push_str(&format!("  FaValue {tmp} = fa_bool({bit});\n"));
                Ok(tmp)
            }
            Endpoint::String(value) => {
                let tmp = self.next_temp();
                out.push_str(&format!(
                    "  FaValue {tmp} = fa_bytes_literal(\"{}\", {});\n",
                    c_string(value),
                    value.len()
                ));
                Ok(tmp)
            }
            Endpoint::Unit => {
                let tmp = self.next_temp();
                out.push_str(&format!("  FaValue {tmp} = fa_unit();\n"));
                Ok(tmp)
            }
            Endpoint::Tuple(items) | Endpoint::Seq(items) => {
                let seq = self.next_temp();
                out.push_str(&format!("  FaValue {seq} = fa_seq_new({});\n", items.len()));
                for (index, item) in items.iter().enumerate() {
                    let value = self.emit_endpoint_value(out, item, env)?;
                    out.push_str(&format!("  fa_seq_set(&{seq}, {index}, {value});\n"));
                }
                Ok(seq)
            }
        }
    }

    fn function_for(&self, name: &str) -> Result<String, String> {
        if self.callables.contains_key(name) {
            return Ok(user_fn_name(name));
        }
        let canonical_name = self.canonical_name(name);
        if is_builtin(&canonical_name) {
            Ok(builtin_fn_name(&canonical_name))
        } else {
            Err(format!("`{name}` cannot be used as a function"))
        }
    }

    fn canonical_name(&self, name: &str) -> String {
        self.stdlib_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    fn next_temp(&mut self) -> String {
        let temp = format!("t{}", self.temp);
        self.temp += 1;
        temp
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "argv"
            | "read_stdin"
            | "split_lines"
            | "range_step"
            | "format_int"
            | "parse_int"
            | "parse_real"
            | "format_real"
            | "concat_bytes"
            | "join_bytes"
            | "add"
            | "sub"
            | "mul"
            | "div"
            | "rem"
            | "eq"
            | "lt"
            | "gt"
            | "le"
            | "ge"
            | "max"
            | "not_empty"
            | "is_empty"
            | "has_faults"
            | "format_faults"
            | "write_stdout"
            | "write_stderr"
            | "select"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "all"
            | "any"
    )
}

fn user_fn_name(name: &str) -> String {
    if name == "main" {
        "flow_program_main".to_string()
    } else {
        format!("flow_node_{}", sanitize_symbol(name))
    }
}

fn builtin_fn_name(name: &str) -> String {
    format!("fa_node_{}", sanitize_symbol(name))
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

    #[test]
    fn llvm_entry_is_only_a_thin_shim_to_unboxed_c_runtime() {
        let module = checked_module(
            r#"
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    0 -> exit_code
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
    fn runtime_emits_unboxed_values_and_direct_builtin_calls() {
        let module = checked_module(
            r#"
                import std.cli { Args }
                import std.bytes { split_lines }
                import std.predicates { not_empty }
                import std.real { parse_real, format_real }
                import std.math { add }
                import std.io { read_stdin, write_stdout }

                program main(args: Args) -> exit_code: Faultable[Int] {
                    () -> read_stdin -> split_lines -> filter not_empty -> map parse_real -> reduce add(identity: 0.0) -> total
                    total -> format_real -> write_stdout -> exit_code
                }
            "#,
        );

        let runtime_c = emit_runtime_c(&module).expect("runtime c");

        assert!(runtime_c.contains("typedef struct FaValue FaValue;"));
        assert!(runtime_c.contains("FaValue *items;"));
        assert!(runtime_c.contains("static FaValue fa_builtin_parse_real(FaValue input)"));
        assert!(runtime_c.contains("fa_filter("));
        assert!(runtime_c.contains("fa_node_not_empty"));
        assert!(runtime_c.contains("fa_map("));
        assert!(runtime_c.contains("fa_node_parse_real"));
        assert!(runtime_c.contains("fa_reduce("));
        assert!(runtime_c.contains("FA_REDUCE_ADD"));
        assert!(!runtime_c.contains("fa_builtin("));
        assert!(!runtime_c.contains("FaValue **items"));
    }
}
