use crate::ast::*;
use crate::{parser, stdlib};
use std::collections::{HashMap, HashSet};

pub fn expand_stdlib_sources(module: &Module) -> Result<Module, String> {
    let mut resolver = Resolver::default();
    let root = resolver.rewrite_root(module)?;
    let mut declarations = resolver.declarations;
    declarations.extend(root);
    Ok(Module { declarations })
}

#[derive(Default)]
struct Resolver {
    modules: HashMap<String, HashMap<String, String>>,
    resolving: HashSet<String>,
    declarations: Vec<Decl>,
}

impl Resolver {
    fn rewrite_root(&mut self, module: &Module) -> Result<Vec<Decl>, String> {
        let callable_names = callable_names(module);
        let mut references = HashMap::new();
        let mut declarations = Vec::new();

        for decl in &module.declarations {
            let Decl::Import(import) = decl else {
                continue;
            };
            let ImportSource::Module(source) = &import.source else {
                continue;
            };
            if stdlib::flow_source(source).is_none() {
                continue;
            }
            self.import_flow_module(source, &import.clause, &mut references)?;
        }

        for name in references.keys().filter(|name| !name.contains('.')) {
            if callable_names.contains(name) {
                return Err(format!("duplicate declaration or import `{name}`"));
            }
        }

        for decl in &module.declarations {
            match decl {
                Decl::Import(import) if is_flow_std_import(import) => {}
                Decl::TypeAlias(alias) => declarations.push(Decl::TypeAlias(rewrite_type_alias(
                    alias.clone(),
                    &references,
                    alias.name.clone(),
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

        for name in &local_names {
            let internal = internal_name(module, &name);
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
                Decl::Node(callable) => {
                    let name = internal_name(module, &callable.name);
                    module_declarations.push(Decl::Node(rewrite_callable(
                        callable,
                        &references,
                        name,
                    )));
                }
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
    for chain in &mut callable.chains {
        rewrite_endpoint(&mut chain.source, references);
        for stage in &mut chain.stages {
            match stage {
                Stage::Endpoint(endpoint) => rewrite_endpoint(endpoint, references),
                Stage::Map(name)
                | Stage::Filter(name)
                | Stage::Scan { op: name, .. }
                | Stage::Reduce { op: name, .. } => rewrite_name(name, references),
                Stage::FaultMap { node, .. } | Stage::Repeat { node, .. } => {
                    rewrite_name(node, references)
                }
            }
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

fn rewrite_endpoint(endpoint: &mut Endpoint, references: &HashMap<String, String>) {
    match endpoint {
        Endpoint::Name(name) => rewrite_name(name, references),
        Endpoint::Tuple(items) | Endpoint::Seq(items) => {
            for item in items {
                rewrite_endpoint(item, references);
            }
        }
        Endpoint::Variable(_)
        | Endpoint::Int(_)
        | Endpoint::Real(_)
        | Endpoint::Bool(_)
        | Endpoint::String(_)
        | Endpoint::Unit => {}
    }
}

fn rewrite_name(name: &mut String, references: &HashMap<String, String>) {
    if let Some(internal) = references.get(name) {
        *name = internal.clone();
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
        .filter_map(|decl| match decl {
            Decl::Node(callable) | Decl::Program(callable) => Some(callable.name.clone()),
            Decl::TypeAlias(_) | Decl::Import(_) => None,
        })
        .collect()
}

fn declaration_names(module: &Module) -> HashSet<String> {
    module
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            Decl::TypeAlias(alias) => Some(alias.name.clone()),
            Decl::Node(callable) | Decl::Program(callable) => Some(callable.name.clone()),
            Decl::Import(_) => None,
        })
        .collect()
}

fn is_flow_std_import(import: &Import) -> bool {
    match &import.source {
        ImportSource::Module(module) => stdlib::flow_source(module).is_some(),
        ImportSource::Local(_) => false,
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
