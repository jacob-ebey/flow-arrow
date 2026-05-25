use crate::ast::*;
use crate::node_ref::{format_static_node_ref, parse_static_node_ref};
use crate::{parser, stdlib};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ModuleId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SymbolId(pub(crate) usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedModuleKind {
    Root,
    Synthetic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSourceModule {
    pub id: ModuleId,
    pub name: String,
    pub kind: ResolvedModuleKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedSymbolKind {
    Type,
    Callable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedSymbolOrigin {
    Source,
    StdlibBuiltin,
    Foreign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSymbol {
    pub id: SymbolId,
    pub module_id: ModuleId,
    pub public_name: String,
    pub internal_name: String,
    pub kind: ResolvedSymbolKind,
    pub origin: ResolvedSymbolOrigin,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ResolvedModule {
    module: Module,
    modules: Vec<ResolvedSourceModule>,
    symbols: Vec<ResolvedSymbol>,
    names: HashMap<String, SymbolId>,
}

impl ResolvedModule {
    pub(crate) fn new_root(module: Module) -> Self {
        Self::from_module(module, "<root>", ResolvedModuleKind::Root)
    }

    pub(crate) fn synthetic(module: Module) -> Self {
        Self::from_module(module, "<synthetic>", ResolvedModuleKind::Synthetic)
    }

    fn from_module(module: Module, module_name: &str, kind: ResolvedModuleKind) -> Self {
        let module_id = ModuleId(0);
        let mut symbols = Vec::new();
        let mut names = HashMap::new();
        for decl in &module.declarations {
            record_decl_symbols(module_id, decl, &mut symbols, &mut names);
        }
        Self {
            module,
            modules: vec![ResolvedSourceModule {
                id: module_id,
                name: module_name.to_string(),
                kind,
            }],
            symbols,
            names,
        }
    }

    pub(crate) fn module(&self) -> &Module {
        &self.module
    }

    pub(crate) fn into_module(self) -> Module {
        self.module
    }

    #[allow(dead_code)]
    pub(crate) fn modules(&self) -> &[ResolvedSourceModule] {
        &self.modules
    }

    pub(crate) fn symbols(&self) -> &[ResolvedSymbol] {
        &self.symbols
    }

    #[allow(dead_code)]
    pub(crate) fn symbol_id(&self, name: &str) -> Option<SymbolId> {
        self.names.get(name).copied()
    }

    #[allow(dead_code)]
    pub(crate) fn symbol(&self, id: SymbolId) -> Option<&ResolvedSymbol> {
        self.symbols.get(id.0)
    }
}

#[allow(dead_code)]
pub fn expand_stdlib_sources(module: &Module) -> Result<Module, String> {
    Ok(resolve_stdlib_sources(module)?.into_module())
}

pub fn expand_sources(module: &Module, base_dir: Option<&Path>) -> Result<Module, String> {
    Ok(resolve_sources(module, base_dir)?.into_module())
}

pub(crate) fn resolve_stdlib_sources(module: &Module) -> Result<ResolvedModule, String> {
    resolve_sources(module, None)
}

pub(crate) fn resolve_sources(
    module: &Module,
    base_dir: Option<&Path>,
) -> Result<ResolvedModule, String> {
    let mut resolver = Resolver::default();
    let root = resolver.rewrite_root(module, base_dir)?;
    let mut declarations = resolver.declarations;
    declarations.extend(root);
    Ok(ResolvedModule::new_root(Module { declarations }))
}

#[derive(Default)]
struct Resolver {
    modules: HashMap<String, HashMap<String, String>>,
    local_modules: HashMap<PathBuf, HashMap<String, String>>,
    resolving: HashSet<String>,
    resolving_local: HashSet<PathBuf>,
    declarations: Vec<Decl>,
}

impl Resolver {
    fn rewrite_root(
        &mut self,
        module: &Module,
        base_dir: Option<&Path>,
    ) -> Result<Vec<Decl>, String> {
        let callable_names = callable_names(module);
        let mut references = HashMap::new();
        let mut declarations = Vec::new();

        for decl in &module.declarations {
            let Decl::Import(import) = decl else {
                continue;
            };
            match &import.source {
                ImportSource::Module(source) if stdlib::flow_source(source).is_some() => {
                    self.import_flow_module(source, &import.clause, &mut references)?;
                }
                ImportSource::Local(path) => {
                    let base_dir = base_dir
                        .ok_or_else(|| "local imports require a source file path".to_string())?;
                    self.import_local_module(base_dir, path, &import.clause, &mut references)?;
                }
                ImportSource::Module(_) => {}
            }
        }

        for name in references.keys().filter(|name| !name.contains('.')) {
            if callable_names.contains(name) {
                return Err(format!("duplicate declaration or import `{name}`"));
            }
        }

        for decl in &module.declarations {
            match decl {
                Decl::Import(import) if is_source_import(import) => {}
                Decl::TypeAlias(alias) => declarations.push(Decl::TypeAlias(rewrite_type_alias(
                    alias.clone(),
                    &references,
                    alias.name.clone(),
                ))),
                Decl::Struct(struct_decl) => declarations.push(Decl::Struct(rewrite_struct_decl(
                    struct_decl.clone(),
                    &references,
                    struct_decl.name.clone(),
                ))),
                Decl::Node(callable) => declarations.push(Decl::Node(rewrite_callable(
                    callable.clone(),
                    &references,
                    callable.name.clone(),
                ))),
                Decl::Program(callable) => declarations.push(Decl::Program(rewrite_callable(
                    callable.clone(),
                    &references,
                    callable.name.clone(),
                ))),
                Decl::Foreign(foreign) => declarations.push(Decl::Foreign(rewrite_foreign_block(
                    foreign.clone(),
                    &references,
                    None,
                ))),
                Decl::Import(_) => declarations.push(decl.clone()),
            }
        }

        Ok(declarations)
    }

    fn import_flow_module(
        &mut self,
        module: &str,
        clause: &ImportClause,
        references: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        let exports = self.expand_flow_module(module)?;
        match clause {
            ImportClause::Alias(alias) => {
                for (name, internal) in exports {
                    insert_reference(references, format!("{alias}.{name}"), internal.clone())?;
                }
            }
            ImportClause::Items(items) => {
                for item in items {
                    let internal = exports.get(&item.name).ok_or_else(|| {
                        format!("module `{module}` does not export `{}`", item.name)
                    })?;
                    let name = item.alias.as_deref().unwrap_or(&item.name);
                    insert_reference(references, name.to_string(), internal.clone())?;
                }
            }
        }
        Ok(())
    }

    fn expand_flow_module(&mut self, module: &str) -> Result<&HashMap<String, String>, String> {
        if self.modules.contains_key(module) {
            return Ok(self.modules.get(module).expect("module was just checked"));
        }
        if !self.resolving.insert(module.to_string()) {
            return Err(format!("cyclic stdlib source import involving `{module}`"));
        }

        let source = stdlib::flow_source(module)
            .ok_or_else(|| format!("unknown source-backed stdlib module `{module}`"))?;
        let parsed = parser::parse(source)
            .map_err(|error| format!("failed to parse stdlib module `{module}`: {error}"))?;
        let mut references = HashMap::new();
        let mut exports = HashMap::new();
        let local_names = declaration_names(&parsed);
        let extern_nodes = extern_node_names(&parsed);
        let type_names = type_names(&parsed);

        for name in &local_names {
            let internal = internal_name(module, name);
            insert_reference(&mut references, name.clone(), internal.clone())?;
        }

        let public_exports = stdlib::flow_exports(module)
            .ok_or_else(|| format!("missing exports for `{module}`"))?;
        for name in public_exports {
            if !local_names.contains(*name) {
                return Err(format!(
                    "stdlib module `{module}` declares missing export `{name}`"
                ));
            }
            if !type_names.contains(*name) && !extern_nodes.contains(*name) {
                return Err(format!(
                    "stdlib module `{module}` exports non-extern node `{name}`"
                ));
            }
            exports.insert((*name).to_string(), internal_name(module, name));
        }

        let mut module_declarations = Vec::new();
        let mut import_index = 0usize;
        for decl in &parsed.declarations {
            let Decl::Import(import) = decl else {
                continue;
            };
            let ImportSource::Module(source) = &import.source else {
                return Err(format!(
                    "stdlib module `{module}` uses unsupported local import"
                ));
            };
            if stdlib::flow_source(source).is_some() {
                self.import_flow_module(source, &import.clause, &mut references)?;
            } else {
                let alias = format!("{}_import_{import_index}", internal_prefix(module));
                import_index += 1;
                rewrite_builtin_import(import, source, &alias, &mut references)?;
                module_declarations.push(Decl::Import(Import {
                    source: ImportSource::Module(source.clone()),
                    clause: ImportClause::Alias(alias),
                }));
            }
        }

        for decl in parsed.declarations {
            match decl {
                Decl::TypeAlias(alias) => {
                    let name = internal_name(module, &alias.name);
                    module_declarations.push(Decl::TypeAlias(rewrite_type_alias(
                        alias,
                        &references,
                        name,
                    )));
                }
                Decl::Struct(struct_decl) => {
                    let name = internal_name(module, &struct_decl.name);
                    module_declarations.push(Decl::Struct(rewrite_struct_decl(
                        struct_decl,
                        &references,
                        name,
                    )));
                }
                Decl::Node(callable) => {
                    let name = internal_name(module, &callable.name);
                    module_declarations.push(Decl::Node(rewrite_callable(
                        callable,
                        &references,
                        name,
                    )));
                }
                Decl::Foreign(foreign) => module_declarations.push(Decl::Foreign(
                    rewrite_foreign_block(foreign, &references, Some(module)),
                )),
                Decl::Program(callable) => {
                    return Err(format!(
                        "stdlib module `{module}` declares program `{}`; source-backed stdlib modules may only export nodes",
                        callable.name
                    ));
                }
                Decl::Import(_) => {}
            }
        }

        self.resolving.remove(module);
        self.modules.insert(module.to_string(), exports);
        self.declarations.extend(module_declarations);
        Ok(self.modules.get(module).expect("module was just inserted"))
    }

    fn import_local_module(
        &mut self,
        base_dir: &Path,
        path: &str,
        clause: &ImportClause,
        references: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        let exports = self.expand_local_module(base_dir, path)?;
        match clause {
            ImportClause::Alias(alias) => {
                for (name, internal) in exports {
                    insert_reference(references, format!("{alias}.{name}"), internal.clone())?;
                }
            }
            ImportClause::Items(items) => {
                for item in items {
                    let internal = exports.get(&item.name).ok_or_else(|| {
                        format!("local module `{path}` does not export `{}`", item.name)
                    })?;
                    let name = item.alias.as_deref().unwrap_or(&item.name);
                    insert_reference(references, name.to_string(), internal.clone())?;
                }
            }
        }
        Ok(())
    }

    fn expand_local_module(
        &mut self,
        base_dir: &Path,
        path: &str,
    ) -> Result<&HashMap<String, String>, String> {
        let full_path = normalize_path(base_dir.join(path))?;
        if self.local_modules.contains_key(&full_path) {
            return Ok(self
                .local_modules
                .get(&full_path)
                .expect("local module was just checked"));
        }
        if !self.resolving_local.insert(full_path.clone()) {
            return Err(format!(
                "cyclic local source import involving `{}`",
                full_path.display()
            ));
        }

        let source = fs::read_to_string(&full_path)
            .map_err(|error| format!("failed to read `{}`: {error}", full_path.display()))?;
        let parsed = parser::parse(&source)
            .map_err(|error| format!("failed to parse `{}`: {error}", full_path.display()))?;
        let mut references = HashMap::new();
        let mut exports = HashMap::new();
        let local_names = declaration_names(&parsed);
        let importable_names = importable_names(&parsed);
        let module_id = full_path.to_string_lossy();

        for name in &local_names {
            let internal = internal_name(&module_id, name);
            insert_reference(&mut references, name.clone(), internal.clone())?;
        }

        for name in importable_names {
            exports.insert(name.clone(), internal_name(&module_id, &name));
        }

        let mut module_declarations = Vec::new();
        let mut import_index = 0usize;
        let module_dir = full_path.parent().unwrap_or_else(|| Path::new("."));
        for decl in &parsed.declarations {
            let Decl::Import(import) = decl else {
                continue;
            };
            match &import.source {
                ImportSource::Local(path) => {
                    self.import_local_module(module_dir, path, &import.clause, &mut references)?;
                }
                ImportSource::Module(source) if stdlib::flow_source(source).is_some() => {
                    self.import_flow_module(source, &import.clause, &mut references)?;
                }
                ImportSource::Module(source) => {
                    let alias = format!("{}_import_{import_index}", internal_prefix(&module_id));
                    import_index += 1;
                    rewrite_builtin_import(import, source, &alias, &mut references)?;
                    module_declarations.push(Decl::Import(Import {
                        source: ImportSource::Module(source.clone()),
                        clause: ImportClause::Alias(alias),
                    }));
                }
            }
        }

        for decl in parsed.declarations {
            match decl {
                Decl::TypeAlias(alias) => {
                    let name = internal_name(&module_id, &alias.name);
                    module_declarations.push(Decl::TypeAlias(rewrite_type_alias(
                        alias,
                        &references,
                        name,
                    )));
                }
                Decl::Struct(struct_decl) => {
                    let name = internal_name(&module_id, &struct_decl.name);
                    module_declarations.push(Decl::Struct(rewrite_struct_decl(
                        struct_decl,
                        &references,
                        name,
                    )));
                }
                Decl::Node(callable) => {
                    let name = internal_name(&module_id, &callable.name);
                    module_declarations.push(Decl::Node(rewrite_callable(
                        callable,
                        &references,
                        name,
                    )));
                }
                Decl::Foreign(foreign) => module_declarations.push(Decl::Foreign(
                    rewrite_foreign_block(foreign, &references, Some(&module_id)),
                )),
                Decl::Program(callable) => {
                    return Err(format!(
                        "local module `{}` declares program `{}`; imported local modules may only export types and nodes",
                        full_path.display(),
                        callable.name
                    ));
                }
                Decl::Import(_) => {}
            }
        }

        self.resolving_local.remove(&full_path);
        self.local_modules.insert(full_path.clone(), exports);
        self.declarations.extend(module_declarations);
        Ok(self
            .local_modules
            .get(&full_path)
            .expect("local module was just inserted"))
    }
}

fn normalize_path(path: PathBuf) -> Result<PathBuf, String> {
    if path.exists() {
        path.canonicalize()
            .map_err(|error| format!("failed to canonicalize `{}`: {error}", path.display()))
    } else {
        Ok(path)
    }
}

fn record_decl_symbols(
    module_id: ModuleId,
    decl: &Decl,
    symbols: &mut Vec<ResolvedSymbol>,
    names: &mut HashMap<String, SymbolId>,
) {
    match decl {
        Decl::TypeAlias(alias) => record_symbol(
            module_id,
            &alias.name,
            &alias.name,
            ResolvedSymbolKind::Type,
            ResolvedSymbolOrigin::Source,
            symbols,
            names,
        ),
        Decl::Struct(struct_decl) => record_symbol(
            module_id,
            &struct_decl.name,
            &struct_decl.name,
            ResolvedSymbolKind::Type,
            ResolvedSymbolOrigin::Source,
            symbols,
            names,
        ),
        Decl::Node(callable) | Decl::Program(callable) => record_symbol(
            module_id,
            &callable.name,
            &callable.name,
            ResolvedSymbolKind::Callable,
            ResolvedSymbolOrigin::Source,
            symbols,
            names,
        ),
        Decl::Foreign(foreign) => {
            for node in &foreign.nodes {
                record_symbol(
                    module_id,
                    &node.name,
                    &node.name,
                    ResolvedSymbolKind::Callable,
                    ResolvedSymbolOrigin::Foreign,
                    symbols,
                    names,
                );
            }
        }
        Decl::Import(import) => record_import_symbols(module_id, import, symbols, names),
    }
}

fn record_import_symbols(
    module_id: ModuleId,
    import: &Import,
    symbols: &mut Vec<ResolvedSymbol>,
    names: &mut HashMap<String, SymbolId>,
) {
    let ImportSource::Module(module) = &import.source else {
        return;
    };
    match &import.clause {
        ImportClause::Alias(alias) => {
            for symbol in stdlib::module_symbols(module) {
                let kind = resolved_stdlib_symbol_kind(symbol.kind);
                let name = format!("{alias}.{}", symbol.name);
                record_symbol(
                    module_id,
                    &name,
                    &name,
                    kind,
                    ResolvedSymbolOrigin::StdlibBuiltin,
                    symbols,
                    names,
                );
            }
        }
        ImportClause::Items(items) => {
            for item in items {
                let Some(symbol) = stdlib::find_export(module, &item.name) else {
                    continue;
                };
                let kind = resolved_stdlib_symbol_kind(symbol.kind);
                let name = item.alias.as_deref().unwrap_or(&item.name);
                record_symbol(
                    module_id,
                    name,
                    name,
                    kind,
                    ResolvedSymbolOrigin::StdlibBuiltin,
                    symbols,
                    names,
                );
            }
        }
    }
}

fn resolved_stdlib_symbol_kind(kind: stdlib::SymbolKind) -> ResolvedSymbolKind {
    match kind {
        stdlib::SymbolKind::Type => ResolvedSymbolKind::Type,
        stdlib::SymbolKind::Node => ResolvedSymbolKind::Callable,
    }
}

fn record_symbol(
    module_id: ModuleId,
    public_name: &str,
    internal_name: &str,
    kind: ResolvedSymbolKind,
    origin: ResolvedSymbolOrigin,
    symbols: &mut Vec<ResolvedSymbol>,
    names: &mut HashMap<String, SymbolId>,
) {
    if names.contains_key(internal_name) {
        return;
    }
    let id = SymbolId(symbols.len());
    symbols.push(ResolvedSymbol {
        id,
        module_id,
        public_name: public_name.to_string(),
        internal_name: internal_name.to_string(),
        kind,
        origin,
    });
    names.insert(internal_name.to_string(), id);
}

fn rewrite_builtin_import(
    import: &Import,
    module: &str,
    alias: &str,
    references: &mut HashMap<String, String>,
) -> Result<(), String> {
    match &import.clause {
        ImportClause::Alias(original_alias) => {
            let mut found = false;
            for symbol in stdlib::module_symbols(module) {
                found = true;
                if matches!(
                    symbol.kind,
                    stdlib::SymbolKind::Node | stdlib::SymbolKind::Type
                ) {
                    insert_reference(
                        references,
                        format!("{original_alias}.{}", symbol.name),
                        format!("{alias}.{}", symbol.name),
                    )?;
                }
            }
            if !found {
                return Err(format!("unknown stdlib module `{module}`"));
            }
        }
        ImportClause::Items(items) => {
            for item in items {
                let symbol = stdlib::find_export(module, &item.name)
                    .ok_or_else(|| format!("module `{module}` does not export `{}`", item.name))?;
                if matches!(
                    symbol.kind,
                    stdlib::SymbolKind::Node | stdlib::SymbolKind::Type
                ) {
                    insert_reference(
                        references,
                        item.alias.as_deref().unwrap_or(&item.name).to_string(),
                        format!("{alias}.{}", symbol.name),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn rewrite_callable(
    mut callable: Callable,
    references: &HashMap<String, String>,
    name: String,
) -> Callable {
    callable.name = name;
    for port in &mut callable.inputs {
        port.ty = rewrite_type_text(&port.ty, references);
    }
    for port in &mut callable.outputs {
        port.ty = rewrite_type_text(&port.ty, references);
    }
    for param in &mut callable.node_params {
        param.input = rewrite_type_text(&param.input, references);
        param.output = rewrite_type_text(&param.output, references);
    }
    for chain in &mut callable.chains {
        rewrite_endpoint(&mut chain.source, references);
        for stage in &mut chain.stages {
            rewrite_stage(stage, references);
        }
    }
    callable
}

fn rewrite_type_alias(
    mut alias: TypeAlias,
    references: &HashMap<String, String>,
    name: String,
) -> TypeAlias {
    alias.name = name;
    alias.ty = rewrite_type_text(&alias.ty, references);
    alias
}

fn rewrite_struct_decl(
    mut struct_decl: StructDecl,
    references: &HashMap<String, String>,
    name: String,
) -> StructDecl {
    struct_decl.name = name;
    for field in &mut struct_decl.fields {
        field.ty = rewrite_type_text(&field.ty, references);
    }
    struct_decl
}

fn rewrite_foreign_block(
    mut foreign: ForeignBlock,
    references: &HashMap<String, String>,
    module_id: Option<&str>,
) -> ForeignBlock {
    if let (Some(module_id), ForeignSource::CHeader { header, source }) =
        (module_id, &mut foreign.source)
    {
        let module_dir = Path::new(module_id)
            .parent()
            .unwrap_or_else(|| Path::new("."));
        *header = resolve_foreign_c_path(module_dir, header);
        if let Some(source) = source {
            *source = resolve_foreign_c_path(module_dir, source);
        }
    }
    for node in &mut foreign.nodes {
        if let Some(module_id) = module_id {
            node.name = internal_name(module_id, &node.name);
        }
        for port in &mut node.inputs {
            port.ty = rewrite_type_text(&port.ty, references);
        }
        for port in &mut node.outputs {
            port.ty = rewrite_type_text(&port.ty, references);
        }
    }
    foreign
}

fn resolve_foreign_c_path(module_dir: &Path, path: &str) -> String {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_string_lossy().to_string()
    } else {
        module_dir.join(path).to_string_lossy().to_string()
    }
}

fn rewrite_endpoint(endpoint: &mut Endpoint, references: &HashMap<String, String>) {
    match endpoint {
        Endpoint::Name(name) => rewrite_name(name, references),
        Endpoint::Tuple(items) | Endpoint::Seq(items) => {
            for item in items {
                rewrite_endpoint(item, references);
            }
        }
        Endpoint::Struct { name, fields } => {
            if let Some(rewritten) = references.get(name) {
                *name = rewritten.clone();
            }
            for (_, value) in fields {
                rewrite_endpoint(value, references);
            }
        }
        Endpoint::Eval { source, stages } => {
            rewrite_endpoint(source, references);
            rewrite_stages(stages, references);
        }
        Endpoint::Variable(_)
        | Endpoint::Int(_)
        | Endpoint::Real(_)
        | Endpoint::Bool(_)
        | Endpoint::String(_)
        | Endpoint::Unit => {}
    }
}

fn rewrite_stages(stages: &mut [Stage], references: &HashMap<String, String>) {
    for stage in stages {
        rewrite_stage(stage, references);
    }
}

fn rewrite_stage(stage: &mut Stage, references: &HashMap<String, String>) {
    match stage {
        Stage::Endpoint(endpoint) => rewrite_endpoint(endpoint, references),
        Stage::Bind(_) => {}
        Stage::Field(_) => {}
        Stage::Map(name)
        | Stage::Filter(name)
        | Stage::Scan { op: name, .. }
        | Stage::Reduce { op: name, .. } => rewrite_name(name, references),
        Stage::FaultMap { node, .. } | Stage::Repeat { node, .. } => rewrite_name(node, references),
        Stage::Match { arms } => {
            for arm in arms {
                match &mut arm.guard {
                    MatchGuard::Call { node, args } => {
                        rewrite_name(node, references);
                        for arg in args {
                            rewrite_endpoint(arg, references);
                        }
                    }
                    MatchGuard::Fallback => {}
                }
                match &mut arm.target {
                    MatchTarget::Node(node) => rewrite_name(node, references),
                    MatchTarget::Value(endpoint) => rewrite_endpoint(endpoint, references),
                }
            }
        }
    }
}

fn rewrite_name(name: &mut String, references: &HashMap<String, String>) {
    let node_ref = parse_static_node_ref(name);
    let base = references
        .get(&node_ref.base)
        .cloned()
        .unwrap_or(node_ref.base);
    let args = node_ref
        .args
        .iter()
        .map(|arg| references.get(arg).cloned().unwrap_or_else(|| arg.clone()))
        .collect::<Vec<_>>();
    let rewritten = format_static_node_ref(&base, &args);
    if rewritten != *name {
        *name = rewritten;
    }
}

fn rewrite_type_text(text: &str, references: &HashMap<String, String>) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::new();
    let mut pos = 0usize;
    while pos < chars.len() {
        let ch = chars[pos];
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = pos;
            pos += 1;
            while pos < chars.len()
                && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_' || chars[pos] == '.')
            {
                pos += 1;
            }
            let name = chars[start..pos].iter().collect::<String>();
            out.push_str(references.get(&name).map(String::as_str).unwrap_or(&name));
        } else {
            out.push(ch);
            pos += 1;
        }
    }
    out
}

fn callable_names(module: &Module) -> HashSet<String> {
    module
        .declarations
        .iter()
        .flat_map(|decl| match decl {
            Decl::Node(callable) | Decl::Program(callable) => vec![callable.name.clone()],
            Decl::Foreign(foreign) => foreign
                .nodes
                .iter()
                .map(|node| node.name.clone())
                .collect::<Vec<_>>(),
            Decl::TypeAlias(_) | Decl::Struct(_) | Decl::Import(_) => Vec::new(),
        })
        .collect()
}

fn declaration_names(module: &Module) -> HashSet<String> {
    module
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Decl::TypeAlias(alias) => Some(vec![alias.name.clone()]),
            Decl::Struct(struct_decl) => Some(vec![struct_decl.name.clone()]),
            Decl::Node(callable) | Decl::Program(callable) => Some(vec![callable.name.clone()]),
            Decl::Foreign(foreign) => Some(
                foreign
                    .nodes
                    .iter()
                    .map(|node| node.name.clone())
                    .collect::<Vec<_>>(),
            ),
            Decl::Import(_) => None,
        })
        .flatten()
        .collect()
}

fn type_names(module: &Module) -> HashSet<String> {
    module
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Decl::TypeAlias(alias) => Some(alias.name.clone()),
            Decl::Struct(struct_decl) => Some(struct_decl.name.clone()),
            Decl::Node(_) | Decl::Program(_) | Decl::Foreign(_) | Decl::Import(_) => None,
        })
        .collect()
}

fn extern_node_names(module: &Module) -> HashSet<String> {
    module
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Decl::Node(callable) if callable.is_extern => Some(callable.name.clone()),
            Decl::TypeAlias(_)
            | Decl::Struct(_)
            | Decl::Node(_)
            | Decl::Program(_)
            | Decl::Foreign(_)
            | Decl::Import(_) => None,
        })
        .collect()
}

fn importable_names(module: &Module) -> HashSet<String> {
    type_names(module)
        .into_iter()
        .chain(extern_node_names(module))
        .collect()
}

fn is_source_import(import: &Import) -> bool {
    match &import.source {
        ImportSource::Module(module) => stdlib::flow_source(module).is_some(),
        ImportSource::Local(_) => true,
    }
}

fn insert_reference(
    references: &mut HashMap<String, String>,
    name: String,
    internal: String,
) -> Result<(), String> {
    if references.insert(name.clone(), internal).is_some() {
        return Err(format!("duplicate declaration or import `{name}`"));
    }
    Ok(())
}

fn internal_name(module: &str, name: &str) -> String {
    format!("{}_{}", internal_prefix(module), sanitize(name))
}

fn internal_prefix(module: &str) -> String {
    format!("__flow_{}", sanitize(module))
}

fn sanitize(name: &str) -> String {
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
