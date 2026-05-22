use crate::ast::*;
use crate::node_ref::{format_static_node_ref, parse_static_node_ref};
use std::collections::HashMap;

pub(crate) fn expand_module(module: &Module) -> Result<Module, String> {
    Monomorphizer::new(module).expand(module)
}

struct Monomorphizer {
    templates: HashMap<String, Callable>,
    instances: HashMap<InstanceKey, String>,
    generated: Vec<Callable>,
    active: Vec<InstanceKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InstanceKey {
    template: String,
    args: Vec<String>,
}

impl Monomorphizer {
    fn new(module: &Module) -> Self {
        let mut templates = HashMap::new();
        for decl in &module.declarations {
            if let Decl::Node(callable) = decl
                && !callable.node_params.is_empty()
            {
                templates.insert(callable.name.clone(), callable.clone());
            }
        }
        Self {
            templates,
            instances: HashMap::new(),
            generated: Vec::new(),
            active: Vec::new(),
        }
    }

    fn expand(mut self, module: &Module) -> Result<Module, String> {
        let mut declarations = Vec::new();
        for decl in &module.declarations {
            match decl {
                Decl::Node(callable) if !callable.node_params.is_empty() => {}
                Decl::Node(callable) => {
                    let mut callable = callable.clone();
                    self.rewrite_callable_refs(&mut callable, &HashMap::new())?;
                    declarations.push(Decl::Node(callable));
                }
                Decl::Program(callable) => {
                    if !callable.node_params.is_empty() {
                        return Err(format!(
                            "program `{}` cannot declare static node parameters",
                            callable.name
                        ));
                    }
                    let mut callable = callable.clone();
                    self.rewrite_callable_refs(&mut callable, &HashMap::new())?;
                    declarations.push(Decl::Program(callable));
                }
                Decl::TypeAlias(alias) => declarations.push(Decl::TypeAlias(alias.clone())),
                Decl::Struct(struct_decl) => declarations.push(Decl::Struct(struct_decl.clone())),
                Decl::Import(import) => declarations.push(Decl::Import(import.clone())),
                Decl::Foreign(foreign) => declarations.push(Decl::Foreign(foreign.clone())),
            }
        }
        declarations.extend(self.generated.drain(..).map(Decl::Node));
        Ok(Module { declarations })
    }

    fn instantiate(&mut self, template: &str, args: &[String]) -> Result<String, String> {
        let key = InstanceKey {
            template: template.to_string(),
            args: args.to_vec(),
        };
        if let Some(name) = self.instances.get(&key) {
            return Ok(name.clone());
        }
        if self.active.iter().any(|active| active == &key) {
            return Err(format!(
                "recursive static node instantiation `{}`",
                format_static_node_ref(template, args)
            ));
        }
        let template_callable = self
            .templates
            .get(template)
            .cloned()
            .ok_or_else(|| format!("node `{template}` does not take static node arguments"))?;
        if template_callable.node_params.len() != args.len() {
            return Err(format!(
                "node `{template}` expected {} static node arguments, found {}",
                template_callable.node_params.len(),
                args.len()
            ));
        }
        let generated_name = self.instance_name(template, args);
        self.instances.insert(key.clone(), generated_name.clone());
        self.active.push(key);

        let bindings = template_callable
            .node_params
            .iter()
            .zip(args.iter())
            .map(|(param, arg)| (param.name.clone(), arg.clone()))
            .collect::<HashMap<_, _>>();
        let mut callable = template_callable;
        callable.name = generated_name.clone();
        callable.is_extern = false;
        callable.node_params.clear();
        self.rewrite_callable_refs(&mut callable, &bindings)?;
        self.generated.push(callable);
        self.active.pop();
        Ok(generated_name)
    }

    fn instance_name(&self, template: &str, args: &[String]) -> String {
        let mut name = format!("__flow_inst_{}", sanitize_name(template));
        for arg in args {
            name.push('_');
            name.push_str(&sanitize_name(arg));
        }
        name
    }

    fn rewrite_callable_refs(
        &mut self,
        callable: &mut Callable,
        bindings: &HashMap<String, String>,
    ) -> Result<(), String> {
        for chain in &mut callable.chains {
            self.rewrite_endpoint(&mut chain.source, bindings)?;
            for stage in &mut chain.stages {
                self.rewrite_stage(stage, bindings)?;
            }
        }
        Ok(())
    }

    fn rewrite_stage(
        &mut self,
        stage: &mut Stage,
        bindings: &HashMap<String, String>,
    ) -> Result<(), String> {
        match stage {
            Stage::Endpoint(endpoint) => self.rewrite_endpoint(endpoint, bindings),
            Stage::Bind(_) | Stage::Field(_) => Ok(()),
            Stage::Map(name)
            | Stage::Filter(name)
            | Stage::Reduce { op: name, .. }
            | Stage::Scan { op: name, .. } => {
                self.rewrite_node_name(name, bindings)?;
                Ok(())
            }
            Stage::FaultMap { node, .. } | Stage::Repeat { node, .. } => {
                self.rewrite_node_name(node, bindings)?;
                Ok(())
            }
            Stage::Match { arms } => {
                for arm in arms {
                    match &mut arm.guard {
                        MatchGuard::Call { node, args } => {
                            self.rewrite_node_name(node, bindings)?;
                            for arg in args {
                                self.rewrite_endpoint(arg, bindings)?;
                            }
                        }
                        MatchGuard::Fallback => {}
                    }
                    match &mut arm.target {
                        MatchTarget::Node(node) => self.rewrite_node_name(node, bindings)?,
                        MatchTarget::Value(endpoint) => {
                            self.rewrite_endpoint(endpoint, bindings)?
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn rewrite_endpoint(
        &mut self,
        endpoint: &mut Endpoint,
        bindings: &HashMap<String, String>,
    ) -> Result<(), String> {
        match endpoint {
            Endpoint::Name(name) => {
                self.rewrite_node_name(name, bindings)?;
                Ok(())
            }
            Endpoint::Tuple(items) | Endpoint::Seq(items) => {
                for item in items {
                    self.rewrite_endpoint(item, bindings)?;
                }
                Ok(())
            }
            Endpoint::Struct { fields, .. } => {
                for (_, item) in fields {
                    self.rewrite_endpoint(item, bindings)?;
                }
                Ok(())
            }
            Endpoint::Eval { source, stages } => {
                self.rewrite_endpoint(source, bindings)?;
                for stage in stages {
                    self.rewrite_stage(stage, bindings)?;
                }
                Ok(())
            }
            Endpoint::Variable(_)
            | Endpoint::Int(_)
            | Endpoint::Real(_)
            | Endpoint::Bool(_)
            | Endpoint::String(_)
            | Endpoint::Unit => Ok(()),
        }
    }

    fn rewrite_node_name(
        &mut self,
        name: &mut String,
        bindings: &HashMap<String, String>,
    ) -> Result<(), String> {
        let node_ref = parse_static_node_ref(name);
        if let Some(actual) = bindings.get(&node_ref.base) {
            if !node_ref.args.is_empty() {
                return Err(format!(
                    "static node parameter `{}` cannot take static node arguments",
                    node_ref.base
                ));
            }
            *name = actual.clone();
            return Ok(());
        }
        let args = node_ref
            .args
            .iter()
            .map(|arg| bindings.get(arg).cloned().unwrap_or_else(|| arg.clone()))
            .collect::<Vec<_>>();
        if args.is_empty() {
            *name = node_ref.base;
        } else {
            *name = self.instantiate(&node_ref.base, &args)?;
        }
        Ok(())
    }
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}
