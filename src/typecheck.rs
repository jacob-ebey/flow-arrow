use crate::ast::*;
use crate::module_resolver::{
    self, ResolvedModule, ResolvedSymbolKind, ResolvedSymbolOrigin, SymbolId,
};
use crate::node_ref::{StaticNodeRef, parse_static_node_ref};
use crate::stdlib::{self, Effect, RuntimeSupport, SymbolKind};
use crate::types::{
    Signature, Type, assignable_type, common_assignable_type, contains_empty_seq,
    is_stream_constructor, match_types, parse_type, primitive_types, sequence_item_type,
    single_or_tuple, stdlib_type_symbol, substitute,
};
use std::collections::HashMap;
use std::path::Path;

#[allow(dead_code)]
pub fn check_module(module: &Module) -> Result<(), String> {
    typed_module(module).map(|_| ())
}

pub(crate) fn check_library_module(module: &Module) -> Result<(), String> {
    typed_library_module(module).map(|_| ())
}

pub fn check_module_with_base(module: &Module, base_dir: &Path) -> Result<(), String> {
    typed_module_with_base(module, base_dir).map(|_| ())
}

pub(crate) fn check_library_module_with_base(
    module: &Module,
    base_dir: &Path,
) -> Result<(), String> {
    typed_library_module_with_base(module, base_dir).map(|_| ())
}

pub(crate) fn semantic_summary_with_base(
    module: &Module,
    base_dir: &Path,
) -> Result<SemanticSummary, String> {
    Ok(typed_library_module_with_base(module, base_dir)?.semantic_summary())
}

pub(crate) fn typed_module(module: &Module) -> Result<TypedModule, String> {
    let resolved = module_resolver::resolve_stdlib_sources(module)?;
    typed_resolved_module(resolved, CheckMode::Program)
}

pub(crate) fn typed_library_module(module: &Module) -> Result<TypedModule, String> {
    let resolved = module_resolver::resolve_stdlib_sources(module)?;
    typed_resolved_module(resolved, CheckMode::Library)
}

pub(crate) fn typed_module_with_base(
    module: &Module,
    base_dir: &Path,
) -> Result<TypedModule, String> {
    let resolved = module_resolver::resolve_sources(module, Some(base_dir))?;
    typed_resolved_module(resolved, CheckMode::Program)
}

pub(crate) fn typed_library_module_with_base(
    module: &Module,
    base_dir: &Path,
) -> Result<TypedModule, String> {
    let resolved = module_resolver::resolve_sources(module, Some(base_dir))?;
    typed_resolved_module(resolved, CheckMode::Library)
}

