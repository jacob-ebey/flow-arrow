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

pub fn lex(source: &str) -> Result<Vec<Token>, String> {
    let mut lexer = Lexer {
        chars: source.chars().collect(),
        pos: 0,
        tokens: Vec::new(),
    };
    lexer.lex_all()?;
    Ok(lexer.tokens)
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    tokens: Vec<Token>,
}

impl Lexer {
    fn lex_all(&mut self) -> Result<(), String> {
        while let Some(ch) = self.peek() {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    self.pos += 1;
                }
                '#' => self.skip_line_comment(),
                '/' if self.peek_next() == Some('*') => self.skip_block_comment()?,
                '-' if self.peek_next() == Some('>') => {
                    self.pos += 2;
                    self.tokens.push(Token::Arrow);
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
                other => return Err(format!("unexpected character `{other}`")),
            }
        }
        self.tokens.push(Token::Eof);
        Ok(())
    }

    fn push_one(&mut self, token: Token) {
        self.pos += 1;
        self.tokens.push(token);
    }

    fn skip_line_comment(&mut self) {
        while let Some(ch) = self.peek() {
            self.pos += 1;
            if ch == '\n' {
                break;
            }
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), String> {
        self.pos += 2;
        while self.pos + 1 < self.chars.len() {
            if self.chars[self.pos] == '*' && self.chars[self.pos + 1] == '/' {
                self.pos += 2;
                return Ok(());
            }
            self.pos += 1;
        }
        Err("unterminated block comment".to_string())
    }

    fn lex_number(&mut self) -> Result<(), String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        let digits_start = self.pos;
        while matches!(self.peek(), Some('0'..='9')) {
            self.pos += 1;
        }
        if digits_start == self.pos {
            return Err("expected digits after `-`".to_string());
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if self.peek() == Some('.') && matches!(self.peek_next(), Some('0'..='9')) {
            self.pos += 1;
            while matches!(self.peek(), Some('0'..='9')) {
                self.pos += 1;
            }
            let text: String = self.chars[start..self.pos].iter().collect();
            let value = text
                .parse::<f64>()
                .map_err(|error| format!("invalid real literal `{text}`: {error}"))?;
            self.tokens.push(Token::Real(value));
        } else {
            let value = text
                .parse::<i64>()
                .map_err(|error| format!("invalid integer literal `{text}`: {error}"))?;
            self.tokens.push(Token::Int(value));
        }
        Ok(())
    }

    fn lex_string(&mut self) -> Result<(), String> {
        self.pos += 1;
        let mut value = String::new();
        while let Some(ch) = self.peek() {
            self.pos += 1;
            match ch {
                '"' => {
                    self.tokens.push(Token::String(value));
                    return Ok(());
                }
                '\\' => {
                    let escaped = self
                        .peek()
                        .ok_or_else(|| "unterminated string escape".to_string())?;
                    self.pos += 1;
                    value.push(match escaped {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => return Err(format!("unsupported string escape `\\{other}`")),
                    });
                }
                other => value.push(other),
            }
        }
        Err("unterminated string literal".to_string())
    }

    fn lex_ident(&mut self) {
        let start = self.pos;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        match text.as_str() {
            "true" => self.tokens.push(Token::Bool(true)),
            "false" => self.tokens.push(Token::Bool(false)),
            _ => self.tokens.push(Token::Ident(text)),
        }
    }

    fn lex_variable(&mut self) -> Result<(), String> {
        self.pos += 1;
        let start = self.pos;
        match self.peek() {
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
            Some(other) => {
                return Err(format!("expected variable name after `$`, found `{other}`"));
            }
            None => return Err("expected variable name after `$`".to_string()),
        }
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        self.tokens.push(Token::Variable(text));
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }
}
