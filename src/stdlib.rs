mod bytes;
mod cli;
mod fault;
mod int;
mod intrinsic;
mod io;
mod math;
mod predicates;
mod real;

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
    bytes::SPLIT_LINES,
    bytes::CONCAT_BYTES,
    bytes::JOIN_BYTES,
    cli::ARGV,
    cli::FLAG_PRESENT,
    cli::FLAG_VALUE,
    io::READ_STDIN,
    io::WRITE_STDOUT,
    io::WRITE_STDERR,
    real::PARSE_REAL,
    real::FORMAT_REAL,
    int::PARSE_INT,
    int::FORMAT_INT,
    math::ADD,
    math::SUB,
    math::MUL,
    math::DIV,
    math::REM,
    math::EQ,
    math::LT,
    math::GT,
    math::LE,
    math::GE,
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
    intrinsic::RANGE_STEP,
    intrinsic::SELECT,
];

pub fn module_symbols(module: &str) -> impl Iterator<Item = &'static StdSymbol> + '_ {
    SYMBOLS.iter().filter(move |symbol| symbol.module == module)
}

pub fn find_export(module: &str, name: &str) -> Option<&'static StdSymbol> {
    SYMBOLS
        .iter()
        .find(|symbol| symbol.module == module && symbol.name == name)
}

pub fn supports_higher_order_call(name: &str) -> bool {
    matches!(
        name,
        "parse_real" | "parse_int" | "not_empty" | "format_int" | "format_real" | "not"
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
