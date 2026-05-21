mod bytes;
mod cli;
mod cv;
mod fault;
mod fs;
mod int;
mod intrinsic;
mod io;
mod math;
mod predicates;
mod real;
mod seq;
mod stream;
mod tuple;

const RUNTIME_C: &str = include_str!("stdlib/runtime.c");
const VECTOR_FLOW: &str = include_str!("stdlib/source/vector.flow");
const MATRIX_FLOW: &str = include_str!("stdlib/source/matrix.flow");
const CV_FLOW: &str = include_str!("stdlib/source/cv.flow");
const VECTOR_EXPORTS: &[&str] = &[
    "sum",
    "mean",
    "neg",
    "abs",
    "add",
    "sub",
    "mul",
    "div",
    "add_scalar",
    "sub_scalar",
    "scalar_sub",
    "mul_scalar",
    "scalar_mul",
    "div_scalar",
    "scalar_div",
    "equals",
    "dot",
    "squared_norm",
    "l1_norm",
    "norm",
    "normalize",
    "cosine_similarity",
    "squared_distance",
    "distance",
];
const MATRIX_EXPORTS: &[&str] = &[
    "rows",
    "cols",
    "flatten",
    "transpose",
    "neg",
    "abs",
    "add",
    "sub",
    "mul",
    "div",
    "add_scalar",
    "sub_scalar",
    "scalar_sub",
    "mul_scalar",
    "scalar_mul",
    "div_scalar",
    "scalar_div",
    "add_row",
    "sub_row",
    "mul_row",
    "div_row",
    "equals",
    "sum",
    "mean",
    "row_sums",
    "column_sums",
    "row_means",
    "column_means",
    "row_norms",
    "column_norms",
    "squared_norm",
    "l1_norm",
    "norm",
    "frobenius_norm",
    "normalize_rows",
    "squared_distance",
    "distance",
    "matvec",
    "vecmat",
    "matmul",
    "outer",
    "gram",
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
    bytes::SPLIT_LINES,
    bytes::CONCAT_BYTES,
    bytes::JOIN_BYTES,
    bytes::TRIM,
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
    fs::READ_FILE,
    fs::WRITE_FILE,
    stream::OPEN_FILE,
    stream::SIZE,
    stream::READ_AT,
    stream::COPY_TO_FILE,
    stream::CLOSE,
    io::READ_STDIN,
    io::WRITE_STDOUT,
    io::WRITE_STDERR,
    real::PARSE_REAL,
    real::FORMAT_REAL,
    real::FROM_INT,
    int::PARSE_INT,
    int::FORMAT_INT,
    int::BIT_AND,
    int::BIT_OR,
    int::BIT_XOR,
    int::BIT_SHL,
    int::BIT_SHR,
    math::ADD,
    math::SUB,
    math::MUL,
    math::DIV,
    math::REM,
    math::NEG,
    math::ABS,
    math::SQRT,
    math::EXP,
    math::SIN,
    math::COS,
    math::EQ,
    math::LT,
    math::GT,
    math::LE,
    math::GE,
    math::MIN,
    math::MAX,
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
    fault::OK,
    fault::EXPECT,
    fault::COLLECT,
    intrinsic::RANGE_STEP,
    intrinsic::SELECT,
    seq::LENGTH,
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
    for part in [
        RUNTIME_C,
        cli::C,
        io::C,
        fs::C,
        stream::C,
        int::C,
        real::C,
        fault::C,
        bytes::C,
        intrinsic::C,
    ] {
        out.push_str(part);
        out.push('\n');
    }
}

pub fn emit_cv_runtime_c(out: &mut String) {
    out.push_str(cv::C);
    out.push('\n');
}

pub fn module_symbols(module: &str) -> impl Iterator<Item = &'static StdSymbol> + '_ {
    SYMBOLS.iter().filter(move |symbol| symbol.module == module)
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
            | "bytes_to_codes"
            | "codes_to_bytes"
            | "byte_length"
            | "length"
            | "inner_length"
            | "transpose"
            | "flatten"
            | "shift_left"
            | "shift_right"
            | "tail"
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
    )
}

const fn ty(module: &'static str, name: &'static str) -> StdSymbol {
    StdSymbol {
        module,
        name,
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
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: None,
        reduce_output: None,
        effect: Effect::Io,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}

const fn unsupported_node(
    module: &'static str,
    name: &'static str,
    input: &'static str,
    output: &'static str,
) -> StdSymbol {
    StdSymbol {
        module,
        name,
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: None,
        reduce_output: None,
        effect: Effect::Pure,
        runtime: RuntimeSupport::Unsupported,
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
        kind: SymbolKind::Node,
        input: Some(input),
        output: Some(output),
        reduce_input: Some(reduce_input),
        reduce_output: Some(reduce_output),
        effect: Effect::Pure,
        runtime: RuntimeSupport::DirectBuiltin,
    }
}
