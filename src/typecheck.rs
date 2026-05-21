use crate::ast::*;
use crate::module_resolver;
use crate::stdlib::{self, Effect, RuntimeSupport, SymbolKind};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

#[allow(dead_code)]
pub fn check_module(module: &Module) -> Result<(), String> {
    let expanded = module_resolver::expand_stdlib_sources(module)?;
    Checker::new(&expanded)?.check()
}

pub fn check_module_with_base(module: &Module, base_dir: &Path) -> Result<(), String> {
    let expanded = module_resolver::expand_sources(module, Some(base_dir))?;
    Checker::new(&expanded)?.check()
}

pub(crate) fn semantic_summary_with_base(
    module: &Module,
    base_dir: &Path,
) -> Result<SemanticSummary, String> {
    let expanded = module_resolver::expand_sources(module, Some(base_dir))?;
    Checker::new(&expanded)?.semantic_summary()
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticSummary {
    pub callables: Vec<CallableSummary>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CallableSummary {
    pub name: String,
    pub variables: Vec<ValueSummary>,
    pub chains: Vec<ChainSummary>,
}

#[derive(Debug, Clone)]
pub(crate) struct ValueSummary {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ChainSummary {
    pub source: EndpointSummary,
    pub stages: Vec<StageSummary>,
}

#[derive(Debug, Clone)]
pub(crate) struct EndpointSummary {
    pub label: String,
    pub ty: String,
}

#[derive(Debug, Clone)]
pub(crate) struct StageSummary {
    pub label: String,
    pub input: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Unit,
    Int,
    Real,
    Bool,
    Bytes,
    Args,
    HttpServerConfig,
    HttpListener,
    HttpRequest,
    HttpResponse,
    SqliteConnection,
    SqliteRow,
    SqliteValue,
    Stream(Box<Type>),
    Fault,
    Faultable(Box<Type>),
    Seq(Box<Type>),
    Tuple(Vec<Type>),
    OneOf(Vec<Type>),
    Var(String),
    EmptySeq,
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
            Type::Seq(item) | Type::Stream(item) => item.contains_faultable(),
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
            Type::Stream(item) => Type::Stream(Box::new(item.strip_faultable())),
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
        checker.collect_type_aliases()?;
        checker.collect_callables()?;
        checker.infer_effects();
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
                Decl::TypeAlias(_) => {}
                Decl::Node(callable) => self.check_callable(callable, CallableKind::Node)?,
                Decl::Program(callable) => self.check_callable(callable, CallableKind::Program)?,
                Decl::Import(_) => {}
            }
        }
        Ok(())
    }

    fn semantic_summary(&self) -> Result<SemanticSummary, String> {
        self.validate_main()?;
        let mut summary = SemanticSummary::default();
        for decl in &self.module.declarations {
            let (callable, kind) = match decl {
                Decl::Node(callable) => (callable, CallableKind::Node),
                Decl::Program(callable) => (callable, CallableKind::Program),
                Decl::TypeAlias(_) | Decl::Import(_) => continue,
            };
            summary
                .callables
                .push(self.summarize_callable(callable, kind)?);
        }
        Ok(summary)
    }

    fn validate_main(&self) -> Result<(), String> {
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
                continue;
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
                let ty = stdlib_type_symbol(symbol.name)?;
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

    fn collect_type_aliases(&mut self) -> Result<(), String> {
        let mut raw = HashMap::new();
        for decl in &self.module.declarations {
            let Decl::TypeAlias(alias) = decl else {
                continue;
            };
            if self.types.contains_key(&alias.name) || raw.contains_key(&alias.name) {
                return Err(format!("duplicate type declaration `{}`", alias.name));
            }
            raw.insert(alias.name.clone(), alias.ty.clone());
        }

        let mut resolved = HashMap::new();
        for name in raw.keys() {
            let mut resolving = Vec::new();
            let ty = self.resolve_type_alias(name, &raw, &mut resolved, &mut resolving)?;
            self.validate_declared_type(&ty)?;
            self.types.insert(name.clone(), ty);
        }
        Ok(())
    }

    fn resolve_type_alias(
        &self,
        name: &str,
        raw: &HashMap<String, String>,
        resolved: &mut HashMap<String, Type>,
        resolving: &mut Vec<String>,
    ) -> Result<Type, String> {
        if let Some(ty) = resolved.get(name) {
            return Ok(ty.clone());
        }
        if resolving.iter().any(|item| item == name) {
            resolving.push(name.to_string());
            return Err(format!("cyclic type alias `{}`", resolving.join(" -> ")));
        }
        let text = raw
            .get(name)
            .ok_or_else(|| format!("unknown type alias `{name}`"))?;
        resolving.push(name.to_string());
        let parsed = parse_type(text)?;
        let ty = self.resolve_type_alias_type(parsed, raw, resolved, resolving)?;
        resolving.pop();
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    }

    fn resolve_type_alias_type(
        &self,
        ty: Type,
        raw: &HashMap<String, String>,
        resolved: &mut HashMap<String, Type>,
        resolving: &mut Vec<String>,
    ) -> Result<Type, String> {
        match ty {
            Type::Var(name) => {
                if let Some(known) = self.types.get(&name) {
                    Ok(known.clone())
                } else if raw.contains_key(&name) {
                    self.resolve_type_alias(&name, raw, resolved, resolving)
                } else {
                    Err(format!("unknown type `{name}`"))
                }
            }
            Type::Faultable(item) => Ok(Type::Faultable(Box::new(
                self.resolve_type_alias_type(*item, raw, resolved, resolving)?,
            ))),
            Type::Seq(item) => Ok(Type::Seq(Box::new(
                self.resolve_type_alias_type(*item, raw, resolved, resolving)?,
            ))),
            Type::Stream(item) => Ok(Type::Stream(Box::new(
                self.resolve_type_alias_type(*item, raw, resolved, resolving)?,
            ))),
            Type::OneOf(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_type_alias_type(item, raw, resolved, resolving)?);
                }
                Ok(Type::OneOf(out))
            }
            Type::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_type_alias_type(item, raw, resolved, resolving)?);
                }
                Ok(Type::Tuple(out))
            }
            other => Ok(other),
        }
    }

    fn collect_callables(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            let (callable, kind) = match decl {
                Decl::Node(callable) => (callable, CallableKind::Node),
                Decl::Program(callable) => (callable, CallableKind::Program),
                Decl::TypeAlias(_) | Decl::Import(_) => continue,
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

    fn infer_effects(&mut self) {
        loop {
            let mut changed = false;
            for decl in &self.module.declarations {
                let (Decl::Node(callable) | Decl::Program(callable)) = decl else {
                    continue;
                };
                if self
                    .symbols
                    .get(&callable.name)
                    .map(|info| info.effect == Effect::Io)
                    .unwrap_or(false)
                {
                    continue;
                }
                if self.callable_uses_io(callable)
                    && let Some(info) = self.symbols.get_mut(&callable.name)
                {
                    info.effect = Effect::Io;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn callable_uses_io(&self, callable: &Callable) -> bool {
        callable.chains.iter().any(|chain| {
            chain.stages.iter().any(|stage| match stage {
                Stage::Endpoint(Endpoint::Name(name))
                | Stage::Map(name)
                | Stage::Filter(name)
                | Stage::Reduce { op: name, .. }
                | Stage::Scan { op: name, .. } => self.symbol_effect(name) == Effect::Io,
                Stage::FaultMap { node, .. } | Stage::Repeat { node, .. } => {
                    self.symbol_effect(node) == Effect::Io
                }
                Stage::Match { arms } => arms.iter().any(|arm| match &arm.target {
                    MatchTarget::Node(node) => self.symbol_effect(node) == Effect::Io,
                    MatchTarget::Value(_) => false,
                }),
                Stage::Bind(_) => false,
                Stage::Endpoint(_) => false,
            })
        })
    }

    fn symbol_effect(&self, name: &str) -> Effect {
        self.symbols
            .get(name)
            .map(|info| info.effect)
            .unwrap_or(Effect::Pure)
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
            Type::Stream(item) => Ok(Type::Stream(Box::new(self.resolve_declared_type(*item)?))),
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
            Type::Stream(item) => {
                if !self.types.values().any(is_stream_constructor) {
                    return Err("unknown type `Stream`".to_string());
                }
                self.validate_declared_type(item)
            }
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
            self.expect_assignable_type(
                &format!("output `{}` of `{}`", output.name, callable.name),
                actual,
                &expected,
            )?;
        }
        Ok(())
    }

    fn summarize_callable(
        &self,
        callable: &Callable,
        kind: CallableKind,
    ) -> Result<CallableSummary, String> {
        let mut env = HashMap::new();
        let mut summary = CallableSummary {
            name: callable.name.clone(),
            variables: Vec::new(),
            chains: Vec::new(),
        };
        for port in &callable.inputs {
            let ty = self.parse_declared_type(&port.ty)?;
            if env.insert(port.name.clone(), ty.clone()).is_some() {
                return Err(format!(
                    "`{}` declares input `{}` more than once",
                    callable.name, port.name
                ));
            }
            summary.variables.push(ValueSummary {
                name: port.name.clone(),
                ty: ty.to_string(),
            });
        }
        for chain in &callable.chains {
            let chain_summary =
                self.summarize_chain(callable, kind, chain, &mut env, &mut summary)?;
            summary.chains.push(chain_summary);
        }
        for output in &callable.outputs {
            let expected = self.parse_declared_type(&output.ty)?;
            let actual = env.get(&output.name).ok_or_else(|| {
                format!(
                    "`{}` declares output `{}` but it is never bound",
                    callable.name, output.name
                )
            })?;
            self.expect_assignable_type(
                &format!("output `{}` of `{}`", output.name, callable.name),
                actual,
                &expected,
            )?;
        }
        Ok(summary)
    }

    fn summarize_chain(
        &self,
        callable: &Callable,
        kind: CallableKind,
        chain: &Chain,
        env: &mut HashMap<String, Type>,
        callable_summary: &mut CallableSummary,
    ) -> Result<ChainSummary, String> {
        let mut value_type = self.endpoint_type(&chain.source, env)?;
        let mut summary = ChainSummary {
            source: EndpointSummary {
                label: format_endpoint_for_error(&chain.source),
                ty: value_type.to_string(),
            },
            stages: Vec::new(),
        };
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            let input_type = value_type.clone();
            let (label, output_type) = match stage {
                Stage::Bind(target) if is_last => {
                    let bindings = binding_target_types(target, &value_type)?;
                    for (name, ty) in bindings {
                        if contains_empty_seq(&ty) {
                            return Err("empty sequence literals need a type context".to_string());
                        }
                        if env.insert(name.clone(), ty.clone()).is_some() {
                            return Err(format!("value `{name}` is bound more than once"));
                        }
                        callable_summary.variables.push(ValueSummary {
                            name,
                            ty: ty.to_string(),
                        });
                    }
                    (format_binding_target_for_error(target), value_type.clone())
                }
                Stage::Endpoint(Endpoint::Name(name)) => (
                    name.clone(),
                    self.apply_node(callable, kind, name, &value_type, false, false)?,
                ),
                Stage::Endpoint(Endpoint::Variable(_)) => {
                    return Err(
                        "variables may only appear as source values or final bindings".to_string(),
                    );
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
                }
                Stage::Bind(_) => {
                    return Err("binding targets may only appear as final stages".to_string());
                }
                Stage::Map(name) => (
                    format!("map {name}"),
                    self.apply_map(callable, name, &value_type)?,
                ),
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let (ok_type, fault_type) =
                        self.apply_fault_map(callable, node, &value_type)?;
                    if env.insert(ok.clone(), ok_type.clone()).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    if env.insert(fault.clone(), fault_type.clone()).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                    callable_summary.variables.push(ValueSummary {
                        name: ok.clone(),
                        ty: ok_type.to_string(),
                    });
                    callable_summary.variables.push(ValueSummary {
                        name: fault.clone(),
                        ty: fault_type.to_string(),
                    });
                    (
                        format!("fault map {node}"),
                        Type::Tuple(vec![ok_type, fault_type]),
                    )
                }
                Stage::Filter(name) => (
                    format!("filter {name}"),
                    self.apply_filter(callable, name, &value_type)?,
                ),
                Stage::Repeat { count, node } => {
                    let count_type = self.endpoint_type(count, env)?;
                    self.expect_type("repeat count", &count_type, &Type::Int)?;
                    (
                        format!("repeat {node}"),
                        self.apply_repeat(callable, node, &value_type)?,
                    )
                }
                Stage::Reduce { op, identity } => {
                    let identity_type = self.endpoint_type(identity, env)?;
                    (
                        format!("reduce {op}"),
                        self.apply_reduce(callable, op, &value_type, &identity_type)?,
                    )
                }
                Stage::Scan { op, identity } => {
                    let identity_type = self.endpoint_type(identity, env)?;
                    (
                        format!("scan {op}"),
                        self.apply_scan(callable, op, &value_type, &identity_type)?,
                    )
                }
                Stage::Match { arms } => (
                    "match".to_string(),
                    self.apply_match(callable, kind, arms, &value_type, env)?,
                ),
            };
            value_type = output_type;
            summary.stages.push(StageSummary {
                label,
                input: input_type.to_string(),
                output: value_type.to_string(),
            });
        }
        Ok(summary)
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
                Stage::Bind(target) if is_last => {
                    for (name, ty) in binding_target_types(target, &value_type)? {
                        if contains_empty_seq(&ty) {
                            return Err("empty sequence literals need a type context".to_string());
                        }
                        if env.insert(name.clone(), ty).is_some() {
                            return Err(format!("value `{name}` is bound more than once"));
                        }
                    }
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value_type =
                        self.apply_node(callable, kind, name, &value_type, false, false)?;
                }
                Stage::Endpoint(Endpoint::Variable(_)) => {
                    return Err(
                        "variables may only appear as source values or final bindings".to_string(),
                    );
                }
                Stage::Endpoint(_) => {
                    return Err("non-name endpoints may only appear as source values".to_string());
                }
                Stage::Bind(_) => {
                    return Err("binding targets may only appear as final stages".to_string());
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
                Stage::Match { arms } => {
                    value_type = self.apply_match(callable, kind, arms, &value_type, env)?;
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
                match item_type {
                    Some(item_type) => Ok(Type::Seq(Box::new(item_type))),
                    None => Ok(Type::EmptySeq),
                }
            }
            Endpoint::Eval { source, stages } => self.inline_eval_type(source, stages, env),
        }
    }

    fn inline_eval_type(
        &self,
        source: &Endpoint,
        stages: &[Stage],
        env: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        let inline_callable = Callable {
            name: "<inline>".to_string(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            chains: Vec::new(),
        };
        let mut value_type = self.endpoint_type(source, env)?;
        for stage in stages {
            match stage {
                Stage::Endpoint(Endpoint::Name(name)) => {
                    value_type = self.apply_node(
                        &inline_callable,
                        CallableKind::Node,
                        name,
                        &value_type,
                        false,
                        false,
                    )?;
                }
                Stage::Endpoint(Endpoint::Variable(_)) => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                Stage::Bind(_) => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                Stage::Endpoint(_) => {
                    return Err(
                        "non-name endpoints may only appear as inline evaluation sources"
                            .to_string(),
                    );
                }
                Stage::Map(name) => {
                    value_type = self.apply_map(&inline_callable, name, &value_type)?;
                }
                Stage::FaultMap { .. } => {
                    return Err("inline evaluations cannot use `fault map`".to_string());
                }
                Stage::Filter(name) => {
                    value_type = self.apply_filter(&inline_callable, name, &value_type)?;
                }
                Stage::Repeat { count, node } => {
                    let count_type = self.endpoint_type(count, env)?;
                    self.expect_type("repeat count", &count_type, &Type::Int)?;
                    value_type = self.apply_repeat(&inline_callable, node, &value_type)?;
                }
                Stage::Reduce { op, identity } => {
                    let identity_type = self.endpoint_type(identity, env)?;
                    value_type =
                        self.apply_reduce(&inline_callable, op, &value_type, &identity_type)?;
                }
                Stage::Scan { op, identity } => {
                    let identity_type = self.endpoint_type(identity, env)?;
                    value_type =
                        self.apply_scan(&inline_callable, op, &value_type, &identity_type)?;
                }
                Stage::Match { arms } => {
                    value_type = self.apply_match(
                        &inline_callable,
                        CallableKind::Node,
                        arms,
                        &value_type,
                        env,
                    )?;
                }
            }
        }
        Ok(value_type)
    }

    fn apply_node(
        &self,
        _callable: &Callable,
        _context: CallableKind,
        name: &str,
        input: &Type,
        as_function: bool,
        allow_effectful_function: bool,
    ) -> Result<Type, String> {
        let node = self
            .symbols
            .get(name)
            .ok_or_else(|| format!("unknown node `{name}`"))?;
        if node.kind == CallableKind::Program {
            return Err(format!("program `{name}` cannot be called from a graph"));
        }
        if as_function && !self.supports_higher_order_call(node) {
            return Err(format!("`{name}` cannot be used as a map/filter function"));
        }
        if as_function && node.effect != Effect::Pure && !allow_effectful_function {
            return Err(format!("`{name}` cannot be used as a map/filter function"));
        }
        if !as_function && node.runtime == RuntimeSupport::ReduceOnly {
            return Err(format!("`{name}` can only be used as a reduce operation"));
        }
        let preserves_faultable_input = node.runtime_name == "collect";
        let mut last_error = None;
        let mut output = None;
        for signature in &node.signatures {
            if !signature.input.contains_faultable() {
                continue;
            }
            let mut vars = HashMap::new();
            match match_types(&signature.input, input, &mut vars) {
                Ok(()) => {
                    if contains_empty_seq(input) {
                        substitute(&signature.input, &vars).ok_or_else(|| {
                            "empty sequence literals need a concrete type context".to_string()
                        })?;
                    }
                    output = Some(substitute(&signature.output, &vars).ok_or_else(|| {
                        format!("`{name}` output type contains unresolved type variables")
                    })?);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        if output.is_none() && !preserves_faultable_input {
            let input_faultable = input.contains_faultable();
            let actual_input = input.strip_faultable();
            for signature in &node.signatures {
                let mut vars = HashMap::new();
                match match_types(&signature.input, &actual_input, &mut vars) {
                    Ok(()) => {
                        if contains_empty_seq(&actual_input) {
                            substitute(&signature.input, &vars).ok_or_else(|| {
                                "empty sequence literals need a concrete type context".to_string()
                            })?;
                        }
                        let plain_output =
                            substitute(&signature.output, &vars).ok_or_else(|| {
                                format!("`{name}` output type contains unresolved type variables")
                            })?;
                        output = Some(
                            if input_faultable && !matches!(plain_output, Type::Faultable(_)) {
                                Type::Faultable(Box::new(plain_output))
                            } else {
                                plain_output
                            },
                        );
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
            }
        }
        let output = output.ok_or_else(|| {
            format!(
                "`{name}` input type mismatch: {}",
                last_error.unwrap_or_else(|| format!("expected callable input, found `{input}`"))
            )
        })?;
        Ok(output)
    }

    fn apply_map(&self, callable: &Callable, name: &str, input: &Type) -> Result<Type, String> {
        let input_faultable = matches!(input, Type::Faultable(_));
        let unwrapped = input.inner_faultable();
        let output = match &unwrapped {
            Type::Seq(item_type) => {
                let output =
                    self.apply_node(callable, CallableKind::Node, name, item_type, true, true)?;
                Type::Seq(Box::new(output))
            }
            Type::Stream(item_type) => {
                let output =
                    self.apply_node(callable, CallableKind::Node, name, item_type, true, true)?;
                Type::Stream(Box::new(output))
            }
            _ => {
                return Err(format!(
                    "`map {name}` expected Seq or Stream input, found `{input}`"
                ));
            }
        };
        Ok(if input_faultable {
            Type::Faultable(Box::new(output))
        } else {
            output
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
        let output = self.apply_node(callable, CallableKind::Node, name, item_type, true, true)?;
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
        let output = self.apply_node(callable, CallableKind::Node, name, item_type, true, false)?;
        self.expect_type(&format!("filter `{name}` result"), &output, &Type::Bool)?;
        Ok(if input_faultable {
            Type::Faultable(Box::new(unwrapped))
        } else {
            unwrapped
        })
    }

    fn apply_repeat(&self, callable: &Callable, name: &str, input: &Type) -> Result<Type, String> {
        let output = self.apply_node(callable, CallableKind::Node, name, input, true, false)?;
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
        let input_faultable = matches!(input, Type::Faultable(_));
        let unwrapped = input.inner_faultable();
        let Type::Seq(item_type) = &unwrapped else {
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
        Ok(if input_faultable || item_faultable {
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
        let result = self.apply_node(callable, CallableKind::Node, name, &pair, true, false)?;
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

    fn apply_match(
        &self,
        callable: &Callable,
        kind: CallableKind,
        arms: &[MatchArm],
        input: &Type,
        env: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        if arms.is_empty() {
            return Err("`match` must contain at least one arm".to_string());
        }
        if !matches!(
            arms.last().map(|arm| &arm.guard),
            Some(MatchGuard::Fallback)
        ) {
            return Err("`match` must end with a `_` fallback arm".to_string());
        }

        let mut result = None;
        for (index, arm) in arms.iter().enumerate() {
            match &arm.guard {
                MatchGuard::Fallback if index + 1 != arms.len() => {
                    return Err("`match` fallback arm must be last".to_string());
                }
                MatchGuard::Fallback => {}
                MatchGuard::Call { node, args } => {
                    let guard = self
                        .symbols
                        .get(node)
                        .ok_or_else(|| format!("unknown match guard `{node}`"))?;
                    if guard.kind == CallableKind::Program {
                        return Err(format!("program `{node}` cannot be used as a match guard"));
                    }
                    if guard.effect != Effect::Pure {
                        return Err(format!("match guard `{node}` must be pure"));
                    }
                    let mut input_items = vec![input.clone()];
                    for arg in args {
                        input_items.push(self.endpoint_type(arg, env)?);
                    }
                    let guard_input = single_or_tuple(input_items);
                    let guard_output =
                        self.apply_node(callable, kind, node, &guard_input, false, false)?;
                    self.expect_type(
                        &format!("match guard `{node}` result"),
                        &guard_output,
                        &Type::Bool,
                    )?;
                }
            }

            let arm_output = match &arm.target {
                MatchTarget::Node(node) => {
                    self.apply_node(callable, kind, node, input, false, false)?
                }
                MatchTarget::Value(endpoint) => self.endpoint_type(endpoint, env)?,
            };
            if let Some(expected) = &result {
                result = Some(common_assignable_type(
                    expected,
                    &arm_output,
                    &format!("match arm `{}` result", format_match_target(&arm.target)),
                )?);
            } else {
                result = Some(arm_output);
            }
        }
        result.ok_or_else(|| "`match` must contain at least one arm".to_string())
    }

    fn supports_higher_order_call(&self, node: &CallableInfo) -> bool {
        !node.is_stdlib || stdlib::supports_higher_order_call(&node.runtime_name)
    }

    fn expect_type(&self, label: &str, actual: &Type, expected: &Type) -> Result<(), String> {
        let mut vars = HashMap::new();
        match_types(expected, actual, &mut vars)
            .map_err(|error| format!("{label} expected `{expected}`, found `{actual}`: {error}"))
    }

    fn expect_assignable_type(
        &self,
        label: &str,
        actual: &Type,
        expected: &Type,
    ) -> Result<(), String> {
        assignable_type(expected, actual)
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
    ])
}

fn stdlib_type_symbol(name: &str) -> Result<Type, String> {
    if name == "Stream" {
        Ok(Type::Stream(Box::new(Type::Var("V".to_string()))))
    } else if name == "Args" {
        Ok(Type::Args)
    } else if name == "Fault" {
        Ok(Type::Fault)
    } else if name == "ServerConfig" {
        Ok(Type::HttpServerConfig)
    } else if name == "Listener" {
        Ok(Type::HttpListener)
    } else if name == "Request" {
        Ok(Type::HttpRequest)
    } else if name == "Response" {
        Ok(Type::HttpResponse)
    } else if name == "Connection" {
        Ok(Type::SqliteConnection)
    } else if name == "Row" {
        Ok(Type::SqliteRow)
    } else if name == "Value" {
        Ok(Type::SqliteValue)
    } else {
        parse_type(name)
    }
}

fn is_stream_constructor(ty: &Type) -> bool {
    matches!(ty, Type::Stream(item) if matches!(item.as_ref(), Type::Var(_)))
}

fn single_or_tuple(mut items: Vec<Type>) -> Type {
    if items.len() == 1 {
        items.remove(0)
    } else {
        Type::Tuple(items)
    }
}

fn binding_target_types(
    target: &BindingTarget,
    value_type: &Type,
) -> Result<Vec<(String, Type)>, String> {
    match target {
        BindingTarget::Variable(name) => Ok(vec![(name.clone(), value_type.clone())]),
        BindingTarget::Tuple(items) => {
            let field_types = match value_type {
                Type::Tuple(field_types) if field_types.len() == items.len() => field_types.clone(),
                Type::Faultable(inner) => {
                    let Type::Tuple(field_types) = inner.as_ref() else {
                        return Err(format!(
                            "binding target `{}` expected tuple input, found `{value_type}`",
                            format_binding_target_for_error(target)
                        ));
                    };
                    if field_types.len() != items.len() {
                        return Err(format!(
                            "binding target `{}` expected {} tuple fields, found {}",
                            format_binding_target_for_error(target),
                            items.len(),
                            field_types.len()
                        ));
                    }
                    field_types
                        .iter()
                        .map(faultable_projection_type)
                        .collect::<Vec<_>>()
                }
                Type::Tuple(field_types) => {
                    return Err(format!(
                        "binding target `{}` expected {} tuple fields, found {}",
                        format_binding_target_for_error(target),
                        items.len(),
                        field_types.len()
                    ));
                }
                _ => {
                    return Err(format!(
                        "binding target `{}` expected tuple input, found `{value_type}`",
                        format_binding_target_for_error(target)
                    ));
                }
            };

            let mut bindings = Vec::new();
            for (item, field_type) in items.iter().zip(field_types.iter()) {
                bindings.extend(binding_target_types(item, field_type)?);
            }
            Ok(bindings)
        }
    }
}

fn faultable_projection_type(ty: &Type) -> Type {
    match ty {
        Type::Faultable(_) => ty.clone(),
        other => Type::Faultable(Box::new(other.clone())),
    }
}

fn format_match_target(target: &MatchTarget) -> String {
    match target {
        MatchTarget::Node(node) => node.clone(),
        MatchTarget::Value(endpoint) => format_endpoint_for_error(endpoint),
    }
}

fn format_binding_target_for_error(target: &BindingTarget) -> String {
    match target {
        BindingTarget::Variable(name) => format!("${name}"),
        BindingTarget::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(format_binding_target_for_error)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn format_endpoint_for_error(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Variable(name) => format!("${name}"),
        Endpoint::Name(name) => name.clone(),
        Endpoint::Int(value) => value.to_string(),
        Endpoint::Real(value) => value.to_string(),
        Endpoint::Bool(value) => value.to_string(),
        Endpoint::String(value) => format!("{value:?}"),
        Endpoint::Unit => "()".to_string(),
        Endpoint::Tuple(items) => format_endpoint_list_for_error(items, "(", ")"),
        Endpoint::Seq(items) => format_endpoint_list_for_error(items, "[", "]"),
        Endpoint::Eval { source, stages } => {
            let mut parts = Vec::with_capacity(stages.len() + 1);
            parts.push(format_endpoint_for_error(source));
            parts.extend(stages.iter().map(format_stage_for_error));
            parts.join(" -> ")
        }
    }
}

fn format_stage_for_error(stage: &Stage) -> String {
    match stage {
        Stage::Endpoint(endpoint) => format_endpoint_for_error(endpoint),
        Stage::Bind(target) => format_binding_target_for_error(target),
        Stage::Map(name) => format!("map {name}"),
        Stage::FaultMap { node, .. } => format!("fault map {node}"),
        Stage::Filter(name) => format!("filter {name}"),
        Stage::Repeat { node, .. } => format!("repeat {node}"),
        Stage::Reduce { op, .. } => format!("reduce {op}"),
        Stage::Scan { op, .. } => format!("scan {op}"),
        Stage::Match { .. } => "match".to_string(),
    }
}

fn format_endpoint_list_for_error(items: &[Endpoint], open: &str, close: &str) -> String {
    let mut output = String::from(open);
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(&format_endpoint_for_error(item));
    }
    output.push_str(close);
    output
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
        (Type::EmptySeq, other) | (other, Type::EmptySeq) => Ok(other.clone()),
        (Type::Faultable(inner), other) | (other, Type::Faultable(inner))
            if inner.as_ref() == other =>
        {
            Ok(Type::Faultable(inner.clone()))
        }
        _ => Err(format!("expected `{left}`, found `{right}`")),
    }
}

fn assignable_type(expected: &Type, actual: &Type) -> Result<(), String> {
    let mut vars = HashMap::new();
    match_types(expected, actual, &mut vars).or_else(|_| {
        if let Type::Faultable(inner) = expected {
            if let Some(actual) = unwrap_faultable_tuple_type(actual) {
                let mut vars = HashMap::new();
                return match_types(inner, &actual, &mut vars);
            }
            let mut vars = HashMap::new();
            match_types(inner, actual, &mut vars)
        } else {
            Err(format!("expected `{expected}`, found `{actual}`"))
        }
    })
}

fn common_assignable_type(current: &Type, next: &Type, label: &str) -> Result<Type, String> {
    if assignable_type(current, next).is_ok() {
        return Ok(current.clone());
    }
    if assignable_type(next, current).is_ok() {
        return Ok(next.clone());
    }
    Err(format!("{label} expected `{current}`, found `{next}`"))
}

fn unwrap_faultable_tuple_type(input: &Type) -> Option<Type> {
    let Type::Tuple(items) = input else {
        return None;
    };
    let mut saw_faultable = false;
    let unwrapped = items
        .iter()
        .map(|item| match item {
            Type::Faultable(inner) => {
                saw_faultable = true;
                inner.as_ref().clone()
            }
            Type::Tuple(_) => {
                if let Some(unwrapped) = unwrap_faultable_tuple_type(item) {
                    saw_faultable = true;
                    unwrapped
                } else {
                    item.clone()
                }
            }
            other => other.clone(),
        })
        .collect::<Vec<_>>();
    saw_faultable.then_some(Type::Tuple(unwrapped))
}

fn contains_empty_seq(input: &Type) -> bool {
    match input {
        Type::EmptySeq => true,
        Type::Faultable(item) | Type::Seq(item) | Type::Stream(item) => contains_empty_seq(item),
        Type::Tuple(items) | Type::OneOf(items) => items.iter().any(contains_empty_seq),
        _ => false,
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
        (Type::Seq(_), Type::EmptySeq) => Ok(()),
        (Type::Seq(expected), Type::Seq(actual)) if matches!(actual.as_ref(), Type::EmptySeq) => {
            match_types(expected, actual, vars)
        }
        (Type::Var(name), actual) => {
            if let Some(bound) = vars.get(name) {
                if bound == actual {
                    Ok(())
                } else if matches!(actual, Type::EmptySeq) && matches!(bound, Type::Seq(_)) {
                    Ok(())
                } else {
                    Err(format!(
                        "type variable `{name}` was `{bound}` then `{actual}`"
                    ))
                }
            } else {
                if matches!(actual, Type::EmptySeq) {
                    return Ok(());
                }
                vars.insert(name.clone(), actual.clone());
                Ok(())
            }
        }
        (Type::Faultable(expected), Type::Faultable(actual)) => match_types(expected, actual, vars),
        (Type::Seq(expected), Type::Seq(actual)) => match_types(expected, actual, vars),
        (Type::Stream(expected), Type::Stream(actual)) => match_types(expected, actual, vars),
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
        Type::Stream(item) => Some(Type::Stream(Box::new(substitute(item, vars)?))),
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
        Type::EmptySeq => Some(Type::EmptySeq),
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
        if name.rsplit('.').next() == Some("Stream") && self.eat('[') {
            let item = self.parse_union_type()?;
            self.expect(']')?;
            return Ok(Type::Stream(Box::new(item)));
        }
        Ok(match name.as_str() {
            "Unit" => Type::Unit,
            "Int" => Type::Int,
            "Real" => Type::Real,
            "Bool" => Type::Bool,
            "Bytes" => Type::Bytes,
            "http.ServerConfig" => Type::HttpServerConfig,
            "http.Listener" => Type::HttpListener,
            "http.Request" => Type::HttpRequest,
            "http.Response" => Type::HttpResponse,
            "sqlite.Connection" => Type::SqliteConnection,
            "sqlite.Row" => Type::SqliteRow,
            "sqlite.Value" => Type::SqliteValue,
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
            Type::HttpServerConfig => write!(f, "http.ServerConfig"),
            Type::HttpListener => write!(f, "http.Listener"),
            Type::HttpRequest => write!(f, "http.Request"),
            Type::HttpResponse => write!(f, "http.Response"),
            Type::SqliteConnection => write!(f, "sqlite.Connection"),
            Type::SqliteRow => write!(f, "sqlite.Row"),
            Type::SqliteValue => write!(f, "sqlite.Value"),
            Type::Stream(item) => write!(f, "Stream[{item}]"),
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
            Type::EmptySeq => write!(f, "[]"),
        }
    }
}
