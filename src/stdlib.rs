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
    ty("std.cli", "Args"),
    node("std.bytes", "split_lines", "Bytes", "Seq[Bytes]"),
    reducible_node(
        "std.bytes",
        "concat_bytes",
        "Seq[Bytes]",
        "Bytes",
        "(Bytes,Bytes)",
        "Bytes",
    ),
    unsupported_node("std.bytes", "join_bytes", "(Seq[Bytes],Bytes)", "Bytes"),
    unsupported_node("std.cli", "argv", "Args", "Seq[Bytes]"),
    unsupported_node("std.cli", "flag_present", "(Args,Bytes)", "Bool"),
    unsupported_node("std.cli", "flag_value", "(Args,Bytes)", "Bytes"),
    io_node("std.io", "read_stdin", "()", "Bytes"),
    io_node("std.io", "write_stdout", "Bytes", "Int"),
    unsupported_io_node("std.io", "write_stderr", "Bytes", "Int"),
    node("std.real", "parse_real", "Bytes", "Real"),
    node("std.real", "format_real", "Real", "Bytes"),
    node("std.int", "parse_int", "Bytes", "Int"),
    node("std.int", "format_int", "Int", "Bytes"),
    reduce_node("std.math", "add", "(Real,Real)", "Real"),
    reducible_node(
        "std.math",
        "add_int",
        "(Int,Int)",
        "Int",
        "(Int,Int)",
        "Int",
    ),
    node("std.math", "sub_int", "(Int,Int)", "Int"),
    node("std.math", "eq_int", "(Int,Int)", "Bool"),
    node("std.predicates", "not_empty", "Bytes", "Bool"),
    node(INTRINSIC_MODULE, "range_step", "(Int,Int,Int)", "Seq[Int]"),
    generic_node(INTRINSIC_MODULE, "select", "(Bool,T,T)", "T"),
];

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

const fn reduce_node(
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
        reduce_input: Some(input),
        reduce_output: Some(output),
        effect: Effect::Pure,
        runtime: RuntimeSupport::ReduceOnly,
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

const fn unsupported_io_node(
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
        runtime: RuntimeSupport::Unsupported,
    }
}

pub fn module_symbols(module: &str) -> impl Iterator<Item = &'static StdSymbol> + '_ {
    SYMBOLS.iter().filter(move |symbol| symbol.module == module)
}

pub fn find_export(module: &str, name: &str) -> Option<&'static StdSymbol> {
    SYMBOLS
        .iter()
        .find(|symbol| symbol.module == module && symbol.name == name)
}

pub fn direct_builtin(name: &str) -> Option<&'static StdSymbol> {
    SYMBOLS.iter().find(|symbol| {
        symbol.name == name
            && symbol.kind == SymbolKind::Node
            && symbol.runtime == RuntimeSupport::DirectBuiltin
    })
}

pub fn function_pointer(name: &str) -> Option<&'static str> {
    match name {
        "parse_real" => Some("@fa_parse_real"),
        "parse_int" => Some("@fa_parse_int"),
        "not_empty" => Some("@fa_not_empty"),
        _ => None,
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
