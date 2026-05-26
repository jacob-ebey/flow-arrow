mod bytes;
mod cli;
mod cv;
mod fault;
mod fs;
mod http;
mod int;
mod intrinsic;
mod io;
mod math;
mod predicates;
mod real;
mod seq;
mod sqlite;
mod stream;
mod tuple;

const RUNTIME_C: &str = include_str!("stdlib/runtime.c");
const RUNTIME_H: &str = include_str!("stdlib/runtime.h");
const VECTOR_FLOW: &str = include_str!("stdlib/source/vector.flow");
const MATRIX_FLOW: &str = include_str!("stdlib/source/matrix.flow");
const CV_FLOW: &str = include_str!("stdlib/source/cv.flow");
const VECTOR_EXPORTS: &[&str] = &[
    "sum_f64",
    "mean_f64",
    "min_f64",
    "max_f64",
    "neg_f64",
    "abs_f64",
    "exp_f64",
    "add_f64",
    "sub_f64",
    "mul_f64",
    "div_f64",
    "add_scalar_f64",
    "sub_scalar_f64",
    "scalar_sub_f64",
    "mul_scalar_f64",
    "scalar_mul_f64",
    "div_scalar_f64",
    "scalar_div_f64",
    "equals_f64",
    "dot_f64",
    "squared_norm_f64",
    "l1_norm_f64",
    "norm_f64",
    "normalize_f64",
    "relu_f64",
    "sigmoid_f64",
    "silu_f64",
    "softmax_f64",
    "cosine_similarity_f64",
    "squared_distance_f64",
    "distance_f64",
    "sum_f32",
    "mean_f32",
    "min_f32",
    "max_f32",
    "neg_f32",
    "abs_f32",
    "exp_f32",
    "add_f32",
    "sub_f32",
    "mul_f32",
    "div_f32",
    "add_scalar_f32",
    "sub_scalar_f32",
    "scalar_sub_f32",
    "mul_scalar_f32",
    "scalar_mul_f32",
    "div_scalar_f32",
    "scalar_div_f32",
    "equals_f32",
    "dot_f32",
    "squared_norm_f32",
    "l1_norm_f32",
    "norm_f32",
    "normalize_f32",
    "relu_f32",
    "sigmoid_f32",
    "silu_f32",
    "softmax_f32",
    "cosine_similarity_f32",
    "squared_distance_f32",
    "distance_f32",
];
const MATRIX_EXPORTS: &[&str] = &[
    "rows_f64",
    "cols_f64",
    "flatten_f64",
    "transpose_f64",
    "neg_f64",
    "abs_f64",
    "add_f64",
    "sub_f64",
    "mul_f64",
    "div_f64",
    "add_scalar_f64",
    "sub_scalar_f64",
    "scalar_sub_f64",
    "mul_scalar_f64",
    "scalar_mul_f64",
    "div_scalar_f64",
    "scalar_div_f64",
    "add_row_f64",
    "sub_row_f64",
    "mul_row_f64",
    "div_row_f64",
    "equals_f64",
    "sum_f64",
    "mean_f64",
    "row_sums_f64",
    "column_sums_f64",
    "row_means_f64",
    "column_means_f64",
    "row_norms_f64",
    "column_norms_f64",
    "squared_norm_f64",
    "l1_norm_f64",
    "norm_f64",
    "frobenius_norm_f64",
    "normalize_rows_f64",
    "row_softmax_f64",
    "squared_distance_f64",
    "distance_f64",
    "matvec_f64",
    "vecmat_f64",
    "matmul_f64",
    "outer_f64",
    "gram_f64",
    "rows_f32",
    "cols_f32",
    "flatten_f32",
    "transpose_f32",
    "neg_f32",
    "abs_f32",
    "add_f32",
    "sub_f32",
    "mul_f32",
    "div_f32",
    "add_scalar_f32",
    "sub_scalar_f32",
    "scalar_sub_f32",
    "mul_scalar_f32",
    "scalar_mul_f32",
    "div_scalar_f32",
    "scalar_div_f32",
    "add_row_f32",
    "sub_row_f32",
    "mul_row_f32",
    "div_row_f32",
    "equals_f32",
    "sum_f32",
    "mean_f32",
    "row_sums_f32",
    "column_sums_f32",
    "row_means_f32",
    "column_means_f32",
    "row_norms_f32",
    "column_norms_f32",
    "squared_norm_f32",
    "l1_norm_f32",
    "norm_f32",
    "frobenius_norm_f32",
    "normalize_rows_f32",
    "row_softmax_f32",
    "squared_distance_f32",
    "distance_f32",
    "matvec_f32",
    "vecmat_f32",
    "matmul_f32",
    "outer_f32",
    "gram_f32",
];
const CV_EXPORTS: &[&str] = &[
    "Size",
    "Pixel",
    "Image",
    "dimensions",
    "width",
    "height",
    "pixels",
    "pixel_rows",
    "normalize",
    "map_pixels",
    "grayscale",
    "invert",
    "threshold",
    "brighten",
    "darken",
    "contrast",
    "red_channel",
    "green_channel",
    "blue_channel",
    "red_matrix",
    "green_matrix",
    "blue_matrix",
    "luma_matrix",
    "sepia",
    "add",
    "sub",
    "absdiff",
    "decode",
    "decode_bmp",
    "load",
    "load_bmp",
    "load_jpeg",
    "load_pgm",
    "load_png",
    "load_pnm",
    "load_ppm",
    "save_bmp",
    "save_jpeg",
    "save_pgm",
    "save_png",
    "save_ppm",
    "decode_pgm",
    "decode_jpeg",
    "decode_png",
    "decode_pnm",
    "decode_ppm",
    "encode_bmp",
    "encode_jpeg",
    "encode_pgm",
    "encode_png",
    "encode_ppm",
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Type,
    Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Pure,
    Io,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSupport {
    DirectBuiltin,
    ReduceOnly,
    Unsupported,
}

#[derive(Debug, Clone, Copy)]
pub struct StdSymbol {
    pub module: &'static str,
    pub name: &'static str,
    pub runtime_name: &'static str,
    pub kind: SymbolKind,
    pub input: Option<&'static str>,
    pub output: Option<&'static str>,
    pub reduce_input: Option<&'static str>,
    pub reduce_output: Option<&'static str>,
    pub effect: Effect,
    pub runtime: RuntimeSupport,
}

pub const INTRINSIC_MODULE: &str = "__intrinsic";

pub const SYMBOLS: &[StdSymbol] = &[
    cli::ARGS,
    fault::FAULT,
    stream::STREAM,
    stream::TO_SEQ,
    stream::DRAIN,
    bytes::SPLIT_LINES,
    bytes::CONCAT_BYTES,
    bytes::JOIN_BYTES,
    bytes::TRIM,
    bytes::CONTAINS,
    bytes::STARTS_WITH,
    bytes::ENDS_WITH,
    bytes::INDEX_OF,
    bytes::LAST_INDEX_OF,
    bytes::BYTE_SLICE,
    bytes::TAKE,
    bytes::DROP,
    bytes::REPLACE,
    bytes::REPEAT_BYTES,
    bytes::ASCII_LOWER,
    bytes::ASCII_UPPER,
    bytes::SPLIT_ON,
    bytes::STRIP_PREFIX,
    bytes::STRIP_SUFFIX,
    bytes::BYTES_TO_CODES,
    bytes::CODES_TO_BYTES,
    bytes::BYTE_LENGTH,
    cli::ARGV,
    cli::FLAG_PRESENT,
    cli::FLAG_VALUE,
    cv::DECODE,
    cv::DECODE_BMP,
    cv::DECODE_JPEG,
    cv::DECODE_PNG,
    cv::DECODE_PNM,
    cv::ENCODE_BMP,
    cv::ENCODE_JPEG,
    cv::ENCODE_PGM,
    cv::ENCODE_PNG,
    cv::ENCODE_PPM,
    io::READ_STDIN,
    io::WRITE_STDOUT,
    io::WRITE_STDERR,
    fs::READ_FILE,
    fs::WRITE_FILE,
    fs::EXISTS,
    fs::IS_FILE,
    fs::IS_DIR,
    fs::FILE_SIZE,
    fs::JOIN_PATH,
    fs::BASENAME,
    fs::DIRNAME,
    fs::LIST_DIR,
    fs::WALK_FILES,
    fs::READ_FILES,
    fs::OPEN_FILE,
    fs::SIZE,
    fs::READ_AT,
    fs::COPY_TO_FILE,
    fs::CLOSE,
    http::SERVER_CONFIG,
    http::LISTENER,
    http::REQUEST,
    http::RESPONSE,
    http::DEFAULT_CONFIG,
    http::WITH_TCP_LISTENER,
    http::WITH_TLS,
    http::WITH_HTTP2,
    http::WITH_HTTP3,
    http::LISTEN,
    http::REQUESTS,
    http::SERVE,
    http::ROUTE,
    http::BODY,
    http::RESPONSE_NODE,
    http::WITH_STATUS,
    http::WITH_HEADER,
    http::TEXT,
    http::JSON,
    http::NOT_FOUND,
    sqlite::CONNECTION,
    sqlite::ROW,
    sqlite::VALUE,
    sqlite::OPEN,
    sqlite::OPEN_READONLY,
    sqlite::OPEN_MEMORY,
    sqlite::CLOSE,
    sqlite::BUSY_TIMEOUT,
    sqlite::FOREIGN_KEYS,
    sqlite::BEGIN,
    sqlite::BEGIN_IMMEDIATE,
    sqlite::COMMIT,
    sqlite::ROLLBACK,
    sqlite::NULL,
    sqlite::INT,
    sqlite::REAL,
    sqlite::TEXT,
    sqlite::BLOB,
    sqlite::EXEC,
    sqlite::QUERY,
    sqlite::QUERY_ALL,
    sqlite::COLUMN_COUNT,
    sqlite::COLUMN_NAME,
    sqlite::VALUE_AT,
    sqlite::VALUE_NAMED,
    sqlite::KIND,
    sqlite::IS_NULL,
    sqlite::AS_INT,
    sqlite::AS_REAL,
    sqlite::AS_TEXT,
    sqlite::AS_BLOB,
    real::PARSE_REAL,
    real::FORMAT_REAL,
    real::FORMAT_REAL_F32,
    real::FROM_INT,
    real::FROM_INT_F32,
    int::PARSE_INT,
    int::FORMAT_INT,
    int::BIT_AND,
    int::BIT_OR,
    int::BIT_XOR,
    int::BIT_SHL,
    int::BIT_SHR,
    math::ADD_I32,
    math::ADD_I64,
    math::ADD_F32,
    math::ADD_F64,
    math::SUB_I32,
    math::SUB_I64,
    math::SUB_F32,
    math::SUB_F64,
    math::MUL_I32,
    math::MUL_I64,
    math::MUL_F32,
    math::MUL_F64,
    math::DIV_I32,
    math::DIV_I64,
    math::DIV_F32,
    math::DIV_F64,
    math::REM_I32,
    math::REM_I64,
    math::REM_F32,
    math::REM_F64,
    math::NEG_I32,
    math::NEG_I64,
    math::NEG_F32,
    math::NEG_F64,
    math::ABS_I32,
    math::ABS_I64,
    math::ABS_F32,
    math::ABS_F64,
    math::SQRT_F32,
    math::SQRT_F64,
    math::EXP_F32,
    math::EXP_F64,
    math::SIN_F32,
    math::SIN_F64,
    math::COS_F32,
    math::COS_F64,
    math::EQ_I32,
    math::EQ_I64,
    math::EQ_F32,
    math::EQ_F64,
    math::LT_I32,
    math::LT_I64,
    math::LT_F32,
    math::LT_F64,
    math::GT_I32,
    math::GT_I64,
    math::GT_F32,
    math::GT_F64,
    math::LE_I32,
    math::LE_I64,
    math::LE_F32,
    math::LE_F64,
    math::GE_I32,
    math::GE_I64,
    math::GE_F32,
    math::GE_F64,
    math::MIN_I32,
    math::MIN_I64,
    math::MIN_F32,
    math::MIN_F64,
    math::MAX_I32,
    math::MAX_I64,
    math::MAX_F32,
    math::MAX_F64,
    predicates::NOT_EMPTY,
    predicates::IS_EMPTY,
    predicates::AND,
    predicates::OR,
    predicates::XOR,
    predicates::NOT,
    predicates::ALL,
    predicates::ANY,
    fault::HAS_FAULTS,
    fault::FORMAT_FAULTS,
    fault::EXPECT,
    fault::COLLECT,
    intrinsic::RANGE_STEP,
    intrinsic::SELECT,
    seq::LENGTH,
    seq::IS_EMPTY,
    seq::GROUP_BY_ID,
    seq::ZIP,
    seq::BROADCAST_LEFT,
    seq::BROADCAST_RIGHT,
    seq::TRANSPOSE,
    seq::FLATTEN,
    seq::INNER_LENGTH,
    seq::SHIFT_RIGHT,
    seq::SHIFT_LEFT,
    seq::HEAD,
    seq::TAIL,
    seq::REVERSE,
    seq::TAKE,
    seq::DROP,
    seq::FILL,
    seq::SLICE,
    seq::LAST,
    seq::GET,
    seq::GET_OR,
    seq::AT,
    seq::APPEND,
    seq::SET,
    seq::CONCAT,
    tuple::FIRST,
    tuple::SECOND,
    tuple::SWAP,
];

pub fn emit_runtime_c(out: &mut String) {
    let mut emitted_headers = Vec::new();
    for headers in [
        cli::H,
        io::H,
        fs::H,
        int::H,
        real::H,
        fault::H,
        bytes::H,
        intrinsic::H,
    ] {
        push_c_headers(out, headers, &mut emitted_headers);
    }

    for part in [
        RUNTIME_C,
        cli::C,
        io::C,
        fs::C,
        int::C,
        real::C,
        fault::C,
        bytes::C,
        intrinsic::C,
    ] {
        push_c_fragment(out, part);
        out.push('\n');
    }
}

pub fn emit_cv_type_h(out: &mut String) {
    let mut emitted_headers = Vec::new();
    push_c_headers(out, cv::TYPE_H, &mut emitted_headers);
}

pub fn emit_cv_runtime_h(out: &mut String) {
    let mut emitted_headers = Vec::new();
    push_c_headers(out, cv::H, &mut emitted_headers);
}

pub fn emit_cv_runtime_c(out: &mut String) {
    push_c_fragment(out, cv::C);
    out.push('\n');
}

pub fn emit_http_runtime_h(out: &mut String) {
    let mut emitted_headers = Vec::new();
    push_c_headers(out, http::H, &mut emitted_headers);
}

pub fn emit_http_runtime_c(out: &mut String) {
    push_c_fragment(out, http::C);
    out.push('\n');
}

pub fn is_runtime_header_type_name(name: &str) -> bool {
    http::HEADER_TYPES.contains(&name) || sqlite::HEADER_TYPES.contains(&name)
}

pub fn emit_sqlite_runtime_h(out: &mut String) {
    let mut emitted_headers = Vec::new();
    push_c_headers(out, sqlite::H, &mut emitted_headers);
}

pub fn emit_sqlite_runtime_c(out: &mut String) {
    push_c_fragment(out, sqlite::C);
    out.push('\n');
}

fn push_c_headers(out: &mut String, headers: &[&'static str], emitted: &mut Vec<&'static str>) {
    for header in headers {
        if emitted.contains(header) {
            continue;
        }
        emitted.push(header);
        push_c_fragment(out, header);
        out.push('\n');
    }
}

fn push_c_fragment(out: &mut String, source: &str) {
    for line in source.lines() {
        if !is_local_c_include(line) {
            out.push_str(line);
            out.push('\n');
        }
    }
}

fn is_local_c_include(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("#include")
        .is_some_and(|rest| rest.trim_start().starts_with('"'))
}

pub fn module_symbols(module: &str) -> impl Iterator<Item = &'static StdSymbol> + '_ {
    SYMBOLS.iter().filter(move |symbol| symbol.module == module)
}

pub fn all_symbols() -> impl Iterator<Item = &'static StdSymbol> {
    SYMBOLS.iter()
}

pub fn find_export(module: &str, name: &str) -> Option<&'static StdSymbol> {
    SYMBOLS
        .iter()
        .find(|symbol| symbol.module == module && symbol.name == name)
}

