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

#[derive(Debug, Clone)]
pub struct TypeScriptArtifacts {
    pub javascript: String,
    pub declarations: String,
}

pub fn emit_javascript(source: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let mut program = parse_typescript(&allocator, source)?;
    transform_typescript(&allocator, &mut program, source)?;
    run_dce(&allocator, &mut program);
    Ok(print_program(&program, SourceType::mjs()))
}

pub fn emit_typescript_artifacts(source: &str) -> Result<TypeScriptArtifacts, String> {
    let allocator = Allocator::default();
    let mut program = parse_typescript(&allocator, source)?;
    let declarations = emit_declarations(&allocator, &program, source)?;
    transform_typescript(&allocator, &mut program, source)?;
    run_dce(&allocator, &mut program);
    let javascript = print_program(&program, SourceType::mjs());
    Ok(TypeScriptArtifacts {
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
    Ok(print_program(&ret.program, SourceType::d_ts()))
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
