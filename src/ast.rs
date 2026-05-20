#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub declarations: Vec<Decl>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    Import(Import),
    Node(Callable),
    Program(Callable),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub source: ImportSource,
    pub clause: ImportClause,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportSource {
    Module(String),
    Local(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportClause {
    Alias(String),
    Items(Vec<ImportItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Callable {
    pub name: String,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub chains: Vec<Chain>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    pub source: Endpoint,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Endpoint {
    Name(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    String(String),
    Unit,
    Tuple(Vec<Endpoint>),
    Seq(Vec<Endpoint>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Endpoint(Endpoint),
    Map(String),
    FaultMap {
        node: String,
        ok: String,
        fault: String,
    },
    Filter(String),
    Repeat { count: Endpoint, node: String },
    Reduce { op: String, identity: Endpoint },
}