pub fn flow_source(module: &str) -> Option<&'static str> {
    match module {
        "std.vector" => Some(VECTOR_FLOW),
        "std.matrix" => Some(MATRIX_FLOW),
        "std.cv" => Some(CV_FLOW),
        _ => None,
    }
}

pub fn flow_exports(module: &str) -> Option<&'static [&'static str]> {
    match module {
        "std.vector" => Some(VECTOR_EXPORTS),
        "std.matrix" => Some(MATRIX_EXPORTS),
        "std.cv" => Some(CV_EXPORTS),
        _ => None,
    }
}

pub fn supports_higher_order_call(name: &str) -> bool {
    matches!(
        name,
        "parse_real"
            | "parse_int"
            | "not_empty"
            | "format_int"
            | "format_real"
            | "from_int"
            | "not"
            | "trim"
            | "ascii_lower"
            | "ascii_upper"
            | "bytes_to_codes"
            | "codes_to_bytes"
            | "byte_length"
            | "length"
            | "is_empty"
            | "inner_length"
            | "transpose"
            | "flatten"
            | "reverse"
            | "shift_left"
            | "shift_right"
            | "take"
            | "drop"
            | "tail"
            | "basename"
            | "dirname"
            | "join_path"
            | "last"
            | "at"
            | "append"
            | "first"
            | "second"
            | "swap"
            | "add"
            | "sub"
            | "mul"
            | "div"
            | "rem"
            | "neg"
            | "abs"
            | "sqrt"
            | "exp"
            | "sin"
            | "cos"
            | "min"
            | "max"
            | "eq"
            | "lt"
            | "gt"
            | "le"
            | "ge"
            | "bit_and"
            | "bit_or"
            | "bit_xor"
            | "bit_shl"
            | "bit_shr"
            | "collect"
            | "column_count"
            | "column_name"
            | "value_at"
            | "value_named"
            | "kind"
            | "is_null"
            | "as_int"
            | "as_real"
            | "as_text"
            | "as_blob"
            | "read_file"
            | "write_file"
            | "exists"
            | "is_file"
            | "is_dir"
            | "file_size"
            | "list_dir"
            | "walk_files"
            | "read_files"
            | "open_file"
            | "size"
            | "read_at"
            | "copy_to_file"
    )
}

