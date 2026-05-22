use crate::ast::*;
use crate::diagnostic::{SourceDiagnostic, SourceSpan};
use crate::lexer::{SpannedToken, Token, lex_spanned};
use crate::node_ref::format_static_node_ref;

pub fn parse(source: &str) -> Result<Module, String> {
    parse_diagnostic(source).map_err(|error| error.message)
}

pub fn parse_diagnostic(source: &str) -> Result<Module, SourceDiagnostic> {
    Parser {
        tokens: lex_spanned(source)?,
        pos: 0,
    }
    .parse_module()
}

struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl Parser {
    fn parse_module(&mut self) -> Result<Module, SourceDiagnostic> {
        let mut declarations = Vec::new();
        while !self.at_eof() {
            declarations.push(self.parse_decl()?);
        }
        Ok(Module { declarations })
    }

    fn parse_decl(&mut self) -> Result<Decl, SourceDiagnostic> {
        match self.peek_ident() {
            Some("type") => {
                self.bump();
                Ok(Decl::TypeAlias(self.parse_type_alias()?))
            }
            Some("struct") => {
                self.bump();
                Ok(Decl::Struct(self.parse_struct_decl()?))
            }
            Some("import") => {
                self.bump();
                Ok(Decl::Import(self.parse_import()?))
            }
            Some("foreign") => {
                self.bump();
                Ok(Decl::Foreign(self.parse_foreign()?))
            }
            Some("extern") => {
                self.bump();
                if self.peek_ident() != Some("node") {
                    return Err(self.error_here("expected `node` after `extern`"));
                }
                self.bump();
                Ok(Decl::Node(self.parse_callable(true)?))
            }
            Some("node") => {
                self.bump();
                Ok(Decl::Node(self.parse_callable(false)?))
            }
            Some("program") => {
                self.bump();
                Ok(Decl::Program(self.parse_callable(false)?))
            }
            _ => Err(self.error_here(format!("expected declaration, found {:?}", self.peek()))),
        }
    }

    fn parse_type_alias(&mut self) -> Result<TypeAlias, SourceDiagnostic> {
        let name = self.expect_ident()?;
        self.expect(Token::Equal)?;
        let ty = self.parse_type_name()?;
        Ok(TypeAlias { name, ty })
    }

    fn parse_struct_decl(&mut self) -> Result<StructDecl, SourceDiagnostic> {
        let name = self.expect_ident()?;
        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        if self.eat(Token::RBrace) {
            return Err(self.error_here("struct declaration cannot be empty"));
        }
        loop {
            fields.push(self.parse_port()?);
            if self.eat(Token::Comma) {
                if self.eat(Token::RBrace) {
                    break;
                }
            } else {
                self.expect(Token::RBrace)?;
                break;
            }
        }
        Ok(StructDecl { name, fields })
    }

    fn parse_import(&mut self) -> Result<Import, SourceDiagnostic> {
        let source = match self.peek().clone() {
            Token::String(path) => {
                self.bump();
                ImportSource::Local(path)
            }
            Token::Ident(_) => ImportSource::Module(self.parse_qualified_ident()?),
            other => {
                return Err(self.error_here(format!("expected import source, found {other:?}")));
            }
        };
        let clause = match self.peek_ident() {
            Some("as") => {
                self.bump();
                ImportClause::Alias(self.expect_ident()?)
            }
            _ => {
                self.expect(Token::LBrace)?;
                let mut items = Vec::new();
                loop {
                    let name = self.expect_ident()?;
                    let alias = if self.peek_ident() == Some("as") {
                        self.bump();
                        Some(self.expect_ident()?)
                    } else {
                        None
                    };
                    items.push(ImportItem { name, alias });
                    if self.eat(Token::Comma) {
                        if self.eat(Token::RBrace) {
                            break;
                        }
                    } else {
                        self.expect(Token::RBrace)?;
                        break;
                    }
                }
                ImportClause::Items(items)
            }
        };
        Ok(Import { source, clause })
    }

    fn parse_foreign(&mut self) -> Result<ForeignBlock, SourceDiagnostic> {
        let target = match self.expect_ident()?.as_str() {
            "js" => ForeignTarget::Js,
            other => {
                return Err(self.error_here(format!(
                    "unsupported foreign target `{other}`; expected `js`"
                )));
            }
        };
        let source = match self.expect_ident()?.as_str() {
            "module" => ForeignSource::Module(self.expect_string()?),
            "global" => ForeignSource::Global(self.expect_string()?),
            other => {
                return Err(self.error_here(format!(
                    "expected foreign source kind `module` or `global`, found `{other}`"
                )));
            }
        };
        self.expect(Token::LBrace)?;
        let mut nodes = Vec::new();
        while !self.eat(Token::RBrace) {
            nodes.push(self.parse_foreign_node()?);
        }
        if nodes.is_empty() {
            return Err(self.error_here("foreign block cannot be empty"));
        }
        Ok(ForeignBlock {
            target,
            source,
            nodes,
        })
    }

    fn parse_foreign_node(&mut self) -> Result<ForeignNode, SourceDiagnostic> {
        let effect = match self.expect_ident()?.as_str() {
            "pure" => ForeignEffect::Pure,
            "io" => ForeignEffect::Io,
            other => {
                return Err(self.error_here(format!(
                    "expected foreign node effect `pure` or `io`, found `{other}`"
                )));
            }
        };
        self.expect_keyword("node")?;
        let name = self.expect_ident()?;
        self.expect(Token::LParen)?;
        let inputs = if self.eat(Token::RParen) {
            Vec::new()
        } else {
            let ports = self.parse_port_list()?;
            self.expect(Token::RParen)?;
            ports
        };
        self.expect(Token::Arrow)?;
        let outputs = self.parse_port_or_list()?;
        self.expect(Token::Equal)?;
        let symbol = self.parse_qualified_ident()?;
        Ok(ForeignNode {
            name,
            effect,
            inputs,
            outputs,
            symbol,
        })
    }

    fn parse_callable(&mut self, is_extern: bool) -> Result<Callable, SourceDiagnostic> {
        let name = self.expect_ident()?;
        let node_params = self.parse_node_params()?;
        self.expect(Token::LParen)?;
        let inputs = if self.eat(Token::RParen) {
            Vec::new()
        } else {
            let ports = self.parse_port_list()?;
            self.expect(Token::RParen)?;
            ports
        };
        self.expect(Token::Arrow)?;
        let outputs = self.parse_port_or_list()?;
        let chains = self.parse_block()?;
        Ok(Callable {
            name,
            is_extern,
            node_params,
            inputs,
            outputs,
            chains,
        })
    }

    fn parse_node_params(&mut self) -> Result<Vec<NodeParam>, SourceDiagnostic> {
        if !self.eat(Token::Less) {
            return Ok(Vec::new());
        }
        let mut params = Vec::new();
        if self.eat(Token::Greater) {
            return Err(self.error_here("static node parameter list cannot be empty"));
        }
        loop {
            let name = self.expect_ident()?;
            self.expect(Token::Colon)?;
            self.expect_keyword("node")?;
            self.expect(Token::LParen)?;
            let input = if self.eat(Token::RParen) {
                "()".to_string()
            } else {
                let input = self.parse_type_name()?;
                self.expect(Token::RParen)?;
                input
            };
            self.expect(Token::Arrow)?;
            let output = self.parse_type_name()?;
            params.push(NodeParam {
                name,
                input,
                output,
            });
            if self.eat(Token::Comma) {
                if self.eat(Token::Greater) {
                    break;
                }
            } else {
                self.expect(Token::Greater)?;
                break;
            }
        }
        Ok(params)
    }

    fn parse_port_or_list(&mut self) -> Result<Vec<Port>, SourceDiagnostic> {
        if self.eat(Token::LParen) {
            let ports = self.parse_port_list()?;
            self.expect(Token::RParen)?;
            Ok(ports)
        } else {
            Ok(vec![self.parse_port()?])
        }
    }

    fn parse_port_list(&mut self) -> Result<Vec<Port>, SourceDiagnostic> {
        let mut ports = vec![self.parse_port()?];
        while self.eat(Token::Comma) {
            ports.push(self.parse_port()?);
        }
        Ok(ports)
    }

    fn parse_port(&mut self) -> Result<Port, SourceDiagnostic> {
        let name = self.expect_ident()?;
        self.expect(Token::Colon)?;
        let ty = self.parse_type_name()?;
        Ok(Port { name, ty })
    }

    fn parse_type_name(&mut self) -> Result<String, SourceDiagnostic> {
        let mut text = String::new();
        let mut depth = 0usize;
        loop {
            match self.peek().clone() {
                Token::Ident(name)
                    if !text.is_empty() && depth == 0 && is_declaration_keyword(&name) =>
                {
                    break;
                }
                Token::Ident(name) => {
                    self.bump();
                    text.push_str(&name);
                }
                Token::Int(value) => {
                    self.bump();
                    text.push_str(&value.to_string());
                }
                Token::LParen => {
                    self.bump();
                    depth += 1;
                    text.push('(');
                }
                Token::RParen if depth > 0 => {
                    self.bump();
                    depth -= 1;
                    text.push(')');
                }
                Token::LBracket => {
                    self.bump();
                    depth += 1;
                    text.push('[');
                }
                Token::RBracket if depth > 0 => {
                    self.bump();
                    depth -= 1;
                    text.push(']');
                }
                Token::Comma if depth > 0 => {
                    self.bump();
                    text.push(',');
                }
                Token::Pipe => {
                    self.bump();
                    text.push('|');
                }
                Token::Dot => {
                    self.bump();
                    text.push('.');
                }
                Token::Comma
                | Token::Equal
                | Token::RParen
                | Token::LBrace
                | Token::RBrace
                | Token::Greater
                | Token::Eof
                    if !text.is_empty() && depth == 0 =>
                {
                    break;
                }
                other if text.is_empty() => {
                    return Err(self.error_here(format!("expected type, found {other:?}")));
                }
                other => {
                    return Err(
                        self.error_here(format!("unexpected token in type `{text}`: {other:?}"))
                    );
                }
            }
        }
        if depth != 0 {
            return Err(self.error_here(format!("unterminated type `{text}`")));
        }
        Ok(text)
    }

    fn parse_block(&mut self) -> Result<Vec<Chain>, SourceDiagnostic> {
        self.expect(Token::LBrace)?;
        let mut chains = Vec::new();
        while !self.eat(Token::RBrace) {
            chains.push(self.parse_chain()?);
        }
        Ok(chains)
    }

    fn parse_chain(&mut self) -> Result<Chain, SourceDiagnostic> {
        let source = self.parse_endpoint()?;
        let mut stages = Vec::new();
        while self.eat(Token::Arrow) {
            stages.push(self.parse_stage()?);
        }
        if stages.is_empty() {
            return Err(self.error_here("chain must contain at least one `->` stage"));
        }
        Ok(Chain { source, stages })
    }

    fn parse_stage(&mut self) -> Result<Stage, SourceDiagnostic> {
        match self.peek_ident() {
            Some("map") => {
                self.bump();
                Ok(Stage::Map(self.parse_node_ref_text()?))
            }
            Some("fault") => {
                self.bump();
                self.expect_keyword("map")?;
                let node = self.parse_node_ref_text()?;
                self.expect(Token::LBrace)?;
                self.expect_keyword("ok")?;
                self.expect(Token::Arrow)?;
                let ok = self.expect_variable()?;
                self.expect(Token::Comma)?;
                self.expect_keyword("fault")?;
                self.expect(Token::Arrow)?;
                let fault = self.expect_variable()?;
                self.expect(Token::RBrace)?;
                Ok(Stage::FaultMap { node, ok, fault })
            }
            Some("filter") => {
                self.bump();
                Ok(Stage::Filter(self.parse_node_ref_text()?))
            }
            Some("field") => {
                self.bump();
                Ok(Stage::Field(self.expect_ident()?))
            }
            Some("repeat") => {
                self.bump();
                self.expect(Token::Less)?;
                let count = match self.peek().clone() {
                    Token::Variable(_) | Token::Int(_) => self.parse_endpoint()?,
                    other => {
                        return Err(
                            self.error_here(format!("expected repeat count, found {other:?}"))
                        );
                    }
                };
                self.expect(Token::Greater)?;
                let node = self.parse_node_ref_text()?;
                Ok(Stage::Repeat { count, node })
            }
            Some("reduce") => {
                self.bump();
                let op = self.parse_node_ref_text()?;
                self.expect(Token::LParen)?;
                self.expect_keyword("identity")?;
                self.expect(Token::Colon)?;
                let identity = self.parse_endpoint()?;
                self.expect(Token::RParen)?;
                Ok(Stage::Reduce { op, identity })
            }
            Some("scan") => {
                self.bump();
                let op = self.parse_node_ref_text()?;
                self.expect(Token::LParen)?;
                self.expect_keyword("identity")?;
                self.expect(Token::Colon)?;
                let identity = self.parse_endpoint()?;
                self.expect(Token::RParen)?;
                Ok(Stage::Scan { op, identity })
            }
            Some("match") => self.parse_match_stage(),
            Some(_) => Ok(Stage::Endpoint(Endpoint::Name(self.parse_node_ref_text()?))),
            _ if matches!(
                self.peek(),
                Token::Variable(_) | Token::Discard | Token::LParen
            ) =>
            {
                Ok(Stage::Bind(self.parse_binding_target()?))
            }
            _ => Ok(Stage::Endpoint(self.parse_endpoint()?)),
        }
    }

    fn parse_binding_target(&mut self) -> Result<BindingTarget, SourceDiagnostic> {
        match self.peek().clone() {
            Token::Discard => {
                self.bump();
                Ok(BindingTarget::Discard)
            }
            Token::Variable(name) => {
                self.bump();
                Ok(BindingTarget::Variable(name))
            }
            Token::LParen => self.parse_binding_tuple(),
            Token::Ident(name) => {
                Err(self.error_here(format!("expected variable `$` prefix for binding `{name}`")))
            }
            other => Err(self.error_here(format!("expected binding target, found {other:?}"))),
        }
    }

    fn parse_binding_tuple(&mut self) -> Result<BindingTarget, SourceDiagnostic> {
        self.expect(Token::LParen)?;
        let first = self.parse_binding_target()?;
        self.expect(Token::Comma)?;
        let mut items = vec![first, self.parse_binding_target()?];
        while self.eat(Token::Comma) {
            items.push(self.parse_binding_target()?);
        }
        self.expect(Token::RParen)?;
        Ok(BindingTarget::Tuple(items))
    }

    fn parse_match_stage(&mut self) -> Result<Stage, SourceDiagnostic> {
        self.expect_keyword("match")?;
        self.expect(Token::LBrace)?;
        let mut arms = Vec::new();
        let mut saw_fallback = false;
        while !self.eat(Token::RBrace) {
            if saw_fallback {
                return Err(self.error_here("`match` fallback arm must be last"));
            }
            let guard = if self.peek_ident() == Some("_") {
                self.bump();
                saw_fallback = true;
                MatchGuard::Fallback
            } else {
                let node = self.parse_node_ref_text()?;
                self.expect(Token::LParen)?;
                let mut args = Vec::new();
                if !self.eat(Token::RParen) {
                    loop {
                        args.push(self.parse_inline_endpoint()?);
                        if self.eat(Token::Comma) {
                            if self.eat(Token::RParen) {
                                break;
                            }
                        } else {
                            self.expect(Token::RParen)?;
                            break;
                        }
                    }
                }
                MatchGuard::Call { node, args }
            };
            self.expect(Token::Arrow)?;
            let target = self.parse_match_target()?;
            arms.push(MatchArm { guard, target });
        }
        if arms.is_empty() {
            return Err(self.error_here("`match` must contain at least one arm"));
        }
        if !saw_fallback {
            return Err(self.error_here("`match` must end with a `_` fallback arm"));
        }
        Ok(Stage::Match { arms })
    }

    fn parse_match_target(&mut self) -> Result<MatchTarget, SourceDiagnostic> {
        match self.peek() {
            Token::Ident(_) => Ok(MatchTarget::Node(self.parse_node_ref_text()?)),
            _ => Ok(MatchTarget::Value(self.parse_inline_endpoint()?)),
        }
    }

    fn parse_endpoint(&mut self) -> Result<Endpoint, SourceDiagnostic> {
        self.parse_endpoint_atom()
    }

    fn parse_inline_endpoint(&mut self) -> Result<Endpoint, SourceDiagnostic> {
        let source = self.parse_endpoint_atom()?;
        let mut stages = Vec::new();
        while self.eat(Token::Arrow) {
            stages.push(self.parse_stage()?);
        }
        if stages.is_empty() {
            Ok(source)
        } else {
            Ok(Endpoint::Eval {
                source: Box::new(source),
                stages,
            })
        }
    }

    fn parse_endpoint_atom(&mut self) -> Result<Endpoint, SourceDiagnostic> {
        match self.peek().clone() {
            Token::Variable(name) => {
                self.bump();
                Ok(Endpoint::Variable(name))
            }
            Token::Discard => Err(self.error_here("discard `$` is only valid as a binding target")),
            Token::Int(value) => {
                self.bump();
                Ok(Endpoint::Int(value))
            }
            Token::Real(value) => {
                self.bump();
                Ok(Endpoint::Real(value))
            }
            Token::Bool(value) => {
                self.bump();
                Ok(Endpoint::Bool(value))
            }
            Token::String(value) => {
                self.bump();
                Ok(Endpoint::String(value))
            }
            Token::LParen => self.parse_tuple_or_unit(),
            Token::LBracket => self.parse_seq(),
            Token::Ident(name) => {
                self.bump();
                if self.peek() == &Token::LBrace {
                    self.parse_struct_literal(name)
                } else {
                    Err(self.error_here(format!("expected variable `$` prefix for value `{name}`")))
                }
            }
            other => Err(self.error_here(format!("expected endpoint, found {other:?}"))),
        }
    }

    fn parse_struct_literal(&mut self, name: String) -> Result<Endpoint, SourceDiagnostic> {
        self.expect(Token::LBrace)?;
        let mut fields = Vec::new();
        if self.eat(Token::RBrace) {
            return Err(self.error_here("struct literal cannot be empty"));
        }
        loop {
            let field = self.expect_ident()?;
            self.expect(Token::Colon)?;
            let value = self.parse_inline_endpoint()?;
            fields.push((field, value));
            if self.eat(Token::Comma) {
                if self.eat(Token::RBrace) {
                    break;
                }
            } else {
                self.expect(Token::RBrace)?;
                break;
            }
        }
        Ok(Endpoint::Struct { name, fields })
    }

    fn parse_tuple_or_unit(&mut self) -> Result<Endpoint, SourceDiagnostic> {
        self.expect(Token::LParen)?;
        if self.eat(Token::RParen) {
            return Ok(Endpoint::Unit);
        }
        let first = self.parse_inline_endpoint()?;
        self.expect(Token::Comma)?;
        let mut items = vec![first, self.parse_inline_endpoint()?];
        while self.eat(Token::Comma) {
            items.push(self.parse_inline_endpoint()?);
        }
        self.expect(Token::RParen)?;
        Ok(Endpoint::Tuple(items))
    }

    fn parse_seq(&mut self) -> Result<Endpoint, SourceDiagnostic> {
        self.expect(Token::LBracket)?;
        let mut items = Vec::new();
        if self.eat(Token::RBracket) {
            return Ok(Endpoint::Seq(items));
        }
        loop {
            items.push(self.parse_inline_endpoint()?);
            if self.eat(Token::Comma) {
                if self.eat(Token::RBracket) {
                    break;
                }
            } else {
                self.expect(Token::RBracket)?;
                break;
            }
        }
        Ok(Endpoint::Seq(items))
    }

    fn parse_qualified_ident(&mut self) -> Result<String, SourceDiagnostic> {
        let mut name = self.expect_ident()?;
        while self.eat(Token::Dot) {
            name.push('.');
            name.push_str(&self.expect_ident()?);
        }
        Ok(name)
    }

    fn parse_node_ref_text(&mut self) -> Result<String, SourceDiagnostic> {
        let base = self.parse_qualified_ident()?;
        if !self.eat(Token::Less) {
            return Ok(base);
        }
        let mut args = Vec::new();
        if self.eat(Token::Greater) {
            return Err(self.error_here("static node argument list cannot be empty"));
        }
        loop {
            args.push(self.parse_qualified_ident()?);
            if self.eat(Token::Comma) {
                if self.eat(Token::Greater) {
                    break;
                }
            } else {
                self.expect(Token::Greater)?;
                break;
            }
        }
        Ok(format_static_node_ref(&base, &args))
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), SourceDiagnostic> {
        match self.peek_ident() {
            Some(found) if found == keyword => {
                self.bump();
                Ok(())
            }
            _ => Err(self.error_here(format!(
                "expected keyword `{keyword}`, found {:?}",
                self.peek()
            ))),
        }
    }

    fn expect_ident(&mut self) -> Result<String, SourceDiagnostic> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.error_here(format!("expected identifier, found {other:?}"))),
        }
    }

    fn expect_string(&mut self) -> Result<String, SourceDiagnostic> {
        match self.peek().clone() {
            Token::String(value) => {
                self.bump();
                Ok(value)
            }
            other => Err(self.error_here(format!("expected string, found {other:?}"))),
        }
    }

    fn expect_variable(&mut self) -> Result<String, SourceDiagnostic> {
        match self.peek().clone() {
            Token::Variable(name) => {
                self.bump();
                Ok(name)
            }
            Token::Ident(name) => {
                Err(self.error_here(format!("expected variable `$` prefix for `{name}`")))
            }
            other => Err(self.error_here(format!("expected variable, found {other:?}"))),
        }
    }

    fn expect(&mut self, expected: Token) -> Result<(), SourceDiagnostic> {
        if self.eat(expected.clone()) {
            Ok(())
        } else {
            Err(self.error_here(format!("expected {expected:?}, found {:?}", self.peek())))
        }
    }

    fn eat(&mut self, expected: Token) -> bool {
        if self.peek() == &expected {
            self.bump();
            true
        } else {
            false
        }
    }

    fn peek_ident(&self) -> Option<&str> {
        match self.peek() {
            Token::Ident(name) => Some(name.as_str()),
            _ => None,
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn span(&self) -> SourceSpan {
        self.tokens[self.pos].span
    }

    fn error_here(&self, message: impl Into<String>) -> SourceDiagnostic {
        SourceDiagnostic::new(message, self.span())
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }
}

fn is_declaration_keyword(name: &str) -> bool {
    matches!(
        name,
        "type" | "struct" | "import" | "foreign" | "extern" | "node" | "program"
    )
}
