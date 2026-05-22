use crate::diagnostic::SourceSpan;
use crate::{parser, stdlib, typecheck};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub fn run_server() -> Result<u8, String> {
    Server::default().run().map_err(|error| error.to_string())?;
    Ok(0)
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, String>,
    shutdown: bool,
}

impl Server {
    fn run(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut reader = io::BufReader::new(stdin.lock());
        let stdout = io::stdout();
        let mut writer = stdout.lock();

        while let Some(source) = read_message(&mut reader)? {
            let Ok(message) = JsonParser::new(&source).parse() else {
                continue;
            };
            let Some(method) = message.get("method").and_then(Json::as_str) else {
                continue;
            };
            let id = message.get("id").cloned();
            let params = message.get("params").unwrap_or(&Json::Null);

            match method {
                "initialize" => {
                    if let Some(id) = id.as_ref() {
                        send_response(&mut writer, id, initialize_result())?;
                    }
                }
                "initialized" => {}
                "shutdown" => {
                    self.shutdown = true;
                    if let Some(id) = id.as_ref() {
                        send_response(&mut writer, id, "null".to_string())?;
                    }
                }
                "exit" => break,
                "textDocument/didOpen" => {
                    if let Some((uri, text)) = did_open_params(params) {
                        self.documents.insert(uri.clone(), text);
                        self.publish_diagnostics(&mut writer, &uri)?;
                    }
                }
                "textDocument/didChange" => {
                    if let Some((uri, text)) = did_change_params(params) {
                        self.documents.insert(uri.clone(), text);
                        self.publish_diagnostics(&mut writer, &uri)?;
                    }
                }
                "textDocument/didClose" => {
                    if let Some(uri) = text_document_uri(params) {
                        self.documents.remove(&uri);
                        send_notification(
                            &mut writer,
                            "textDocument/publishDiagnostics",
                            format!("{{\"uri\":{},\"diagnostics\":[]}}", json_string(&uri)),
                        )?;
                    }
                }
                "textDocument/completion" => {
                    if let Some(id) = id.as_ref() {
                        let result = self
                            .document_analysis(params)
                            .map(|analysis| completion_result(&analysis))
                            .unwrap_or_else(|| "[]".to_string());
                        send_response(&mut writer, id, result)?;
                    }
                }
                "textDocument/definition" => {
                    if let Some(id) = id.as_ref() {
                        let result = self
                            .document_analysis(params)
                            .and_then(|analysis| {
                                position_param(params)
                                    .and_then(|position| definition_result(&analysis, position))
                            })
                            .unwrap_or_else(|| "null".to_string());
                        send_response(&mut writer, id, result)?;
                    }
                }
                "textDocument/hover" => {
                    if let Some(id) = id.as_ref() {
                        let result = self
                            .document_analysis(params)
                            .and_then(|analysis| {
                                position_param(params)
                                    .and_then(|position| hover_result(&analysis, position))
                            })
                            .unwrap_or_else(|| "null".to_string());
                        send_response(&mut writer, id, result)?;
                    }
                }
                "textDocument/inlayHint" => {
                    if let Some(id) = id.as_ref() {
                        let result = self
                            .document_analysis(params)
                            .map(|analysis| inlay_hints_result(&analysis, range_param(params)))
                            .unwrap_or_else(|| "[]".to_string());
                        send_response(&mut writer, id, result)?;
                    }
                }
                "textDocument/documentSymbol" => {
                    if let Some(id) = id.as_ref() {
                        let result = self
                            .document_analysis(params)
                            .map(|analysis| document_symbols_result(&analysis))
                            .unwrap_or_else(|| "[]".to_string());
                        send_response(&mut writer, id, result)?;
                    }
                }
                _ => {
                    if let Some(id) = id.as_ref() {
                        send_response(&mut writer, id, "null".to_string())?;
                    }
                }
            }

            if self.shutdown && method == "exit" {
                break;
            }
        }
        Ok(())
    }

    fn document_analysis(&self, params: &Json) -> Option<Analysis> {
        let uri = text_document_uri(params)?;
        let source = self.documents.get(&uri)?;
        Some(Analysis::new(uri, source.clone()))
    }

    fn publish_diagnostics(&self, writer: &mut impl Write, uri: &str) -> io::Result<()> {
        let source = self.documents.get(uri).map(String::as_str).unwrap_or("");
        let diagnostics = diagnostics_for(uri, source);
        send_notification(
            writer,
            "textDocument/publishDiagnostics",
            format!(
                "{{\"uri\":{},\"diagnostics\":{diagnostics}}}",
                json_string(uri)
            ),
        )
    }
}

fn initialize_result() -> String {
    concat!(
        "{\"capabilities\":{",
        "\"textDocumentSync\":1,",
        "\"completionProvider\":{\"resolveProvider\":false,\"triggerCharacters\":[\".\",\"$\"]},",
        "\"definitionProvider\":true,",
        "\"hoverProvider\":true,",
        "\"inlayHintProvider\":true,",
        "\"documentSymbolProvider\":true",
        "},\"serverInfo\":{\"name\":\"flowarrow-lsp\",\"version\":\"0.1.0\"}}"
    )
    .to_string()
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(length) = content_length else {
        return Ok(None);
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(Some(String::from_utf8_lossy(&body).into_owned()))
}

fn send_response(writer: &mut impl Write, id: &Json, result: String) -> io::Result<()> {
    send_json(
        writer,
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{result}}}",
            id.to_json()
        ),
    )
}

fn send_notification(writer: &mut impl Write, method: &str, params: String) -> io::Result<()> {
    send_json(
        writer,
        format!(
            "{{\"jsonrpc\":\"2.0\",\"method\":{},\"params\":{params}}}",
            json_string(method)
        ),
    )
}

fn send_json(writer: &mut impl Write, body: String) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()
}

fn did_open_params(params: &Json) -> Option<(String, String)> {
    let doc = params.get("textDocument")?;
    Some((
        doc.get("uri")?.as_str()?.to_string(),
        doc.get("text")?.as_str()?.to_string(),
    ))
}

fn did_change_params(params: &Json) -> Option<(String, String)> {
    let uri = text_document_uri(params)?;
    let changes = params.get("contentChanges")?.as_array()?;
    let text = changes.first()?.get("text")?.as_str()?.to_string();
    Some((uri, text))
}

fn text_document_uri(params: &Json) -> Option<String> {
    params
        .get("textDocument")?
        .get("uri")?
        .as_str()
        .map(str::to_string)
}

fn position_param(params: &Json) -> Option<Position> {
    let position = params.get("position")?;
    Some(Position {
        line: position.get("line")?.as_u32()?,
        character: position.get("character")?.as_u32()?,
    })
}

fn range_param(params: &Json) -> Option<Range> {
    let range = params.get("range")?;
    Some(Range {
        start: json_position(range.get("start")?)?,
        end: json_position(range.get("end")?)?,
    })
}

fn json_position(value: &Json) -> Option<Position> {
    Some(Position {
        line: value.get("line")?.as_u32()?,
        character: value.get("character")?.as_u32()?,
    })
}

fn diagnostics_for(uri: &str, source: &str) -> String {
    let analysis = Analysis::new(uri.to_string(), source.to_string());
    match parser::parse_diagnostic(source) {
        Ok(module) => {
            let has_main = module
                .declarations
                .iter()
                .any(|decl| matches!(decl, crate::ast::Decl::Program(callable) if callable.name == "main"));
            if !has_main {
                return "[]".to_string();
            }
            let base_dir = uri_to_path(uri)
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."));
            match typecheck::check_module_with_base(&module, &base_dir) {
                Ok(()) => "[]".to_string(),
                Err(error) => diagnostics_json(
                    &error,
                    analysis
                        .diagnostic_range(&error)
                        .unwrap_or_else(|| Range::point(Position::default())),
                ),
            }
        }
        Err(error) => diagnostics_json(&error.message, source_span_to_range(error.span)),
    }
}

