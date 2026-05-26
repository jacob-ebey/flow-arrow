use crate::ast::*;
use crate::parser;
use std::fs;
use std::path::Path;

const INDENT: &str = "    ";
const MULTILINE_IMPORT_ITEM_LIMIT: usize = 4;
const MAX_INLINE_IMPORT_WIDTH: usize = 100;
const MAX_INLINE_STRUCT_WIDTH: usize = 100;

pub fn format_source(source: &str) -> Result<String, String> {
    let module = parser::parse(source)?;
    let comments = CommentLayout::collect(source, &module);
    Ok(Formatter {
        comments,
        output: String::new(),
    }
    .format_module(&module))
}

pub fn format_file(path: &Path) -> Result<bool, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let formatted = format_source(&source)?;
    if formatted == source {
        return Ok(false);
    }
    fs::write(path, formatted)
        .map_err(|error| format!("failed to write `{}`: {error}", path.display()))?;
    Ok(true)
}

pub fn check_file(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let formatted = format_source(&source)?;
    if formatted == source {
        Ok(())
    } else {
        Err(format!("`{}` is not formatted", path.display()))
    }
}

#[derive(Debug, Clone, Default)]
struct CommentLayout {
    decls: Vec<DeclComments>,
}

#[derive(Debug, Clone, Default)]
struct DeclComments {
    leading: Vec<String>,
    chains: Vec<ChainComments>,
}

#[derive(Debug, Clone, Default)]
struct ChainComments {
    leading_blank: bool,
    leading: Vec<String>,
    trailing: Option<String>,
}

impl CommentLayout {
    fn collect(source: &str, module: &Module) -> Self {
        let mut layout = CommentLayout {
            decls: module
                .declarations
                .iter()
                .map(|decl| DeclComments {
                    leading: Vec::new(),
                    chains: match decl {
                        Decl::Node(callable) | Decl::Program(callable) => {
                            vec![ChainComments::default(); callable.chains.len()]
                        }
                        Decl::TypeAlias(_)
                        | Decl::Struct(_)
                        | Decl::Import(_)
                        | Decl::Foreign(_) => Vec::new(),
                    },
                })
                .collect(),
        };

        let mut pending_top = Vec::new();
        let mut pending_body = Vec::new();
        let mut pending_body_blank = false;
        let mut decl_index = 0usize;
        let mut current_callable = None::<usize>;
        let mut current_chain = 0usize;
        let mut brace_depth = 0i32;
        let mut paren_depth = 0i32;
        let mut bracket_depth = 0i32;
        let mut block_comment = None::<Vec<String>>;

        for line in source.lines() {
            if let Some(lines) = &mut block_comment {
                lines.push(line.trim_end().to_string());
                if line.contains("*/") {
                    let lines = block_comment.take().unwrap_or_default();
                    if current_callable.is_some() {
                        pending_body.extend(lines);
                    } else {
                        pending_top.extend(lines);
                    }
                }
                continue;
            }

            let trimmed = line.trim_start();
            if trimmed.starts_with("/*") {
                let comment = line.trim_end().trim_start().to_string();
                if trimmed.contains("*/") {
                    if current_callable.is_some() {
                        pending_body.push(comment);
                    } else {
                        pending_top.push(comment);
                    }
                } else {
                    block_comment = Some(vec![comment]);
                }
                continue;
            }

            let (code, line_comment) = split_line_comment(line);
            let code_trimmed = code.trim();
            if code_trimmed.is_empty() {
                if let Some(comment) = line_comment {
                    if current_callable.is_some() {
                        pending_body.push(comment);
                    } else {
                        pending_top.push(comment);
                    }
                } else if current_callable.is_some() {
                    pending_body_blank = true;
                }
                continue;
            }

            let starts_top_decl = brace_depth == 0 && starts_declaration(code_trimmed);
            if starts_top_decl && decl_index < layout.decls.len() {
                layout.decls[decl_index].leading.append(&mut pending_top);
                if starts_callable(code_trimmed) {
                    current_callable = Some(decl_index);
                    current_chain = 0;
                }
                decl_index += 1;
            }

            let starts_chain = current_callable.is_some()
                && brace_depth == 1
                && paren_depth == 0
                && bracket_depth == 0
                && !code_trimmed.starts_with("->")
                && !code_trimmed.starts_with('}');
            if starts_chain {
                if let Some(decl) = current_callable
                    && let Some(chain_comments) = layout
                        .decls
                        .get_mut(decl)
                        .and_then(|decl| decl.chains.get_mut(current_chain))
                {
                    chain_comments.leading_blank = pending_body_blank;
                    chain_comments.leading.append(&mut pending_body);
                    chain_comments.trailing = line_comment;
                }
                pending_body_blank = false;
                current_chain += 1;
            }

            update_depths(code, &mut brace_depth, &mut paren_depth, &mut bracket_depth);
            if current_callable.is_some() && brace_depth == 0 {
                current_callable = None;
                pending_body.clear();
                pending_body_blank = false;
            }
        }

        if decl_index < layout.decls.len() {
            layout.decls[decl_index].leading.append(&mut pending_top);
        }

        layout
    }
}

