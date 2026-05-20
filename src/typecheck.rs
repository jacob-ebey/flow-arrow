use crate::ast::*;
use crate::module_resolver;
use crate::stdlib::{self, Effect, RuntimeSupport, SymbolKind};
use std::collections::HashMap;
use std::fmt;

pub fn check_module(module: &Module) -> Result<(), String> {
    let expanded = module_resolver::expand_stdlib_sources(module)?;
    Checker::new(&expanded)?.check()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Unit,
    Int,
    Real,
    Bool,
    Bytes,
    Args,
    Fault,
    Faultable(Box<Type>),
    Seq(Box<Type>),
    Tuple(Vec<Type>),
    OneOf(Vec<Type>),
    Var(String),
}

#[derive(Debug, Clone)]
struct Signature {
    input: Type,
    output: Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallableKind {
    Node,
    Program,
}

#[derive(Debug, Clone)]
struct CallableInfo {
    signatures: Vec<Signature>,
    reduce_signatures: Vec<Signature>,
    kind: CallableKind,
    effect: Effect,
    runtime: RuntimeSupport,
    is_stdlib: bool,
    runtime_name: String,
}

struct Checker<'a> {
    module: &'a Module,
    symbols: HashMap<String, CallableInfo>,
    types: HashMap<String, Type>,
}

impl Type {
    fn contains_faultable(&self) -> bool {
        match self {
            Type::Faultable(_) => true,
            Type::Seq(item) => item.contains_faultable(),
            Type::Tuple(items) => items.iter().any(Type::contains_faultable),
            Type::OneOf(items) => items.iter().any(Type::contains_faultable),
            _ => false,
        }
    }

    fn inner_faultable(&self) -> Type {
        match self {
            Type::Faultable(item) => (**item).clone(),
            other => other.clone(),
        }
    }

    fn strip_faultable(&self) -> Type {
        match self {
            Type::Faultable(item) => item.strip_faultable(),
            Type::Seq(item) => Type::Seq(Box::new(item.strip_faultable())),
            Type::Tuple(items) => Type::Tuple(items.iter().map(Type::strip_faultable).collect()),
            Type::OneOf(items) => Type::OneOf(items.iter().map(Type::strip_faultable).collect()),
            other => other.clone(),
        }
    }
}

impl<'a> Checker<'a> {
    fn new(module: &'a Module) -> Result<Self, String> {
        let mut checker = Self {
            module,
            symbols: HashMap::new(),
            types: primitive_types(),
        };
        checker.import_intrinsics()?;
        checker.collect_imports()?;
        checker.collect_callables()?;
        Ok(checker)
    }

    fn check(&self) -> Result<(), String> {
        let main = self
            .symbols
            .get("main")
            .ok_or_else(|| "missing `program main`".to_string())?;
        if main.kind != CallableKind::Program {
            return Err("`main` must be declared as a program".to_string());
        }
        let main_signature = main
            .signatures
            .first()
            .ok_or_else(|| "`main` has no signature".to_string())?;
        self.expect_type("program main input", &main_signature.input, &Type::Args)?;
        if main_signature.output != Type::Int
            && main_signature.output != Type::Faultable(Box::new(Type::Int))
        {
            return Err(format!(
                "program main output expected `Int` or `Faultable[Int]`, found `{}`",
                main_signature.output
            ));
        }

        for decl in &self.module.declarations {
            match decl {
                Decl::Node(callable) => self.check_callable(callable, CallableKind::Node)?,
                Decl::Program(callable) => self.check_callable(callable, CallableKind::Program)?,
                Decl::Import(_) => {}
            }
        }
        Ok(())
    }

    fn import_intrinsics(&mut self) -> Result<(), String> {
        for symbol in stdlib::module_symbols(stdlib::INTRINSIC_MODULE) {
            self.insert_std_symbol(symbol.name, symbol)?;
        }
        Ok(())
    }

