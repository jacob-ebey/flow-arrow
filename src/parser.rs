use crate::ast::*;
use crate::lexer::{Token, lex};

pub fn parse(source: &str) -> Result<Module, String> {
    Parser {
        tokens: lex(source)?,
        pos: 0,
    }
    .parse_module()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn parse_module(&mut self) -> Result<Module, String> {
        let mut declarations = Vec::new();
        while !self.at_eof() {
            declarations.push(self.parse_decl()?);
        }
        Ok(Module { declarations })
    }

    fn parse_decl(&mut self) -> Result<Decl, String> {
        match self.peek_ident() {
            Some("import") => {
                self.bump();
                Ok(Decl::Import(self.parse_import()?))
            }
            Some("node") => {
                self.bump();
                Ok(Decl::Node(self.parse_callable()?))
            }
            Some("program") => {
                self.bump();
                Ok(Decl::Program(self.parse_callable()?))
            }
            _ => Err(format!("expected declaration, found {:?}", self.peek())),
        }
    }

    fn parse_import(&mut self) -> Result<Import, String> {
        let source = match self.peek().clone() {
            Token::String(path) => {
                self.bump();
                ImportSource::Local(path)
            }
            Token::Ident(_) => ImportSource::Module(self.parse_qualified_ident()?),
            other => return Err(format!("expected import source, found {other:?}")),
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

    fn parse_callable(&mut self) -> Result<Callable, String> {
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
        let chains = self.parse_block()?;
        Ok(Callable {
            name,
            inputs,
            outputs,
            chains,
        })
    }

    fn parse_port_or_list(&mut self) -> Result<Vec<Port>, String> {
        if self.eat(Token::LParen) {
            let ports = self.parse_port_list()?;
            self.expect(Token::RParen)?;
            Ok(ports)
        } else {
            Ok(vec![self.parse_port()?])
        }
    }

    fn parse_port_list(&mut self) -> Result<Vec<Port>, String> {
        let mut ports = vec![self.parse_port()?];
        while self.eat(Token::Comma) {
            ports.push(self.parse_port()?);
        }
        Ok(ports)
    }

    fn parse_port(&mut self) -> Result<Port, String> {
        let name = self.expect_ident()?;
        self.expect(Token::Colon)?;
        let ty = self.parse_type_name()?;
        Ok(Port { name, ty })
    }

    fn parse_type_name(&mut self) -> Result<String, String> {
        let mut text = String::new();
        let mut depth = 0usize;
        loop {
            match self.peek().clone() {
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
                Token::Comma | Token::RParen | Token::LBrace if !text.is_empty() && depth == 0 => {
                    break;
                }
                other if text.is_empty() => return Err(format!("expected type, found {other:?}")),
                other => return Err(format!("unexpected token in type `{text}`: {other:?}")),
            }
        }
        if depth != 0 {
            return Err(format!("unterminated type `{text}`"));
        }
        Ok(text)
    }

    fn parse_block(&mut self) -> Result<Vec<Chain>, String> {
        self.expect(Token::LBrace)?;
        let mut chains = Vec::new();
        while !self.eat(Token::RBrace) {
            chains.push(self.parse_chain()?);
        }
        Ok(chains)
    }

    fn parse_chain(&mut self) -> Result<Chain, String> {
        let source = self.parse_endpoint()?;
        let mut stages = Vec::new();
        while self.eat(Token::Arrow) {
            stages.push(self.parse_stage()?);
        }
        if stages.is_empty() {
            return Err("chain must contain at least one `->` stage".to_string());
        }
        Ok(Chain { source, stages })
    }

    fn parse_stage(&mut self) -> Result<Stage, String> {
        match self.peek_ident() {
            Some("map") => {
                self.bump();
                Ok(Stage::Map(self.expect_ident()?))
            }
            Some("fault") => {
                self.bump();
                self.expect_keyword("map")?;
                let node = self.expect_ident()?;
                self.expect(Token::LBrace)?;
                self.expect_keyword("ok")?;
                self.expect(Token::Arrow)?;
                let ok = self.expect_ident()?;
                self.expect(Token::Comma)?;
                self.expect_keyword("fault")?;
                self.expect(Token::Arrow)?;
                let fault = self.expect_ident()?;
                self.expect(Token::RBrace)?;
                Ok(Stage::FaultMap { node, ok, fault })
            }
            Some("filter") => {
                self.bump();
                Ok(Stage::Filter(self.expect_ident()?))
            }
            Some("repeat") => {
                self.bump();
                self.expect(Token::Less)?;
                let count = match self.peek().clone() {
                    Token::Ident(_) | Token::Int(_) => self.parse_endpoint()?,
                    other => return Err(format!("expected repeat count, found {other:?}")),
                };
                self.expect(Token::Greater)?;
                let node = self.expect_ident()?;
                Ok(Stage::Repeat { count, node })
            }
            Some("reduce") => {
                self.bump();
                let op = self.expect_ident()?;
                self.expect(Token::LParen)?;
                self.expect_keyword("identity")?;
                self.expect(Token::Colon)?;
                let identity = self.parse_endpoint()?;
                self.expect(Token::RParen)?;
                Ok(Stage::Reduce { op, identity })
            }
            _ => Ok(Stage::Endpoint(self.parse_endpoint()?)),
        }
    }

    fn parse_endpoint(&mut self) -> Result<Endpoint, String> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.bump();
                if self.eat(Token::Dot) {
                    let member = self.expect_ident()?;
                    Ok(Endpoint::Name(format!("{name}.{member}")))
                } else {
                    Ok(Endpoint::Name(name))
                }
            }
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
            other => Err(format!("expected endpoint, found {other:?}")),
        }
    }

    fn parse_tuple_or_unit(&mut self) -> Result<Endpoint, String> {
        self.expect(Token::LParen)?;
        if self.eat(Token::RParen) {
            return Ok(Endpoint::Unit);
        }
        let first = self.parse_endpoint()?;
        self.expect(Token::Comma)?;
        let mut items = vec![first, self.parse_endpoint()?];
        while self.eat(Token::Comma) {
            items.push(self.parse_endpoint()?);
        }
        self.expect(Token::RParen)?;
        Ok(Endpoint::Tuple(items))
    }

    fn parse_seq(&mut self) -> Result<Endpoint, String> {
        self.expect(Token::LBracket)?;
        let mut items = Vec::new();
        if self.eat(Token::RBracket) {
            return Ok(Endpoint::Seq(items));
        }
        loop {
            items.push(self.parse_endpoint()?);
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

    fn parse_qualified_ident(&mut self) -> Result<String, String> {
        let mut name = self.expect_ident()?;
        while self.eat(Token::Dot) {
            name.push('.');
            name.push_str(&self.expect_ident()?);
        }
        Ok(name)
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), String> {
        match self.peek_ident() {
            Some(found) if found == keyword => {
                self.bump();
                Ok(())
            }
            _ => Err(format!(
                "expected keyword `{keyword}`, found {:?}",
                self.peek()
            )),
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(format!("expected identifier, found {other:?}")),
        }
    }

    fn expect(&mut self, expected: Token) -> Result<(), String> {
        if self.eat(expected.clone()) {
            Ok(())
        } else {
            Err(format!("expected {expected:?}, found {:?}", self.peek()))
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
        &self.tokens[self.pos]
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }
}