struct Formatter {
    comments: CommentLayout,
    output: String,
}

impl Formatter {
    fn format_module(mut self, module: &Module) -> String {
        for (index, decl) in module.declarations.iter().enumerate() {
            if index > 0 && needs_blank_between(&module.declarations[index - 1], decl) {
                self.blank_line();
            }
            if let Some(comments) = self.comments.decls.get(index).cloned() {
                self.write_comments("", &comments.leading);
            }
            self.format_decl(index, decl);
        }
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        self.output
    }

    fn format_decl(&mut self, decl_index: usize, decl: &Decl) {
        match decl {
            Decl::TypeAlias(alias) => {
                self.line(format!(
                    "type {} = {}",
                    alias.name,
                    format_type_name(&alias.ty)
                ));
            }
            Decl::Struct(struct_decl) => self.format_struct(struct_decl),
            Decl::Import(import) => self.format_import(import),
            Decl::Foreign(foreign) => self.format_foreign(foreign),
            Decl::Node(callable) => {
                let kind = if callable.is_extern {
                    "extern node"
                } else {
                    "node"
                };
                self.format_callable(decl_index, kind, callable);
            }
            Decl::Program(callable) => self.format_callable(decl_index, "program", callable),
        }
    }

    fn format_foreign(&mut self, foreign: &ForeignBlock) {
        let target = match foreign.target {
            ForeignTarget::Js => "js",
            ForeignTarget::C => "c",
        };
        let source = match &foreign.source {
            ForeignSource::Module(specifier) => format!("module {}", format_string(specifier)),
            ForeignSource::Global(name) => format!("global {}", format_string(name)),
            ForeignSource::CHeader { header, source } => {
                let mut out = format!("header {}", format_string(header));
                if let Some(source) = source {
                    out.push_str(" source ");
                    out.push_str(&format_string(source));
                }
                out
            }
        };
        self.line(format!("foreign {target} {source} {{"));
        for node in &foreign.nodes {
            let effect = match node.effect {
                ForeignEffect::Pure => "pure",
                ForeignEffect::Io => "io",
            };
            let inputs = node
                .inputs
                .iter()
                .map(format_port)
                .collect::<Vec<_>>()
                .join(", ");
            self.line(format!(
                "{INDENT}{effect} node {}({inputs}) -> {} = {}",
                node.name,
                format_port_or_list(&node.outputs),
                node.symbol
            ));
        }
        self.line("}");
    }

    fn format_struct(&mut self, struct_decl: &StructDecl) {
        self.line(format!("struct {} {{", struct_decl.name));
        for field in &struct_decl.fields {
            self.line(format!("{INDENT}{},", format_port(field)));
        }
        self.line("}");
    }