    fn collect_imports(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            let Decl::Import(import) = decl else {
                continue;
            };
            let ImportSource::Module(module) = &import.source else {
                return Err("local imports are not supported by this compiler yet".to_string());
            };
            if module == stdlib::INTRINSIC_MODULE {
                return Err("intrinsic module imports are compiler-internal".to_string());
            }
            match &import.clause {
                ImportClause::Alias(alias) => {
                    let mut found = false;
                    for symbol in stdlib::module_symbols(module) {
                        found = true;
                        if symbol.runtime == RuntimeSupport::Unsupported {
                            continue;
                        }
                        let name = format!("{alias}.{}", symbol.name);
                        self.insert_std_symbol(&name, symbol)?;
                    }
                    if !found {
                        return Err(format!("unknown stdlib module `{module}`"));
                    }
                }
                ImportClause::Items(items) => {
                    for item in items {
                        let symbol = stdlib::find_export(module, &item.name).ok_or_else(|| {
                            format!("module `{module}` does not export `{}`", item.name)
                        })?;
                        let name = item.alias.as_deref().unwrap_or(&item.name);
                        self.insert_std_symbol(name, symbol)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn insert_std_symbol(
        &mut self,
        name: &str,
        symbol: &'static stdlib::StdSymbol,
    ) -> Result<(), String> {
        if symbol.runtime == RuntimeSupport::Unsupported {
            return Err(format!(
                "`{}.{}` is declared in the stdlib but is not implemented by this compiler backend yet",
                symbol.module, symbol.name
            ));
        }
        match symbol.kind {
            SymbolKind::Type => {
                let ty = parse_type(symbol.name)?;
                if self.types.insert(name.to_string(), ty).is_some() {
                    return Err(format!("duplicate type import `{name}`"));
                }
            }
            SymbolKind::Node => {
                let signatures = stdlib_signatures(symbol)?;
                let reduce_signatures = stdlib_reduce_signatures(symbol)?;
                self.insert_symbol(
                    name,
                    CallableInfo {
                        signatures,
                        reduce_signatures,
                        kind: CallableKind::Node,
                        effect: symbol.effect,
                        runtime: symbol.runtime,
                        is_stdlib: true,
                        runtime_name: symbol.name.to_string(),
                    },
                )?;
            }
        }
        Ok(())
    }

    fn collect_callables(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            let (callable, kind) = match decl {
                Decl::Node(callable) => (callable, CallableKind::Node),
                Decl::Program(callable) => (callable, CallableKind::Program),
                Decl::Import(_) => continue,
            };
            let info = CallableInfo {
                signatures: vec![self.callable_signature(callable)?],
                reduce_signatures: Vec::new(),
                kind,
                effect: Effect::Pure,
                runtime: RuntimeSupport::DirectBuiltin,
                is_stdlib: false,
                runtime_name: callable.name.clone(),
            };
            self.insert_symbol(&callable.name, info)?;
        }
        Ok(())
    }

    fn insert_symbol(&mut self, name: &str, info: CallableInfo) -> Result<(), String> {
        if self.symbols.insert(name.to_string(), info).is_some() {
            return Err(format!("duplicate declaration or import `{name}`"));
        }
        Ok(())
    }

    fn callable_signature(&self, callable: &Callable) -> Result<Signature, String> {
        Ok(Signature {
            input: self.port_types(&callable.inputs)?,
            output: self.port_types(&callable.outputs)?,
        })
    }

    fn port_types(&self, ports: &[Port]) -> Result<Type, String> {
        let mut types = Vec::with_capacity(ports.len());
        for port in ports {
            types.push(self.parse_declared_type(&port.ty)?);
        }
        Ok(match types.len() {
            0 => Type::Unit,
            1 => types.remove(0),
            _ => Type::Tuple(types),
        })
    }

    fn parse_declared_type(&self, text: &str) -> Result<Type, String> {
        let ty = self.resolve_declared_type(parse_type(text)?)?;
        self.validate_declared_type(&ty)?;
        Ok(ty)
    }

    fn resolve_declared_type(&self, ty: Type) -> Result<Type, String> {
        match ty {
            Type::Var(name) => self
                .types
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("unknown type `{name}`")),
            Type::Faultable(item) => Ok(Type::Faultable(Box::new(
                self.resolve_declared_type(*item)?,
            ))),
            Type::Seq(item) => Ok(Type::Seq(Box::new(self.resolve_declared_type(*item)?))),
            Type::OneOf(items) => {
                let mut resolved = Vec::with_capacity(items.len());
                for item in items {
                    resolved.push(self.resolve_declared_type(item)?);
                }
                Ok(Type::OneOf(resolved))
            }
            Type::Tuple(items) => {
                let mut resolved = Vec::with_capacity(items.len());
                for item in items {
                    resolved.push(self.resolve_declared_type(item)?);
                }
                Ok(Type::Tuple(resolved))
            }
            other => Ok(other),
        }
    }

    fn validate_declared_type(&self, ty: &Type) -> Result<(), String> {
        match ty {
            Type::Faultable(item) | Type::Seq(item) => self.validate_declared_type(item),
            Type::OneOf(items) => {
                for item in items {
                    self.validate_declared_type(item)?;
                }
                Ok(())
            }
            Type::Tuple(items) => {
                for item in items {
                    self.validate_declared_type(item)?;
                }
                Ok(())
            }
            Type::Var(name) => Err(format!("unknown type `{name}`")),
            primitive if self.types.values().any(|known| known == primitive) => Ok(()),
            other => Err(format!("unknown type `{other}`")),
        }
    }

    fn check_callable(&self, callable: &Callable, kind: CallableKind) -> Result<(), String> {
        let mut env = HashMap::new();
        for port in &callable.inputs {
            let ty = self.parse_declared_type(&port.ty)?;
            if env.insert(port.name.clone(), ty).is_some() {
                return Err(format!(
                    "`{}` declares input `{}` more than once",
                    callable.name, port.name
                ));
            }
        }
        for chain in &callable.chains {
            self.check_chain(callable, kind, chain, &mut env)?;
        }
        for output in &callable.outputs {
            let expected = self.parse_declared_type(&output.ty)?;
            let actual = env.get(&output.name).ok_or_else(|| {
                format!(
                    "`{}` declares output `{}` but it is never bound",
                    callable.name, output.name
                )
            })?;
            self.expect_type(
                &format!("output `{}` of `{}`", output.name, callable.name),
                actual,
                &expected,
            )?;
        }
        Ok(())
    }

    fn check_chain(
        &self,
        callable: &Callable,
        kind: CallableKind,
        chain: &Chain,
        env: &mut HashMap<String, Type>,
    ) -> Result<(), String> {
        let mut value_type = self.endpoint_type(&chain.source, env)?;
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Endpoint(Endpoint::Variable(name)) if is_last => {
                    if env.insert(name.clone(), value_type.clone()).is_some() {
                        return Err(format!("value `{name}` is bound more than once"));
                    }
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value_type = self.apply_node(callable, kind, name, &value_type, false)?;
                }
                Stage::Endpoint(Endpoint::Variable(_)) => {
                    return Err(
                        "variables may only appear as source values or final bindings".to_string(),
                    );
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
                }
                Stage::Map(name) => {
                    value_type = self.apply_map(callable, name, &value_type)?;
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let (ok_type, fault_type) =
                        self.apply_fault_map(callable, node, &value_type)?;
                    if env.insert(ok.clone(), ok_type).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_type).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                }
                Stage::Filter(name) => {
                    value_type = self.apply_filter(callable, name, &value_type)?;
                }
                Stage::Repeat { count, node } => {
                    let count_type = self.endpoint_type(count, env)?;
                    self.expect_type("repeat count", &count_type, &Type::Int)?;
                    value_type = self.apply_repeat(callable, node, &value_type)?;
                }
                Stage::Reduce { op, identity } => {
                    let identity_type = self.endpoint_type(identity, env)?;
                    value_type = self.apply_reduce(callable, op, &value_type, &identity_type)?;
                }
                Stage::Scan { op, identity } => {
                    let identity_type = self.endpoint_type(identity, env)?;
                    value_type = self.apply_scan(callable, op, &value_type, &identity_type)?;
                }
            }
        }
        Ok(())
    }

    fn endpoint_type(
        &self,
        endpoint: &Endpoint,
        env: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Int(_) => Ok(Type::Int),
            Endpoint::Real(_) => Ok(Type::Real),
            Endpoint::Bool(_) => Ok(Type::Bool),
            Endpoint::String(_) => Ok(Type::Bytes),
            Endpoint::Unit => Ok(Type::Unit),
            Endpoint::Tuple(items) => {
                let mut types = Vec::with_capacity(items.len());
                for item in items {
                    types.push(self.endpoint_type(item, env)?);
                }
                Ok(Type::Tuple(types))
            }
            Endpoint::Seq(items) => {
                let mut item_type = None;
                for item in items {
                    let ty = self.endpoint_type(item, env)?;
                    if let Some(expected) = &item_type {
                        item_type = Some(sequence_item_type(expected, &ty).map_err(|error| {
                            format!("sequence literal item type mismatch: {error}")
                        })?);
                    } else {
                        item_type = Some(ty);
                    }
                }
                let item_type = item_type
                    .ok_or_else(|| "empty sequence literals need a type context".to_string())?;
                Ok(Type::Seq(Box::new(item_type)))
            }
        }
    }

    fn apply_node(
        &self,
        callable: &Callable,
        context: CallableKind,
        name: &str,
        input: &Type,
        as_function: bool,
    ) -> Result<Type, String> {
        let node = self
            .symbols
            .get(name)
            .ok_or_else(|| format!("unknown node `{name}`"))?;
        if node.kind == CallableKind::Program {
            return Err(format!("program `{name}` cannot be called from a graph"));
        }
        if context == CallableKind::Node && node.effect == Effect::Io {
            return Err(format!(
                "`{}` cannot use effectful stdlib node `{name}` outside a program",
                callable.name
            ));
        }
        if as_function && !self.supports_higher_order_call(node) {
            return Err(format!("`{name}` cannot be used as a map/filter function"));
        }
        if !as_function && node.runtime == RuntimeSupport::ReduceOnly {
            return Err(format!("`{name}` can only be used as a reduce operation"));
        }
        let input_faultable = input.contains_faultable();
        let actual_input = input.strip_faultable();
        let mut last_error = None;
        let mut output = None;
        for signature in &node.signatures {
            let mut vars = HashMap::new();
            match match_types(&signature.input, &actual_input, &mut vars) {
                Ok(()) => {
                    output = Some(substitute(&signature.output, &vars).ok_or_else(|| {
                        format!("`{name}` output type contains unresolved type variables")
                    })?);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let output = output.ok_or_else(|| {
            format!(
                "`{name}` input type mismatch: {}",
                last_error
                    .unwrap_or_else(|| format!("expected callable input, found `{actual_input}`"))
            )
        })?;
        Ok(
            if input_faultable && !matches!(output, Type::Faultable(_)) {
                Type::Faultable(Box::new(output))
            } else {
                output
            },
        )
    }

    fn apply_map(&self, callable: &Callable, name: &str, input: &Type) -> Result<Type, String> {
        let input_faultable = matches!(input, Type::Faultable(_));
        let unwrapped = input.inner_faultable();
        let Type::Seq(item_type) = &unwrapped else {
            return Err(format!("`map {name}` expected Seq input, found `{input}`"));
        };
        let output = self.apply_node(callable, CallableKind::Node, name, item_type, true)?;
        let seq = Type::Seq(Box::new(output));
        Ok(if input_faultable {
            Type::Faultable(Box::new(seq))
        } else {
            seq
        })
    }

    fn apply_fault_map(
        &self,
        callable: &Callable,
        name: &str,
        input: &Type,
    ) -> Result<(Type, Type), String> {
        let unwrapped = input.inner_faultable();
        let Type::Seq(item_type) = &unwrapped else {
            return Err(format!(
                "`fault map {name}` expected Seq input, found `{input}`"
            ));
        };
        let output = self.apply_node(callable, CallableKind::Node, name, item_type, true)?;
        let Type::Faultable(ok) = output else {
            return Err(format!(
                "`fault map {name}` expected a faultable node, found output `{output}`"
            ));
        };
        Ok((Type::Seq(ok), Type::Seq(Box::new(Type::Fault))))
    }

    fn apply_filter(&self, callable: &Callable, name: &str, input: &Type) -> Result<Type, String> {
        let input_faultable = matches!(input, Type::Faultable(_));
        let unwrapped = input.inner_faultable();
        let Type::Seq(item_type) = &unwrapped else {
            return Err(format!(
                "`filter {name}` expected Seq input, found `{input}`"
            ));
        };
        let output = self.apply_node(callable, CallableKind::Node, name, item_type, true)?;
        self.expect_type(&format!("filter `{name}` result"), &output, &Type::Bool)?;
        Ok(if input_faultable {
            Type::Faultable(Box::new(unwrapped))
        } else {
            unwrapped
        })
    }

    fn apply_repeat(&self, callable: &Callable, name: &str, input: &Type) -> Result<Type, String> {
        let output = self.apply_node(callable, CallableKind::Node, name, input, true)?;
        self.expect_type(&format!("repeat `{name}` result"), &output, input)?;
        Ok(output)
    }

    fn apply_reduce(
        &self,
        callable: &Callable,
        name: &str,
        input: &Type,
        identity: &Type,
    ) -> Result<Type, String> {
        let Type::Seq(item_type) = input else {
            return Err(format!(
                "`reduce {name}` expected Seq input, found `{input}`"
            ));
        };
        let node = self
            .symbols
            .get(name)
            .ok_or_else(|| format!("unknown reduce operation `{name}`"))?;
        if node.kind == CallableKind::Program || node.effect != Effect::Pure {
            return Err(format!("`{name}` cannot be used as a reduce operation"));
        }
        if node.reduce_signatures.is_empty() {
            return Err(format!("`{name}` is not implemented as a reduce operation"));
        }
        let signatures = &node.reduce_signatures;
        let item_faultable = matches!(item_type.as_ref(), Type::Faultable(_));
        let plain_item_type = item_type.inner_faultable();
        let pair = Type::Tuple(vec![plain_item_type.clone(), plain_item_type.clone()]);
        let mut last_error = None;
        let mut output = None;
        for signature in signatures {
            let mut vars = HashMap::new();
            match match_types(&signature.input, &pair, &mut vars) {
                Ok(()) => {
                    output =
                        Some(substitute(&signature.output, &vars).ok_or_else(|| {
                            format!("`reduce {name}` has unresolved output type")
                        })?);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let output = output.ok_or_else(|| {
            format!(
                "`reduce {name}` operation mismatch: {}",
                last_error.unwrap_or_else(|| format!("expected reduce input, found `{pair}`"))
            )
        })?;
        self.expect_type(
            &format!("reduce `{name}` result"),
            &output,
            &plain_item_type,
        )?;
        self.expect_type(
            &format!("reduce `{name}` identity"),
            identity,
            &plain_item_type,
        )?;
        if callable.name.is_empty() {
            return Err("internal error: empty callable name".to_string());
        }
        Ok(if item_faultable {
            Type::Faultable(Box::new(plain_item_type))
        } else {
            plain_item_type
        })
    }

    fn apply_scan(
        &self,
        callable: &Callable,
        name: &str,
        input: &Type,
        identity: &Type,
    ) -> Result<Type, String> {
        let input_faultable = matches!(input, Type::Faultable(_));
        let unwrapped = input.inner_faultable();
        let Type::Seq(item_type) = &unwrapped else {
            return Err(format!("`scan {name}` expected Seq input, found `{input}`"));
        };
        let plain_item_type = item_type.inner_faultable();
        self.expect_type(
            &format!("scan `{name}` identity"),
            identity,
            &plain_item_type,
        )?;
        let pair = Type::Tuple(vec![plain_item_type.clone(), plain_item_type.clone()]);
        let result = self.apply_node(callable, CallableKind::Node, name, &pair, true)?;
        self.expect_type(&format!("scan `{name}` result"), &result, &plain_item_type)?;
        if callable.name.is_empty() {
            return Err("internal error: empty callable name".to_string());
        }
        let seq = Type::Seq(Box::new(plain_item_type));
        Ok(if input_faultable {
            Type::Faultable(Box::new(seq))
        } else {
            seq
        })
    }

    fn supports_higher_order_call(&self, node: &CallableInfo) -> bool {
        !node.is_stdlib || stdlib::supports_higher_order_call(&node.runtime_name)
    }

    fn expect_type(&self, label: &str, actual: &Type, expected: &Type) -> Result<(), String> {
        let mut vars = HashMap::new();
        match_types(expected, actual, &mut vars)
            .map_err(|error| format!("{label} expected `{expected}`, found `{actual}`: {error}"))
    }
}

fn primitive_types() -> HashMap<String, Type> {
    HashMap::from([
        ("Unit".to_string(), Type::Unit),
        ("Int".to_string(), Type::Int),
        ("Real".to_string(), Type::Real),
        ("Bool".to_string(), Type::Bool),
        ("Bytes".to_string(), Type::Bytes),
        ("Fault".to_string(), Type::Fault),
        ("i1".to_string(), Type::Bool),
        ("i8".to_string(), Type::Int),
        ("i16".to_string(), Type::Int),
        ("i32".to_string(), Type::Int),
        ("i64".to_string(), Type::Int),
        ("f16".to_string(), Type::Real),
        ("float".to_string(), Type::Real),
        ("double".to_string(), Type::Real),
        ("ptr".to_string(), Type::Bytes),
        ("void".to_string(), Type::Unit),
        (
            "Number".to_string(),
            Type::OneOf(vec![Type::Int, Type::Real]),
        ),
    ])
}

fn stdlib_signatures(symbol: &stdlib::StdSymbol) -> Result<Vec<Signature>, String> {
    if symbol.module == "std.math" {
        match symbol.name {
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                return Ok(numeric_binary_signatures());
            }
            "neg" | "abs" => return Ok(numeric_unary_signatures()),
            "sqrt" => return Ok(numeric_real_unary_signatures()),
            "eq" | "lt" | "gt" | "le" | "ge" => return Ok(numeric_comparison_signatures()),
            _ => {}
        }
    }
    Ok(vec![Signature {
        input: parse_type(
            symbol
                .input
                .ok_or_else(|| format!("stdlib node `{}` has no input type", symbol.name))?,
        )?,
        output: parse_type(
            symbol
                .output
                .ok_or_else(|| format!("stdlib node `{}` has no output type", symbol.name))?,
        )?,
    }])
}

fn stdlib_reduce_signatures(symbol: &stdlib::StdSymbol) -> Result<Vec<Signature>, String> {
    if symbol.module == "std.math" && symbol.name == "add" {
        return Ok(vec![
            numeric_signature(Type::Int, Type::Int),
            numeric_signature(Type::Real, Type::Real),
        ]);
    }
    match (symbol.reduce_input, symbol.reduce_output) {
        (Some(input), Some(output)) => Ok(vec![Signature {
            input: parse_type(input)?,
            output: parse_type(output)?,
        }]),
        _ => Ok(Vec::new()),
    }
}

fn numeric_binary_signatures() -> Vec<Signature> {
    vec![
        numeric_signature(Type::Int, Type::Int),
        numeric_signature(Type::Int, Type::Real),
        numeric_signature(Type::Real, Type::Int),
        numeric_signature(Type::Real, Type::Real),
    ]
}

fn numeric_unary_signatures() -> Vec<Signature> {
    vec![
        Signature {
            input: Type::Int,
            output: Type::Int,
        },
        Signature {
            input: Type::Real,
            output: Type::Real,
        },
    ]
}

fn numeric_real_unary_signatures() -> Vec<Signature> {
    vec![
        Signature {
            input: Type::Int,
            output: Type::Real,
        },
        Signature {
            input: Type::Real,
            output: Type::Real,
        },
    ]
}

fn numeric_comparison_signatures() -> Vec<Signature> {
    vec![
        comparison_signature(Type::Int, Type::Int),
        comparison_signature(Type::Int, Type::Real),
        comparison_signature(Type::Real, Type::Int),
        comparison_signature(Type::Real, Type::Real),
    ]
}

fn numeric_signature(left: Type, right: Type) -> Signature {
    let output = if left == Type::Int && right == Type::Int {
        Type::Int
    } else {
        Type::Real
    };
    Signature {
        input: Type::Tuple(vec![left, right]),
        output,
    }
}

fn comparison_signature(left: Type, right: Type) -> Signature {
    Signature {
        input: Type::Tuple(vec![left, right]),
        output: Type::Bool,
    }
}

fn sequence_item_type(left: &Type, right: &Type) -> Result<Type, String> {
    if left == right {
        return Ok(left.clone());
    }
    match (left, right) {
        (Type::Faultable(inner), other) | (other, Type::Faultable(inner))
            if inner.as_ref() == other =>
        {
            Ok(Type::Faultable(inner.clone()))
        }
        _ => Err(format!("expected `{left}`, found `{right}`")),
    }
}

fn match_types(
    expected: &Type,
    actual: &Type,
    vars: &mut HashMap<String, Type>,
) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    match (expected, actual) {
        (Type::Var(name), actual) => {
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
        (Type::Faultable(expected), Type::Faultable(actual)) => match_types(expected, actual, vars),
        (Type::Seq(expected), Type::Seq(actual)) => match_types(expected, actual, vars),
        (Type::OneOf(expected), actual) => {
            let mut errors = Vec::new();
            for expected in expected {
                let mut candidate_vars = vars.clone();
                match match_types(expected, actual, &mut candidate_vars) {
                    Ok(()) => {
                        *vars = candidate_vars;
                        return Ok(());
                    }
                    Err(error) => errors.push(error),
                }
            }
            Err(errors.pop().unwrap_or_else(|| {
                format!(
                    "expected one of `{}`, found `{actual}`",
                    Type::OneOf(expected.clone())
                )
            }))
        }
        (expected, Type::OneOf(actual)) => {
            for actual in actual {
                match_types(expected, actual, vars)?;
            }
            Ok(())
        }
        (Type::Tuple(expected), Type::Tuple(actual)) if expected.len() == actual.len() => {
            for (expected, actual) in expected.iter().zip(actual) {
                match_types(expected, actual, vars)?;
            }
            Ok(())
        }
        _ => Err(format!("expected `{expected}`, found `{actual}`")),
    }
}

fn substitute(ty: &Type, vars: &HashMap<String, Type>) -> Option<Type> {
    match ty {
        Type::Var(name) => vars.get(name).cloned(),
        Type::Faultable(item) => Some(Type::Faultable(Box::new(substitute(item, vars)?))),
        Type::Seq(item) => Some(Type::Seq(Box::new(substitute(item, vars)?))),
        Type::OneOf(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute(item, vars)?);
            }
            Some(Type::OneOf(out))
        }
        Type::Tuple(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute(item, vars)?);
            }
            Some(Type::Tuple(out))
        }
        other => Some(other.clone()),
    }
}

