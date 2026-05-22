use crate::ast::*;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

#[derive(Debug, Clone, Copy, Default)]
pub struct MermaidOptions {
    pub compact: bool,
}

pub fn emit_module(module: &Module) -> Result<String, String> {
    emit_module_with_options(module, MermaidOptions::default())
}

pub fn emit_module_with_options(
    module: &Module,
    options: MermaidOptions,
) -> Result<String, String> {
    let mut emitter = MermaidEmitter {
        out: String::new(),
        id_counts: HashMap::new(),
        classes: Vec::new(),
        boundary_names: collect_boundary_names(module),
        boundary_prefixes: collect_boundary_prefixes(module),
        current_callable: String::new(),
        output_names: HashSet::new(),
        options,
    };
    emitter.line("flowchart TD");
    for decl in &module.declarations {
        match decl {
            Decl::TypeAlias(_) | Decl::Struct(_) | Decl::Import(_) => {}
            Decl::Node(callable) => {
                let kind = if callable.is_extern {
                    "extern node"
                } else {
                    "node"
                };
                emitter.emit_callable(callable, kind)?;
            }
            Decl::Program(callable) => emitter.emit_callable(callable, "program")?,
        }
    }
    emitter.emit_legend();
    emitter.emit_styles();
    Ok(emitter.out)
}

struct MermaidEmitter {
    out: String,
    id_counts: HashMap<String, usize>,
    classes: Vec<(String, &'static str)>,
    boundary_names: HashSet<String>,
    boundary_prefixes: Vec<String>,
    current_callable: String,
    output_names: HashSet<String>,
    options: MermaidOptions,
}

#[derive(Debug, Clone)]
struct NodeRef {
    id: String,
    value_label: Option<String>,
}

impl MermaidEmitter {
    fn emit_callable(&mut self, callable: &Callable, kind: &str) -> Result<(), String> {
        self.current_callable = sanitize_id(&callable.name);
        self.output_names = callable
            .outputs
            .iter()
            .map(|port| port.name.clone())
            .collect();
        let subgraph_id = format!("callable_{}", sanitize_id(&callable.name));
        self.line(&format!(
            "  subgraph {subgraph_id}[\"{}\"]",
            escape_label(&format!("{kind} {}", callable.name))
        ));

        let mut env = HashMap::new();
        for port in &callable.inputs {
            let input = self.variable_node(
                &format!("${}: {}", port.name, port.ty),
                Some(&format!("${}", port.name)),
                "    ",
            );
            env.insert(port.name.clone(), vec![input]);
        }

        for chain in &callable.chains {
            self.emit_chain(chain, &mut env)?;
        }

        self.line("  end");
        Ok(())
    }

