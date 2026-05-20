use crate::ast::*;
use crate::stdlib::{self, RuntimeSupport};
use std::collections::{BTreeSet, HashMap};

pub fn emit_module(module: &Module) -> Result<String, String> {
    let mut generator = Codegen {
        out: String::new(),
        temp: 0,
        string_id: 0,
        strings: Vec::new(),
        callables: HashMap::new(),
        stdlib_names: HashMap::new(),
    };
    generator.collect_imports(module)?;
    generator.collect_callables(module)?;
    generator.emit_prelude();
    for decl in &module.declarations {
        match decl {
            Decl::Node(callable) => generator.emit_callable(callable, false)?,
            Decl::Program(callable) => generator.emit_callable(callable, true)?,
            Decl::Import(_) => {}
        }
    }
    generator.emit_entrypoint();
    generator.emit_strings();
    Ok(generator.out)
}

struct Codegen<'a> {
    out: String,
    temp: usize,
    string_id: usize,
    strings: Vec<(String, Vec<u8>)>,
    callables: HashMap<String, &'a Callable>,
    stdlib_names: HashMap<String, String>,
}

impl<'a> Codegen<'a> {
    fn collect_imports(&mut self, module: &Module) -> Result<(), String> {
        for decl in &module.declarations {
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

    fn collect_callables(&mut self, module: &'a Module) -> Result<(), String> {
        for decl in &module.declarations {
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

    fn emit_prelude(&mut self) {
        self.line("declare ptr @fa_unit()");
        self.line("declare ptr @fa_int(i64)");
        self.line("declare ptr @fa_real(double)");
        self.line("declare ptr @fa_bool(i1)");
        self.line("declare ptr @fa_cstr(ptr)");
        self.line("declare ptr @fa_args(i32, ptr)");
        self.line("declare ptr @fa_seq_new(i64)");
        self.line("declare void @fa_seq_set(ptr, i64, ptr)");
        self.line("declare ptr @fa_seq_get(ptr, i64)");
        self.line("declare ptr @fa_builtin(ptr, ptr)");
        self.line("declare ptr @fa_map(ptr, ptr)");
        self.line("declare ptr @fa_fault_map(ptr, ptr)");
        self.line("declare ptr @fa_filter(ptr, ptr)");
        self.line("declare ptr @fa_repeat(ptr, ptr, ptr)");
        self.line("declare ptr @fa_reduce(ptr, ptr, ptr)");
        self.line("declare ptr @fa_parse_real(ptr)");
        self.line("declare ptr @fa_parse_int(ptr)");
        self.line("declare ptr @fa_not_empty(ptr)");
        self.line("declare i32 @fa_value_to_exit_code(ptr)");
        self.line("");
    }

    fn emit_callable(&mut self, callable: &Callable, is_program: bool) -> Result<(), String> {
        let symbol = if is_program {
            "flow_program_main".to_string()
        } else {
            format!("flow_node_{}", sanitize_symbol(&callable.name))
        };
        self.line(&format!("define ptr @{symbol}(ptr %__input) {{"));
        let mut env = HashMap::new();
        match callable.inputs.as_slice() {
            [] => {
                let tmp = self.next_temp();
                self.line(&format!("  {tmp} = call ptr @fa_unit()"));
            }
            [port] => {
                env.insert(port.name.clone(), "%__input".to_string());
            }
            ports => {
                for (index, port) in ports.iter().enumerate() {
                    let tmp = self.next_temp();
                    self.line(&format!(
                        "  {tmp} = call ptr @fa_seq_get(ptr %__input, i64 {index})"
                    ));
                    env.insert(port.name.clone(), tmp);
                }
            }
        }
        for chain in &callable.chains {
            self.emit_chain(chain, &mut env)?;
        }
        let result = self.emit_outputs(callable, &env)?;
        self.line(&format!("  ret ptr {result}"));
        self.line("}");
        self.line("");
        Ok(())
    }

    fn emit_outputs(
        &mut self,
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
                self.line(&format!(
                    "  {seq} = call ptr @fa_seq_new(i64 {})",
                    outputs.len()
                ));
                for (index, output) in outputs.iter().enumerate() {
                    let value = env
                        .get(&output.name)
                        .ok_or_else(|| format!("output `{}` is never bound", output.name))?;
                    self.line(&format!(
                        "  call void @fa_seq_set(ptr {seq}, i64 {index}, ptr {value})"
                    ));
                }
                Ok(seq)
            }
        }
    }

    fn emit_chain(
        &mut self,
        chain: &Chain,
        env: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        let mut value = self.emit_endpoint_value(&chain.source, env)?;
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Endpoint(Endpoint::Name(name)) if is_last => {
                    let canonical_name = self.canonical_name(name);
                    if self.callables.contains_key(name)
                        || stdlib::direct_builtin(&canonical_name).is_some()
                    {
                        value = self.emit_call(name, value)?;
                    } else if env.insert(name.clone(), value.clone()).is_some() {
                        return Err(format!("value `{name}` is bound more than once"));
                    }
                }
                Stage::Endpoint(endpoint) => match endpoint {
                    Endpoint::Name(name) => value = self.emit_call(name, value)?,
                    _ => return Err("non-name endpoints may only appear as values".to_string()),
                },
                Stage::Map(node) => {
                    let function = self.function_pointer_for(node)?;
                    let tmp = self.next_temp();
                    self.line(&format!(
                        "  {tmp} = call ptr @fa_map(ptr {value}, ptr {function})"
                    ));
                    value = tmp;
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let function = self.function_pointer_for(node)?;
                    let pair = self.next_temp();
                    self.line(&format!(
                        "  {pair} = call ptr @fa_fault_map(ptr {value}, ptr {function})"
                    ));
                    let ok_value = self.next_temp();
                    self.line(&format!(
                        "  {ok_value} = call ptr @fa_seq_get(ptr {pair}, i64 0)"
                    ));
                    let fault_value = self.next_temp();
                    self.line(&format!(
                        "  {fault_value} = call ptr @fa_seq_get(ptr {pair}, i64 1)"
                    ));
                    if env.insert(ok.clone(), ok_value).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_value).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                Stage::Filter(predicate) => {
                    let function = self.function_pointer_for(predicate)?;
                    let tmp = self.next_temp();
                    self.line(&format!(
                        "  {tmp} = call ptr @fa_filter(ptr {value}, ptr {function})"
                    ));
                    value = tmp;
                }
                Stage::Repeat { count, node } => {
                    let count_value = self.emit_endpoint_value(count, env)?;
                    let function = self.function_pointer_for(node)?;
                    let tmp = self.next_temp();
                    self.line(&format!(
                        "  {tmp} = call ptr @fa_repeat(ptr {value}, ptr {count_value}, ptr {function})"
                    ));
                    value = tmp;
                }
                Stage::Reduce { op, identity } => {
                    let canonical_op = self.canonical_name(op);
                    let op_name = self.emit_string_ptr(&canonical_op);
                    let identity_value = self.emit_endpoint_value(identity, env)?;
                    let tmp = self.next_temp();
                    self.line(&format!(
                        "  {tmp} = call ptr @fa_reduce(ptr {value}, ptr {op_name}, ptr {identity_value})"
                    ));
                    value = tmp;
                }
            }
        }
        Ok(())
    }