fn semantic_summary_for(uri: &str, source: &str) -> Option<typecheck::SemanticSummary> {
    let module = parser::parse(source).ok()?;
    let base_dir = uri_to_path(uri)
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    typecheck::semantic_summary_with_base(&module, &base_dir).ok()
}

fn diagnostics_json(message: &str, range: Range) -> String {
    format!(
        "[{{\"range\":{},\"severity\":1,\"source\":\"flowarrow\",\"message\":{}}}]",
        range_json(range),
        json_string(message)
    )
}

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    fn as_u32(&self) -> Option<u32> {
        match self {
            Json::Number(value) if *value >= 0.0 => Some(*value as u32),
            _ => None,
        }
    }

    fn to_json(&self) -> String {
        match self {
            Json::Null => "null".to_string(),
            Json::Bool(value) => value.to_string(),
            Json::Number(value) => {
                if value.fract() == 0.0 {
                    format!("{value:.0}")
                } else {
                    value.to_string()
                }
            }
            Json::String(value) => json_string(value),
            Json::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(Json::to_json)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Json::Object(map) => format!(
                "{{{}}}",
                map.iter()
                    .map(|(key, value)| format!("{}:{}", json_string(key), value.to_json()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}

struct JsonParser<'a> {
    chars: Vec<char>,
    pos: usize,
    source: &'a str,
}

impl<'a> JsonParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            source,
        }
    }

    fn parse(mut self) -> Result<Json, String> {
        let value = self.value()?;
        self.whitespace();
        if self.peek().is_some() {
            return Err("trailing JSON input".to_string());
        }
        Ok(value)
    }

    fn value(&mut self) -> Result<Json, String> {
        self.whitespace();
        match self.peek() {
            Some('n') => self.literal("null", Json::Null),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('"') => self.string().map(Json::String),
            Some('[') => self.array(),
            Some('{') => self.object(),
            Some('-' | '0'..='9') => self.number(),
            Some(ch) => Err(format!("unexpected JSON character `{ch}`")),
            None => Err("unexpected end of JSON".to_string()),
        }
    }

    fn literal(&mut self, text: &str, value: Json) -> Result<Json, String> {
        for expected in text.chars() {
            if self.bump() != Some(expected) {
                return Err(format!("expected JSON literal `{text}`"));
            }
        }
        Ok(value)
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut map = BTreeMap::new();
        self.whitespace();
        if self.eat('}') {
            return Ok(Json::Object(map));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            self.expect(':')?;
            let value = self.value()?;
            map.insert(key, value);
            self.whitespace();
            if self.eat('}') {
                break;
            }
            self.expect(',')?;
        }
        Ok(Json::Object(map))
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.whitespace();
        if self.eat(']') {
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.whitespace();
            if self.eat(']') {
                break;
            }
            self.expect(',')?;
        }
        Ok(Json::Array(items))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        while let Some(ch) = self.bump() {
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let escaped = self
                        .bump()
                        .ok_or_else(|| "unterminated JSON escape".to_string())?;
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let mut value = 0u32;
                            for _ in 0..4 {
                                let digit = self
                                    .bump()
                                    .and_then(|digit| digit.to_digit(16))
                                    .ok_or_else(|| "invalid JSON unicode escape".to_string())?;
                                value = value * 16 + digit;
                            }
                            if let Some(decoded) = char::from_u32(value) {
                                out.push(decoded);
                            }
                        }
                        other => return Err(format!("unsupported JSON escape `\\{other}`")),
                    }
                }
                other => out.push(other),
            }
        }
        Err("unterminated JSON string".to_string())
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        self.eat('-');
        while matches!(self.peek(), Some('0'..='9')) {
            self.pos += 1;
        }
        if self.eat('.') {
            while matches!(self.peek(), Some('0'..='9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            let _ = self.eat('+') || self.eat('-');
            while matches!(self.peek(), Some('0'..='9')) {
                self.pos += 1;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|error| format!("invalid JSON number `{text}` in `{}`: {error}", self.source))
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.eat(expected) {
            Ok(())
        } else {
            Err(format!("expected JSON `{expected}`"))
        }
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Position {
    line: u32,
    character: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Range {
    start: Position,
    end: Position,
}

impl Range {
    fn point(position: Position) -> Self {
        Self {
            start: position,
            end: Position {
                line: position.line,
                character: position.character + 1,
            },
        }
    }

    fn contains(&self, position: Position) -> bool {
        compare_positions(self.start, position) <= 0 && compare_positions(position, self.end) < 0
    }
}

fn compare_positions(left: Position, right: Position) -> i32 {
    match left.line.cmp(&right.line) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Equal => match left.character.cmp(&right.character) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Equal => 0,
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Ident(String),
    Variable(String),
    String(String),
    Int,
    Real,
    Bool,
    Arrow,
    Symbol(char),
}

#[derive(Debug, Clone, PartialEq)]
struct LspToken {
    kind: TokenKind,
    range: Range,
}

#[derive(Debug, Clone)]
struct Analysis {
    uri: String,
    tokens: Vec<LspToken>,
    symbols: Vec<Symbol>,
    imports: Vec<ImportInfo>,
    callables: Vec<CallableScope>,
    semantic: Option<typecheck::SemanticSummary>,
}

#[derive(Debug, Clone)]
struct Symbol {
    name: String,
    kind: SymbolKind,
    range: Range,
    selection_range: Range,
    detail: String,
    uri: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymbolKind {
    Type,
    Node,
    Program,
    Import,
}

#[derive(Debug, Clone)]
struct ImportInfo {
    source: ImportSourceInfo,
    alias: Option<(String, Range)>,
    items: Vec<ImportItemInfo>,
}

#[derive(Debug, Clone)]
enum ImportSourceInfo {
    Module(String),
    Local(String, Range),
}

#[derive(Debug, Clone)]
struct ImportItemInfo {
    imported: String,
    local: String,
    range: Range,
    alias_range: Option<Range>,
}

impl ImportInfo {
    fn range_for_name(&self, name: &str) -> Option<Range> {
        if let Some((alias, range)) = &self.alias {
            if alias == name || name.starts_with(&format!("{alias}.")) {
                return Some(*range);
            }
        }
        for item in &self.items {
            if item.local == name || item.imported == name {
                return Some(item.alias_range.unwrap_or(item.range));
            }
        }
        match &self.source {
            ImportSourceInfo::Local(path, range) if path == name => Some(*range),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct CallableScope {
    name: String,
    body_range: Range,
    variables: Vec<VariableSymbol>,
    sources: Vec<SourceUse>,
    stages: Vec<StageUse>,
}

#[derive(Debug, Clone)]
struct VariableSymbol {
    name: String,
    range: Range,
    detail: String,
}

#[derive(Debug, Clone)]
struct SourceUse {
    range: Range,
    chain_index: usize,
}

#[derive(Debug, Clone)]
struct StageUse {
    range: Range,
    arrow_range: Range,
    chain_index: usize,
    stage_index: usize,
}

struct StageHover {
    label: String,
    input: String,
    output: String,
    is_arrow: bool,
}

impl Analysis {
    fn new(uri: String, source: String) -> Self {
        let tokens = lex_lsp(&source);
        let mut analysis = Self {
            semantic: semantic_summary_for(&uri, &source),
            uri,
            tokens,
            symbols: Vec::new(),
            imports: Vec::new(),
            callables: Vec::new(),
        };
        analysis.collect();
        analysis
    }

    fn collect(&mut self) {
        let mut pos = 0usize;
        while pos < self.tokens.len() {
            let Some(keyword) = self.ident_at(pos).map(str::to_string) else {
                pos += 1;
                continue;
            };
            match keyword.as_str() {
                "type" | "struct" => pos = self.collect_type(pos),
                "import" => pos = self.collect_import(pos),
                "foreign" => pos += 1,
                "extern" if self.ident_at(pos + 1) == Some("node") => {
                    pos = self.collect_callable(pos + 1, "node")
                }
                "node" | "program" => pos = self.collect_callable(pos, &keyword),
                _ => pos += 1,
            }
        }
    }

    fn collect_type(&mut self, pos: usize) -> usize {
        let Some((name, range, next)) = self.next_ident(pos + 1) else {
            return pos + 1;
        };
        let detail = self.detail_until(
            next,
            &[
                "type", "struct", "foreign", "import", "extern", "node", "program",
            ],
        );
        self.symbols.push(Symbol {
            name,
            kind: SymbolKind::Type,
            range: self.tokens[pos].range,
            selection_range: range,
            detail,
            uri: self.uri.clone(),
        });
        next
    }

    fn collect_import(&mut self, pos: usize) -> usize {
        let mut index = pos + 1;
        let Some(source) = self.parse_import_source(&mut index) else {
            return pos + 1;
        };
        let mut alias = None;
        let mut items = Vec::new();
        if self.ident_at(index) == Some("as") {
            if let Some((name, range, next)) = self.next_ident(index + 1) {
                alias = Some((name.clone(), range));
                self.symbols.push(Symbol {
                    name,
                    kind: SymbolKind::Import,
                    range: self.tokens[pos].range,
                    selection_range: range,
                    detail: "import alias".to_string(),
                    uri: self.uri.clone(),
                });
                index = next;
            }
        } else if self.is_symbol(index, '{') {
            index += 1;
            while index < self.tokens.len() && !self.is_symbol(index, '}') {
                if let Some((imported, range, next)) = self.next_ident(index) {
                    index = next;
                    let mut local = imported.clone();
                    let mut alias_range = None;
                    if self.ident_at(index) == Some("as")
                        && let Some((alias_name, alias_name_range, after_alias)) =
                            self.next_ident(index + 1)
                    {
                        local = alias_name;
                        alias_range = Some(alias_name_range);
                        index = after_alias;
                    }
                    let selection_range = alias_range.unwrap_or(range);
                    self.symbols.push(Symbol {
                        name: local.clone(),
                        kind: SymbolKind::Import,
                        range: self.tokens[pos].range,
                        selection_range,
                        detail: format!("import {imported}"),
                        uri: self.uri.clone(),
                    });
                    items.push(ImportItemInfo {
                        imported,
                        local,
                        range,
                        alias_range,
                    });
                    if self.is_symbol(index, ',') {
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
            if self.is_symbol(index, '}') {
                index += 1;
            }
        }
        self.imports.push(ImportInfo {
            source,
            alias,
            items,
        });
        index
    }

    fn collect_callable(&mut self, pos: usize, keyword: &str) -> usize {
        let Some((name, range, mut index)) = self.next_ident(pos + 1) else {
            return pos + 1;
        };
        let kind = if keyword == "program" {
            SymbolKind::Program
        } else {
            SymbolKind::Node
        };
        let detail = self.callable_detail(pos, &name);
        self.symbols.push(Symbol {
            name: name.clone(),
            kind,
            range: self.tokens[pos].range,
            selection_range: range,
            detail,
            uri: self.uri.clone(),
        });

        let mut variables = Vec::new();
        let mut output_start = index;
        if let Some(close) = self.matching_delimiter(index) {
            self.collect_port_variables(index + 1, close, "input", &mut variables);
            output_start = close + 1;
        }

        while output_start < self.tokens.len()
            && !matches!(self.tokens[output_start].kind, TokenKind::Arrow)
        {
            if self.is_symbol(output_start, '{') {
                break;
            }
            output_start += 1;
        }
        if output_start < self.tokens.len()
            && matches!(self.tokens[output_start].kind, TokenKind::Arrow)
        {
            output_start += 1;
            let output_end = (output_start..self.tokens.len())
                .find(|candidate| self.is_symbol(*candidate, '{'))
                .unwrap_or(output_start);
            if self.is_symbol(output_start, '(') {
                if let Some(close) = self.matching_delimiter(output_start) {
                    self.collect_port_variables(output_start + 1, close, "output", &mut variables);
                }
            } else {
                self.collect_port_variables(output_start, output_end, "output", &mut variables);
            }
        }

        while index < self.tokens.len() && !self.is_symbol(index, '{') {
            index += 1;
        }
        let body_start = self
            .tokens
            .get(index)
            .map(|token| token.range.start)
            .unwrap_or(range.end);
        let body_end_index = self.matching_brace(index).unwrap_or(index);
        let body_end = self
            .tokens
            .get(body_end_index)
            .map(|token| token.range.end)
            .unwrap_or(body_start);
        self.collect_bindings(index + 1, body_end_index, &mut variables);
        let (sources, stages) = self.collect_pipeline_uses(index + 1, body_end_index);
        self.callables.push(CallableScope {
            name,
            body_range: Range {
                start: body_start,
                end: body_end,
            },
            variables,
            sources,
            stages,
        });
        body_end_index + 1
    }

    fn collect_port_variables(
        &self,
        start: usize,
        end: usize,
        label: &str,
        variables: &mut Vec<VariableSymbol>,
    ) {
        let mut index = start;
        while index < end {
            if let Some((port_name, port_range, next)) = self.next_ident(index) {
                index = next;
                if self.is_symbol(index, ':') {
                    let type_start = index + 1;
                    index += 1;
                    let mut depth = 0usize;
                    while index < end {
                        match self.tokens[index].kind {
                            TokenKind::Symbol('(') | TokenKind::Symbol('[') => depth += 1,
                            TokenKind::Symbol(')') | TokenKind::Symbol(']') if depth > 0 => {
                                depth -= 1;
                            }
                            TokenKind::Symbol(',') if depth == 0 => break,
                            _ => {}
                        }
                        index += 1;
                    }
                    variables.push(VariableSymbol {
                        name: port_name,
                        range: port_range,
                        detail: format!("{label}: {}", self.type_text(type_start, index)),
                    });
                }
            } else {
                index += 1;
            }
            if self.is_symbol(index, ',') {
                index += 1;
            }
        }
    }

    fn collect_bindings(&self, start: usize, end: usize, variables: &mut Vec<VariableSymbol>) {
        let mut index = start;
        while index < end {
            if matches!(self.tokens[index].kind, TokenKind::Arrow)
                && let Some((_, next, bindings)) = self.binding_target_at(index + 1)
            {
                for (name, range) in bindings {
                    variables.push(VariableSymbol {
                        name,
                        range,
                        detail: "value".to_string(),
                    });
                }
                index = next;
                continue;
            }
            index += 1;
        }
    }

    fn collect_pipeline_uses(&self, start: usize, end: usize) -> (Vec<SourceUse>, Vec<StageUse>) {
        let mut sources = Vec::new();
        let mut stages = Vec::new();
        let mut depth = 1usize;
        let mut expect_source = true;
        let mut chain_index = 0usize;
        let mut stage_index = 0usize;
        let mut last_depth_one_line = None::<u32>;
        let mut index = start;
        while index < end {
            match self.tokens[index].kind {
                TokenKind::Symbol('{') => {
                    depth += 1;
                    index += 1;
                    continue;
                }
                TokenKind::Symbol('}') => {
                    depth = depth.saturating_sub(1);
                    index += 1;
                    continue;
                }
                _ => {}
            }

            if depth == 1 {
                if matches!(self.tokens[index].kind, TokenKind::Arrow) {
                    if let Some((range, next, is_final)) = self.stage_use_range(index + 1) {
                        stages.push(StageUse {
                            range,
                            arrow_range: self.tokens[index].range,
                            chain_index,
                            stage_index,
                        });
                        stage_index += 1;
                        if is_final {
                            expect_source = true;
                            chain_index += 1;
                            stage_index = 0;
                        }
                        last_depth_one_line = Some(self.tokens[index].range.start.line);
                        index = next;
                        continue;
                    }
                } else if self.is_source_start(index, expect_source, last_depth_one_line) {
                    if !expect_source {
                        chain_index += 1;
                        stage_index = 0;
                    }
                    sources.push(SourceUse {
                        range: self.endpoint_use_range(index),
                        chain_index,
                    });
                    expect_source = false;
                }
                last_depth_one_line = Some(self.tokens[index].range.start.line);
            }
            index += 1;
        }
        (sources, stages)
    }

    fn is_source_start(
        &self,
        index: usize,
        expect_source: bool,
        last_depth_one_line: Option<u32>,
    ) -> bool {
        if !self.is_endpoint_start(index) {
            return false;
        }
        expect_source
            || last_depth_one_line
                .map(|line| self.tokens[index].range.start.line > line)
                .unwrap_or(true)
    }

    fn is_endpoint_start(&self, index: usize) -> bool {
        matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(
                TokenKind::Variable(_)
                    | TokenKind::String(_)
                    | TokenKind::Int
                    | TokenKind::Real
                    | TokenKind::Bool
                    | TokenKind::Symbol('(')
                    | TokenKind::Symbol('[')
            )
        )
    }

    fn endpoint_use_range(&self, index: usize) -> Range {
        if matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Symbol('(') | TokenKind::Symbol('['))
        ) {
            if let Some(end) = self.matching_delimiter(index) {
                return Range {
                    start: self.tokens[index].range.start,
                    end: self.tokens[end].range.end,
                };
            }
        }
        self.tokens[index].range
    }

    fn stage_use_range(&self, index: usize) -> Option<(Range, usize, bool)> {
        match self.tokens.get(index).map(|token| &token.kind)? {
            TokenKind::Ident(keyword) if keyword == "map" || keyword == "filter" => {
                let (range, next) = self.name_range_at(index + 1)?;
                Some((range, next, false))
            }
            TokenKind::Ident(keyword) if keyword == "fault" => {
                let (range, next) = self.name_range_at(index + 2)?;
                Some((range, next, true))
            }
            TokenKind::Ident(keyword) if keyword == "repeat" => {
                let mut cursor = index + 1;
                while cursor < self.tokens.len() && !self.is_symbol(cursor, '>') {
                    cursor += 1;
                }
                let (range, next) = self.name_range_at(cursor + 1)?;
                Some((range, next, false))
            }
            TokenKind::Ident(keyword) if keyword == "reduce" || keyword == "scan" => {
                let (range, next) = self.name_range_at(index + 1)?;
                Some((range, next, false))
            }
            TokenKind::Ident(keyword) if keyword == "match" => {
                Some((self.tokens[index].range, index + 1, false))
            }
            TokenKind::Ident(_) => {
                let (range, next) = self.name_range_at(index)?;
                Some((range, next, false))
            }
            TokenKind::Variable(_) => {
                let (range, next, _) = self.binding_target_at(index)?;
                Some((range, next, true))
            }
            TokenKind::Symbol('(') => {
                let (range, next, _) = self.binding_target_at(index)?;
                Some((range, next, true))
            }
            _ => Some((self.tokens[index].range, index + 1, false)),
        }
    }

    fn binding_target_at(&self, index: usize) -> Option<(Range, usize, Vec<(String, Range)>)> {
        match self.tokens.get(index)? {
            LspToken {
                kind: TokenKind::Variable(name),
                range,
            } => {
                let bindings = if name.is_empty() {
                    Vec::new()
                } else {
                    vec![(name.clone(), *range)]
                };
                Some((*range, index + 1, bindings))
            }
            LspToken {
                kind: TokenKind::Symbol('('),
                range,
            } => {
                let mut cursor = index + 1;
                let (_, next, mut bindings) = self.binding_target_at(cursor)?;
                cursor = next;
                if !self.is_symbol(cursor, ',') {
                    return None;
                }
                while self.is_symbol(cursor, ',') {
                    let (_, next, item_bindings) = self.binding_target_at(cursor + 1)?;
                    bindings.extend(item_bindings);
                    cursor = next;
                }
                if !self.is_symbol(cursor, ')') {
                    return None;
                }
                Some((
                    Range {
                        start: range.start,
                        end: self.tokens[cursor].range.end,
                    },
                    cursor + 1,
                    bindings,
                ))
            }
            _ => None,
        }
    }

    fn name_range_at(&self, index: usize) -> Option<(Range, usize)> {
        let (_, next) = self.qualified_name_at(index)?;
        let end = next.checked_sub(1)?;
        Some((
            Range {
                start: self.tokens[index].range.start,
                end: self.tokens[end].range.end,
            },
            next,
        ))
    }

    fn parse_import_source(&self, index: &mut usize) -> Option<ImportSourceInfo> {
        match self.tokens.get(*index).map(|token| &token.kind) {
            Some(TokenKind::String(path)) => {
                let range = self.tokens[*index].range;
                *index += 1;
                Some(ImportSourceInfo::Local(path.clone(), range))
            }
            Some(TokenKind::Ident(_)) => {
                let (module, next) = self.qualified_name_at(*index)?;
                *index = next;
                Some(ImportSourceInfo::Module(module))
            }
            _ => None,
        }
    }

    fn symbol_at(&self, position: Position) -> Option<SymbolRef> {
        let index = self.token_index_at(position)?;
        match &self.tokens[index].kind {
            TokenKind::Variable(name) => Some(SymbolRef::Variable(name.clone())),
            TokenKind::String(path) => Some(SymbolRef::ImportPath(path.clone())),
            TokenKind::Ident(_) => {
                let (name, _) = self.qualified_name_around(index)?;
                Some(SymbolRef::Name(name))
            }
            _ => None,
        }
    }

    fn token_index_at(&self, position: Position) -> Option<usize> {
        self.tokens
            .iter()
            .position(|token| token.range.contains(position))
    }

    fn callable_at(&self, position: Position) -> Option<&CallableScope> {
        self.callables
            .iter()
            .find(|callable| callable.body_range.contains(position))
    }

    fn semantic_callable(&self, callable_name: &str) -> Option<&typecheck::CallableSummary> {
        self.semantic
            .as_ref()?
            .callables
            .iter()
            .find(|callable| callable.name == callable_name)
    }

    fn semantic_variable_type(&self, callable_name: &str, name: &str) -> Option<&str> {
        self.semantic_callable(callable_name)?
            .variables
            .iter()
            .rev()
            .find(|variable| variable.name == name)
            .map(|variable| variable.ty.as_str())
    }

    fn semantic_stage_at(
        &self,
        callable: &CallableScope,
        position: Position,
    ) -> Option<StageHover> {
        let stage_use = callable
            .stages
            .iter()
            .find(|stage| stage.range.contains(position) || stage.arrow_range.contains(position))?;
        let stage = self
            .semantic_callable(&callable.name)?
            .chains
            .get(stage_use.chain_index)?
            .stages
            .get(stage_use.stage_index)?;
        Some(StageHover {
            label: stage.label.clone(),
            input: stage.input.clone(),
            output: stage.output.clone(),
            is_arrow: stage_use.arrow_range.contains(position),
        })
    }

    fn semantic_source_at(
        &self,
        callable: &CallableScope,
        position: Position,
    ) -> Option<&typecheck::EndpointSummary> {
        let source = callable
            .sources
            .iter()
            .find(|source| source.range.contains(position))?;
        self.semantic_callable(&callable.name)?
            .chains
            .get(source.chain_index)
            .map(|chain| &chain.source)
    }

    fn local_symbol(&self, name: &str) -> Option<&Symbol> {
        self.symbols.iter().find(|symbol| {
            symbol.name == name
                && matches!(
                    symbol.kind,
                    SymbolKind::Node | SymbolKind::Program | SymbolKind::Type
                )
        })
    }

    fn imported_symbol(&self, name: &str) -> Option<ImportedDefinition> {
        for import in &self.imports {
            match &import.source {
                ImportSourceInfo::Module(module) => {
                    if let Some((alias, range)) = &import.alias
                        && let Some(member) = name.strip_prefix(&format!("{alias}."))
                    {
                        if stdlib::find_export(module, member).is_some() {
                            return Some(ImportedDefinition::Range(*range));
                        }
                    }
                    for item in &import.items {
                        if item.local == name {
                            return Some(ImportedDefinition::Range(
                                item.alias_range.unwrap_or(item.range),
                            ));
                        }
                    }
                }
                ImportSourceInfo::Local(path, path_range) => {
                    let base = uri_to_path(&self.uri)
                        .and_then(|path| path.parent().map(Path::to_path_buf))
                        .unwrap_or_else(|| PathBuf::from("."));
                    let full_path = base.join(path);
                    if let Some((alias, _)) = &import.alias
                        && let Some(member) = name.strip_prefix(&format!("{alias}."))
                    {
                        if let Some(location) = local_import_location(&full_path, member) {
                            return Some(ImportedDefinition::Location(location));
                        }
                    }
                    for item in &import.items {
                        if item.local == name {
                            if let Some(location) =
                                local_import_location(&full_path, &item.imported)
                            {
                                return Some(ImportedDefinition::Location(location));
                            }
                            return Some(ImportedDefinition::Range(
                                item.alias_range.unwrap_or(item.range),
                            ));
                        }
                    }
                    if name == path {
                        return Some(ImportedDefinition::Range(*path_range));
                    }
                }
            }
        }
        None
    }

    fn diagnostic_range(&self, message: &str) -> Option<Range> {
        let mut names = backtick_items(message);
        if message.contains("does not export") {
            names.reverse();
        }
        for name in names {
            if name.contains(' ') || name.contains('[') || name.contains(']') {
                continue;
            }
            if let Some(range) = self.range_for_name(&name) {
                return Some(range);
            }
        }
        self.symbols
            .iter()
            .find(|symbol| message.contains(&symbol.name))
            .map(|symbol| symbol.selection_range)
    }

    fn range_for_name(&self, name: &str) -> Option<Range> {
        if let Some(symbol) = self.symbols.iter().find(|symbol| symbol.name == name) {
            return Some(symbol.selection_range);
        }
        for callable in &self.callables {
            if let Some(variable) = callable
                .variables
                .iter()
                .find(|variable| variable.name == name || format!("${}", variable.name) == name)
            {
                return Some(variable.range);
            }
        }
        for import in &self.imports {
            if let Some(range) = import.range_for_name(name) {
                return Some(range);
            }
        }
        self.token_range_for_name(name)
    }

    fn token_range_for_name(&self, name: &str) -> Option<Range> {
        for index in 0..self.tokens.len() {
            match &self.tokens[index].kind {
                TokenKind::Variable(variable)
                    if variable == name || format!("${variable}") == name =>
                {
                    return Some(self.tokens[index].range);
                }
                TokenKind::Ident(ident) if ident == name => return Some(self.tokens[index].range),
                TokenKind::Ident(_) => {
                    if let Some((qualified, range)) = self.qualified_name_range_around(index)
                        && qualified == name
                    {
                        return Some(range);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn completion_symbols(&self) -> Vec<Completion> {
        let mut completions = Vec::new();
        for keyword in [
            "import", "type", "struct", "foreign", "pure", "io", "module", "global", "header",
            "source", "c", "js", "extern", "node", "program", "map", "filter", "field", "repeat",
            "reduce", "scan", "match", "fault", "ok", "identity", "true", "false",
        ] {
            completions.push(Completion {
                label: keyword.to_string(),
                kind: CompletionKind::Keyword,
                detail: "keyword".to_string(),
            });
        }
        for ty in [
            "Unit",
            "Int",
            "Real",
            "Bool",
            "Bytes",
            "Fault",
            "Seq[]",
            "Faultable[]",
        ] {
            completions.push(Completion {
                label: ty.to_string(),
                kind: CompletionKind::Type,
                detail: "type".to_string(),
            });
        }
        for module in stdlib_modules() {
            completions.push(Completion {
                label: module,
                kind: CompletionKind::Module,
                detail: "stdlib module".to_string(),
            });
        }
        for symbol in &self.symbols {
            completions.push(Completion {
                label: symbol.name.clone(),
                kind: match symbol.kind {
                    SymbolKind::Type => CompletionKind::Type,
                    SymbolKind::Node | SymbolKind::Program => CompletionKind::Function,
                    SymbolKind::Import => CompletionKind::Reference,
                },
                detail: symbol.detail.clone(),
            });
        }
        for callable in &self.callables {
            for variable in &callable.variables {
                let detail = self
                    .semantic_variable_type(&callable.name, &variable.name)
                    .unwrap_or(&variable.detail);
                completions.push(Completion {
                    label: format!("${}", variable.name),
                    kind: CompletionKind::Variable,
                    detail: format!("{} {detail}", callable.name),
                });
            }
        }
        for import in &self.imports {
            match &import.source {
                ImportSourceInfo::Module(module) => {
                    if let Some((alias, _)) = &import.alias {
                        for symbol in stdlib::module_symbols(module) {
                            completions.push(Completion {
                                label: format!("{alias}.{}", symbol.name),
                                kind: completion_kind_for_stdlib(symbol.kind),
                                detail: stdlib_detail(symbol),
                            });
                        }
                    }
                    for item in &import.items {
                        if let Some(symbol) = stdlib::find_export(module, &item.imported) {
                            completions.push(Completion {
                                label: item.local.clone(),
                                kind: completion_kind_for_stdlib(symbol.kind),
                                detail: stdlib_detail(symbol),
                            });
                        }
                    }
                }
                ImportSourceInfo::Local(_, _) => {
                    for item in &import.items {
                        completions.push(Completion {
                            label: item.local.clone(),
                            kind: CompletionKind::Function,
                            detail: format!("import {}", item.imported),
                        });
                    }
                }
            }
        }
        sort_dedup_completions(completions)
    }

    fn ident_at(&self, index: usize) -> Option<&str> {
        match self.tokens.get(index).map(|token| &token.kind) {
            Some(TokenKind::Ident(name)) => Some(name.as_str()),
            _ => None,
        }
    }

    fn next_ident(&self, index: usize) -> Option<(String, Range, usize)> {
        match self.tokens.get(index) {
            Some(LspToken {
                kind: TokenKind::Ident(name),
                range,
            }) => Some((name.clone(), *range, index + 1)),
            _ => None,
        }
    }

    fn is_symbol(&self, index: usize, expected: char) -> bool {
        matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Symbol(ch)) if *ch == expected
        )
    }

    fn matching_brace(&self, open_index: usize) -> Option<usize> {
        if !self.is_symbol(open_index, '{') {
            return None;
        }
        let mut depth = 0usize;
        for index in open_index..self.tokens.len() {
            match self.tokens[index].kind {
                TokenKind::Symbol('{') => depth += 1,
                TokenKind::Symbol('}') => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn matching_delimiter(&self, open_index: usize) -> Option<usize> {
        let (open, close) = match self.tokens.get(open_index).map(|token| &token.kind) {
            Some(TokenKind::Symbol('(')) => ('(', ')'),
            Some(TokenKind::Symbol('[')) => ('[', ']'),
            Some(TokenKind::Symbol('{')) => ('{', '}'),
            _ => return None,
        };
        let mut depth = 0usize;
        for index in open_index..self.tokens.len() {
            match self.tokens[index].kind {
                TokenKind::Symbol(ch) if ch == open => depth += 1,
                TokenKind::Symbol(ch) if ch == close => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn qualified_name_at(&self, start: usize) -> Option<(String, usize)> {
        let mut index = start;
        let mut name = self.ident_at(index)?.to_string();
        index += 1;
        while self.is_symbol(index, '.') {
            let Some(part) = self.ident_at(index + 1) else {
                break;
            };
            name.push('.');
            name.push_str(part);
            index += 2;
        }
        Some((name, index))
    }

    fn qualified_name_around(&self, index: usize) -> Option<(String, usize)> {
        let mut start = index;
        while start >= 2 && self.is_symbol(start - 1, '.') && self.ident_at(start - 2).is_some() {
            start -= 2;
        }
        self.qualified_name_at(start)
    }

    fn qualified_name_range_around(&self, index: usize) -> Option<(String, Range)> {
        let mut start = index;
        while start >= 2 && self.is_symbol(start - 1, '.') && self.ident_at(start - 2).is_some() {
            start -= 2;
        }
        let (name, end) = self.qualified_name_at(start)?;
        let end_token = end.checked_sub(1)?;
        Some((
            name,
            Range {
                start: self.tokens[start].range.start,
                end: self.tokens[end_token].range.end,
            },
        ))
    }

    fn callable_detail(&self, keyword_index: usize, name: &str) -> String {
        let mut index = keyword_index;
        let mut out = Vec::new();
        while index < self.tokens.len() {
            out.push(self.token_text(index));
            if self.is_symbol(index, '{') {
                break;
            }
            index += 1;
        }
        let detail = out.join(" ");
        if detail.is_empty() {
            name.to_string()
        } else {
            detail
        }
    }

    fn detail_until(&self, start: usize, stop_keywords: &[&str]) -> String {
        let mut index = start;
        let mut out = Vec::new();
        while index < self.tokens.len() {
            if let Some(ident) = self.ident_at(index)
                && stop_keywords.contains(&ident)
            {
                break;
            }
            out.push(self.token_text(index));
            index += 1;
        }
        out.join(" ")
    }

    fn type_text(&self, start: usize, end: usize) -> String {
        (start..end)
            .map(|index| self.token_text(index))
            .collect::<Vec<_>>()
            .join("")
    }

    fn token_text(&self, index: usize) -> String {
        match &self.tokens[index].kind {
            TokenKind::Ident(name) => name.clone(),
            TokenKind::Variable(name) => format!("${name}"),
            TokenKind::String(value) => format!("{value:?}"),
            TokenKind::Int => "0".to_string(),
            TokenKind::Real => "0.0".to_string(),
            TokenKind::Bool => "true".to_string(),
            TokenKind::Arrow => "->".to_string(),
            TokenKind::Symbol(ch) => ch.to_string(),
        }
    }
}

enum SymbolRef {
    Name(String),
    Variable(String),
    ImportPath(String),
}

enum ImportedDefinition {
    Range(Range),
    Location(Location),
}

#[derive(Debug, Clone)]
struct Location {
    uri: String,
    range: Range,
}

#[derive(Debug, Clone)]
struct Completion {
    label: String,
    kind: CompletionKind,
    detail: String,
}

#[derive(Debug, Clone, Copy)]
enum CompletionKind {
    Function,
    Keyword,
    Type,
    Module,
    Reference,
    Variable,
}

fn lex_lsp(source: &str) -> Vec<LspToken> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut pos = 0usize;
    let mut position = Position::default();
    while pos < chars.len() {
        let ch = chars[pos];
        match ch {
            ' ' | '\t' | '\r' | '\n' => advance(ch, &mut position, &mut pos),
            '#' => {
                while pos < chars.len() && chars[pos] != '\n' {
                    advance(chars[pos], &mut position, &mut pos);
                }
            }
            '/' if chars.get(pos + 1) == Some(&'*') => {
                advance('/', &mut position, &mut pos);
                advance('*', &mut position, &mut pos);
                while pos + 1 < chars.len() {
                    if chars[pos] == '*' && chars[pos + 1] == '/' {
                        advance('*', &mut position, &mut pos);
                        advance('/', &mut position, &mut pos);
                        break;
                    }
                    advance(chars[pos], &mut position, &mut pos);
                }
            }
            '-' if chars.get(pos + 1) == Some(&'>') => {
                let start = position;
                advance('-', &mut position, &mut pos);
                advance('>', &mut position, &mut pos);
                tokens.push(LspToken {
                    kind: TokenKind::Arrow,
                    range: Range {
                        start,
                        end: position,
                    },
                });
            }
            '"' => tokens.push(lex_lsp_string(&chars, &mut pos, &mut position)),
            '$' => {
                let start = position;
                advance('$', &mut position, &mut pos);
                let name_start = pos;
                while pos < chars.len() && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_')
                {
                    advance(chars[pos], &mut position, &mut pos);
                }
                tokens.push(LspToken {
                    kind: TokenKind::Variable(chars[name_start..pos].iter().collect()),
                    range: Range {
                        start,
                        end: position,
                    },
                });
            }
            '-' | '0'..='9' => tokens.push(lex_lsp_number(&chars, &mut pos, &mut position)),
            ch if ch.is_ascii_alphabetic() || ch == '_' => {
                let start = position;
                let name_start = pos;
                while pos < chars.len() && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_')
                {
                    advance(chars[pos], &mut position, &mut pos);
                }
                let text: String = chars[name_start..pos].iter().collect();
                let kind = match text.as_str() {
                    "true" => TokenKind::Bool,
                    "false" => TokenKind::Bool,
                    _ => TokenKind::Ident(text),
                };
                tokens.push(LspToken {
                    kind,
                    range: Range {
                        start,
                        end: position,
                    },
                });
            }
            '(' | ')' | '{' | '}' | '[' | ']' | '<' | '>' | ',' | ':' | '=' | '|' | '.' => {
                let start = position;
                advance(ch, &mut position, &mut pos);
                tokens.push(LspToken {
                    kind: TokenKind::Symbol(ch),
                    range: Range {
                        start,
                        end: position,
                    },
                });
            }
            other => advance(other, &mut position, &mut pos),
        }
    }
    tokens
}

fn lex_lsp_string(chars: &[char], pos: &mut usize, position: &mut Position) -> LspToken {
    let start = *position;
    advance('"', position, pos);
    let mut value = String::new();
    while *pos < chars.len() {
        let ch = chars[*pos];
        advance(ch, position, pos);
        match ch {
            '"' => break,
            '\\' if *pos < chars.len() => {
                let escaped = chars[*pos];
                advance(escaped, position, pos);
                value.push(match escaped {
                    '"' => '"',
                    '\\' => '\\',
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                });
            }
            other => value.push(other),
        }
    }
    LspToken {
        kind: TokenKind::String(value),
        range: Range {
            start,
            end: *position,
        },
    }
}

fn lex_lsp_number(chars: &[char], pos: &mut usize, position: &mut Position) -> LspToken {
    let start = *position;
    if chars.get(*pos) == Some(&'-') {
        advance('-', position, pos);
    }
    while matches!(chars.get(*pos), Some(ch) if ch.is_ascii_digit()) {
        advance(chars[*pos], position, pos);
    }
    let mut is_real = false;
    if chars.get(*pos) == Some(&'.')
        && matches!(chars.get(*pos + 1), Some(ch) if ch.is_ascii_digit())
    {
        is_real = true;
        advance('.', position, pos);
        while matches!(chars.get(*pos), Some(ch) if ch.is_ascii_digit()) {
            advance(chars[*pos], position, pos);
        }
    }
    LspToken {
        kind: if is_real {
            TokenKind::Real
        } else {
            TokenKind::Int
        },
        range: Range {
            start,
            end: *position,
        },
    }
}

fn advance(ch: char, position: &mut Position, pos: &mut usize) {
    *pos += 1;
    if ch == '\n' {
        position.line += 1;
        position.character = 0;
    } else {
        position.character += 1;
    }
}

fn completion_result(analysis: &Analysis) -> String {
    format!(
        "[{}]",
        analysis
            .completion_symbols()
            .into_iter()
            .map(completion_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn completion_json(completion: Completion) -> String {
    format!(
        "{{\"label\":{},\"kind\":{},\"detail\":{}}}",
        json_string(&completion.label),
        completion_kind_number(completion.kind),
        json_string(&completion.detail)
    )
}

fn definition_result(analysis: &Analysis, position: Position) -> Option<String> {
    match analysis.symbol_at(position)? {
        SymbolRef::Variable(name) => {
            let callable = analysis.callable_at(position)?;
            let variable = callable.variables.iter().rev().find(|variable| {
                variable.name == name && compare_positions(variable.range.start, position) <= 0
            })?;
            Some(location_json(&analysis.uri, variable.range))
        }
        SymbolRef::Name(name) => {
            if let Some(symbol) = analysis.local_symbol(&name) {
                return Some(location_json(&symbol.uri, symbol.selection_range));
            }
            match analysis.imported_symbol(&name)? {
                ImportedDefinition::Range(range) => Some(location_json(&analysis.uri, range)),
                ImportedDefinition::Location(location) => {
                    Some(location_json(&location.uri, location.range))
                }
            }
        }
        SymbolRef::ImportPath(path) => {
            let base = uri_to_path(&analysis.uri)
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."));
            let full_path = base.join(path);
            Some(location_json(
                &path_to_uri(&full_path),
                Range::point(Position::default()),
            ))
        }
    }
}

fn hover_result(analysis: &Analysis, position: Position) -> Option<String> {
    if let Some(callable) = analysis.callable_at(position) {
        if let Some(stage) = analysis.semantic_stage_at(callable, position) {
            if stage.is_arrow || (!stage.label.starts_with('$') && !stage.label.starts_with('(')) {
                return Some(hover_json(&format!(
                    "{}: {} -> {}",
                    stage.label, stage.input, stage.output
                )));
            }
        }
        if let Some(source) = analysis.semantic_source_at(callable, position) {
            return Some(hover_json(&format!("{}: {}", source.label, source.ty)));
        }
    }

    let contents = match analysis.symbol_at(position)? {
        SymbolRef::Variable(name) => {
            let callable = analysis.callable_at(position)?;
            let variable = callable
                .variables
                .iter()
                .rev()
                .find(|variable| variable.name == name)?;
            let ty = analysis
                .semantic_variable_type(&callable.name, &variable.name)
                .unwrap_or(&variable.detail);
            format!("${}: {}", variable.name, ty)
        }
        SymbolRef::Name(name) => {
            if let Some(symbol) = analysis.local_symbol(&name) {
                symbol.detail.clone()
            } else if let Some(detail) = imported_detail(analysis, &name) {
                detail
            } else {
                return None;
            }
        }
        SymbolRef::ImportPath(path) => format!("local import {path:?}"),
    };
    Some(hover_json(&contents))
}

fn hover_json(contents: &str) -> String {
    format!(
        "{{\"contents\":{{\"kind\":\"markdown\",\"value\":{}}}}}",
        json_string(&format!("```flow\n{contents}\n```"))
    )
}

fn inlay_hints_result(analysis: &Analysis, requested_range: Option<Range>) -> String {
    let mut hints = Vec::new();
    for callable in &analysis.callables {
        for variable in &callable.variables {
            if variable.detail != "value" {
                continue;
            }
            if let Some(range) = requested_range
                && !range.contains(variable.range.start)
            {
                continue;
            }
            let Some(ty) = analysis.semantic_variable_type(&callable.name, &variable.name) else {
                continue;
            };
            hints.push(format!(
                "{{\"position\":{},\"label\":{},\"kind\":1,\"paddingLeft\":false,\"paddingRight\":true}}",
                position_json(variable.range.end),
                json_string(&format!(": {ty}"))
            ));
        }
    }
    format!("[{}]", hints.join(","))
}

fn document_symbols_result(analysis: &Analysis) -> String {
    format!(
        "[{}]",
        analysis
            .symbols
            .iter()
            .filter(|symbol| symbol.kind != SymbolKind::Import)
            .map(document_symbol_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn document_symbol_json(symbol: &Symbol) -> String {
    format!(
        "{{\"name\":{},\"kind\":{},\"range\":{},\"selectionRange\":{},\"detail\":{}}}",
        json_string(&symbol.name),
        symbol_kind_number(symbol.kind),
        range_json(symbol.range),
        range_json(symbol.selection_range),
        json_string(&symbol.detail)
    )
}

fn imported_detail(analysis: &Analysis, name: &str) -> Option<String> {
    for import in &analysis.imports {
        let ImportSourceInfo::Module(module) = &import.source else {
            continue;
        };
        if let Some((alias, _)) = &import.alias
            && let Some(member) = name.strip_prefix(&format!("{alias}."))
            && let Some(symbol) = stdlib::find_export(module, member)
        {
            return Some(stdlib_detail(symbol));
        }
        for item in &import.items {
            if item.local == name
                && let Some(symbol) = stdlib::find_export(module, &item.imported)
            {
                return Some(stdlib_detail(symbol));
            }
        }
    }
    None
}

fn local_import_location(path: &Path, name: &str) -> Option<Location> {
    let source = fs::read_to_string(path).ok()?;
    let analysis = Analysis::new(path_to_uri(path), source);
    let symbol = analysis.local_symbol(name)?;
    Some(Location {
        uri: symbol.uri.clone(),
        range: symbol.selection_range,
    })
}

fn sort_dedup_completions(mut completions: Vec<Completion>) -> Vec<Completion> {
    completions.sort_by(|left, right| left.label.cmp(&right.label));
    completions.dedup_by(|left, right| left.label == right.label);
    completions
}

fn stdlib_modules() -> Vec<String> {
    let mut modules = stdlib::all_symbols()
        .map(|symbol| symbol.module.to_string())
        .filter(|module| module != stdlib::INTRINSIC_MODULE)
        .collect::<Vec<_>>();
    modules.sort();
    modules.dedup();
    modules
}

fn stdlib_detail(symbol: &stdlib::StdSymbol) -> String {
    match symbol.kind {
        stdlib::SymbolKind::Type => format!("type {}.{}", symbol.module, symbol.name),
        stdlib::SymbolKind::Node => format!(
            "{}.{}: {} -> {}",
            symbol.module,
            symbol.name,
            symbol.input.unwrap_or("()"),
            symbol.output.unwrap_or("()")
        ),
    }
}

fn completion_kind_for_stdlib(kind: stdlib::SymbolKind) -> CompletionKind {
    match kind {
        stdlib::SymbolKind::Type => CompletionKind::Type,
        stdlib::SymbolKind::Node => CompletionKind::Function,
    }
}

fn completion_kind_number(kind: CompletionKind) -> u8 {
    match kind {
        CompletionKind::Function => 3,
        CompletionKind::Keyword => 14,
        CompletionKind::Type => 7,
        CompletionKind::Module => 9,
        CompletionKind::Reference => 18,
        CompletionKind::Variable => 6,
    }
}

fn symbol_kind_number(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Type => 5,
        SymbolKind::Node => 12,
        SymbolKind::Program => 12,
        SymbolKind::Import => 13,
    }
}

fn location_json(uri: &str, range: Range) -> String {
    format!(
        "{{\"uri\":{},\"range\":{}}}",
        json_string(uri),
        range_json(range)
    )
}

fn range_json(range: Range) -> String {
    format!(
        "{{\"start\":{},\"end\":{}}}",
        position_json(range.start),
        position_json(range.end)
    )
}

fn source_span_to_range(span: SourceSpan) -> Range {
    Range {
        start: Position {
            line: span.start.line,
            character: span.start.character,
        },
        end: Position {
            line: span.end.line,
            character: span.end.character,
        },
    }
}

fn position_json(position: Position) -> String {
    format!(
        "{{\"line\":{},\"character\":{}}}",
        position.line, position.character
    )
}

fn backtick_items(message: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = message;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        items.push(after_start[..end].to_string());
        rest = &after_start[end + 1..];
    }
    items
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(path)))
}

fn path_to_uri(path: &Path) -> String {
    format!("file://{}", percent_encode(&path.to_string_lossy()))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'%'
            && pos + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[pos + 1..pos + 3])
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            pos += 3;
            continue;
        }
        out.push(bytes[pos]);
        pos += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_finds_local_definitions_and_variables() {
        let source = r#"
            import std.math { add }

            node inc(value: Int) -> out: Int {
                ($value, 1) -> add -> $out
            }

            program main(args: Args) -> exit_code: Int {
                0 -> inc -> $exit_code
            }
        "#;
        let analysis = Analysis::new("file:///tmp/main.flow".to_string(), source.to_string());
        assert!(analysis.local_symbol("inc").is_some());
        assert!(
            analysis
                .completion_symbols()
                .iter()
                .any(|item| item.label == "add")
        );
        assert!(
            analysis
                .completion_symbols()
                .iter()
                .any(|item| item.label == "$exit_code")
        );
        let callable = analysis
            .callables
            .iter()
            .find(|callable| callable.name == "inc")
            .expect("inc scope");
        assert!(
            callable
                .variables
                .iter()
                .any(|variable| variable.name == "value")
        );
        assert!(
            callable
                .variables
                .iter()
                .any(|variable| variable.name == "out")
        );
    }

    #[test]
    fn json_parser_reads_request_objects() {
        let parsed = JsonParser::new(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"x":true}}"#,
        )
        .parse()
        .expect("parse");
        assert_eq!(
            parsed.get("method").and_then(Json::as_str),
            Some("initialize")
        );
        assert_eq!(parsed.get("id").and_then(Json::as_u32), Some(1));
    }

    #[test]
    fn parser_diagnostics_use_source_spans() {
        let error =
            parser::parse_diagnostic("program main(args: Args) -> exit_code: Int {\n    @\n}\n")
                .expect_err("parse should fail");
        assert_eq!(error.span.start.line, 1);
        assert_eq!(error.span.start.character, 4);
    }

    #[test]
    fn lsp_typecheck_diagnostics_point_at_relevant_token() {
        let source = r#"import std.bytes { missing }
import std.cli { Args }

program main(args: Args) -> exit_code: Int {
    0 -> $exit_code
}
"#;
        let diagnostics = diagnostics_for("file:///tmp/main.flow", source);
        let parsed = JsonParser::new(&diagnostics)
            .parse()
            .expect("parse diagnostics");
        let first = parsed
            .as_array()
            .and_then(|items| items.first())
            .expect("diagnostic");
        let start = first
            .get("range")
            .and_then(|range| range.get("start"))
            .expect("range start");
        assert_eq!(start.get("line").and_then(Json::as_u32), Some(0));
        assert_eq!(start.get("character").and_then(Json::as_u32), Some(19));
    }

    #[test]
    fn hover_uses_concrete_typecheck_types() {
        let source = r#"import std.cli { Args }
import std.int { parse_int }

program main(args: Args) -> exit_code: Int {
    ["1"] -> map parse_int -> $numbers
    0 -> $exit_code
}
"#;
        let analysis = Analysis::new("file:///tmp/main.flow".to_string(), source.to_string());

        let parse_int_hover =
            hover_result(&analysis, position_of(source, "parse_int ->")).expect("stage hover");
        assert!(parse_int_hover.contains("map parse_int: Seq[Bytes] -> Seq[Faultable[Int]]"));

        let numbers_hover =
            hover_result(&analysis, position_of(source, "$numbers")).expect("variable hover");
        assert!(numbers_hover.contains("$numbers: Seq[Faultable[Int]]"));

        let source_hover =
            hover_result(&analysis, position_of(source, "[\"1\"]")).expect("source hover");
        assert!(source_hover.contains(r#"[\"1\"]: Seq[Bytes]"#));
    }

    #[test]
    fn lsp_infers_destructured_binding_types() {
        let source = r#"import std.cli { Args }

node pair(input: Int) -> out: Faultable[(Int, Bytes)] {
    ($input, "x") -> $out
}

program main(args: Args) -> exit_code: Faultable[Int] {
    1 -> pair -> ($left, $right)
    $left -> $exit_code
}
"#;
        let analysis = Analysis::new("file:///tmp/main.flow".to_string(), source.to_string());
        let callable = analysis
            .callables
            .iter()
            .find(|callable| callable.name == "main")
            .expect("main scope");
        assert!(
            callable
                .variables
                .iter()
                .any(|variable| variable.name == "left")
        );
        assert!(
            callable
                .variables
                .iter()
                .any(|variable| variable.name == "right")
        );

        let right_hover =
            hover_result(&analysis, position_of(source, "$right")).expect("right hover");
        assert!(right_hover.contains("$right: Faultable[Bytes]"));

        let arrow_hover =
            hover_result(&analysis, position_of(source, "-> ($left")).expect("arrow hover");
        assert!(
            arrow_hover
                .contains("($left, $right): Faultable[(Int,Bytes)] -> Faultable[(Int,Bytes)]")
        );

        let right_completion = analysis
            .completion_symbols()
            .into_iter()
            .find(|completion| completion.label == "$right")
            .expect("right completion");
        assert!(right_completion.detail.contains("Faultable[Bytes]"));

        let hints = inlay_hints_result(&analysis, None);
        assert!(hints.contains(": Faultable[Int]"));
        assert!(hints.contains(": Faultable[Bytes]"));
    }

    #[test]
    fn lsp_summarizes_library_modules_for_destructured_repeat_bindings() {
        let source = r#"import std.math { add }

extern node fib(depth: Int) -> result: Int {
    (0, 1) -> repeat<$depth> _fib_step -> ($result, $)
}

node _fib_step(a: Int, b: Int) -> (next_a: Int, next_b: Int) {
    $b       -> $next_a
    ($a, $b) -> add -> $next_b
}
"#;
        let analysis = Analysis::new("file:///tmp/fib.flow".to_string(), source.to_string());

        let result_hover =
            hover_result(&analysis, position_of(source, "$result")).expect("result hover");
        assert!(result_hover.contains("$result: Int"));

        let repeat_hover =
            hover_result(&analysis, position_of(source, "_fib_step ->")).expect("repeat hover");
        assert!(repeat_hover.contains("repeat _fib_step: (Int,Int) -> (Int,Int)"));

        let completions = analysis.completion_symbols();
        let result_completion = completions
            .iter()
            .find(|completion| completion.label == "$result")
            .expect("result completion");
        assert!(result_completion.detail.contains("Int"));
        assert!(!completions.iter().any(|completion| completion.label == "$"));

        let hints = inlay_hints_result(&analysis, None);
        assert!(hints.contains(": Int"));
    }

    fn position_of(source: &str, needle: &str) -> Position {
        let offset = source.find(needle).expect("needle");
        let mut line = 0u32;
        let mut character = 0u32;
        for ch in source[..offset].chars() {
            if ch == '\n' {
                line += 1;
                character = 0;
            } else {
                character += 1;
            }
        }
        Position { line, character }
    }
}