    fn emit_chain(
        &mut self,
        chain: &Chain,
        env: &mut HashMap<String, Vec<NodeRef>>,
    ) -> Result<(), String> {
        let mut current = self.emit_source_endpoint(&chain.source, env)?;
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Bind(target) if is_last => {
                    self.bind_target(target, &current, env, None)?;
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    let operation = self.operation_node(name, "call", "    ");
                    self.edges(&current, &operation, None, "    ");
                    current = vec![operation];
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
                    let operation = self.collection_node(&format!("map {name}"), "map", "    ");
                    self.edges(&current, &operation, None, "    ");
                    current = vec![operation];
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let operation =
                        self.fault_node(&format!("fault map {node}"), "fault_map", "    ");
                    self.edges(&current, &operation, None, "    ");
                    if self.options.compact {
                        self.bind_fault_result(ok, "ok", &operation, env, "    ")?;
                        self.bind_fault_result(fault, "fault", &operation, env, "    ")?;
                    } else {
                        let result_id = self.unique_id("fault_results", node);
                        self.line(&format!("    subgraph {result_id}[\"fault map results\"]"));
                        self.bind_fault_result(ok, "ok", &operation, env, "      ")?;
                        self.bind_fault_result(fault, "fault", &operation, env, "      ")?;
                        self.line("    end");
                    }
                    current = Vec::new();
                }
                Stage::Filter(name) => {
                    let operation =
                        self.collection_node(&format!("filter {name}"), "filter", "    ");
                    self.edges(&current, &operation, None, "    ");
                    current = vec![operation];
                }
                Stage::Field(name) => {
                    let operation = self.operation_node(&format!("field {name}"), "field", "    ");
                    self.edges(&current, &operation, None, "    ");
                    current = vec![operation];
                }
                Stage::Repeat { count, node } => {
                    let operation = self.collection_node(
                        &format!("repeat<{}> {node}", endpoint_label(count)),
                        "repeat",
                        "    ",
                    );
                    self.edges(&current, &operation, None, "    ");
                    let count = self.emit_endpoint(count, env)?;
                    self.edges(&count, &operation, Some("count"), "    ");
                    current = vec![operation];
                }
                Stage::Reduce { op, identity } => {
                    let operation = self.collection_node(
                        &format!("reduce {op}\nidentity: {}", endpoint_label(identity)),
                        "reduce",
                        "    ",
                    );
                    self.edges(&current, &operation, None, "    ");
                    let identity = self.emit_endpoint(identity, env)?;
                    self.edges(&identity, &operation, Some("identity"), "    ");
                    current = vec![operation];
                }
                Stage::Scan { op, identity } => {
                    let operation = self.collection_node(
                        &format!("scan {op}\nidentity: {}", endpoint_label(identity)),
                        "scan",
                        "    ",
                    );
                    self.edges(&current, &operation, None, "    ");
                    let identity = self.emit_endpoint(identity, env)?;
                    self.edges(&identity, &operation, Some("identity"), "    ");
                    current = vec![operation];
                }
                Stage::Match { arms } => {
                    let operation = self.decision_node("match ?", "    ");
                    self.edges(&current, &operation, Some("subject"), "    ");
                    let mut branches = Vec::new();
                    for (arm_index, arm) in arms.iter().enumerate() {
                        if let MatchGuard::Call { args, .. } = &arm.guard {
                            for arg in args {
                                let arg_nodes = self.emit_endpoint(arg, env)?;
                                self.edges(&arg_nodes, &operation, Some("guard arg"), "    ");
                            }
                        }
                        let arm_id = self.unique_id("match_arm", &arm_index.to_string());
                        self.line(&format!(
                            "    subgraph {arm_id}[\"arm: {}\"]",
                            escape_label(&match_guard_label(&arm.guard))
                        ));
                        let branch = self.match_target_node(&arm.target, "      ");
                        self.line("    end");
                        self.edge(
                            &operation,
                            &branch,
                            Some(&match_guard_label(&arm.guard)),
                            "    ",
                        );
                        branches.push(branch);
                    }
                    current = branches;
                }
            }
        }
        Ok(())
    }

    fn bind_variable(
        &mut self,
        name: &str,
        current: &[NodeRef],
        env: &mut HashMap<String, Vec<NodeRef>>,
    ) -> Result<(), String> {
        let value_label = format!("${name}");
        let refs = if self.options.compact && !self.output_names.contains(name) {
            current
                .iter()
                .cloned()
                .map(|node| node.with_value_label(value_label.clone()))
                .collect()
        } else {
            let variable = self.variable_node(&value_label, Some(&value_label), "    ");
            self.edges(current, &variable, Some("binds"), "    ");
            vec![variable]
        };
        if env.insert(name.to_string(), refs).is_some() {
            return Err(format!("value `{name}` is bound more than once"));
        }
        Ok(())
    }

    fn bind_target(
        &mut self,
        target: &BindingTarget,
        current: &[NodeRef],
        env: &mut HashMap<String, Vec<NodeRef>>,
        edge_label: Option<&str>,
    ) -> Result<(), String> {
        match target {
            BindingTarget::Discard => Ok(()),
            BindingTarget::Variable(name) => {
                if let Some(edge_label) = edge_label {
                    let value_label = format!("${name}");
                    let variable = self.variable_node(&value_label, Some(&value_label), "    ");
                    self.edges(current, &variable, Some(edge_label), "    ");
                    if env.insert(name.to_string(), vec![variable]).is_some() {
                        return Err(format!("value `{name}` is bound more than once"));
                    }
                    Ok(())
                } else {
                    self.bind_variable(name, current, env)
                }
            }
            BindingTarget::Tuple(items) => {
                for (index, item) in items.iter().enumerate() {
                    let label = match edge_label {
                        Some(prefix) => format!("{prefix}.f{index}"),
                        None => format!("f{index}"),
                    };
                    self.bind_target(item, current, env, Some(&label))?;
                }
                Ok(())
            }
        }
    }

    fn bind_fault_result(
        &mut self,
        name: &str,
        edge_label: &str,
        operation: &NodeRef,
        env: &mut HashMap<String, Vec<NodeRef>>,
        indent: &str,
    ) -> Result<(), String> {
        let value_label = format!("${name}");
        let refs = if self.options.compact && !self.output_names.contains(name) {
            vec![operation.clone().with_value_label(value_label)]
        } else {
            let variable = self.variable_node(&value_label, Some(&value_label), indent);
            self.edge(operation, &variable, Some(edge_label), indent);
            vec![variable]
        };
        if env.insert(name.to_string(), refs).is_some() {
            return Err(format!("value `{name}` is bound more than once"));
        }
        Ok(())
    }

    fn emit_source_endpoint(
        &mut self,
        endpoint: &Endpoint,
        env: &HashMap<String, Vec<NodeRef>>,
    ) -> Result<Vec<NodeRef>, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Tuple(items) | Endpoint::Seq(items) => {
                let source = self.literal_node(
                    &format!("input\n{}", endpoint_label(endpoint)),
                    "input",
                    "    ",
                );
                let dependencies = self.emit_endpoint_items(items, env)?;
                self.edges(&dependencies, &source, Some("item"), "    ");
                Ok(vec![source])
            }
            Endpoint::Struct { fields, .. } => {
                let source = self.literal_node(
                    &format!("input\n{}", endpoint_label(endpoint)),
                    "input",
                    "    ",
                );
                let dependencies = self.emit_struct_field_items(fields, env)?;
                self.edges(&dependencies, &source, Some("field"), "    ");
                Ok(vec![source])
            }
            Endpoint::Eval { .. } => {
                let source = self.literal_node(
                    &format!("input\n{}", endpoint_label(endpoint)),
                    "input",
                    "    ",
                );
                let dependencies = self.emit_endpoint(endpoint, env)?;
                self.edges(&dependencies, &source, Some("item"), "    ");
                Ok(vec![source])
            }
            Endpoint::Unit => Ok(Vec::new()),
            _ => Ok(vec![self.literal_node(
                &endpoint_label(endpoint),
                "literal",
                "    ",
            )]),
        }
    }

    fn emit_endpoint(
        &mut self,
        endpoint: &Endpoint,
        env: &HashMap<String, Vec<NodeRef>>,
    ) -> Result<Vec<NodeRef>, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Tuple(items) | Endpoint::Seq(items) => {
                let mut sources = Vec::new();
                for item in items {
                    sources.extend(self.emit_endpoint(item, env)?);
                }
                Ok(sources)
            }
            Endpoint::Struct { fields, .. } => self.emit_struct_field_items(fields, env),
            Endpoint::Eval { source, .. } => self.emit_endpoint(source, env),
            _ => Ok(Vec::new()),
        }
    }

    fn emit_endpoint_items(
        &mut self,
        items: &[Endpoint],
        env: &HashMap<String, Vec<NodeRef>>,
    ) -> Result<Vec<NodeRef>, String> {
        let mut sources = Vec::new();
        for item in items {
            sources.extend(self.emit_endpoint(item, env)?);
        }
        Ok(sources)
    }

    fn emit_struct_field_items(
        &mut self,
        fields: &[(String, Endpoint)],
        env: &HashMap<String, Vec<NodeRef>>,
    ) -> Result<Vec<NodeRef>, String> {
        let mut sources = Vec::new();
        for (_, item) in fields {
            sources.extend(self.emit_endpoint(item, env)?);
        }
        Ok(sources)
    }

    fn operation_node(&mut self, label: &str, role: &str, indent: &str) -> NodeRef {
        let class = if self.is_boundary_node(label) {
            "boundary"
        } else {
            "op"
        };
        let shape = if class == "boundary" {
            NodeShape::Subroutine
        } else {
            NodeShape::Rect
        };
        self.node(label, role, shape, class, indent)
    }

    fn collection_node(&mut self, label: &str, role: &str, indent: &str) -> NodeRef {
        self.node(label, role, NodeShape::Rect, "collection", indent)
    }

    fn fault_node(&mut self, label: &str, role: &str, indent: &str) -> NodeRef {
        self.node(label, role, NodeShape::Rect, "fault", indent)
    }

    fn literal_node(&mut self, label: &str, role: &str, indent: &str) -> NodeRef {
        self.node(label, role, NodeShape::Round, "literal", indent)
    }

    fn variable_node(&mut self, label: &str, value_label: Option<&str>, indent: &str) -> NodeRef {
        self.node_with_value_label(
            label,
            "value",
            NodeShape::Round,
            "value",
            value_label,
            indent,
        )
    }

    fn decision_node(&mut self, label: &str, indent: &str) -> NodeRef {
        self.node(label, "match", NodeShape::Diamond, "decision", indent)
    }

    fn match_target_node(&mut self, target: &MatchTarget, indent: &str) -> NodeRef {
        match target {
            MatchTarget::Node(node) => self.operation_node(node, "match_target", indent),
            MatchTarget::Value(endpoint) => self.literal_node(
                &match_target_label(&MatchTarget::Value(endpoint.clone())),
                "match_value",
                indent,
            ),
        }
    }

    fn node(
        &mut self,
        label: &str,
        role: &str,
        shape: NodeShape,
        class: &'static str,
        indent: &str,
    ) -> NodeRef {
        self.node_with_value_label(label, role, shape, class, None, indent)
    }

    fn node_with_value_label(
        &mut self,
        label: &str,
        role: &str,
        shape: NodeShape,
        class: &'static str,
        value_label: Option<&str>,
        indent: &str,
    ) -> NodeRef {
        let id = self.unique_id(role, label);
        let escaped = escape_label(label);
        match shape {
            NodeShape::Rect => self.line(&format!("{indent}{id}[\"{escaped}\"]")),
            NodeShape::Round => self.line(&format!("{indent}{id}([\"{escaped}\"])")),
            NodeShape::Diamond => self.line(&format!("{indent}{id}{{\"{escaped}\"}}")),
            NodeShape::Subroutine => self.line(&format!("{indent}{id}[[\"{escaped}\"]]")),
        }
        self.classes.push((id.clone(), class));
        NodeRef {
            id,
            value_label: value_label.map(ToString::to_string),
        }
    }

    fn edge(&mut self, from: &NodeRef, to: &NodeRef, label: Option<&str>, indent: &str) {
        let label = label.or(from.value_label.as_deref()).or(Some("value"));
        match label {
            Some(label) => self.line(&format!(
                "{indent}{} -- \"{}\" --> {}",
                from.id,
                escape_label(label),
                to.id
            )),
            None => self.line(&format!("{indent}{} --> {}", from.id, to.id)),
        }
    }

    fn edges(&mut self, from: &[NodeRef], to: &NodeRef, label: Option<&str>, indent: &str) {
        for source in from {
            self.edge(source, to, label, indent);
        }
    }

    fn is_boundary_node(&self, name: &str) -> bool {
        self.boundary_names.contains(name)
            || self
                .boundary_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
            || matches!(
                name,
                "read_stdin" | "write_stdout" | "write_stderr" | "open_file" | "listen" | "serve"
            )
    }

    fn unique_id(&mut self, role: &str, label: &str) -> String {
        let base = format!(
            "{}_{}_{}",
            self.current_callable,
            sanitize_id(role),
            sanitize_id(label)
        );
        let base = if base.len() > 96 {
            base.chars().take(96).collect::<String>()
        } else {
            base
        };
        let count = self.id_counts.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            base
        } else {
            format!("{base}_{}", *count)
        }
    }

    fn emit_legend(&mut self) {
        self.current_callable = "legend".to_string();
        self.line("  subgraph legend[\"legend\"]");
        self.node(
            "value / binding",
            "value",
            NodeShape::Round,
            "value",
            "    ",
        );
        self.node("pure operation", "op", NodeShape::Rect, "op", "    ");
        self.node(
            "boundary operation",
            "boundary",
            NodeShape::Subroutine,
            "boundary",
            "    ",
        );
        self.node(
            "collection operator",
            "collection",
            NodeShape::Rect,
            "collection",
            "    ",
        );
        self.node(
            "match / decision",
            "decision",
            NodeShape::Diamond,
            "decision",
            "    ",
        );
        self.node("fault path", "fault", NodeShape::Rect, "fault", "    ");
        self.line("  end");
    }

    fn emit_styles(&mut self) {
        self.line("  classDef value fill:#e8f4ff,stroke:#2f6f9f,color:#102a43");
        self.line("  classDef literal fill:#f7f9fb,stroke:#9aa6b2,color:#1f2933");
        self.line("  classDef op fill:#ffffff,stroke:#59636e,color:#111827");
        self.line("  classDef boundary fill:#fff4df,stroke:#b87918,color:#3f2a05");
        self.line("  classDef collection fill:#ecfdf3,stroke:#2f855a,color:#123524");
        self.line("  classDef decision fill:#f4ecff,stroke:#7c3aed,color:#2d124d");
        self.line("  classDef fault fill:#ffecec,stroke:#c64242,color:#5a1111");
        for (id, class) in self.classes.clone() {
            self.line(&format!("  class {id} {class}"));
        }
    }

    fn line(&mut self, line: &str) {
        self.out.push_str(line);
        self.out.push('\n');
    }
}