    fn emit_call(&mut self, name: &str, input: String) -> Result<String, String> {
        if self.callables.contains_key(name) {
            let tmp = self.next_temp();
            self.line(&format!(
                "  {tmp} = call ptr @flow_node_{}(ptr {input})",
                sanitize_symbol(name)
            ));
            return Ok(tmp);
        }
        let canonical_name = self.canonical_name(name);
        let name_ptr = self.emit_string_ptr(&canonical_name);
        let tmp = self.next_temp();
        self.line(&format!(
            "  {tmp} = call ptr @fa_builtin(ptr {name_ptr}, ptr {input})"
        ));
        Ok(tmp)
    }

    fn emit_endpoint_value(
        &mut self,
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
                self.line(&format!("  {tmp} = call ptr @fa_int(i64 {value})"));
                Ok(tmp)
            }
            Endpoint::Real(value) => {
                let tmp = self.next_temp();
                self.line(&format!(
                    "  {tmp} = call ptr @fa_real(double {:.17e})",
                    value
                ));
                Ok(tmp)
            }
            Endpoint::Bool(value) => {
                let tmp = self.next_temp();
                let bit = if *value { 1 } else { 0 };
                self.line(&format!("  {tmp} = call ptr @fa_bool(i1 {bit})"));
                Ok(tmp)
            }
            Endpoint::String(value) => {
                let ptr = self.emit_string_ptr(value);
                let tmp = self.next_temp();
                self.line(&format!("  {tmp} = call ptr @fa_cstr(ptr {ptr})"));
                Ok(tmp)
            }
            Endpoint::Unit => {
                let tmp = self.next_temp();
                self.line(&format!("  {tmp} = call ptr @fa_unit()"));
                Ok(tmp)
            }
            Endpoint::Tuple(items) | Endpoint::Seq(items) => {
                let seq = self.next_temp();
                self.line(&format!(
                    "  {seq} = call ptr @fa_seq_new(i64 {})",
                    items.len()
                ));
                for (index, item) in items.iter().enumerate() {
                    let value = self.emit_endpoint_value(item, env)?;
                    self.line(&format!(
                        "  call void @fa_seq_set(ptr {seq}, i64 {index}, ptr {value})"
                    ));
                }
                Ok(seq)
            }
        }
    }

    fn emit_entrypoint(&mut self) {
        self.line("define i32 @main(i32 %argc, ptr %argv) {");
        let args = self.next_temp();
        self.line(&format!(
            "  {args} = call ptr @fa_args(i32 %argc, ptr %argv)"
        ));
        let value = self.next_temp();
        self.line(&format!(
            "  {value} = call ptr @flow_program_main(ptr {args})"
        ));
        let exit = self.next_temp();
        self.line(&format!(
            "  {exit} = call i32 @fa_value_to_exit_code(ptr {value})"
        ));
        self.line(&format!("  ret i32 {exit}"));
        self.line("}");
        self.line("");
    }

    fn function_pointer_for(&self, name: &str) -> Result<String, String> {
        if self.callables.contains_key(name) {
            return Ok(format!("@flow_node_{}", sanitize_symbol(name)));
        }
        let canonical_name = self.canonical_name(name);
        stdlib::function_pointer(&canonical_name)
            .map(ToString::to_string)
            .ok_or_else(|| format!("`{name}` cannot be used as a map/filter function yet"))
    }

    fn emit_string_ptr(&mut self, value: &str) -> String {
        let global = format!("@.flow.str.{}", self.string_id);
        self.string_id += 1;
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        let len = bytes.len();
        self.strings.push((global.clone(), bytes));
        let tmp = self.next_temp();
        self.line(&format!(
            "  {tmp} = getelementptr inbounds [{len} x i8], ptr {global}, i64 0, i64 0"
        ));
        tmp
    }

    fn emit_strings(&mut self) {
        let mut seen = BTreeSet::new();
        let strings = std::mem::take(&mut self.strings);
        for (global, bytes) in strings {
            if seen.insert(global.clone()) {
                self.line(&format!(
                    "{global} = private unnamed_addr constant [{} x i8] c\"{}\"",
                    bytes.len(),
                    llvm_bytes(&bytes)
                ));
            }
        }
    }

    fn next_temp(&mut self) -> String {
        let temp = format!("%t{}", self.temp);
        self.temp += 1;
        temp
    }

    fn line(&mut self, line: &str) {
        self.out.push_str(line);
        self.out.push('\n');
    }

    fn canonical_name(&self, name: &str) -> String {
        self.stdlib_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }
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

fn llvm_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &byte in bytes {
        match byte {
            b'\\' => out.push_str("\\5C"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\{byte:02X}")),
        }
    }
    out
}
