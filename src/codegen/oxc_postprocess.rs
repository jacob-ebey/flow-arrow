use oxc::allocator::Allocator;
use oxc::ast::ast::Program;
use oxc::codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc::diagnostics::OxcDiagnostic;
use oxc::isolated_declarations::{IsolatedDeclarations, IsolatedDeclarationsOptions};
use oxc::minifier::{CompressOptions, Compressor};
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};
use std::path::Path;

const INTERNAL_TYPE_ALIASES: &[&str] = &[
    "FaArgs",
    "FaFault",
    "FaFaultable",
    "FaStream",
    "FaHttpServerConfig",
    "FaHttpListener",
    "FaHttpRequest",
    "FaHttpResponse",
    "FaSqliteConnection",
    "FaSqliteRow",
    "FaSqliteValue",
];

#[derive(Debug, Clone)]
pub struct JavaScriptArtifacts {
    pub javascript: String,
    pub declarations: String,
}

pub fn emit_typescript(source: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let mut program = parse_typescript(&allocator, source)?;
    run_dce(&allocator, &mut program);
    Ok(strip_unused_internal_type_aliases(print_program(
        &program,
        SourceType::ts(),
    )))
}

pub fn emit_javascript_artifacts(source: &str) -> Result<JavaScriptArtifacts, String> {
    let allocator = Allocator::default();
    let mut program = parse_typescript(&allocator, source)?;
    run_dce(&allocator, &mut program);
    let declarations = emit_declarations(&allocator, &program, source)?;
    transform_typescript(&allocator, &mut program, source)?;
    let javascript = print_program(&program, SourceType::mjs());
    Ok(JavaScriptArtifacts {
        javascript,
        declarations,
    })
}

fn parse_typescript<'a>(allocator: &'a Allocator, source: &'a str) -> Result<Program<'a>, String> {
    let ret = Parser::new(allocator, source, SourceType::ts()).parse();
    if ret.errors.is_empty() {
        Ok(ret.program)
    } else {
        Err(format_oxc_errors(
            "OXC failed to parse generated TypeScript",
            source,
            ret.errors,
        ))
    }
}

fn run_dce<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
    Compressor::new(allocator).dead_code_elimination(program, CompressOptions::dce());
}

fn transform_typescript<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    source: &str,
) -> Result<(), String> {
    let ret = SemanticBuilder::new()
        .with_excess_capacity(2.0)
        .with_enum_eval(true)
        .build(program);
    if !ret.errors.is_empty() {
        return Err(format_oxc_errors(
            "OXC failed to analyze generated TypeScript",
            source,
            ret.errors,
        ));
    }

    let transform = Transformer::new(
        allocator,
        Path::new("flowarrow-generated.ts"),
        &TransformOptions::default(),
    )
    .build_with_scoping(ret.semantic.into_scoping(), program);
    if !transform.errors.is_empty() {
        return Err(format_oxc_errors(
            "OXC failed to transpile generated TypeScript",
            source,
            transform.errors,
        ));
    }
    Ok(())
}

fn emit_declarations<'a>(
    allocator: &'a Allocator,
    program: &Program<'a>,
    source: &str,
) -> Result<String, String> {
    let ret =
        IsolatedDeclarations::new(allocator, IsolatedDeclarationsOptions::default()).build(program);
    if !ret.errors.is_empty() {
        return Err(format_oxc_errors(
            "OXC failed to emit TypeScript declarations",
            source,
            ret.errors,
        ));
    }
    Ok(strip_unused_internal_type_aliases(print_program(
        &ret.program,
        SourceType::d_ts(),
    )))
}

fn print_program(program: &Program<'_>, source_type: SourceType) -> String {
    Codegen::new()
        .with_source_type(source_type)
        .with_options(CodegenOptions {
            comments: CommentOptions::disabled(),
            ..CodegenOptions::default()
        })
        .build(program)
        .code
}

fn format_oxc_errors(context: &str, source: &str, errors: Vec<OxcDiagnostic>) -> String {
    let mut out = context.to_string();
    for error in errors {
        out.push_str("\n");
        out.push_str(&format!("{:?}", error.with_source_code(source.to_string())));
    }
    out
}

fn strip_unused_internal_type_aliases(mut source: String) -> String {
    // OXC DCE is runtime-focused and does not prune type-only aliases.
    loop {
        let mut changed = false;
        for name in INTERNAL_TYPE_ALIASES {
            let Some((start, end)) = find_top_level_type_alias(&source, name) else {
                continue;
            };
            let without_alias = format!("{}{}", &source[..start], &source[end..]);
            if !contains_identifier(&without_alias, name) {
                source = without_alias;
                changed = true;
                break;
            }
        }
        if !changed {
            return source;
        }
    }
}

fn find_top_level_type_alias(source: &str, name: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;
    let prefix = format!("type {name}");
    while let Some(relative_start) = source[search_from..].find(&prefix) {
        let start = search_from + relative_start;
        if !starts_line_type_alias(source, start, name) {
            search_from = start + prefix.len();
            continue;
        }

        let mut brace_depth = 0usize;
        for (relative_index, ch) in source[start..].char_indices() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                ';' if brace_depth == 0 => {
                    let mut end = start + relative_index + ch.len_utf8();
                    while source[end..].starts_with('\n') {
                        end += 1;
                    }
                    return Some((start, end));
                }
                _ => {}
            }
        }
        return None;
    }
    None
}

fn starts_line_type_alias(source: &str, start: usize, name: &str) -> bool {
    let line_prefix = &source[..start];
    if !line_prefix
        .rsplit_once('\n')
        .map_or(line_prefix, |(_, line)| line)
        .trim()
        .is_empty()
    {
        return false;
    }

    let after_name = &source[start + "type ".len() + name.len()..];
    after_name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_whitespace() || ch == '<' || ch == '=')
}

fn contains_identifier(source: &str, name: &str) -> bool {
    let mut search_from = 0;
    while let Some(relative_start) = source[search_from..].find(name) {
        let start = search_from + relative_start;
        let end = start + name.len();
        let before = source[..start].chars().next_back();
        let after = source[end..].chars().next();
        if before.is_none_or(|ch| !is_identifier_char(ch))
            && after.is_none_or(|ch| !is_identifier_char(ch))
        {
            return true;
        }
        search_from = end;
    }
    false
}

fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_unused_internal_type_aliases_after_dce() {
        let source = r#"
type FaArgs = { argv: string[] };
type FaFault = { message: string };
type FaFaultable<T> = { is_fault: true; fault: FaFault } | { is_fault: false; value: T };

export function live(value: bigint): bigint {
    return value;
}
"#;

        let output = emit_typescript(source).expect("typescript");

        assert!(!output.contains("FaArgs"));
        assert!(!output.contains("FaFault"));
        assert!(!output.contains("FaFaultable"));
        assert!(output.contains("export function live(value: bigint): bigint"));
    }

    #[test]
    fn keeps_internal_type_aliases_used_by_exports() {
        let source = r#"
type FaArgs = { argv: string[] };
type FaFault = { message: string };
type FaFaultable<T> = { is_fault: true; fault: FaFault } | { is_fault: false; value: T };

export function main(args: FaArgs): FaFaultable<bigint> {
    return { is_fault: false, value: 0n };
}
"#;

        let output = emit_typescript(source).expect("typescript");

        assert!(output.contains("type FaArgs"));
        assert!(output.contains("type FaFault"));
        assert!(output.contains("type FaFaultable"));
        assert!(!output.contains("export type FaArgs"));
    }
}
