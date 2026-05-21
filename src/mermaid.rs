use crate::ast::*;
use std::collections::HashMap;
use std::fmt::Write;

pub fn emit_module(module: &Module) -> Result<String, String> {
    let mut emitter = MermaidEmitter {
        out: String::new(),
        next_id: 0,
    };
    emitter.line("flowchart TD");
    for decl in &module.declarations {
        match decl {
            Decl::TypeAlias(_) | Decl::Import(_) => {}
            Decl::Node(callable) => emitter.emit_callable(callable, "node")?,
            Decl::Program(callable) => emitter.emit_callable(callable, "program")?,
        }
    }
    Ok(emitter.out)
}

struct MermaidEmitter {
    out: String,
    next_id: usize,
}

impl MermaidEmitter {
    fn emit_callable(&mut self, callable: &Callable, kind: &str) -> Result<(), String> {
        let subgraph_id = format!("callable_{}", sanitize_id(&callable.name));
        self.line(&format!(
            "  subgraph {subgraph_id}[\"{}\"]",
            escape_label(&format!("{kind} {}", callable.name))
        ));

        let mut env = HashMap::new();
        for port in &callable.inputs {
            let input = self.variable_node(&format!("${}: {}", port.name, port.ty), "    ");
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
        env: &mut HashMap<String, Vec<String>>,
    ) -> Result<(), String> {
        let mut current = self.emit_source_endpoint(&chain.source, env)?;
        for (index, stage) in chain.stages.iter().enumerate() {
            let is_last = index + 1 == chain.stages.len();
            match stage {
                Stage::Endpoint(Endpoint::Variable(name)) if is_last => {
                    self.bind_variable(name, &current, env)?;
                }
                Stage::Endpoint(Endpoint::Name(name)) => {
                    let operation = self.node(name, "    ");
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
                Stage::Map(name) => {
                    let operation = self.node(&format!("map {name}"), "    ");
                    self.edges(&current, &operation, None, "    ");
                    current = vec![operation];
                }
                Stage::FaultMap { node, ok, fault } => {
                    if !is_last {
                        return Err("`fault map` must be the final stage in a chain".to_string());
                    }
                    let operation = self.node(&format!("fault map {node}"), "    ");
                    self.edges(&current, &operation, None, "    ");
                    let ok_node = self.variable_node(&format!("${ok}"), "    ");
                    self.edge(&operation, &ok_node, Some("ok"), "    ");
                    if env.insert(ok.clone(), vec![ok_node]).is_some() {
                        return Err(format!("value `{ok}` is bound more than once"));
                    }
                    let fault_node = self.variable_node(&format!("${fault}"), "    ");
                    self.edge(&operation, &fault_node, Some("fault"), "    ");
                    if env.insert(fault.clone(), vec![fault_node]).is_some() {
                        return Err(format!("value `{fault}` is bound more than once"));
                    }
                    current = Vec::new();
                }
                Stage::Filter(name) => {
                    let operation = self.node(&format!("filter {name}"), "    ");
                    self.edges(&current, &operation, None, "    ");
                    current = vec![operation];
                }
                Stage::Repeat { count, node } => {
                    let operation =
                        self.node(&format!("repeat<{}> {node}", endpoint_label(count)), "    ");
                    self.edges(&current, &operation, None, "    ");
                    let count = self.emit_endpoint(count, env)?;
                    self.edges(&count, &operation, Some("count"), "    ");
                    current = vec![operation];
                }
                Stage::Reduce { op, identity } => {
                    let operation = self.node(
                        &format!("reduce {op}\nidentity: {}", endpoint_label(identity)),
                        "    ",
                    );
                    self.edges(&current, &operation, None, "    ");
                    let identity = self.emit_endpoint(identity, env)?;
                    self.edges(&identity, &operation, Some("identity"), "    ");
                    current = vec![operation];
                }
                Stage::Scan { op, identity } => {
                    let operation = self.node(
                        &format!("scan {op}\nidentity: {}", endpoint_label(identity)),
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
                    for arm in arms {
                        if let MatchGuard::Call { args, .. } = &arm.guard {
                            for arg in args {
                                let arg_nodes = self.emit_endpoint(arg, env)?;
                                self.edges(&arg_nodes, &operation, Some("guard arg"), "    ");
                            }
                        }
                        let branch = self.node(&match_target_label(&arm.target), "    ");
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
        current: &[String],
        env: &mut HashMap<String, Vec<String>>,
    ) -> Result<(), String> {
        let variable = self.variable_node(&format!("${name}"), "    ");
        self.edges(current, &variable, None, "    ");
        if env.insert(name.to_string(), vec![variable]).is_some() {
            return Err(format!("value `{name}` is bound more than once"));
        }
        Ok(())
    }

    fn emit_source_endpoint(
        &mut self,
        endpoint: &Endpoint,
        env: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<String>, String> {
        match endpoint {
            Endpoint::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown value `{name}`")),
            Endpoint::Name(name) => Err(format!("expected value, found node `{name}`")),
            Endpoint::Tuple(items) | Endpoint::Seq(items) => {
                let source =
                    self.input_node(&format!("input\n{}", endpoint_label(endpoint)), "    ");
                let dependencies = self.emit_endpoint_items(items, env)?;
                self.edges(&dependencies, &source, None, "    ");
                Ok(vec![source])
            }
            Endpoint::Unit => Ok(Vec::new()),
            _ => Ok(vec![self.input_node(&endpoint_label(endpoint), "    ")]),
        }
    }

    fn emit_endpoint(
        &mut self,
        endpoint: &Endpoint,
        env: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<String>, String> {
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
            _ => Ok(Vec::new()),
        }
    }

    fn emit_endpoint_items(
        &mut self,
        items: &[Endpoint],
        env: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<String>, String> {
        let mut sources = Vec::new();
        for item in items {
            sources.extend(self.emit_endpoint(item, env)?);
        }
        Ok(sources)
    }

    fn node(&mut self, label: &str, indent: &str) -> String {
        let id = format!("n{}", self.next_id);
        self.next_id += 1;
        self.line(&format!("{indent}{id}[\"{}\"]", escape_label(label)));
        id
    }

    fn input_node(&mut self, label: &str, indent: &str) -> String {
        let id = format!("n{}", self.next_id);
        self.next_id += 1;
        self.line(&format!("{indent}{id}([\"{}\"])", escape_label(label)));
        id
    }

    fn variable_node(&mut self, label: &str, indent: &str) -> String {
        self.input_node(label, indent)
    }

    fn decision_node(&mut self, label: &str, indent: &str) -> String {
        let id = format!("n{}", self.next_id);
        self.next_id += 1;
        self.line(&format!("{indent}{id}{{\"{}\"}}", escape_label(label)));
        id
    }

    fn edge(&mut self, from: &str, to: &str, label: Option<&str>, indent: &str) {
        match label {
            Some(label) => self.line(&format!(
                "{indent}{from} -- \"{}\" --> {to}",
                escape_label(label)
            )),
            None => self.line(&format!("{indent}{from} --> {to}")),
        }
    }

    fn edges(&mut self, from: &[String], to: &str, label: Option<&str>, indent: &str) {
        for source in from {
            self.edge(source, to, label, indent);
        }
    }

    fn line(&mut self, line: &str) {
        self.out.push_str(line);
        self.out.push('\n');
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