    fn format_import(&mut self, import: &Import) {
        let source = match &import.source {
            ImportSource::Module(name) => name.clone(),
            ImportSource::Local(path) => format_string(path),
        };
        match &import.clause {
            ImportClause::Alias(alias) => self.line(format!("import {source} as {alias}")),
            ImportClause::Items(items) => {
                let inline = format!(
                    "import {source} {{ {} }}",
                    items
                        .iter()
                        .map(format_import_item)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                if items.len() <= MULTILINE_IMPORT_ITEM_LIMIT
                    && inline.chars().count() <= MAX_INLINE_IMPORT_WIDTH
                {
                    self.line(inline);
                } else {
                    self.line(format!("import {source} {{"));
                    for item in items {
                        self.line(format!("{INDENT}{},", format_import_item(item)));
                    }
                    self.line("}");
                }
            }
        }
    }

    fn format_callable(&mut self, decl_index: usize, kind: &str, callable: &Callable) {
        let node_params = format_node_params(&callable.node_params);
        let inputs = callable
            .inputs
            .iter()
            .map(format_port)
            .collect::<Vec<_>>()
            .join(", ");
        self.line(format!(
            "{kind} {}{node_params}({inputs}) -> {} {{",
            callable.name,
            format_port_or_list(&callable.outputs)
        ));
        let mut group = Vec::new();
        for (index, chain) in callable.chains.iter().enumerate() {
            let comments = self
                .comments
                .decls
                .get(decl_index)
                .and_then(|decl| decl.chains.get(index))
                .cloned()
                .unwrap_or_default();
            if comments.leading_blank || !comments.leading.is_empty() {
                self.flush_chain_group(&mut group);
                if comments.leading_blank && index > 0 {
                    self.blank_line();
                }
                self.write_comments(INDENT, &comments.leading);
            }
            if chain
                .stages
                .iter()
                .any(|stage| matches!(stage, Stage::Match { .. }))
                || should_format_endpoint_multiline(&chain.source)
            {
                self.flush_chain_group(&mut group);
                self.format_chain_multiline(chain, comments.trailing);
                continue;
            }
            group.push(FormattedChain {
                parts: format_chain_parts(chain),
                trailing: comments.trailing,
            });
        }
        self.flush_chain_group(&mut group);
        self.line("}");
    }

    fn flush_chain_group(&mut self, group: &mut Vec<FormattedChain>) {
        if group.is_empty() {
            return;
        }

        let widths = chain_group_widths(group);
        for chain in group.drain(..) {
            let mut line = format!("{INDENT}{}", format_aligned_chain(&chain.parts, &widths));
            if let Some(comment) = chain.trailing {
                line.push_str("  ");
                line.push_str(&comment);
            }
            self.line(line);
        }
    }

    fn format_chain_multiline(&mut self, chain: &Chain, trailing: Option<String>) {
        if should_format_endpoint_multiline(&chain.source) {
            self.format_chain_with_multiline_source(chain, trailing);
            return;
        }

        let mut first = format!("{INDENT}{}", format_endpoint(&chain.source));
        if let Some(comment) = trailing {
            first.push_str("  ");
            first.push_str(&comment);
        }
        self.line(first);
        for stage in &chain.stages {
            match stage {
                Stage::Match { arms } => {
                    self.line(format!("{INDENT}-> match {{"));
                    for arm in arms {
                        self.line(format!("{INDENT}{INDENT}{}", format_match_arm(arm)));
                    }
                    self.line(format!("{INDENT}}}"));
                }
                other => self.line(format!("{INDENT}-> {}", format_stage(other))),
            }
        }
    }

    fn format_chain_with_multiline_source(&mut self, chain: &Chain, trailing: Option<String>) {
        let first_multiline_stage = chain
            .stages
            .iter()
            .position(|stage| matches!(stage, Stage::Match { .. }))
            .unwrap_or(chain.stages.len());
        let inline_stages = &chain.stages[..first_multiline_stage];
        let remaining_stages = &chain.stages[first_multiline_stage..];
        let mut source_lines = format_multiline_endpoint(&chain.source, INDENT);
        let mut last = source_lines
            .pop()
            .unwrap_or_else(|| format!("{INDENT}{}", format_endpoint(&chain.source)));
        if !inline_stages.is_empty() {
            last.push_str(" -> ");
            last.push_str(
                &inline_stages
                    .iter()
                    .map(format_stage)
                    .collect::<Vec<_>>()
                    .join(" -> "),
            );
        }
        if let Some(comment) = trailing {
            last.push_str("  ");
            last.push_str(&comment);
        }
        for line in source_lines {
            self.line(line);
        }
        self.line(last);
        for stage in remaining_stages {
            match stage {
                Stage::Match { arms } => {
                    self.line(format!("{INDENT}-> match {{"));
                    for arm in arms {
                        self.line(format!("{INDENT}{INDENT}{}", format_match_arm(arm)));
                    }
                    self.line(format!("{INDENT}}}"));
                }
                other => self.line(format!("{INDENT}-> {}", format_stage(other))),
            }
        }
    }

    fn write_comments(&mut self, indent: &str, comments: &[String]) {
        for comment in comments {
            self.line(format!("{indent}{}", comment.trim_start()));
        }
    }

    fn line(&mut self, text: impl AsRef<str>) {
        self.output.push_str(text.as_ref());
        self.output.push('\n');
    }

    fn blank_line(&mut self) {
        if !self.output.ends_with("\n\n") {
            self.output.push('\n');
        }
    }
}

struct FormattedChain {
    parts: Vec<String>,
    trailing: Option<String>,
}

fn format_import_item(item: &ImportItem) -> String {
    match &item.alias {
        Some(alias) => format!("{} as {alias}", item.name),
        None => item.name.clone(),
    }
}

fn needs_blank_between(left: &Decl, right: &Decl) -> bool {
    !matches!(
        (left, right),
        (Decl::Import(_), Decl::Import(_))
            | (Decl::TypeAlias(_), Decl::TypeAlias(_))
            | (Decl::Struct(_), Decl::Struct(_))
    )
}

fn format_callable_port_list(ports: &[Port]) -> String {
    ports.iter().map(format_port).collect::<Vec<_>>().join(", ")
}

fn format_node_params(params: &[NodeParam]) -> String {
    if params.is_empty() {
        return String::new();
    }
    format!(
        "<{}>",
        params
            .iter()
            .map(|param| format!(
                "{}: node({}) -> {}",
                param.name,
                format_type_name(&param.input),
                format_type_name(&param.output)
            ))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn format_port_or_list(ports: &[Port]) -> String {
    match ports {
        [port] => format_port(port),
        ports => format!("({})", format_callable_port_list(ports)),
    }
}

fn format_port(port: &Port) -> String {
    format!("{}: {}", port.name, format_type_name(&port.ty))
}

fn format_type_name(ty: &str) -> String {
    let mut output = String::new();
    let mut chars = ty.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ',' => {
                output.push(',');
                output.push(' ');
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            '|' => {
                trim_trailing_spaces(&mut output);
                output.push_str(" | ");
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
            }
            _ => output.push(ch),
        }
    }
    trim_trailing_spaces(&mut output);
    output
}

fn format_chain_parts(chain: &Chain) -> Vec<String> {
    let mut parts = Vec::with_capacity(chain.stages.len() + 1);
    parts.push(format_endpoint(&chain.source));
    parts.extend(chain.stages.iter().map(format_stage));
    parts
}

fn chain_group_widths(group: &[FormattedChain]) -> Vec<usize> {
    let max_arrow_count = group
        .iter()
        .map(|chain| chain.parts.len().saturating_sub(1))
        .max()
        .unwrap_or(0);
    let mut widths = vec![0; max_arrow_count];
    for chain in group {
        for (index, part) in chain.parts.iter().take(chain.parts.len() - 1).enumerate() {
            widths[index] = widths[index].max(part.chars().count());
        }
    }
    widths
}

fn format_aligned_chain(parts: &[String], widths: &[usize]) -> String {
    let mut output = String::new();
    for (index, part) in parts.iter().enumerate() {
        output.push_str(part);
        if index < parts.len() - 1 {
            for _ in 0..widths[index].saturating_sub(part.chars().count()) {
                output.push(' ');
            }
            output.push_str(" -> ");
        }
    }
    output
}

fn format_stage(stage: &Stage) -> String {
    match stage {
        Stage::Endpoint(endpoint) => format_endpoint(endpoint),
        Stage::Bind(target) => format_binding_target(target),
        Stage::Map(name) => format!("map {name}"),
        Stage::FaultMap { node, ok, fault } => {
            format!("fault map {node} {{ ok -> ${ok}, fault -> ${fault} }}")
        }
        Stage::Filter(name) => format!("filter {name}"),
        Stage::Field(name) => format!("field {name}"),
        Stage::Repeat { count, node } => format!("repeat<{}> {node}", format_endpoint(count)),
        Stage::Reduce { op, identity } => {
            format!("reduce {op}(identity: {})", format_endpoint(identity))
        }
        Stage::Scan { op, identity } => {
            format!("scan {op}(identity: {})", format_endpoint(identity))
        }
        Stage::Match { arms } => format!(
            "match {{ {} }}",
            arms.iter()
                .map(format_match_arm)
                .collect::<Vec<_>>()
                .join(" ")
        ),
    }
}

fn format_binding_target(target: &BindingTarget) -> String {
    match target {
        BindingTarget::Discard => "$".to_string(),
        BindingTarget::Variable(name) => format!("${name}"),
        BindingTarget::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(format_binding_target)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn format_match_arm(arm: &MatchArm) -> String {
    let guard = match &arm.guard {
        MatchGuard::Fallback => "_".to_string(),
        MatchGuard::Call { node, args } => format!(
            "{}({})",
            node,
            args.iter()
                .map(format_endpoint)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    format!("{guard} -> {}", format_match_target(&arm.target))
}

fn format_match_target(target: &MatchTarget) -> String {
    match target {
        MatchTarget::Node(node) => node.clone(),
        MatchTarget::Value(endpoint) => format_endpoint(endpoint),
    }
}

fn format_endpoint(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Variable(name) => format!("${name}"),
        Endpoint::Name(name) => name.clone(),
        Endpoint::Int(value) => value.to_string(),
        Endpoint::Real(value) => format_real(*value),
        Endpoint::Bool(value) => value.to_string(),
        Endpoint::String(value) => format_string(value),
        Endpoint::Unit => "()".to_string(),
        Endpoint::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(format_endpoint)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Endpoint::Seq(items) => format!(
            "[{}]",
            items
                .iter()
                .map(format_endpoint)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Endpoint::Struct { name, fields } => format!(
            "{name} {{ {} }}",
            fields
                .iter()
                .map(|(field, value)| format!("{field}: {}", format_endpoint(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Endpoint::Eval { source, stages } => {
            let mut parts = Vec::with_capacity(stages.len() + 1);
            parts.push(format_endpoint(source));
            parts.extend(stages.iter().map(format_stage));
            parts.join(" -> ")
        }
    }
}

fn should_format_endpoint_multiline(endpoint: &Endpoint) -> bool {
    matches!(endpoint, Endpoint::Struct { .. })
        && format_endpoint(endpoint).chars().count() > MAX_INLINE_STRUCT_WIDTH
}

fn format_multiline_endpoint(endpoint: &Endpoint, indent: &str) -> Vec<String> {
    match endpoint {
        Endpoint::Struct { name, fields } => {
            let mut lines = vec![format!("{indent}{name} {{")];
            for (index, (field, value)) in fields.iter().enumerate() {
                let comma = if index + 1 == fields.len() { "" } else { "," };
                lines.push(format!(
                    "{indent}{INDENT}{field}: {}{comma}",
                    format_endpoint(value)
                ));
            }
            lines.push(format!("{indent}}}"));
            lines
        }
        _ => vec![format!("{indent}{}", format_endpoint(endpoint))],
    }
}

fn format_real(value: f64) -> String {
    let mut text = value.to_string();
    if text.contains('e') || text.contains('E') {
        text = format!("{value:.15}");
        while text.contains('.') && text.ends_with('0') {
            text.pop();
        }
    }
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

fn format_string(value: &str) -> String {
    let mut output = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\t' => output.push_str("\\t"),
            '\r' => output.push_str("\\r"),
            other => output.push(other),
        }
    }
    output.push('"');
    output
}

fn split_line_comment(line: &str) -> (&str, Option<String>) {
    let mut escaped = false;
    let mut in_string = false;
    for (index, ch) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
        } else if ch == '#' {
            let code = &line[..index];
            let comment = line[index..].trim_end().to_string();
            return (code, Some(comment));
        }
    }
    (line, None)
}

fn update_depths(
    code: &str,
    brace_depth: &mut i32,
    paren_depth: &mut i32,
    bracket_depth: &mut i32,
) {
    let mut escaped = false;
    let mut in_string = false;
    for ch in code.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => *brace_depth += 1,
            '}' => *brace_depth -= 1,
            '(' => *paren_depth += 1,
            ')' => *paren_depth -= 1,
            '[' => *bracket_depth += 1,
            ']' => *bracket_depth -= 1,
            _ => {}
        }
    }
}

fn starts_declaration(text: &str) -> bool {
    starts_keyword(text, "import")
        || starts_keyword(text, "type")
        || starts_keyword(text, "struct")
        || starts_keyword(text, "foreign")
        || starts_keyword(text, "extern")
        || starts_keyword(text, "node")
        || starts_keyword(text, "program")
}

fn starts_callable(text: &str) -> bool {
    starts_keyword(text, "extern")
        || starts_keyword(text, "node")
        || starts_keyword(text, "program")
}

fn starts_keyword(text: &str, keyword: &str) -> bool {
    text == keyword
        || text
            .strip_prefix(keyword)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|ch| ch.is_whitespace())
}

fn trim_trailing_spaces(text: &mut String) {
    while text.ends_with(' ') {
        text.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_formats(input: &str, expected: &str) {
        let formatted = format_source(input).expect("format");
        assert_eq!(formatted, expected);
        assert_eq!(
            format_source(&formatted).expect("format twice"),
            formatted,
            "formatting should be idempotent"
        );
        assert_eq!(
            parser::parse(input).expect("parse input"),
            parser::parse(&formatted).expect("parse formatted"),
            "formatting should preserve the parsed program"
        );
    }

    #[test]
    fn formats_dense_program_spacing() {
        assert_formats(
            r#"import   std.cli{Args}
type Pair=(i64,f64)
program   main( args:Args)->exit_code:i64{(1,2)->add->$sum
$sum-> $exit_code}"#,
            r#"import std.cli { Args }

type Pair = (i64, f64)

program main(args: Args) -> exit_code: i64 {
    (1, 2) -> add -> $sum
    $sum   -> $exit_code
}
"#,
        );
    }

    #[test]
    fn formats_alias_and_multiline_imports() {
        assert_formats(
            r#"import std.math{add as plus,sub,mul,div,eq}
import "./helpers.flow" as helper
program main(args:Args)->exit_code:i64{0->$exit_code}"#,
            r#"import std.math {
    add as plus,
    sub,
    mul,
    div,
    eq,
}
import "./helpers.flow" as helper

program main(args: Args) -> exit_code: i64 {
    0 -> $exit_code
}
"#,
        );
    }

    #[test]
    fn formats_callable_ports_and_type_whitespace() {
        assert_formats(
            r#"node split(input:(Seq[f64],Seq[f64]),value:i64|f64)->(left:Seq[f64],right:Faultable[i64]){$input->first->$left
$value->wrap->$right}"#,
            r#"node split(input: (Seq[f64], Seq[f64]), value: i64 | f64) -> (left: Seq[f64], right: Faultable[i64]) {
    $input -> first -> $left
    $value -> wrap  -> $right
}
"#,
        );
    }

    #[test]
    fn formats_extern_node_declarations() {
        assert_formats(
            r#"extern node expose(value:i64)->out:i64{$value->$out}"#,
            r#"extern node expose(value: i64) -> out: i64 {
    $value -> $out
}
"#,
        );
    }

    #[test]
    fn formats_tuple_binding_targets() {
        assert_formats(
            r#"program main(args:Args)->exit_code:i64{(1,2)->pair->($left,$right)
$left->$exit_code}"#,
            r#"program main(args: Args) -> exit_code: i64 {
    (1, 2) -> pair -> ($left, $right)
    $left  -> $exit_code
}
"#,
        );
    }

    #[test]
    fn formats_long_struct_literals_on_multiple_lines() {
        assert_formats(
            r#"struct JobSummary{total_score:i64,peak_score:i64,total_weight:i64,peak_weight:i64}
extern node score_batch(width:i64)->summary:JobSummary{JobSummary{total_score:$total_score,peak_score:$peak_score,total_weight:$total_weight,peak_weight:$peak_weight}->$summary}"#,
            r#"struct JobSummary {
    total_score: i64,
    peak_score: i64,
    total_weight: i64,
    peak_weight: i64,
}

extern node score_batch(width: i64) -> summary: JobSummary {
    JobSummary {
        total_score: $total_score,
        peak_score: $peak_score,
        total_weight: $total_weight,
        peak_weight: $peak_weight
    } -> $summary
}
"#,
        );
    }

    #[test]
    fn formats_literals_without_changing_real_kinds() {
        assert_formats(
            "program main(args: Args) -> exit_code: i64 {\n    [0.0,\"a\\n\\\"b\",true,false,()] -> sink -> $exit_code\n}\n",
            "program main(args: Args) -> exit_code: i64 {\n    [0.0, \"a\\n\\\"b\", true, false, ()] -> sink -> $exit_code\n}\n",
        );
    }

    #[test]
    fn formats_combinator_stages() {
        assert_formats(
            r#"program main(args: Args) -> exit_code: i64 {
["1","bad"]->fault map parse_real{ok->$numbers,fault->$faults}
$numbers->filter positive->map abs->reduce add(identity:0.0)->$total
$total->scan add(identity: 0.0)->repeat<$total> emit->$exit_code
}"#,
            r#"program main(args: Args) -> exit_code: i64 {
    ["1", "bad"] -> fault map parse_real { ok -> $numbers, fault -> $faults }
    $numbers     -> filter positive         -> map abs             -> reduce add(identity: 0.0) -> $total
    $total       -> scan add(identity: 0.0) -> repeat<$total> emit -> $exit_code
}
"#,
        );
    }

    #[test]
    fn formats_match_stage_as_multiline_block() {
        assert_formats(
            r#"program main(args:Args)->exit_code:i64{0->match{eq(0)->zero _->one}->$exit_code}
node zero(x:i64)->y:i64{0->$y}
node one(x:i64)->y:i64{1->$y}"#,
            r#"program main(args: Args) -> exit_code: i64 {
    0
    -> match {
        eq(0) -> zero
        _ -> one
    }
    -> $exit_code
}

node zero(x: i64) -> y: i64 {
    0 -> $y
}

node one(x: i64) -> y: i64 {
    1 -> $y
}
"#,
        );
    }

    #[test]
    fn formats_match_inline_value_targets() {
        assert_formats(
            r#"program main(args:Args)->exit_code:i64{0->match{eq(0)->0 _->1}->$exit_code}"#,
            r#"program main(args: Args) -> exit_code: i64 {
    0
    -> match {
        eq(0) -> 0
        _ -> 1
    }
    -> $exit_code
}
"#,
        );
    }

    #[test]
    fn preserves_standalone_and_trailing_line_comments() {
        assert_formats(
            r#"# module docs
import std.cli { Args }

# entry docs
program main(args: Args) -> exit_code: i64 {
    # bind status
    0 -> $exit_code # status code
}
"#,
            r#"# module docs
import std.cli { Args }

# entry docs
program main(args: Args) -> exit_code: i64 {
    # bind status
    0 -> $exit_code  # status code
}
"#,
        );
    }

    #[test]
    fn keeps_comments_before_multiline_chain_continuations_on_the_next_chain() {
        assert_formats(
            r#"program main(args: Args) -> exit_code: i64 {
    # first line
    [$a,
     $b]
        -> concat -> $joined

    # second line
    ["done",
     "\n"]
        -> concat -> $exit_code
}
"#,
            r#"program main(args: Args) -> exit_code: i64 {
    # first line
    [$a, $b] -> concat -> $joined

    # second line
    ["done", "\n"] -> concat -> $exit_code
}
"#,
        );
    }

    #[test]
    fn blank_lines_reset_chain_alignment_groups() {
        assert_formats(
            r#"program main(args: Args) -> exit_code: i64 {
$a->first_long->$x
$bb->f->$y

$c->g->$z
$longer->h->$exit_code
}
"#,
            r#"program main(args: Args) -> exit_code: i64 {
    $a  -> first_long -> $x
    $bb -> f          -> $y

    $c      -> g -> $z
    $longer -> h -> $exit_code
}
"#,
        );
    }

    #[test]
    fn preserves_hashes_inside_strings() {
        assert_formats(
            r##"program main(args: Args) -> exit_code: i64 {
    "#not-comment" -> echo -> $exit_code # comment
}
"##,
            r##"program main(args: Args) -> exit_code: i64 {
    "#not-comment" -> echo -> $exit_code  # comment
}
"##,
        );
    }

    #[test]
    fn preserves_standalone_block_comments() {
        assert_formats(
            r#"/* module */
program main(args: Args) -> exit_code: i64 {
    /*
     * body
     */
    0 -> $exit_code
}
"#,
            r#"/* module */
program main(args: Args) -> exit_code: i64 {
    /*
    * body
    */
    0 -> $exit_code
}
"#,
        );
    }

    #[test]
    fn wraps_long_imports_even_when_item_count_is_small() {
        assert_formats(
            r#"import std.very_long_module_name { incredibly_long_imported_name, another_extremely_long_imported_name }
program main(args: Args) -> exit_code: i64 { 0 -> $exit_code }"#,
            r#"import std.very_long_module_name {
    incredibly_long_imported_name,
    another_extremely_long_imported_name,
}

program main(args: Args) -> exit_code: i64 {
    0 -> $exit_code
}
"#,
        );
    }

    #[test]
    fn formats_checked_in_flow_sources_without_changing_the_ast() {
        for (name, source) in [
            (
                "examples/99-bottles/main.flow",
                include_str!("../examples/99-bottles/main.flow"),
            ),
            (
                "examples/add-numbers-from-args/main.flow",
                include_str!("../examples/add-numbers-from-args/main.flow"),
            ),
            (
                "examples/add-numbers-from-stdin/main.flow",
                include_str!("../examples/add-numbers-from-stdin/main.flow"),
            ),
            (
                "examples/concurrency/main.flow",
                include_str!("../examples/concurrency/main.flow"),
            ),
            (
                "examples/fibonacci/main.flow",
                include_str!("../examples/fibonacci/main.flow"),
            ),
            (
                "examples/grayscale-image/main.flow",
                include_str!("../examples/grayscale-image/main.flow"),
            ),
            (
                "examples/json-parser/main.flow",
                include_str!("../examples/json-parser/main.flow"),
            ),
            (
                "examples/json/main.flow",
                include_str!("../examples/json/main.flow"),
            ),
            (
                "examples/parse-and-sum-lines/main.flow",
                include_str!("../examples/parse-and-sum-lines/main.flow"),
            ),
            (
                "src/stdlib/source/cv.flow",
                include_str!("stdlib/source/cv.flow"),
            ),
            (
                "src/stdlib/source/matrix.flow",
                include_str!("stdlib/source/matrix.flow"),
            ),
            (
                "src/stdlib/source/vector.flow",
                include_str!("stdlib/source/vector.flow"),
            ),
        ] {
            let formatted = format_source(source).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                parser::parse(source).unwrap_or_else(|error| panic!("{name}: {error}")),
                parser::parse(&formatted).unwrap_or_else(|error| panic!("{name}: {error}")),
                "{name}"
            );
            assert_eq!(
                format_source(&formatted).unwrap_or_else(|error| panic!("{name}: {error}")),
                formatted,
                "{name}"
            );
        }
    }
}
