use crate::ast::*;
use crate::stdlib::{self, Effect, RuntimeSupport, SymbolKind};
use std::collections::HashMap;
use std::fmt;

pub fn check_module(module: &Module) -> Result<(), String> {
    Checker::new(module)?.check()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Unit,
    Int,
    Real,
    Bool,
    Bytes,
    Args,
    Seq(Box<Type>),
    Tuple(Vec<Type>),
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
    signature: Signature,
    reduce_signature: Option<Signature>,
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
        self.expect_type("program main input", &main.signature.input, &Type::Args)?;
        self.expect_type("program main output", &main.signature.output, &Type::Int)?;

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
                if self.types.insert(name.to_string(), Type::Args).is_some() {
                    return Err(format!("duplicate type import `{name}`"));
                }
            }
            SymbolKind::Node => {
                let input =
                    parse_type(symbol.input.ok_or_else(|| {
                        format!("stdlib node `{}` has no input type", symbol.name)
                    })?)?;
                let output =
                    parse_type(symbol.output.ok_or_else(|| {
                        format!("stdlib node `{}` has no output type", symbol.name)
                    })?)?;
                self.insert_symbol(
                    name,
                    CallableInfo {
                        signature: Signature { input, output },
                        reduce_signature: match (symbol.reduce_input, symbol.reduce_output) {
                            (Some(input), Some(output)) => Some(Signature {
                                input: parse_type(input)?,
                                output: parse_type(output)?,
                            }),
                            _ => None,
                        },
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
                signature: self.callable_signature(callable)?,
                reduce_signature: None,
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
            Type::Seq(item) => Ok(Type::Seq(Box::new(self.resolve_declared_type(*item)?))),
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
            Type::Seq(item) => self.validate_declared_type(item),
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
                Stage::Endpoint(Endpoint::Name(name))
                    if is_last && !self.symbols.contains_key(name) =>
                {
                    if env.insert(name.clone(), value_type.clone()).is_some() {
                        return Err(format!("value `{name}` is bound more than once"));
                    }
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value_type = self.apply_node(callable, kind, name, &value_type, false)?;
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
                }
                Stage::Map(name) => {
                    value_type = self.apply_map(callable, name, &value_type)?;
                }
                Stage::Filter(name) => {
                    value_type = self.apply_filter(callable, name, &value_type)?;
                }
                Stage::Reduce { op, identity } => {
                    let identity_type = self.endpoint_type(identity, env)?;
                    value_type = self.apply_reduce(callable, op, &value_type, &identity_type)?;
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
            Endpoint::Name(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
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
                        self.expect_type("sequence literal item", &ty, expected)?;
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
        if as_function && !self.is_function_pointer_compatible(node) {
            return Err(format!("`{name}` cannot be used as a map/filter function"));
        }
        if !as_function && node.runtime == RuntimeSupport::ReduceOnly {
            return Err(format!("`{name}` can only be used as a reduce operation"));
        }
        let mut vars = HashMap::new();
        match_types(&node.signature.input, input, &mut vars)
            .map_err(|error| format!("`{name}` input type mismatch: {error}"))?;
        substitute(&node.signature.output, &vars)
            .ok_or_else(|| format!("`{name}` output type contains unresolved type variables"))
    }

    fn apply_map(&self, callable: &Callable, name: &str, input: &Type) -> Result<Type, String> {
        let Type::Seq(item_type) = input else {
            return Err(format!("`map {name}` expected Seq input, found `{input}`"));
        };
        let output = self.apply_node(callable, CallableKind::Node, name, item_type, true)?;
        Ok(Type::Seq(Box::new(output)))
    }

    fn apply_filter(&self, callable: &Callable, name: &str, input: &Type) -> Result<Type, String> {
        let Type::Seq(item_type) = input else {
            return Err(format!(
                "`filter {name}` expected Seq input, found `{input}`"
            ));
        };
        let output = self.apply_node(callable, CallableKind::Node, name, item_type, true)?;
        self.expect_type(&format!("filter `{name}` result"), &output, &Type::Bool)?;
        Ok(input.clone())
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
        let signature = node
            .reduce_signature
            .as_ref()
            .ok_or_else(|| format!("`{name}` is not implemented as a reduce operation"))?;
        let pair = Type::Tuple(vec![(**item_type).clone(), (**item_type).clone()]);
        let mut vars = HashMap::new();
        match_types(&signature.input, &pair, &mut vars)
            .map_err(|error| format!("`reduce {name}` operation mismatch: {error}"))?;
        let output = substitute(&signature.output, &vars)
            .ok_or_else(|| format!("`reduce {name}` has unresolved output type"))?;
        self.expect_type(&format!("reduce `{name}` result"), &output, item_type)?;
        self.expect_type(&format!("reduce `{name}` identity"), identity, item_type)?;
        if callable.name.is_empty() {
            return Err("internal error: empty callable name".to_string());
        }
        Ok((**item_type).clone())
    }

    fn is_function_pointer_compatible(&self, node: &CallableInfo) -> bool {
        !node.is_stdlib || stdlib::function_pointer(&node.runtime_name).is_some()
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
    ])
}

fn match_types(
    expected: &Type,
    actual: &Type,
    vars: &mut HashMap<String, Type>,
) -> Result<(), String> {
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
        (Type::Seq(expected), Type::Seq(actual)) => match_types(expected, actual, vars),
        (Type::Tuple(expected), Type::Tuple(actual)) if expected.len() == actual.len() => {
            for (expected, actual) in expected.iter().zip(actual) {
                match_types(expected, actual, vars)?;
            }
            Ok(())
        }
        (expected, actual) if expected == actual => Ok(()),
        _ => Err(format!("expected `{expected}`, found `{actual}`")),
    }
}

fn substitute(ty: &Type, vars: &HashMap<String, Type>) -> Option<Type> {
    match ty {
        Type::Var(name) => vars.get(name).cloned(),
        Type::Seq(item) => Some(Type::Seq(Box::new(substitute(item, vars)?))),
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
        let ty = self.parse_type()?;
        if self.peek().is_some() {
            return Err(format!("unexpected type syntax near `{}`", self.rest()));
        }
        Ok(ty)
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
        let mut items = vec![self.parse_type()?];
        while self.eat(',') {
            items.push(self.parse_type()?);
        }
        self.expect(')')?;
        Ok(Type::Tuple(items))
    }

    fn parse_named_type(&mut self) -> Result<Type, String> {
        let name = self.ident();
        if name == "Seq" && self.eat('[') {
            let item = self.parse_type()?;
            self.expect(']')?;
            return Ok(Type::Seq(Box::new(item)));
        }
        Ok(match name.as_str() {
            "Unit" => Type::Unit,
            "Int" => Type::Int,
            "Real" => Type::Real,
            "Bool" => Type::Bool,
            "Bytes" => Type::Bytes,
            "Args" => Type::Args,
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
            Type::Seq(item) => write!(f, "Seq[{item}]"),
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
