#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

mod ast;
#[cfg(not(target_arch = "wasm32"))]
mod build;
mod codegen;
mod diagnostic;
mod fmt;
mod lexer;
#[cfg(not(target_arch = "wasm32"))]
mod llvm_backend;
#[cfg(not(target_arch = "wasm32"))]
mod lsp;
#[cfg(not(target_arch = "wasm32"))]
mod mermaid;
mod module_resolver;
mod monomorphize;
mod node_ref;
mod parser;
mod stdlib;
mod typecheck;
#[cfg(target_arch = "wasm32")]
mod wasm_api;

#[cfg(not(target_arch = "wasm32"))]
pub use build::{
    BuildOptimization, BuildOptions, BuildOutput, BuildTarget, CrateType, NativeTarget, WasmTarget,
    build_file, build_file_with_options,
};
pub use fmt::{check_file as check_format_file, format_file, format_source};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeScriptCompileMode {
    Program,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeScriptCompileOptions {
    pub mode: TypeScriptCompileMode,
    pub worker_concurrency: bool,
}

impl Default for TypeScriptCompileOptions {
    fn default() -> Self {
        Self {
            mode: TypeScriptCompileMode::Program,
            worker_concurrency: false,
        }
    }
}

pub fn compile_typescript_source(source: &str) -> Result<String, String> {
    compile_typescript_source_with_options(source, TypeScriptCompileOptions::default())
}

pub fn compile_typescript_library_source(source: &str) -> Result<String, String> {
    compile_typescript_source_with_options(
        source,
        TypeScriptCompileOptions {
            mode: TypeScriptCompileMode::Library,
            ..TypeScriptCompileOptions::default()
        },
    )
}

pub fn compile_llvm_ir_source(source: &str) -> Result<String, String> {
    compile_llvm_ir_source_with_options(source, TypeScriptCompileOptions::default())
}

pub fn compile_llvm_ir_library_source(source: &str) -> Result<String, String> {
    compile_llvm_ir_source_with_options(
        source,
        TypeScriptCompileOptions {
            mode: TypeScriptCompileMode::Library,
            ..TypeScriptCompileOptions::default()
        },
    )
}

pub fn compile_typescript_source_with_options(
    source: &str,
    options: TypeScriptCompileOptions,
) -> Result<String, String> {
    let module = parser::parse_diagnostic(source)
        .map_err(|error| diagnostic::format_source_diagnostic(&error))?;
    match options.mode {
        TypeScriptCompileMode::Program => typecheck::check_module(&module)
            .map_err(|error| diagnostic::format_flowarrow_error(source, &error))?,
        TypeScriptCompileMode::Library => typecheck::check_library_module(&module)
            .map_err(|error| diagnostic::format_flowarrow_error(source, &error))?,
    }
    codegen::emit_typescript_with_options(
        &module,
        codegen::TypeScriptBackendOptions {
            worker_concurrency: options.worker_concurrency,
        },
    )
}

pub fn compile_javascript_artifacts_source_with_options(
    source: &str,
    options: TypeScriptCompileOptions,
) -> Result<(String, String), String> {
    let module = parser::parse_diagnostic(source)
        .map_err(|error| diagnostic::format_source_diagnostic(&error))?;
    match options.mode {
        TypeScriptCompileMode::Program => typecheck::check_module(&module)
            .map_err(|error| diagnostic::format_flowarrow_error(source, &error))?,
        TypeScriptCompileMode::Library => typecheck::check_library_module(&module)
            .map_err(|error| diagnostic::format_flowarrow_error(source, &error))?,
    }
    let artifacts = codegen::emit_javascript_artifacts_with_options(
        &module,
        codegen::TypeScriptBackendOptions {
            worker_concurrency: options.worker_concurrency,
        },
    )?;
    Ok((artifacts.declarations, artifacts.javascript))
}

pub fn compile_llvm_ir_source_with_options(
    source: &str,
    options: TypeScriptCompileOptions,
) -> Result<String, String> {
    let module = parser::parse_diagnostic(source)
        .map_err(|error| diagnostic::format_source_diagnostic(&error))?;
    match options.mode {
        TypeScriptCompileMode::Program => typecheck::check_module(&module)
            .map_err(|error| diagnostic::format_flowarrow_error(source, &error))?,
        TypeScriptCompileMode::Library => typecheck::check_library_module(&module)
            .map_err(|error| diagnostic::format_flowarrow_error(source, &error))?,
    }
    codegen::emit_llvm_ir_preview(&module)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_lsp_server() -> Result<u8, String> {
    lsp::run_server()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_file(path: &Path) -> Result<u8, String> {
    run_file_with_args(path, std::iter::empty::<String>())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_file_with_args<I, S>(path: &Path, args: I) -> Result<u8, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let build = build_file(path, None)?;
    let status = std::process::Command::new(&build.executable)
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to run `{}`: {error}", build.executable.display()))?;
    Ok(status.code().unwrap_or(1).try_into().unwrap_or(1))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn typecheck_file(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let module = parser::parse(&source)?;
    typecheck::check_module_with_base(&module, path.parent().unwrap_or_else(|| Path::new(".")))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mermaid_file(path: &Path) -> Result<String, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let module = parser::parse(&source)?;
    typecheck::check_module_with_base(&module, path.parent().unwrap_or_else(|| Path::new(".")))?;
    mermaid::emit_module(&module)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mermaid_file_compact(path: &Path) -> Result<String, String> {
    mermaid_file_with_options(path, mermaid::MermaidOptions { compact: true })
}

#[cfg(not(target_arch = "wasm32"))]
fn mermaid_file_with_options(
    path: &Path,
    options: mermaid::MermaidOptions,
) -> Result<String, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let module = parser::parse(&source)?;
    typecheck::check_module_with_base(&module, path.parent().unwrap_or_else(|| Path::new(".")))?;
    mermaid::emit_module_with_options(&module, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_99_bottles_declarations() {
        let source = include_str!("../examples/99-bottles/main.flow");
        let module = parser::parse(source).expect("parse");
        let names = module
            .declarations
            .iter()
            .filter_map(|decl| match decl {
                ast::Decl::Node(callable) | ast::Decl::Program(callable) => {
                    Some(callable.name.as_str())
                }
                ast::Decl::TypeAlias(_) | ast::Decl::Import(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["main", "verse_for", "bottle_word", "final_verse_node"]
        );
    }

    #[test]
    fn emits_llvm_for_map_reduce() {
        let source = include_str!("../examples/99-bottles/main.flow");
        let module = parser::parse(source).expect("parse");
        let llvm = codegen::emit_module(&module).expect("llvm");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains("static inline FaBytes flow_node_verse_for"));
        assert!(runtime_c.contains("for (size_t"));
        assert!(!runtime_c.contains("FaValue"));
        assert!(llvm.contains("define i32 @main"));
    }

    #[test]
    fn typechecks_example_without_llvm_codegen() {
        typecheck_file(Path::new("examples/add-numbers-from-stdin/main.flow")).expect("typecheck");
    }

    #[test]
    fn emits_mermaid_execution_graph() {
        let graph = mermaid_file(Path::new("examples/add-numbers-from-stdin/main.flow"))
            .expect("mermaid graph");
        assert!(graph.starts_with("flowchart TD\n"));
        assert!(graph.contains("subgraph callable_main[\"program main\"]"));
        assert!(graph.contains("([\"$args: Args\"])"));
        assert!(graph.contains("[[\"read_stdin\"]]"));
        assert!(graph.contains("([\"$input\"])"));
        assert!(graph.contains("([\"$raw_lines\"])"));
        assert!(graph.contains("[\"filter not_empty\"]"));
        assert!(graph.contains("([\"$lines\"])"));
        assert!(graph.contains("[\"map parse_real\"]"));
        assert!(graph.contains("([\"$numbers\"])"));
        assert!(graph.contains("[\"reduce add<br/>identity: 0.0\"]"));
        assert!(graph.contains("([\"$total_bytes\"])"));
        assert!(graph.contains("([\"input<br/>[$total_bytes, &quot;\\n&quot;]\"])"));
        assert!(graph.contains("[[\"write_stdout\"]]"));
        assert!(graph.contains("read_stdin -- \"binds\" -->"));
        assert!(graph.contains("-- \"$input\" -->"));
        assert!(graph.contains("-- \"item\" -->"));
        assert!(graph.contains("classDef value"));
        assert!(graph.contains("classDef boundary"));
        assert!(graph.contains("classDef collection"));
        assert!(graph.contains("subgraph legend[\"legend\"]"));
        assert!(!graph.contains("[\"0.0\"]"));
        assert!(graph.contains("([\"$exit_code\"])"));
    }

    #[test]
    fn emits_mermaid_match_as_decision_branches() {
        let graph =
            mermaid_file(Path::new("examples/http-server/main.flow")).expect("mermaid graph");
        assert!(graph.contains("subgraph callable_handle_request[\"node handle_request\"]"));
        assert!(graph.contains("([\"$req: http.Request\"])"));
        assert!(graph.contains("{\"match ?\"}"));
        assert!(graph.contains("subgraph handle_request_match_arm_0"));
        assert!(graph.contains("-- \"http.route(&quot;GET&quot;, &quot;/health&quot;)\" -->"));
        assert!(graph.contains("[\"health_response\"]"));
        assert!(graph.contains("-- \"_\" -->"));
        assert!(graph.contains("[[\"http.not_found\"]]"));
        assert!(graph.contains("([\"$response\"])"));
        assert!(graph.contains("class handle_request_match_match_20_3f decision"));
        assert!(!graph.contains("match<br/>http.route"));
    }

    #[test]
    fn emits_compact_mermaid_execution_graph() {
        let graph = mermaid_file_compact(Path::new("examples/add-numbers-from-stdin/main.flow"))
            .expect("compact mermaid graph");
        assert!(graph.starts_with("flowchart TD\n"));
        assert!(graph.contains("[[\"read_stdin\"]]"));
        assert!(graph.contains("[\"split_lines\"]"));
        assert!(graph.contains("read_stdin -- \"$input\" -->"));
        assert!(graph.contains("split_lines -- \"$raw_lines\" -->"));
        assert!(!graph.contains("([\"$input\"])"));
        assert!(!graph.contains("([\"$raw_lines\"])"));
        assert!(graph.contains("([\"$exit_code\"])"));
    }

    #[test]
    fn typecheck_rejects_unknown_stdlib_export() {
        let source = r#"
            import std.cli { Args }
            import std.bytes { missing }

            program main(args: Args) -> exit_code: Int {
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("does not export `missing`"));
    }

    #[test]
    fn typecheck_accepts_argv() {
        let source = r#"
            import std.cli { Args }
            import std.cli { argv }

            program main(args: Args) -> exit_code: Int {
                $args -> argv -> $raw_args
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
    }

    #[test]
    fn compiles_typescript_source_in_memory() {
        let source = r#"
            import std.math { add }

            extern node increment(value: Int) -> out: Int {
                ($value, 1) -> add -> $out
            }
        "#;
        let ts = compile_typescript_library_source(source).expect("typescript");
        assert!(ts.contains("export function increment(value: bigint): bigint"));
        assert!(ts.contains("1n"));
    }

    #[test]
    fn typescript_source_names_do_not_shadow_runtime_helpers() {
        let source = r#"
            import std.int { parse_int }

            extern node parse(bytes: Bytes) -> out: Faultable[Int] {
                $bytes -> parse_int -> $faOk
                $faOk -> $out
            }
        "#;
        let ts = compile_typescript_library_source(source).expect("typescript");
        assert!(ts.contains("return faParseInt(bytes);"));
        assert!(!ts.contains("const faOk"));
    }

    #[test]
    fn typescript_worker_concurrency_is_opt_in() {
        let source = r#"
            import std.math { add, mul }

            extern node score_batch(width: Int) -> scores: Seq[Int] {
                (1, $width, 1) -> range_step -> $jobs
                $jobs -> map score_job -> $scores
            }

            node score_job(n: Int) -> score: Int {
                ($n, $n) -> mul -> $square
                ($square, $n) -> add -> $score
            }
        "#;
        let sequential = compile_typescript_library_source(source).expect("typescript");
        assert!(!sequential.contains("new Worker"));
        assert!(!sequential.contains("SharedArrayBuffer"));

        let workers = compile_typescript_source_with_options(
            source,
            TypeScriptCompileOptions {
                mode: TypeScriptCompileMode::Library,
                worker_concurrency: true,
            },
        )
        .expect("typescript workers");
        assert!(workers.contains("export async function score_batch"));
        assert!(workers.contains("export async function __flowarrow_setup_workers"));
        assert!(workers.contains("export async function __flowarrow_teardown_workers"));
        assert!(workers.contains("const __flowarrow_worker_mappers"));
        assert!(workers.contains("faUseSharedNumericSequences = true"));
        assert!(workers.contains("function faScalarInputBuffer"));
        assert!(workers.contains("Promise<Array<bigint>>"));
        assert!(workers.contains("new runtime.Worker(runtime.workerUrl, { type: \"module\" })"));
        assert!(workers.contains("node:worker_threads"));
        assert!(
            workers
                .contains("new runtime.Worker(new URL(runtime.workerUrl), { type: \"module\" })")
        );
        assert!(!workers.contains("eval: true"));
        assert!(workers.contains("faScalarWorkerPools"));
        assert!(workers.contains("SharedArrayBuffer"));
        assert!(workers.contains("faParallelMapBigInt"));
    }

    #[test]
    fn compiles_llvm_ir_preview_in_memory() {
        let fib_source = r#"
            import std.math { add }

            extern node fib(depth: Int) -> result: Int {
                (0, 1) -> repeat<$depth> _fib_step -> ($result, $)
            }

            node _fib_step(a: Int, b: Int) -> (next_a: Int, next_b: Int) {
                $b       -> $next_a
                ($a, $b) -> add -> $next_b
            }
        "#;
        let llvm = compile_llvm_ir_library_source(fib_source).expect("llvm ir");
        assert!(llvm.starts_with("; FlowArrow LLVM IR preview\n"));
        assert!(llvm.contains("define i64 @flow_node_fib(i64 %input)"));
        assert!(llvm.contains("@flow_repeat__fib_step"));
        assert!(llvm.contains("define { i64, i64 } @flow_node__fib_step"));
        assert!(llvm.contains(" add i64 "));

        let concurrency_source = r#"
            import std.math { add, max, mul }

            extern node score_batch(width: Int) -> (total: Int, peak: Int) {
                (1, $width, 1) -> range_step              -> $jobs
                $jobs          -> map score_job           -> $scores
                $scores        -> reduce add(identity: 0) -> $total
                $scores        -> reduce max(identity: 0) -> $peak
            }

            node score_job(n: Int) -> score: Int {
                ($n, $n)      -> mul -> $square
                ($square, $n) -> add -> $score
            }
        "#;
        let llvm = compile_llvm_ir_library_source(concurrency_source).expect("llvm ir");
        assert!(llvm.contains("define { i64, i64 } @flow_node_score_batch(i64 %input)"));
        assert!(llvm.contains("@flow_builtin_range_step"));
        assert!(llvm.contains("@flow_map_score_job"));
        assert!(llvm.contains("@flow_reduce_add"));
        assert!(llvm.contains("@flow_reduce_max"));
        assert!(llvm.contains(" mul i64 "));
    }

    #[test]
    fn in_memory_typescript_compile_rejects_local_imports() {
        let source = r#"
            import "./lib.flow" { helper }

            extern node demo(value: Int) -> out: Int {
                $value -> helper -> $out
            }
        "#;
        let error = compile_typescript_library_source(source).expect_err("local import");
        assert!(error.contains("local imports require a source file path"));
    }

    #[test]
    fn in_memory_typescript_compile_reports_parse_line_numbers() {
        let source = "extern node broken(value: Int) -> out: Int {\n    @\n}\n";

        let error = compile_typescript_library_source(source).expect_err("parse error");

        assert!(error.contains("line 2, column 5"), "{error}");
        assert!(error.contains("unexpected character `@`"), "{error}");
    }

    #[test]
    fn in_memory_javascript_artifact_compile_reports_typecheck_line_numbers() {
        let source = r#"import std.bytes { missing }

extern node demo(value: Int) -> out: Int {
    $value -> missing -> $out
}
"#;

        let error = compile_javascript_artifacts_source_with_options(
            source,
            TypeScriptCompileOptions {
                mode: TypeScriptCompileMode::Library,
                ..TypeScriptCompileOptions::default()
            },
        )
        .expect_err("typecheck error");

        assert!(error.contains("line 1, column 20"), "{error}");
        assert!(error.contains("does not export `missing`"), "{error}");
    }

    #[test]
    fn parses_and_typechecks_match() {
        let source = r#"
            import std.cli { Args }
            import std.math { eq }

            program main(args: Args) -> exit_code: Int {
                0 -> match {
                    eq(0) -> zero
                    _ -> one
                } -> $exit_code
            }

            node zero(x: Int) -> y: Int {
                0 -> $y
            }

            node one(x: Int) -> y: Int {
                1 -> $y
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
    }

    #[test]
    fn parses_and_typechecks_static_node_params() {
        let source = r#"
            import std.cli { Args }
            import std.math { add }

            node increment(x: Int) -> y: Int {
                ($x, 1) -> add -> $y
            }

            node twice<step: node(Int) -> Int>(x: Int) -> y: Int {
                $x -> step -> step -> $y
            }

            node wrap<inner: node(Int) -> Int>(x: Int) -> y: Int {
                $x -> twice<inner> -> $y
            }

            program main(args: Args) -> exit_code: Int {
                40 -> wrap<increment> -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
    }

    #[test]
    fn typecheck_rejects_static_node_param_signature_mismatch() {
        let source = r#"
            import std.cli { Args }
            import std.int { format_int }

            node twice<step: node(Int) -> Int>(x: Int) -> y: Int {
                $x -> step -> step -> $y
            }

            program main(args: Args) -> exit_code: Int {
                40 -> twice<format_int> -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("does not match static node parameter `step`"));
    }

    #[test]
    fn parses_and_typechecks_match_inline_value_targets() {
        let source = r#"
            import std.cli { Args }
            import std.math { eq }

            program main(args: Args) -> exit_code: Int {
                0 -> match {
                    eq(0) -> 0
                    _ -> 1
                } -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
    }

    #[test]
    fn parse_rejects_match_without_fallback() {
        let source = r#"
            import std.cli { Args }
            import std.math { eq }

            program main(args: Args) -> exit_code: Int {
                0 -> match {
                    eq(0) -> zero
                } -> $exit_code
            }

            node zero(x: Int) -> y: Int {
                0 -> $y
            }
        "#;
        let error = parser::parse(source).expect_err("parse should fail");
        assert!(error.contains("fallback"));
    }

    #[test]
    fn parse_rejects_match_fallback_before_last_arm() {
        let source = r#"
            import std.cli { Args }
            import std.math { eq }

            program main(args: Args) -> exit_code: Int {
                0 -> match {
                    _ -> zero
                    eq(0) -> one
                } -> $exit_code
            }

            node zero(x: Int) -> y: Int {
                0 -> $y
            }

            node one(x: Int) -> y: Int {
                1 -> $y
            }
        "#;
        let error = parser::parse(source).expect_err("parse should fail");
        assert!(error.contains("fallback arm must be last"));
    }

    #[test]
    fn typecheck_rejects_match_guard_that_is_not_bool() {
        let source = r#"
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                0 -> match {
                    identity_int() -> zero
                    _ -> zero
                } -> $exit_code
            }

            node identity_int(x: Int) -> y: Int {
                $x -> $y
            }

            node zero(x: Int) -> y: Int {
                0 -> $y
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("match guard `identity_int` result"));
    }

    #[test]
    fn typecheck_rejects_match_arm_output_mismatch() {
        let source = r#"
            import std.cli { Args }
            import std.math { eq }

            program main(args: Args) -> exit_code: Int {
                0 -> match {
                    eq(0) -> zero
                    _ -> bytes
                } -> $exit_code
            }

            node zero(x: Int) -> y: Int {
                0 -> $y
            }

            node bytes(x: Int) -> y: Bytes {
                "bad" -> $y
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("match arm `bytes` result"));
    }

    #[test]
    fn typecheck_rejects_match_inline_value_output_mismatch() {
        let source = r#"
            import std.cli { Args }
            import std.math { eq }

            program main(args: Args) -> exit_code: Int {
                0 -> match {
                    eq(0) -> 0
                    _ -> "bad"
                } -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("match arm `\"bad\"` result"));
    }

    #[test]
    fn typecheck_accepts_stream_map_with_pure_node() {
        let source = r#"
            import std.cli { Args }
            import std.fs { open_file }

            program main(args: Args) -> exit_code: Int {
                "input.txt" -> open_file -> map id_bytes -> $stream
                0 -> $exit_code
            }

            node id_bytes(input: Bytes) -> output: Bytes {
                $input -> $output
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
    }

    #[test]
    fn typecheck_rejects_stream_map_with_effectful_node() {
        let source = r#"
            import std.cli { Args }
            import std.fs { open_file }
            import std.io { write_stdout }

            program main(args: Args) -> exit_code: Faultable[Int] {
                "input.txt" -> open_file -> map write_stdout -> $stream
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("cannot be used as a map/filter function"));
    }

    #[test]
    fn std_http_import_emits_h2o_runtime() {
        let source = r#"
            import std.cli { Args }
            import std.http as http

            program main(args: Args) -> exit_code: Faultable[Int] {
                () -> http.default_config -> $config
                $config -> http.listen -> $listener
                $listener -> http.requests -> $requests
                $requests -> map handle -> $responses
                ($listener, $responses) -> http.serve -> $exit_code
            }

            node handle(req: http.Request) -> response: http.Response {
                $req -> http.response -> $response0
                ($response0, 200) -> http.with_status -> $response1
                ($response1, "{\"ok\":true}\n") -> http.json -> $response
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains("#include <h2o.h>"));
        assert!(runtime_c.contains("fa_http_serve"));
    }

    #[test]
    fn dollar_prefixed_values_do_not_collide_with_node_names() {
        let source = r#"
            import std.cli { Args }
            import std.math { add }

            program main(args: Args) -> exit_code: Int {
                0 -> $add
                ($add, 1) -> add -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
    }

    #[test]
    fn typecheck_rejects_node_input_type_mismatch() {
        let source = r#"
            import std.cli { Args }
            import std.int { format_int }

            program main(args: Args) -> exit_code: Int {
                "not an int" -> format_int -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("`format_int` input type mismatch"));
    }

    #[test]
    fn typecheck_and_codegen_resolve_stdlib_aliases() {
        let source = r#"
            import std.cli { Args }
            import std.math as math

            program main(args: Args) -> exit_code: Int {
                (3, 1) -> math.sub -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains(" - "));
    }

    #[test]
    fn typecheck_and_codegen_inline_empty_sqlite_params() {
        let source = r#"
            import std.cli { Args }
            import std.sqlite as sqlite
            import std.tuple { first }

            program main(args: Args) -> exit_code: Faultable[Int] {
                () -> sqlite.open_memory -> $conn0
                ($conn0, "CREATE TABLE todos (title TEXT NOT NULL)", []) -> sqlite.exec -> first -> $conn1
                ($conn1, "INSERT INTO todos (title) VALUES (?)", ["write sqlite docs" -> sqlite.text]) -> sqlite.exec -> first -> $conn2
                $conn2 -> sqlite.close -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains("FaSeq_SqliteValue_new(0)"));
        assert!(runtime_c.contains("fa_sqlite_text"));
    }

    #[test]
    fn typecheck_and_codegen_destructure_tuple_binding() {
        let source = r#"
            import std.cli { Args }

            node pair(input: Int) -> out: Faultable[(Int, Int)] {
                ($input, 2) -> $out
            }

            program main(args: Args) -> exit_code: Faultable[Int] {
                1 -> pair -> ($left, $right)
                $left -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains(".value.f0"));
        assert!(runtime_c.contains(".is_fault"));
    }

    #[test]
    fn typecheck_rejects_non_final_tuple_binding() {
        let source = r#"
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                (1, 2) -> ($left, $right) -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("binding targets may only appear as final stages"));
    }

    #[test]
    fn typecheck_rejects_tuple_binding_arity_mismatch() {
        let source = r#"
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                (1, 2) -> ($left, $middle, $right)
                $left -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("expected 3 tuple fields, found 2"));
    }

    #[test]
    fn typecheck_rejects_untyped_empty_sequence_binding() {
        let source = r#"
            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                [] -> $empty
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("empty sequence literals need a type context"));
    }

    #[test]
    fn typechecks_mixed_numeric_add_and_llvm_type_names() {
        let source = r#"
            import std.cli { Args }
            import std.math { add }

            node mixed(left: i64, right: double) -> out: double {
                ($left, $right) -> add -> $out
            }

            node numeric_identity(value: Int|Real) -> out: Int|Real {
                $value -> $out
            }

            program main(args: Args) -> exit_code: Int {
                (1, 2.5) -> add -> $total
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(!runtime_c.contains("FaValue"));
    }

    #[test]
    fn type_aliases_resolve_in_typecheck_and_codegen() {
        let source = r#"
            type Pixel = (Real,(Real,Real))
            type Row = Seq[Pixel]
            type Size = (Int,Int)
            type Image = (Size,Seq[Row])

            import std.cli { Args }

            node passthrough(image: Image) -> out: Image {
                $image -> $out
            }

            program main(args: Args) -> exit_code: Int {
                ((1, 1), [[(0.1, (0.2, 0.3))]]) -> passthrough -> $image
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains("flow_node_passthrough"));
    }

    #[test]
    fn type_alias_cycles_are_rejected() {
        let source = r#"
            type A = B
            type B = A

            import std.cli { Args }

            program main(args: Args) -> exit_code: Int {
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        let error = typecheck::check_module(&module).expect_err("typecheck should fail");
        assert!(error.contains("cyclic type alias"));
    }

    #[test]
    fn source_stdlib_type_alias_imports_rewrite_into_user_signatures() {
        let source = r#"
            import std.cli { Args }
            import std.cv { Image, Pixel, Size, grayscale }
            import std.tuple { first }

            node image_size(image: Image) -> size: Size {
                $image -> first -> $size
            }

            node keep_pixel(pixel: Pixel) -> out: Pixel {
                $pixel -> $out
            }

            node process(image: Image) -> out: Image {
                $image -> grayscale -> $out
            }

            program main(args: Args) -> exit_code: Int {
                ((1, 1), [[(0.2, (0.4, 0.6))]]) -> process -> $image
                $image -> image_size -> $size
                (0.2, (0.4, 0.6)) -> keep_pixel -> $pixel
                0 -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains("flow_node_process"));
        assert!(runtime_c.contains("flow_node___flow_std_cv_grayscale"));
        assert!(runtime_c.contains("FaTuple_Real_Real"));
    }

    #[test]
    fn llvm_backend_runs_numeric_math_variants() {
        let root =
            std::env::temp_dir().join(format!("flowarrow-math-variants-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }
                import std.math { sub, eq, max }

                program main(args: Args) -> exit_code: Int {
                    (5, 2.5) -> sub -> $mixed_sub
                    (2, 2.5) -> max -> $mixed_max
                    ($mixed_sub, $mixed_max) -> eq -> $real_ok

                    (4, 7) -> max -> $int_max
                    ($int_max, 7) -> eq -> $max_ok

                    (9, 4) -> sub -> $int_sub
                    ($int_sub, 5) -> eq -> $sub_ok

                    ($real_ok, $max_ok, false) -> select -> $first_ok
                    ($first_ok, $sub_ok, false) -> select -> $all_ok
                    ($all_ok, 0, 1) -> select -> $exit_code
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn build_uses_stable_target_dir_and_entrypoint_name() {
        let root = unique_temp_root("build-layout");
        let path = root.join("tool.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    0 -> $exit_code
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");

        assert_eq!(
            build.build_dir,
            root.join("build").join(build::host_target())
        );
        assert_eq!(
            build.executable,
            build
                .build_dir
                .join(format!("tool{}", std::env::consts::EXE_SUFFIX))
        );
        assert!(build.executable.exists());
        assert!(build.build_dir.join(".cache").is_dir());
        assert!(build.build_dir.join(".cache/main.ll").exists());
        assert!(build.build_dir.join(".cache/runtime.ll").exists());
        assert!(!build.build_dir.join(".cache/runtime.c").exists());
        assert!(build.build_dir.join(".cache/build.hash").exists());
        assert!(!build.build_dir.join("app").exists());
    }

    #[test]
    fn build_recompiles_stable_executable_when_source_changes() {
        let root = unique_temp_root("build-recompile");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    1 -> $exit_code
                }
            "#,
        )
        .expect("write source");
        let first = build_file(&path, None).expect("first build");
        let first_output = Command::new(&first.executable).output().expect("first run");
        assert_eq!(first_output.status.code(), Some(1));

        fs::write(
            &path,
            r#"
                import std.cli { Args }

                program main(args: Args) -> exit_code: Int {
                    2 -> $exit_code
                }
            "#,
        )
        .expect("rewrite source");
        let second = build_file(&path, None).expect("second build");
        let second_output = Command::new(&second.executable)
            .output()
            .expect("second run");

        assert_eq!(second.executable, first.executable);
        assert_eq!(second_output.status.code(), Some(2));
    }

    #[test]
    fn typechecks_parse_and_sum_lines_example() {
        typecheck_file(Path::new("examples/parse-and-sum-lines/main.flow")).expect("typecheck");
    }

    #[test]
    fn llvm_backend_runs_parse_and_sum_lines_with_faults() {
        let build =
            build_file(Path::new("examples/parse-and-sum-lines/main.flow"), None).expect("build");

        let mut ok_child = Command::new(&build.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ok");
        ok_child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(b"1\n2\n3.5\n")
            .expect("write stdin");
        let ok_output = ok_child.wait_with_output().expect("run ok");
        assert!(
            ok_output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&ok_output.stderr)
        );
        assert_eq!(String::from_utf8(ok_output.stdout).expect("utf8"), "6.5\n");
        assert_eq!(String::from_utf8(ok_output.stderr).expect("utf8"), "");

        let mut fault_child = Command::new(&build.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn fault");
        fault_child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(b"1\nwat\n3\n")
            .expect("write stdin");
        let fault_output = fault_child.wait_with_output().expect("run fault");
        assert_eq!(fault_output.status.code(), Some(65));
        assert_eq!(String::from_utf8(fault_output.stdout).expect("utf8"), "");
        assert_eq!(
            String::from_utf8(fault_output.stderr).expect("utf8"),
            "line 2: expected Real, got \"wat\"\n"
        );
    }

    #[test]
    fn llvm_backend_runs_add_numbers_from_args() {
        let build =
            build_file(Path::new("examples/add-numbers-from-args/main.flow"), None).expect("build");
        let output = Command::new(&build.executable)
            .args(["1.5", "2.5", "3"])
            .output()
            .expect("run");
        assert!(
            output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "7\n");
        assert_eq!(String::from_utf8(output.stderr).expect("utf8"), "");
    }

    #[test]
    fn typechecks_and_codegen_parse_int_and_add_reduce() {
        let source = r#"
            import std.cli { Args }
            import std.io { read_stdin, write_stdout }
            import std.bytes { split_lines }
            import std.int { parse_int, format_int }
            import std.math { add }

            program main(args: Args) -> exit_code: Faultable[Int] {
                () -> read_stdin -> split_lines -> map parse_int -> $numbers
                $numbers -> reduce add(identity: 0) -> $total
                $total -> format_int -> $output
                $output -> write_stdout -> $exit_code
            }
        "#;
        let module = parser::parse(source).expect("parse");
        typecheck::check_module(&module).expect("typecheck");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains("for (size_t"));
        assert!(!runtime_c.contains("FaValue"));
    }

    #[test]
    fn llvm_backend_runs_99_bottles() {
        let build = build_file(Path::new("examples/99-bottles/main.flow"), None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf8");
        assert!(stdout.starts_with(
            "99 bottles of beer on the wall, 99 bottles of beer.\n\
             Take one down and pass it around, 98 bottles of beer on the wall.\n\n\
             98 bottles of beer on the wall, 98 bottles of beer.\n"
        ));
        assert!(stdout.contains(
            "1 bottle of beer on the wall, 1 bottle of beer.\n\
             Take one down and pass it around, 0 bottles of beer on the wall.\n\n"
        ));
        assert!(stdout.ends_with(
            "No more bottles of beer on the wall, no more bottles of beer.\n\
             Go to the store and buy some more, 99 bottles of beer on the wall.\n"
        ));
    }

    #[test]
    fn llvm_backend_runs_add_numbers_from_stdin() {
        let source = include_str!("../examples/add-numbers-from-stdin/main.flow");
        let module = parser::parse(source).expect("parse");
        let runtime_c = codegen::emit_runtime_c(&module).expect("runtime c");
        assert!(runtime_c.contains("for (size_t"));
        assert!(!runtime_c.contains("FaValue"));

        let build = build_file(Path::new("examples/add-numbers-from-stdin/main.flow"), None)
            .expect("build");
        let mut child = Command::new(&build.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(b"1\n2\n3.5\n")
            .expect("write stdin");
        let output = child.wait_with_output().expect("run");
        assert!(
            output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "6.5\n");
    }

    #[test]
    fn llvm_backend_runs_fibonacci_from_stdin() {
        let build = build_file(Path::new("examples/fibonacci/main.flow"), None).expect("build");
        let mut child = Command::new(&build.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(b"12\n")
            .expect("write stdin");
        let output = child.wait_with_output().expect("run");
        assert!(
            output.status.success(),
            "program failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).expect("utf8"), "144\n");
    }

    #[test]
    fn llvm_backend_runs_new_math_nodes() {
        let root = std::env::temp_dir().join(format!("flowarrow-new-math-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }
                import std.math { mul, div, rem, lt, gt, le, ge, eq }

                program main(args: Args) -> exit_code: Int {
                    # mul: 3 * 4 = 12
                    (3, 4) -> mul -> $product
                    ($product, 12) -> eq -> $mul_ok

                    # div: 10 / 3 = 3 (truncating)
                    (10, 3) -> div -> $quotient
                    ($quotient, 3) -> eq -> $div_ok

                    # rem: 10 % 3 = 1
                    (10, 3) -> rem -> $remainder
                    ($remainder, 1) -> eq -> $rem_ok

                    # lt: 2 < 5 = true
                    (2, 5) -> lt -> $lt_ok

                    # gt: 7 > 3 = true
                    (7, 3) -> gt -> $gt_ok

                    # le: 4 <= 4 = true
                    (4, 4) -> le -> $le_ok

                    # ge: 5 >= 3 = true
                    (5, 3) -> ge -> $ge_ok

                    ($mul_ok, $div_ok, false) -> select -> $s1
                    ($s1, $rem_ok, false) -> select -> $s2
                    ($s2, $lt_ok, false) -> select -> $s3
                    ($s3, $gt_ok, false) -> select -> $s4
                    ($s4, $le_ok, false) -> select -> $s5
                    ($s5, $ge_ok, false) -> select -> $all_ok
                    ($all_ok, 0, 1) -> select -> $exit_code
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "new math nodes failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn llvm_backend_runs_predicate_logic_nodes() {
        let root =
            std::env::temp_dir().join(format!("flowarrow-predicates-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }
                import std.predicates { and, or, not }

                program main(args: Args) -> exit_code: Int {
                    # and(true, true) = true
                    (true, true) -> and -> $and_tt
                    # or(false, true) = true
                    (false, true) -> or -> $or_ft
                    # not(false) = true
                    false -> not -> $not_false

                    ($and_tt, $or_ft, false) -> select -> $s1
                    ($s1, $not_false, false) -> select -> $all_ok
                    ($all_ok, 0, 1) -> select -> $exit_code
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "predicate logic nodes failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn llvm_backend_runs_base_predicates() {
        let root =
            std::env::temp_dir().join(format!("flowarrow-base-predicates-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }
                import std.predicates { is_empty, xor, not, all, any }

                program main(args: Args) -> exit_code: Int {
                    "" -> is_empty -> $empty_ok
                    "x" -> is_empty -> $nonempty_is_empty
                    $nonempty_is_empty -> not -> $nonempty_ok

                    (true, false) -> xor -> $xor_tf
                    (true, true) -> xor -> $xor_tt
                    $xor_tt -> not -> $xor_tt_ok

                    [true, true, true] -> all -> $all_true_ok
                    [true, false, true] -> all -> $all_mixed
                    $all_mixed -> not -> $all_mixed_ok

                    [false, true, false] -> any -> $any_mixed_ok
                    [false, false] -> any -> $any_false
                    $any_false -> not -> $any_false_ok

                    ($empty_ok, $nonempty_ok, false) -> select -> $s1
                    ($s1, $xor_tf, false) -> select -> $s2
                    ($s2, $xor_tt_ok, false) -> select -> $s3
                    ($s3, $all_true_ok, false) -> select -> $s4
                    ($s4, $all_mixed_ok, false) -> select -> $s5
                    ($s5, $any_mixed_ok, false) -> select -> $s6
                    ($s6, $any_false_ok, false) -> select -> $all_ok
                    ($all_ok, 0, 1) -> select -> $exit_code
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "base predicates failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn llvm_backend_runs_join_bytes() {
        let root =
            std::env::temp_dir().join(format!("flowarrow-join-bytes-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.bytes { join_bytes, concat_bytes }
                import std.cli { Args }
                import std.io { write_stdout }

                program main(args: Args) -> exit_code: Int {
                    (["hello", "world"], " ") -> join_bytes -> $joined
                    [$joined, "\n"] -> concat_bytes -> $output
                    $output -> write_stdout -> $exit_code
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "join_bytes failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("utf8"),
            "hello world\n"
        );
    }

    #[test]
    fn llvm_backend_match_skips_unselected_arm_and_later_guard() {
        let root = unique_temp_root("match-lazy");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }
                import std.fault { expect }
                import std.int { parse_int }
                import std.math { eq }

                program main(args: Args) -> exit_code: Int {
                    0 -> match {
                        eq(0) -> zero
                        bad_guard() -> one
                        _ -> bad_body
                    } -> $exit_code
                }

                node zero(x: Int) -> y: Int {
                    0 -> $y
                }

                node one(x: Int) -> y: Int {
                    1 -> $y
                }

                node bad_guard(x: Int) -> r: Bool {
                    "not-an-int" -> parse_int -> expect -> $n
                    ($n, 0) -> eq -> $r
                }

                node bad_body(x: Int) -> y: Int {
                    "not-an-int" -> parse_int -> expect -> $y
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "match laziness failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn llvm_backend_match_emits_inline_value_targets() {
        let root = unique_temp_root("match-inline-values");
        let path = root.join("main.flow");
        fs::write(
            &path,
            r#"
                import std.cli { Args }
                import std.math { eq }

                program main(args: Args) -> exit_code: Int {
                    0 -> match {
                        eq(0) -> 0
                        _ -> 1
                    } -> $exit_code
                }
            "#,
        )
        .expect("write source");

        let build = build_file(&path, None).expect("build");
        let output = Command::new(&build.executable).output().expect("run");
        assert!(
            output.status.success(),
            "inline match values failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn unique_temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("flowarrow-{name}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root).expect("temp dir");
        root
    }
}
