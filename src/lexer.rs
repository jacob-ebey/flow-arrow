use crate::diagnostic::{SourceDiagnostic, SourcePosition, SourceSpan};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Variable(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    String(String),
    Arrow,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Less,
    Greater,
    Comma,
    Colon,
    Equal,
    Pipe,
    Dot,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: SourceSpan,
}

pub fn lex_spanned(source: &str) -> Result<Vec<SpannedToken>, SourceDiagnostic> {
    let mut lexer = Lexer {
        chars: source.chars().collect(),
        pos: 0,
        position: SourcePosition::new(0, 0),
        tokens: Vec::new(),
    };
    lexer.lex_all()?;
    Ok(lexer.tokens)
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    position: SourcePosition,
    tokens: Vec<SpannedToken>,
}

impl Lexer {
    fn lex_all(&mut self) -> Result<(), SourceDiagnostic> {
        while let Some(ch) = self.peek() {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    self.advance(ch);
                }
                '#' => self.skip_line_comment(),
                '/' if self.peek_next() == Some('*') => self.skip_block_comment()?,
                '-' if self.peek_next() == Some('>') => {
                    let start = self.position;
                    self.advance('-');
                    self.advance('>');
                    self.push(Token::Arrow, start);
                }
                '-' | '0'..='9' => self.lex_number()?,
                '"' => self.lex_string()?,
                '$' => self.lex_variable()?,
                '(' => self.push_one(Token::LParen),
                ')' => self.push_one(Token::RParen),
                '{' => self.push_one(Token::LBrace),
                '}' => self.push_one(Token::RBrace),
                '[' => self.push_one(Token::LBracket),
                ']' => self.push_one(Token::RBracket),
                '<' => self.push_one(Token::Less),
                '>' => self.push_one(Token::Greater),
                ',' => self.push_one(Token::Comma),
                ':' => self.push_one(Token::Colon),
                '=' => self.push_one(Token::Equal),
                '|' => self.push_one(Token::Pipe),
                '.' => self.push_one(Token::Dot),
                ch if ch.is_ascii_alphabetic() || ch == '_' => self.lex_ident(),
                other => {
                    return Err(self.error_here(format!("unexpected character `{other}`")));
                }
            }
        }
        self.push(Token::Eof, self.position);
        Ok(())
    }

    fn push_one(&mut self, token: Token) {
        let start = self.position;
        let ch = self.peek().expect("push_one only called at a character");
        self.advance(ch);
        self.push(token, start);
    }

    fn push(&mut self, token: Token, start: SourcePosition) {
        self.tokens.push(SpannedToken {
            token,
            span: SourceSpan::new(start, self.position),
        });
    }

    fn error_here(&self, message: impl Into<String>) -> SourceDiagnostic {
        SourceDiagnostic::new(message, SourceSpan::point(self.position))
    }

    fn skip_line_comment(&mut self) {
        while let Some(ch) = self.peek() {
            self.advance(ch);
            if ch == '\n' {
                break;
            }
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), SourceDiagnostic> {
        let start = self.position;
        self.advance('/');
        self.advance('*');
        while self.pos + 1 < self.chars.len() {
            if self.chars[self.pos] == '*' && self.chars[self.pos + 1] == '/' {
                self.advance('*');
                self.advance('/');
                return Ok(());
            }
            self.advance(self.chars[self.pos]);
        }
        Err(SourceDiagnostic::new(
            "unterminated block comment",
            SourceSpan::point(start),
        ))
    }

    fn lex_number(&mut self) -> Result<(), SourceDiagnostic> {
        let start = self.pos;
        let start_position = self.position;
        if self.peek() == Some('-') {
            self.advance('-');
        }
        let digits_start = self.pos;
        while matches!(self.peek(), Some('0'..='9')) {
            self.advance(self.chars[self.pos]);
        }
        if digits_start == self.pos {
            return Err(SourceDiagnostic::new(
                "expected digits after `-`",
                SourceSpan::point(start_position),
            ));
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if self.peek() == Some('.') && matches!(self.peek_next(), Some('0'..='9')) {
            self.advance('.');
            while matches!(self.peek(), Some('0'..='9')) {
                self.advance(self.chars[self.pos]);
            }
            let text: String = self.chars[start..self.pos].iter().collect();
            let value = text.parse::<f64>().map_err(|error| {
                SourceDiagnostic::new(
                    format!("invalid real literal `{text}`: {error}"),
                    SourceSpan::new(start_position, self.position),
                )
            })?;
            self.push(Token::Real(value), start_position);
        } else {
            let value = text.parse::<i64>().map_err(|error| {
                SourceDiagnostic::new(
                    format!("invalid integer literal `{text}`: {error}"),
                    SourceSpan::new(start_position, self.position),
                )
            })?;
            self.push(Token::Int(value), start_position);
        }
        Ok(())
    }

    fn lex_string(&mut self) -> Result<(), SourceDiagnostic> {
        let start = self.position;
        self.advance('"');
        let mut value = String::new();
        while let Some(ch) = self.peek() {
            self.advance(ch);
            match ch {
                '"' => {
                    self.push(Token::String(value), start);
                    return Ok(());
                }
                '\\' => {
                    let escaped = self.peek().ok_or_else(|| {
                        SourceDiagnostic::new(
                            "unterminated string escape",
                            SourceSpan::point(self.position),
                        )
                    })?;
                    self.advance(escaped);
                    value.push(match escaped {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => {
                            return Err(SourceDiagnostic::new(
                                format!("unsupported string escape `\\{other}`"),
                                SourceSpan::point(self.position),
                            ));
                        }
                    });
                }
                other => value.push(other),
            }
        }
        Err(SourceDiagnostic::new(
            "unterminated string literal",
            SourceSpan::point(start),
        ))
    }

    fn lex_ident(&mut self) {
        let start = self.pos;
        let start_position = self.position;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
            self.advance(self.chars[self.pos]);
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let token = match text.as_str() {
            "true" => Token::Bool(true),
            "false" => Token::Bool(false),
            _ => Token::Ident(text),
        };
        self.push(token, start_position);
    }

    fn lex_variable(&mut self) -> Result<(), SourceDiagnostic> {
        let start_position = self.position;
        self.advance('$');
        let start = self.pos;
        match self.peek() {
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
            Some(other) => {
                return Err(
                    self.error_here(format!("expected variable name after `$`, found `{other}`"))
                );
            }
            None => return Err(self.error_here("expected variable name after `$`")),
        }
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
            self.advance(self.chars[self.pos]);
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        self.push(Token::Variable(text), start_position);
        Ok(())
    }

    fn advance(&mut self, ch: char) {
        self.pos += 1;
        if ch == '\n' {
            self.position.line += 1;
            self.position.character = 0;
        } else {
            self.position.character += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }
}