fn parse_type(text: &str) -> Result<Type, String> {
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
    fn parse(&mut self) -> Result<Type, String> {
        let ty = self.parse_union_type()?;
        if self.peek().is_some() {
            return Err(format!("unexpected type syntax near `{}`", self.rest()));
        }
        Ok(ty)
    }

    fn parse_union_type(&mut self) -> Result<Type, String> {
        let mut items = vec![self.parse_type()?];
        while self.eat('|') {
            items.push(self.parse_type()?);
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Type::OneOf(items)
        })
    }

    fn parse_type(&mut self) -> Result<Type, String> {
        match self.peek() {
            Some('(') => self.parse_tuple_or_unit(),
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => self.parse_named_type(),
            _ => Err(format!("expected type, found `{}`", self.rest())),
        }
    }

    fn parse_tuple_or_unit(&mut self) -> Result<Type, String> {
        self.expect('(')?;
        if self.eat(')') {
            return Ok(Type::Unit);
        }
        let mut items = vec![self.parse_union_type()?];
        while self.eat(',') {
            items.push(self.parse_union_type()?);
        }
        self.expect(')')?;
        Ok(Type::Tuple(items))
    }

    fn parse_named_type(&mut self) -> Result<Type, String> {
        let name = self.ident();
        if name == "Seq" && self.eat('[') {
            let item = self.parse_union_type()?;
            self.expect(']')?;
            return Ok(Type::Seq(Box::new(item)));
        }
        if name == "Faultable" && self.eat('[') {
            let item = self.parse_union_type()?;
            self.expect(']')?;
            return Ok(Type::Faultable(Box::new(item)));
        }
        Ok(match name.as_str() {
            "Unit" => Type::Unit,
            "Int" => Type::Int,
            "Real" => Type::Real,
            "Bool" => Type::Bool,
            "Bytes" => Type::Bytes,
            "Args" => Type::Args,
            "Fault" => Type::Fault,
            "i1" => Type::Bool,
            "i8" | "i16" | "i32" | "i64" => Type::Int,
            "f16" | "float" | "double" => Type::Real,
            "ptr" => Type::Bytes,
            "void" => Type::Unit,
            _ => Type::Var(name),
        })
    }

    fn ident(&mut self) -> String {
        let start = self.pos;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
        {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.eat(expected) {
            Ok(())
        } else {
            Err(format!("expected `{expected}`, found `{}`", self.rest()))
        }
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn rest(&self) -> String {
        self.chars[self.pos..].iter().collect()
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unit => write!(f, "()"),
            Type::Int => write!(f, "Int"),
            Type::Real => write!(f, "Real"),
            Type::Bool => write!(f, "Bool"),
            Type::Bytes => write!(f, "Bytes"),
            Type::Args => write!(f, "Args"),
            Type::Fault => write!(f, "Fault"),
            Type::Faultable(item) => write!(f, "Faultable[{item}]"),
            Type::Seq(item) => write!(f, "Seq[{item}]"),
            Type::OneOf(items) => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        write!(f, "|")?;
                    }
                    write!(f, "{item}")?;
                }
                Ok(())
            }
            Type::Tuple(items) => {
                write!(f, "(")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
            Type::Var(name) => write!(f, "{name}"),
        }
    }
}