const fn ty(module: &'static str, name: &'static str) -> StdSymbol {
    StdSymbol {
        module,
        name,
        runtime_name: name,
        kind: SymbolKind::Type,
        input: None,
        output: None,
        reduce_input: None,
        reduce_output: None,
        effect: Effect::Pure,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}

const fn node(
    module: &'static str,
    name: &'static str,
    input: &'static str,
    output: &'static str,
) -> StdSymbol {
    StdSymbol {
        module,
        name,
        runtime_name: name,
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: None,
        reduce_output: None,
        effect: Effect::Pure,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}

const fn runtime_node(
    module: &'static str,
    name: &'static str,
    runtime_name: &'static str,
    input: &'static str,
    output: &'static str,
) -> StdSymbol {
    StdSymbol {
        module,
        name,
        runtime_name,
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: None,
        reduce_output: None,
        effect: Effect::Pure,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}

const fn generic_node(
    module: &'static str,
    name: &'static str,
    input: &'static str,
    output: &'static str,
) -> StdSymbol {
    node(module, name, input, output)
}

const fn io_node(
    module: &'static str,
    name: &'static str,
    input: &'static str,
    output: &'static str,
) -> StdSymbol {
    StdSymbol {
        module,
        name,
        runtime_name: name,
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: None,
        reduce_output: None,
        effect: Effect::Io,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}

const fn reducible_node(
    module: &'static str,
    name: &'static str,
    input: &'static str,
    output: &'static str,
    reduce_input: &'static str,
    reduce_output: &'static str,
) -> StdSymbol {
    StdSymbol {
        module,
        name,
        runtime_name: name,
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: Some(reduce_input),
        reduce_output: Some(reduce_output),
        effect: Effect::Pure,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}

const fn runtime_reducible_node(
    module: &'static str,
    name: &'static str,
    runtime_name: &'static str,
    input: &'static str,
    output: &'static str,
    reduce_input: &'static str,
    reduce_output: &'static str,
) -> StdSymbol {
    StdSymbol {
        module,
        name,
        runtime_name,
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: Some(reduce_input),
        reduce_output: Some(reduce_output),
        effect: Effect::Pure,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_c_includes_are_stripped_from_runtime_fragments() {
        let mut out = String::new();
        push_c_fragment(
            &mut out,
            "#include \"runtime.h\"\n#include <stdio.h>\nstatic int value;\n",
        );

        assert!(!out.contains("#include \"runtime.h\""));
        assert!(out.contains("#include <stdio.h>"));
        assert!(out.contains("static int value;"));
    }
}