pub(crate) fn typed_resolved_module(
    resolved: ResolvedModule,
    mode: CheckMode,
) -> Result<TypedModule, String> {
    let module = resolved.module().clone();
    Checker::new(&module)?.typed_module(resolved, mode)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckMode {
    Program,
    Library,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedModule {
    pub resolved: ResolvedModule,
    pub symbols: Vec<TypedSymbol>,
    pub callables: Vec<TypedCallable>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedSymbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: TypedSymbolKind,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum TypedSymbolKind {
    Type(Type),
    Callable(TypedCallableSymbol),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedCallableSymbol {
    pub signatures: Vec<Signature>,
    pub reduce_signatures: Vec<Signature>,
    pub effect: Effect,
    pub runtime: RuntimeSupport,
    pub runtime_name: String,
    pub origin: ResolvedSymbolOrigin,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedCallable {
    pub id: Option<SymbolId>,
    pub name: String,
    pub kind: TypedCallableKind,
    pub signature: Signature,
    pub effect: Effect,
    pub node_params: Vec<TypedNodeParam>,
    pub inputs: Vec<TypedPort>,
    pub outputs: Vec<TypedPort>,
    pub variables: Vec<TypedPort>,
    pub chains: Vec<TypedChain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedCallableKind {
    Node,
    Program,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedNodeParam {
    pub name: String,
    pub signature: Signature,
}

#[derive(Debug, Clone)]
pub(crate) struct TypedPort {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub(crate) struct TypedChain {
    pub source: TypedEndpoint,
    pub stages: Vec<TypedStage>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedEndpoint {
    pub label: String,
    pub ty: Type,
    pub kind: TypedEndpointKind,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum TypedEndpointKind {
    Variable(String),
    NodeRef {
        name: String,
        symbol: Option<SymbolId>,
    },
    Int(i64),
    Real(f64),
    Bool(bool),
    String(String),
    Unit,
    Tuple(Vec<TypedEndpoint>),
    Seq(Vec<TypedEndpoint>),
    Struct {
        name: String,
        symbol: Option<SymbolId>,
        fields: Vec<(String, TypedEndpoint)>,
    },
    Eval {
        source: Box<TypedEndpoint>,
        stages: Vec<TypedStage>,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedStage {
    pub label: String,
    pub input: Type,
    pub output: Type,
    pub symbol: Option<SymbolId>,
    pub kind: TypedStageKind,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum TypedStageKind {
    Call {
        name: String,
        symbol: Option<SymbolId>,
    },
    Bind {
        target: BindingTarget,
    },
    Map {
        name: String,
        symbol: Option<SymbolId>,
    },
    FaultMap {
        node: String,
        symbol: Option<SymbolId>,
        ok: String,
        fault: String,
    },
    Filter {
        name: String,
        symbol: Option<SymbolId>,
    },
    Field {
        name: String,
    },
    Repeat {
        node: String,
        symbol: Option<SymbolId>,
        count: TypedEndpoint,
    },
    Reduce {
        op: String,
        symbol: Option<SymbolId>,
        identity: TypedEndpoint,
    },
    Scan {
        op: String,
        symbol: Option<SymbolId>,
        identity: TypedEndpoint,
    },
    Match {
        arms: Vec<TypedMatchArm>,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct TypedMatchArm {
    pub guard: TypedMatchGuard,
    pub target: TypedMatchTarget,
    pub output: Type,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum TypedMatchGuard {
    Call {
        node: String,
        symbol: Option<SymbolId>,
        args: Vec<TypedEndpoint>,
    },
    Fallback,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum TypedMatchTarget {
    Node {
        name: String,
        symbol: Option<SymbolId>,
    },
    Value(TypedEndpoint),
}

impl TypedModule {
    pub(crate) fn module(&self) -> &Module {
        self.resolved.module()
    }

    pub(crate) fn semantic_summary(&self) -> SemanticSummary {
        SemanticSummary {
            callables: self
                .callables
                .iter()
                .map(TypedCallable::semantic_summary)
                .collect(),
        }
    }

    pub(crate) fn signature_for(&self, name: &str) -> Option<Signature> {
        let id = self.resolved.symbol_id(name)?;
        self.symbols.iter().find_map(|symbol| {
            if symbol.id != id {
                return None;
            }
            match &symbol.kind {
                TypedSymbolKind::Callable(callable) => callable.signatures.first().cloned(),
                TypedSymbolKind::Type(_) => None,
            }
        })
    }

    #[allow(dead_code)]
    pub(crate) fn type_for(&self, name: &str) -> Option<Type> {
        let id = self.resolved.symbol_id(name)?;
        self.symbols.iter().find_map(|symbol| {
            if symbol.id != id {
                return None;
            }
            match &symbol.kind {
                TypedSymbolKind::Type(ty) => Some(ty.clone()),
                TypedSymbolKind::Callable(_) => None,
            }
        })
    }
}

impl TypedCallable {
    fn semantic_summary(&self) -> CallableSummary {
        let variables = self
            .variables
            .iter()
            .map(|port| ValueSummary {
                name: port.name.clone(),
                ty: port.ty.to_string(),
            })
            .collect::<Vec<_>>();
        CallableSummary {
            name: self.name.clone(),
            variables,
            chains: self
                .chains
                .iter()
                .map(|chain| ChainSummary {
                    source: EndpointSummary {
                        label: chain.source.label.clone(),
                        ty: chain.source.ty.to_string(),
                    },
                    stages: chain
                        .stages
                        .iter()
                        .map(|stage| StageSummary {
                            label: stage.label.clone(),
                            input: stage.input.to_string(),
                            output: stage.output.to_string(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
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
    node_params: Vec<NodeParamInfo>,
    kind: CallableKind,
    effect: Effect,
    runtime: RuntimeSupport,
    is_stdlib: bool,
    runtime_name: String,
}

#[derive(Debug, Clone)]
struct NodeParamInfo {
    name: String,
    signature: Signature,
}

struct Checker<'a> {
    module: &'a Module,
    symbols: HashMap<String, CallableInfo>,
    types: HashMap<String, Type>,
}

#[allow(dead_code)]
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
        checker.collect_foreigns()?;
        checker.collect_callables()?;
        checker.infer_effects();
        Ok(checker)
    }

    fn check(&self) -> Result<(), String> {
        self.validate_main()?;
        self.check_library()
    }

    fn check_library(&self) -> Result<(), String> {
        for decl in &self.module.declarations {
            match decl {
                Decl::TypeAlias(_) | Decl::Struct(_) => {}
                Decl::Foreign(foreign) => self.check_foreign(foreign)?,
                Decl::Node(callable) => self.check_callable(callable, CallableKind::Node)?,
                Decl::Program(callable) => self.check_callable(callable, CallableKind::Program)?,
                Decl::Import(_) => {}
            }
        }
        Ok(())
    }

    fn semantic_summary(&self) -> Result<SemanticSummary, String> {
        let mut summary = SemanticSummary::default();
        for decl in &self.module.declarations {
            let (callable, kind) = match decl {
                Decl::Node(callable) => (callable, CallableKind::Node),
                Decl::Program(callable) => (callable, CallableKind::Program),
                Decl::TypeAlias(_) | Decl::Struct(_) | Decl::Foreign(_) | Decl::Import(_) => {
                    continue;
                }
            };
            summary
                .callables
                .push(self.summarize_callable(callable, kind)?);
        }
        Ok(summary)
    }

    fn typed_module(
        &self,
        resolved: ResolvedModule,
        mode: CheckMode,
    ) -> Result<TypedModule, String> {
        if mode == CheckMode::Program {
            self.validate_main()?;
        }
        let mut callables = Vec::new();
        for decl in &self.module.declarations {
            match decl {
                Decl::TypeAlias(_) | Decl::Struct(_) | Decl::Import(_) => {}
                Decl::Foreign(foreign) => self.check_foreign(foreign)?,
                Decl::Node(callable) => {
                    callables.push(self.type_callable(&resolved, callable, CallableKind::Node)?);
                }
                Decl::Program(callable) => {
                    callables.push(self.type_callable(
                        &resolved,
                        callable,
                        CallableKind::Program,
                    )?);
                }
            }
        }
        let symbols = self.typed_symbols(&resolved);
        Ok(TypedModule {
            resolved,
            symbols,
            callables,
        })
    }

    fn typed_symbols(&self, resolved: &ResolvedModule) -> Vec<TypedSymbol> {
        resolved
            .symbols()
            .iter()
            .filter_map(|symbol| match symbol.kind {
                ResolvedSymbolKind::Type => {
                    self.types.get(&symbol.internal_name).map(|ty| TypedSymbol {
                        id: symbol.id,
                        name: symbol.internal_name.clone(),
                        kind: TypedSymbolKind::Type(ty.clone()),
                    })
                }
                ResolvedSymbolKind::Callable => {
                    self.symbols
                        .get(&symbol.internal_name)
                        .map(|info| TypedSymbol {
                            id: symbol.id,
                            name: symbol.internal_name.clone(),
                            kind: TypedSymbolKind::Callable(TypedCallableSymbol {
                                signatures: info.signatures.clone(),
                                reduce_signatures: info.reduce_signatures.clone(),
                                effect: info.effect,
                                runtime: info.runtime,
                                runtime_name: info.runtime_name.clone(),
                                origin: symbol.origin,
                            }),
                        })
                }
            })
            .collect()
    }

    fn type_callable(
        &self,
        resolved: &ResolvedModule,
        callable: &Callable,
        kind: CallableKind,
    ) -> Result<TypedCallable, String> {
        if kind == CallableKind::Program && !callable.node_params.is_empty() {
            return Err(format!(
                "program `{}` cannot declare static node parameters",
                callable.name
            ));
        }
        if callable.is_extern && !callable.node_params.is_empty() {
            return Err(format!(
                "extern node `{}` cannot declare static node parameters",
                callable.name
            ));
        }
        let mut env = HashMap::new();
        let mut inputs = Vec::new();
        let mut variables = Vec::new();
        for port in &callable.inputs {
            let ty = self.parse_declared_type(&port.ty)?;
            if env.insert(port.name.clone(), ty.clone()).is_some() {
                return Err(format!(
                    "`{}` declares input `{}` more than once",
                    callable.name, port.name
                ));
            }
            let port = TypedPort {
                name: port.name.clone(),
                ty,
            };
            variables.push(port.clone());
            inputs.push(port);
        }
        let mut chains = Vec::new();
        for chain in &callable.chains {
            chains.push(self.type_chain(
                resolved,
                callable,
                kind,
                chain,
                &mut env,
                &mut variables,
            )?);
        }
        let mut outputs = Vec::new();
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
            outputs.push(TypedPort {
                name: output.name.clone(),
                ty: expected,
            });
        }
        let info = self
            .symbols
            .get(&callable.name)
            .ok_or_else(|| format!("missing callable `{}`", callable.name))?;
        Ok(TypedCallable {
            id: resolved.symbol_id(&callable.name),
            name: callable.name.clone(),
            kind: match kind {
                CallableKind::Node => TypedCallableKind::Node,
                CallableKind::Program => TypedCallableKind::Program,
            },
            signature: info
                .signatures
                .first()
                .cloned()
                .ok_or_else(|| format!("`{}` has no signature", callable.name))?,
            effect: info.effect,
            node_params: info
                .node_params
                .iter()
                .map(|param| TypedNodeParam {
                    name: param.name.clone(),
                    signature: param.signature.clone(),
                })
                .collect(),
            inputs,
            outputs,
            variables,
            chains,
        })
    }

    fn type_chain(
        &self,
        resolved: &ResolvedModule,
        callable: &Callable,
        kind: CallableKind,
        chain: &Chain,
        env: &mut HashMap<String, Type>,
        variables: &mut Vec<TypedPort>,
    ) -> Result<TypedChain, String> {
        let source = self.type_endpoint(resolved, &chain.source, env)?;
        let mut value_type = source.ty.clone();
        let mut stages = Vec::new();
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            let typed_stage = self.type_stage(
                resolved,
                callable,
                kind,
                stage,
                &value_type,
                is_last,
                env,
                variables,
            )?;
            value_type = typed_stage.output.clone();
            stages.push(typed_stage);
        }
        Ok(TypedChain { source, stages })
    }

    #[allow(clippy::too_many_arguments)]
    fn type_stage(
        &self,
        resolved: &ResolvedModule,
        callable: &Callable,
        kind: CallableKind,
        stage: &Stage,
        input: &Type,
        is_last: bool,
        env: &mut HashMap<String, Type>,
        variables: &mut Vec<TypedPort>,
    ) -> Result<TypedStage, String> {
        let input_type = input.clone();
        let symbol = stage_symbol_id(resolved, stage);
        let (label, output, kind) = match stage {
            Stage::Bind(target) if is_last => {
                let bindings = binding_target_types(target, input)?;
                for (name, ty) in bindings {
                    if contains_empty_seq(&ty) {
                        return Err("empty sequence literals need a type context".to_string());
                    }
                    if env.insert(name.clone(), ty.clone()).is_some() {
                        return Err(format!("value `{name}` is bound more than once"));
                    }
                    variables.push(TypedPort { name, ty });
                }
                (
                    format_binding_target_for_error(target),
                    input.clone(),
                    TypedStageKind::Bind {
                        target: target.clone(),
                    },
                )
            }
            Stage::Endpoint(Endpoint::Name(name)) => (
                name.clone(),
                self.apply_node(callable, kind, name, input, false, false)?,
                TypedStageKind::Call {
                    name: name.clone(),
                    symbol,
                },
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
                self.apply_map(callable, name, input)?,
                TypedStageKind::Map {
                    name: name.clone(),
                    symbol,
                },
            ),
            Stage::FaultMap { node, ok, fault } => {
                if !is_last {
                    return Err("`fault map` must be the final stage in a chain".to_string());
                }
                let (ok_type, fault_type) = self.apply_fault_map(callable, node, input)?;
                if env.insert(ok.clone(), ok_type.clone()).is_some() {
                    return Err(format!("value `{ok}` is bound more than once"));
                }
                if env.insert(fault.clone(), fault_type.clone()).is_some() {
                    return Err(format!("value `{fault}` is bound more than once"));
                }
                variables.push(TypedPort {
                    name: ok.clone(),
                    ty: ok_type.clone(),
                });
                variables.push(TypedPort {
                    name: fault.clone(),
                    ty: fault_type.clone(),
                });
                (
                    format!("fault map {node}"),
                    Type::Tuple(vec![ok_type, fault_type]),
                    TypedStageKind::FaultMap {
                        node: node.clone(),
                        symbol,
                        ok: ok.clone(),
                        fault: fault.clone(),
                    },
                )
            }
            Stage::Filter(name) => (
                format!("filter {name}"),
                self.apply_filter(callable, name, input)?,
                TypedStageKind::Filter {
                    name: name.clone(),
                    symbol,
                },
            ),
            Stage::Field(name) => (
                format!("field {name}"),
                self.apply_field(name, input)?,
                TypedStageKind::Field { name: name.clone() },
            ),
            Stage::Repeat { count, node } => {
                let count = self.type_endpoint(resolved, count, env)?;
                self.expect_type("repeat count", &count.ty, &Type::Int)?;
                (
                    format!("repeat {node}"),
                    self.apply_repeat(callable, node, input)?,
                    TypedStageKind::Repeat {
                        node: node.clone(),
                        symbol,
                        count,
                    },
                )
            }
            Stage::Reduce { op, identity } => {
                let identity = self.type_endpoint(resolved, identity, env)?;
                (
                    format!("reduce {op}"),
                    self.apply_reduce(callable, op, input, &identity.ty)?,
                    TypedStageKind::Reduce {
                        op: op.clone(),
                        symbol,
                        identity,
                    },
                )
            }
            Stage::Scan { op, identity } => {
                let identity = self.type_endpoint(resolved, identity, env)?;
                (
                    format!("scan {op}"),
                    self.apply_scan(callable, op, input, &identity.ty)?,
                    TypedStageKind::Scan {
                        op: op.clone(),
                        symbol,
                        identity,
                    },
                )
            }
            Stage::Match { arms } => {
                let (arms, output) =
                    self.type_match_arms(resolved, callable, kind, arms, input, env)?;
                ("match".to_string(), output, TypedStageKind::Match { arms })
            }
        };
        Ok(TypedStage {
            label,
            input: input_type,
            output,
            symbol,
            kind,
        })
    }

    fn type_endpoint(
        &self,
        resolved: &ResolvedModule,
        endpoint: &Endpoint,
        env: &HashMap<String, Type>,
    ) -> Result<TypedEndpoint, String> {
        let label = format_endpoint_for_error(endpoint);
        let (ty, kind) = match endpoint {
            Endpoint::Variable(name) => (
                env.get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown value `{name}`"))?,
                TypedEndpointKind::Variable(name.clone()),
            ),
            Endpoint::Name(name) => return Err(format!("expected value, found node `{name}`")),
            Endpoint::Int(value) => (Type::Int, TypedEndpointKind::Int(*value)),
            Endpoint::Real(value) => (Type::Real, TypedEndpointKind::Real(*value)),
            Endpoint::Bool(value) => (Type::Bool, TypedEndpointKind::Bool(*value)),
            Endpoint::String(value) => (Type::Bytes, TypedEndpointKind::String(value.clone())),
            Endpoint::Unit => (Type::Unit, TypedEndpointKind::Unit),
            Endpoint::Tuple(items) => {
                let items = items
                    .iter()
                    .map(|item| self.type_endpoint(resolved, item, env))
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    Type::Tuple(items.iter().map(|item| item.ty.clone()).collect()),
                    TypedEndpointKind::Tuple(items),
                )
            }
            Endpoint::Seq(items) => {
                let items = items
                    .iter()
                    .map(|item| self.type_endpoint(resolved, item, env))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut item_type = None;
                for item in &items {
                    if let Some(expected) = &item_type {
                        item_type =
                            Some(sequence_item_type(expected, &item.ty).map_err(|error| {
                                format!("sequence literal item type mismatch: {error}")
                            })?);
                    } else {
                        item_type = Some(item.ty.clone());
                    }
                }
                let ty = match item_type {
                    Some(item_type) => Type::Seq(Box::new(item_type)),
                    None => Type::EmptySeq,
                };
                (ty, TypedEndpointKind::Seq(items))
            }
            Endpoint::Struct { name, fields } => {
                let ty = self.struct_literal_type(name, fields, env)?;
                let fields = fields
                    .iter()
                    .map(|(field, endpoint)| {
                        Ok((field.clone(), self.type_endpoint(resolved, endpoint, env)?))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                (
                    ty,
                    TypedEndpointKind::Struct {
                        name: name.clone(),
                        symbol: resolved.symbol_id(name),
                        fields,
                    },
                )
            }
            Endpoint::Eval { source, stages } => {
                let source = self.type_endpoint(resolved, source, env)?;
                let stages = self.type_inline_stages(resolved, &source.ty, stages, env)?;
                let ty = stages
                    .last()
                    .map(|stage| stage.output.clone())
                    .unwrap_or_else(|| source.ty.clone());
                (
                    ty,
                    TypedEndpointKind::Eval {
                        source: Box::new(source),
                        stages,
                    },
                )
            }
        };
        Ok(TypedEndpoint { label, ty, kind })
    }

    fn type_inline_stages(
        &self,
        resolved: &ResolvedModule,
        source_ty: &Type,
        stages: &[Stage],
        env: &HashMap<String, Type>,
    ) -> Result<Vec<TypedStage>, String> {
        let inline_callable = Callable {
            name: "<inline>".to_string(),
            is_extern: false,
            node_params: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            chains: Vec::new(),
        };
        let mut env = env.clone();
        let mut variables = Vec::new();
        let mut value_type = source_ty.clone();
        let mut out = Vec::new();
        for stage in stages {
            match stage {
                Stage::Endpoint(Endpoint::Variable(_)) | Stage::Bind(_) => {
                    return Err("inline evaluations cannot bind values".to_string());
                }
                Stage::Endpoint(Endpoint::Name(_)) => {}
                Stage::Endpoint(_) => {
                    return Err(
                        "non-name endpoints may only appear as inline evaluation sources"
                            .to_string(),
                    );
                }
                Stage::FaultMap { .. } => {
                    return Err("inline evaluations cannot use `fault map`".to_string());
                }
                Stage::Map(_)
                | Stage::Filter(_)
                | Stage::Field(_)
                | Stage::Repeat { .. }
                | Stage::Reduce { .. }
                | Stage::Scan { .. }
                | Stage::Match { .. } => {}
            }
            let typed = self.type_stage(
                resolved,
                &inline_callable,
                CallableKind::Node,
                stage,
                &value_type,
                false,
                &mut env,
                &mut variables,
            )?;
            value_type = typed.output.clone();
            out.push(typed);
        }
        Ok(out)
    }

    fn type_match_arms(
        &self,
        resolved: &ResolvedModule,
        callable: &Callable,
        kind: CallableKind,
        arms: &[MatchArm],
        input: &Type,
        env: &HashMap<String, Type>,
    ) -> Result<(Vec<TypedMatchArm>, Type), String> {
        if arms.is_empty() {
            return Err("`match` must contain at least one arm".to_string());
        }
        if !matches!(
            arms.last().map(|arm| &arm.guard),
            Some(MatchGuard::Fallback)
        ) {
            return Err("`match` must end with a `_` fallback arm".to_string());
        }

        let mut typed_arms = Vec::with_capacity(arms.len());
        let mut result = None;
        for (index, arm) in arms.iter().enumerate() {
            let guard = match &arm.guard {
                MatchGuard::Fallback if index + 1 != arms.len() => {
                    return Err("`match` fallback arm must be last".to_string());
                }
                MatchGuard::Fallback => TypedMatchGuard::Fallback,
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
                    let args = args
                        .iter()
                        .map(|arg| self.type_endpoint(resolved, arg, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    let guard_input = single_or_tuple(
                        std::iter::once(input.clone())
                            .chain(args.iter().map(|arg| arg.ty.clone()))
                            .collect(),
                    );
                    let guard_output =
                        self.apply_node(callable, kind, node, &guard_input, false, false)?;
                    self.expect_type(
                        &format!("match guard `{node}` result"),
                        &guard_output,
                        &Type::Bool,
                    )?;
                    TypedMatchGuard::Call {
                        node: node.clone(),
                        symbol: callable_symbol_id(resolved, node),
                        args,
                    }
                }
            };

            let (target, arm_output) = match &arm.target {
                MatchTarget::Node(node) => (
                    TypedMatchTarget::Node {
                        name: node.clone(),
                        symbol: callable_symbol_id(resolved, node),
                    },
                    self.apply_node(callable, kind, node, input, false, false)?,
                ),
                MatchTarget::Value(endpoint) => {
                    let endpoint = self.type_endpoint(resolved, endpoint, env)?;
                    let ty = endpoint.ty.clone();
                    (TypedMatchTarget::Value(endpoint), ty)
                }
            };
            if let Some(expected) = &result {
                result = Some(common_assignable_type(
                    expected,
                    &arm_output,
                    &format!("match arm `{}` result", format_match_target(&arm.target)),
                )?);
            } else {
                result = Some(arm_output.clone());
            }
            typed_arms.push(TypedMatchArm {
                guard,
                target,
                output: arm_output,
            });
        }
        Ok((
            typed_arms,
            result.ok_or_else(|| "`match` must contain at least one arm".to_string())?,
        ))
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
                        node_params: Vec::new(),
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
        let mut raw_aliases = HashMap::new();
        let mut raw_structs = HashMap::new();
        for decl in &self.module.declarations {
            match decl {
                Decl::TypeAlias(alias) => {
                    if self.types.contains_key(&alias.name)
                        || raw_aliases.contains_key(&alias.name)
                        || raw_structs.contains_key(&alias.name)
                    {
                        return Err(format!("duplicate type declaration `{}`", alias.name));
                    }
                    raw_aliases.insert(alias.name.clone(), alias.ty.clone());
                }
                Decl::Struct(struct_decl) => {
                    if self.types.contains_key(&struct_decl.name)
                        || raw_aliases.contains_key(&struct_decl.name)
                        || raw_structs.contains_key(&struct_decl.name)
                    {
                        return Err(format!("duplicate type declaration `{}`", struct_decl.name));
                    }
                    raw_structs.insert(struct_decl.name.clone(), struct_decl.fields.clone());
                }
                _ => {}
            }
        }

        let mut resolved = HashMap::new();
        for name in raw_structs.keys() {
            let mut resolving = Vec::new();
            let ty = self.resolve_struct_type(
                name,
                &raw_aliases,
                &raw_structs,
                &mut resolved,
                &mut resolving,
            )?;
            self.validate_declared_type(&ty)?;
            self.types.insert(name.clone(), ty);
        }
        for name in raw_aliases.keys() {
            let mut resolving = Vec::new();
            let ty = self.resolve_type_alias(
                name,
                &raw_aliases,
                &raw_structs,
                &mut resolved,
                &mut resolving,
            )?;
            self.validate_declared_type(&ty)?;
            self.types.insert(name.clone(), ty);
        }
        Ok(())
    }

    fn resolve_type_alias(
        &self,
        name: &str,
        raw_aliases: &HashMap<String, String>,
        raw_structs: &HashMap<String, Vec<Port>>,
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
        let text = raw_aliases
            .get(name)
            .ok_or_else(|| format!("unknown type alias `{name}`"))?;
        resolving.push(name.to_string());
        let parsed = parse_type(text)?;
        let ty =
            self.resolve_type_alias_type(parsed, raw_aliases, raw_structs, resolved, resolving)?;
        resolving.pop();
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    }

    fn resolve_struct_type(
        &self,
        name: &str,
        raw_aliases: &HashMap<String, String>,
        raw_structs: &HashMap<String, Vec<Port>>,
        resolved: &mut HashMap<String, Type>,
        resolving: &mut Vec<String>,
    ) -> Result<Type, String> {
        if let Some(ty) = resolved.get(name) {
            return Ok(ty.clone());
        }
        if resolving.iter().any(|item| item == name) {
            resolving.push(name.to_string());
            return Err(format!(
                "cyclic type declaration `{}`",
                resolving.join(" -> ")
            ));
        }
        let fields = raw_structs
            .get(name)
            .ok_or_else(|| format!("unknown struct `{name}`"))?;
        resolving.push(name.to_string());
        let mut resolved_fields = Vec::with_capacity(fields.len());
        for field in fields {
            if resolved_fields
                .iter()
                .any(|(existing, _): &(String, Type)| existing == &field.name)
            {
                return Err(format!(
                    "struct `{name}` declares field `{}` more than once",
                    field.name
                ));
            }
            let ty = self.resolve_type_alias_type(
                parse_type(&field.ty)?,
                raw_aliases,
                raw_structs,
                resolved,
                resolving,
            )?;
            resolved_fields.push((field.name.clone(), ty));
        }
        resolving.pop();
        let ty = Type::Struct {
            name: name.to_string(),
            fields: resolved_fields,
        };
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    }

    fn resolve_type_alias_type(
        &self,
        ty: Type,
        raw_aliases: &HashMap<String, String>,
        raw_structs: &HashMap<String, Vec<Port>>,
        resolved: &mut HashMap<String, Type>,
        resolving: &mut Vec<String>,
    ) -> Result<Type, String> {
        match ty {
            Type::Var(name) => {
                if let Some(known) = self.types.get(&name) {
                    Ok(known.clone())
                } else if raw_aliases.contains_key(&name) {
                    self.resolve_type_alias(&name, raw_aliases, raw_structs, resolved, resolving)
                } else if raw_structs.contains_key(&name) {
                    self.resolve_struct_type(&name, raw_aliases, raw_structs, resolved, resolving)
                } else {
                    Err(format!("unknown type `{name}`"))
                }
            }
            Type::Faultable(item) => Ok(Type::Faultable(Box::new(self.resolve_type_alias_type(
                *item,
                raw_aliases,
                raw_structs,
                resolved,
                resolving,
            )?))),
            Type::Seq(item) => Ok(Type::Seq(Box::new(self.resolve_type_alias_type(
                *item,
                raw_aliases,
                raw_structs,
                resolved,
                resolving,
            )?))),
            Type::Stream(item) => Ok(Type::Stream(Box::new(self.resolve_type_alias_type(
                *item,
                raw_aliases,
                raw_structs,
                resolved,
                resolving,
            )?))),
            Type::OneOf(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_type_alias_type(
                        item,
                        raw_aliases,
                        raw_structs,
                        resolved,
                        resolving,
                    )?);
                }
                Ok(Type::OneOf(out))
            }
            Type::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.resolve_type_alias_type(
                        item,
                        raw_aliases,
                        raw_structs,
                        resolved,
                        resolving,
                    )?);
                }
                Ok(Type::Tuple(out))
            }
            Type::Struct { name, fields } => {
                let mut out = Vec::with_capacity(fields.len());
                for (field, ty) in fields {
                    out.push((
                        field,
                        self.resolve_type_alias_type(
                            ty,
                            raw_aliases,
                            raw_structs,
                            resolved,
                            resolving,
                        )?,
                    ));
                }
                Ok(Type::Struct { name, fields: out })
            }
            other => Ok(other),
        }
    }

    fn collect_callables(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            let (callable, kind) = match decl {
                Decl::Node(callable) => (callable, CallableKind::Node),
                Decl::Program(callable) => (callable, CallableKind::Program),
                Decl::TypeAlias(_) | Decl::Struct(_) | Decl::Foreign(_) | Decl::Import(_) => {
                    continue;
                }
            };
            let info = CallableInfo {
                signatures: vec![self.callable_signature(callable)?],
                reduce_signatures: Vec::new(),
                node_params: self.node_param_infos(callable)?,
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

    fn collect_foreigns(&mut self) -> Result<(), String> {
        for decl in &self.module.declarations {
            let Decl::Foreign(foreign) = decl else {
                continue;
            };
            for node in &foreign.nodes {
                self.insert_symbol(
                    &node.name,
                    CallableInfo {
                        signatures: vec![Signature {
                            input: self.port_types(&node.inputs)?,
                            output: self.port_types(&node.outputs)?,
                        }],
                        reduce_signatures: Vec::new(),
                        node_params: Vec::new(),
                        kind: CallableKind::Node,
                        effect: match node.effect {
                            ForeignEffect::Pure => Effect::Pure,
                            ForeignEffect::Io => Effect::Io,
                        },
                        runtime: RuntimeSupport::DirectBuiltin,
                        is_stdlib: false,
                        runtime_name: node.name.clone(),
                    },
                )?;
            }
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
                Stage::Bind(_) | Stage::Field(_) => false,
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

    fn node_param_infos(&self, callable: &Callable) -> Result<Vec<NodeParamInfo>, String> {
        let mut params = Vec::new();
        for param in &callable.node_params {
            if params
                .iter()
                .any(|existing: &NodeParamInfo| existing.name == param.name)
            {
                return Err(format!(
                    "`{}` declares static node parameter `{}` more than once",
                    callable.name, param.name
                ));
            }
            params.push(NodeParamInfo {
                name: param.name.clone(),
                signature: Signature {
                    input: self.parse_declared_type(&param.input)?,
                    output: self.parse_declared_type(&param.output)?,
                },
            });
        }
        Ok(params)
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
            Type::Struct { name, fields } => {
                let mut resolved = Vec::with_capacity(fields.len());
                for (field, ty) in fields {
                    resolved.push((field, self.resolve_declared_type(ty)?));
                }
                Ok(Type::Struct {
                    name,
                    fields: resolved,
                })
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
            Type::Struct { name, fields } => {
                let _ = name;
                for (_, ty) in fields {
                    self.validate_declared_type(ty)?;
                }
                Ok(())
            }
            Type::Var(name) => Err(format!("unknown type `{name}`")),
            primitive if self.types.values().any(|known| known == primitive) => Ok(()),
            other => Err(format!("unknown type `{other}`")),
        }
    }

    fn check_callable(&self, callable: &Callable, kind: CallableKind) -> Result<(), String> {
        if kind == CallableKind::Program && !callable.node_params.is_empty() {
            return Err(format!(
                "program `{}` cannot declare static node parameters",
                callable.name
            ));
        }
        if callable.is_extern && !callable.node_params.is_empty() {
            return Err(format!(
                "extern node `{}` cannot declare static node parameters",
                callable.name
            ));
        }
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

    fn check_foreign(&self, foreign: &ForeignBlock) -> Result<(), String> {
        for node in &foreign.nodes {
            let mut inputs = HashMap::new();
            for port in &node.inputs {
                let ty = self.parse_declared_type(&port.ty)?;
                self.validate_declared_type(&ty)?;
                if inputs.insert(port.name.clone(), ()).is_some() {
                    return Err(format!(
                        "foreign node `{}` declares input `{}` more than once",
                        node.name, port.name
                    ));
                }
            }
            let mut outputs = HashMap::new();
            for port in &node.outputs {
                let ty = self.parse_declared_type(&port.ty)?;
                self.validate_declared_type(&ty)?;
                if outputs.insert(port.name.clone(), ()).is_some() {
                    return Err(format!(
                        "foreign node `{}` declares output `{}` more than once",
                        node.name, port.name
                    ));
                }
            }
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
                Stage::Field(name) => (
                    format!("field {name}"),
                    self.apply_field(name, &value_type)?,
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
                Stage::Field(name) => {
                    value_type = self.apply_field(name, &value_type)?;
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
            Endpoint::Struct { name, fields } => self.struct_literal_type(name, fields, env),
            Endpoint::Eval { source, stages } => self.inline_eval_type(source, stages, env),
        }
    }

    fn struct_literal_type(
        &self,
        name: &str,
        fields: &[(String, Endpoint)],
        env: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        let Some(Type::Struct {
            name: struct_name,
            fields: expected_fields,
        }) = self.types.get(name)
        else {
            return Err(format!("unknown struct `{name}`"));
        };
        if fields.len() != expected_fields.len() {
            return Err(format!(
                "struct `{name}` literal expected {} fields, found {}",
                expected_fields.len(),
                fields.len()
            ));
        }
        let mut seen = HashMap::new();
        for (field, endpoint) in fields {
            let expected = expected_fields
                .iter()
                .find(|(expected, _)| expected == field)
                .map(|(_, ty)| ty)
                .ok_or_else(|| format!("struct `{name}` has no field `{field}`"))?;
            if seen.insert(field, ()).is_some() {
                return Err(format!("struct `{name}` literal repeats field `{field}`"));
            }
            let actual = self.endpoint_type(endpoint, env)?;
            self.expect_assignable_type(
                &format!("struct `{name}` field `{field}`"),
                &actual,
                expected,
            )?;
        }
        Ok(Type::Struct {
            name: struct_name.clone(),
            fields: expected_fields.clone(),
        })
    }

    fn apply_field(&self, field: &str, input: &Type) -> Result<Type, String> {
        let Type::Struct { name, fields } = input else {
            return Err(format!(
                "field `{field}` expected struct input, found `{input}`"
            ));
        };
        fields
            .iter()
            .find(|(candidate, _)| candidate == field)
            .map(|(_, ty)| ty.clone())
            .ok_or_else(|| format!("struct `{name}` has no field `{field}`"))
    }

    fn inline_eval_type(
        &self,
        source: &Endpoint,
        stages: &[Stage],
        env: &HashMap<String, Type>,
    ) -> Result<Type, String> {
        let inline_callable = Callable {
            name: "<inline>".to_string(),
            is_extern: false,
            node_params: Vec::new(),
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
                Stage::Field(name) => {
                    value_type = self.apply_field(name, &value_type)?;
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
        callable: &Callable,
        _context: CallableKind,
        name: &str,
        input: &Type,
        as_function: bool,
        allow_effectful_function: bool,
    ) -> Result<Type, String> {
        let node = self.resolve_node(callable, name)?;
        if node.kind == CallableKind::Program {
            return Err(format!("program `{name}` cannot be called from a graph"));
        }
        if as_function && !self.supports_higher_order_call(&node) {
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

    fn resolve_node(&self, callable: &Callable, name: &str) -> Result<CallableInfo, String> {
        let node_ref = parse_static_node_ref(name);
        if node_ref.args.is_empty() {
            if let Some(param) = callable
                .node_params
                .iter()
                .find(|param| param.name == node_ref.base)
            {
                let input = self.parse_declared_type(&param.input)?;
                let output = self.parse_declared_type(&param.output)?;
                return Ok(CallableInfo {
                    signatures: vec![Signature { input, output }],
                    reduce_signatures: Vec::new(),
                    node_params: Vec::new(),
                    kind: CallableKind::Node,
                    effect: Effect::Pure,
                    runtime: RuntimeSupport::DirectBuiltin,
                    is_stdlib: false,
                    runtime_name: param.name.clone(),
                });
            }
        }
        let info = self
            .symbols
            .get(&node_ref.base)
            .ok_or_else(|| format!("unknown node `{name}`"))?;
        if node_ref.args.is_empty() {
            if !info.node_params.is_empty() {
                return Err(format!(
                    "node `{}` requires {} static node arguments",
                    node_ref.base,
                    info.node_params.len()
                ));
            }
            return Ok(info.clone());
        }
        self.validate_static_node_args(callable, &node_ref, info)?;
        Ok(CallableInfo {
            node_params: Vec::new(),
            ..info.clone()
        })
    }

    fn validate_static_node_args(
        &self,
        callable: &Callable,
        node_ref: &StaticNodeRef,
        template: &CallableInfo,
    ) -> Result<(), String> {
        if template.node_params.is_empty() {
            return Err(format!(
                "node `{}` does not take static node arguments",
                node_ref.base
            ));
        }
        if template.node_params.len() != node_ref.args.len() {
            return Err(format!(
                "node `{}` expected {} static node arguments, found {}",
                node_ref.base,
                template.node_params.len(),
                node_ref.args.len()
            ));
        }
        for (param, actual) in template.node_params.iter().zip(&node_ref.args) {
            let actual_ref = parse_static_node_ref(actual);
            if !actual_ref.args.is_empty() {
                return Err(format!(
                    "static node argument `{actual}` cannot itself take static arguments"
                ));
            }
            let actual_info = self.resolve_static_arg(callable, actual, &actual_ref)?;
            if actual_info.kind == CallableKind::Program {
                return Err(format!(
                    "program `{actual}` cannot be used as a static node argument"
                ));
            }
            if !actual_info.node_params.is_empty() {
                return Err(format!(
                    "generic node `{actual}` cannot be used as a static node argument without instantiation"
                ));
            }
            if actual_info.effect != Effect::Pure {
                return Err(format!(
                    "`{actual}` cannot be used as a static node argument because it is effectful"
                ));
            }
            if !self.callable_matches_signature(actual, &actual_info, &param.signature) {
                return Err(format!(
                    "`{actual}` does not match static node parameter `{}`: expected `node({}) -> {}`",
                    param.name, param.signature.input, param.signature.output
                ));
            }
        }
        Ok(())
    }

    fn resolve_static_arg(
        &self,
        callable: &Callable,
        actual: &str,
        actual_ref: &StaticNodeRef,
    ) -> Result<CallableInfo, String> {
        if let Some(param) = callable
            .node_params
            .iter()
            .find(|param| param.name == actual_ref.base)
        {
            let input = self.parse_declared_type(&param.input)?;
            let output = self.parse_declared_type(&param.output)?;
            return Ok(CallableInfo {
                signatures: vec![Signature { input, output }],
                reduce_signatures: Vec::new(),
                node_params: Vec::new(),
                kind: CallableKind::Node,
                effect: Effect::Pure,
                runtime: RuntimeSupport::DirectBuiltin,
                is_stdlib: false,
                runtime_name: param.name.clone(),
            });
        }
        self.symbols
            .get(&actual_ref.base)
            .cloned()
            .ok_or_else(|| format!("unknown static node argument `{actual}`"))
    }

    fn callable_matches_signature(
        &self,
        _name: &str,
        info: &CallableInfo,
        expected: &Signature,
    ) -> bool {
        info.signatures.iter().any(|signature| {
            let mut vars = HashMap::new();
            match_types(&expected.input, &signature.input, &mut vars).is_ok()
                && match_types(&expected.output, &signature.output, &mut vars).is_ok()
        })
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

fn binding_target_types(
    target: &BindingTarget,
    value_type: &Type,
) -> Result<Vec<(String, Type)>, String> {
    match target {
        BindingTarget::Discard => Ok(Vec::new()),
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

fn stage_symbol_id(resolved: &ResolvedModule, stage: &Stage) -> Option<SymbolId> {
    let name = match stage {
        Stage::Endpoint(Endpoint::Name(name))
        | Stage::Map(name)
        | Stage::Filter(name)
        | Stage::Reduce { op: name, .. }
        | Stage::Scan { op: name, .. } => name,
        Stage::FaultMap { node, .. } | Stage::Repeat { node, .. } => node,
        Stage::Bind(_)
        | Stage::Field(_)
        | Stage::Match { .. }
        | Stage::Endpoint(Endpoint::Variable(_))
        | Stage::Endpoint(Endpoint::Int(_))
        | Stage::Endpoint(Endpoint::Real(_))
        | Stage::Endpoint(Endpoint::Bool(_))
        | Stage::Endpoint(Endpoint::String(_))
        | Stage::Endpoint(Endpoint::Unit)
        | Stage::Endpoint(Endpoint::Tuple(_))
        | Stage::Endpoint(Endpoint::Seq(_))
        | Stage::Endpoint(Endpoint::Struct { .. })
        | Stage::Endpoint(Endpoint::Eval { .. }) => return None,
    };
    callable_symbol_id(resolved, name)
}

fn callable_symbol_id(resolved: &ResolvedModule, name: &str) -> Option<SymbolId> {
    let node_ref = parse_static_node_ref(name);
    resolved.symbol_id(&node_ref.base)
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
        BindingTarget::Discard => "$".to_string(),
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
        Endpoint::Struct { name, fields } => format!(
            "{name} {{ {} }}",
            fields
                .iter()
                .map(|(field, value)| format!("{field}: {}", format_endpoint_for_error(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
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
        Stage::Field(name) => format!("field {name}"),
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
    if symbol.module == "std.math" && matches!(symbol.name, "add" | "min" | "max") {
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