#[derive(Debug, Clone, Copy)]
enum NodeShape {
    Rect,
    Round,
    Diamond,
    Subroutine,
}

impl NodeRef {
    fn with_value_label(mut self, value_label: String) -> Self {
        self.value_label = Some(value_label);
        self
    }
}

fn endpoint_label(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Variable(name) => format!("${name}"),
        Endpoint::Name(name) => name.clone(),
        Endpoint::Int(value) => value.to_string(),
        Endpoint::Real(value) => {
            let mut text = value.to_string();
            if !text.contains('.') && !text.contains('e') && !text.contains('E') {
                text.push_str(".0");
            }
            text
        }
        Endpoint::Bool(value) => value.to_string(),
        Endpoint::String(value) => {
            let mut out = String::from("\"");
            for ch in value.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    other => out.push(other),
                }
            }
            out.push('"');
            out
        }
        Endpoint::Unit => "()".to_string(),
        Endpoint::Tuple(items) => endpoint_list_label(items, "(", ")"),
        Endpoint::Seq(items) => endpoint_list_label(items, "[", "]"),
        Endpoint::Struct { name, fields } => format!(
            "{name} {{ {} }}",
            fields
                .iter()
                .map(|(field, value)| format!("{field}: {}", endpoint_label(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Endpoint::Eval { source, stages } => {
            let mut parts = Vec::with_capacity(stages.len() + 1);
            parts.push(endpoint_label(source));
            parts.extend(stages.iter().map(stage_label));
            parts.join(" -> ")
        }
    }
}

fn stage_label(stage: &Stage) -> String {
    match stage {
        Stage::Endpoint(endpoint) => endpoint_label(endpoint),
        Stage::Bind(target) => binding_target_label(target),
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

fn binding_target_label(target: &BindingTarget) -> String {
    match target {
        BindingTarget::Discard => "$".to_string(),
        BindingTarget::Variable(name) => format!("${name}"),
        BindingTarget::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(binding_target_label)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn match_guard_label(guard: &MatchGuard) -> String {
    match guard {
        MatchGuard::Fallback => "_".to_string(),
        MatchGuard::Call { node, args } => format!(
            "{}({})",
            node,
            args.iter()
                .map(endpoint_label)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn match_target_label(target: &MatchTarget) -> String {
    match target {
        MatchTarget::Node(node) => node.clone(),
        MatchTarget::Value(endpoint) => endpoint_label(endpoint),
    }
}

fn endpoint_list_label(items: &[Endpoint], open: &str, close: &str) -> String {
    let mut out = String::from(open);
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&endpoint_label(item));
    }
    out.push_str(close);
    out
}

fn collect_boundary_names(module: &Module) -> HashSet<String> {
    let mut names = HashSet::new();
    for decl in &module.declarations {
        let Decl::Import(import) = decl else {
            continue;
        };
        let ImportSource::Module(source) = &import.source else {
            continue;
        };
        if !is_boundary_module(source) {
            continue;
        }
        if let ImportClause::Items(items) = &import.clause {
            for item in items {
                names.insert(item.alias.clone().unwrap_or_else(|| item.name.clone()));
            }
        }
    }
    names
}

fn collect_boundary_prefixes(module: &Module) -> Vec<String> {
    let mut prefixes = Vec::new();
    for decl in &module.declarations {
        let Decl::Import(import) = decl else {
            continue;
        };
        let ImportSource::Module(source) = &import.source else {
            continue;
        };
        if !is_boundary_module(source) {
            continue;
        }
        if let ImportClause::Alias(alias) = &import.clause {
            prefixes.push(format!("{alias}."));
        }
    }
    prefixes
}

fn is_boundary_module(source: &str) -> bool {
    matches!(
        source,
        "std.io" | "std.fs" | "std.http" | "std.sqlite" | "std.stream" | "std.cv"
    )
}

fn escape_label(label: &str) -> String {
    let mut escaped = String::new();
    for ch in label.chars() {
        match ch {
            '"' => escaped.push_str("&quot;"),
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\n' => escaped.push_str("<br/>"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn sanitize_id(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            let _ = write!(out, "_{:x}", ch as u32);
        }
    }
    if out.is_empty() {
        "anonymous".to_string()
    } else {
        out
    }
}
