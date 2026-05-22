use std::fmt;

use crate::lexer::{Token, lex_spanned};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    pub line: u32,
    pub character: u32,
}

impl SourcePosition {
    pub const fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: SourcePosition,
    pub end: SourcePosition,
}

impl SourceSpan {
    pub const fn new(start: SourcePosition, end: SourcePosition) -> Self {
        Self { start, end }
    }

    pub fn point(position: SourcePosition) -> Self {
        Self {
            start: position,
            end: SourcePosition {
                line: position.line,
                character: position.character + 1,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiagnostic {
    pub message: String,
    pub span: SourceSpan,
}

impl SourceDiagnostic {
    pub fn new(message: impl Into<String>, span: SourceSpan) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for SourceDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub fn format_source_diagnostic(error: &SourceDiagnostic) -> String {
    format!(
        "line {}, column {}: {}",
        error.span.start.line + 1,
        error.span.start.character + 1,
        error.message
    )
}

pub fn format_flowarrow_error(source: &str, message: &str) -> String {
    let Some(span) = diagnostic_span_for_message(source, message) else {
        return message.to_string();
    };
    format!(
        "line {}, column {}: {}",
        span.start.line + 1,
        span.start.character + 1,
        message
    )
}

fn diagnostic_span_for_message(source: &str, message: &str) -> Option<SourceSpan> {
    let mut names = backtick_items(message);
    if message.contains("does not export") {
        names.reverse();
    }
    let tokens = lex_spanned(source).ok()?;
    for name in names {
        if name.contains(' ') || name.contains('[') || name.contains(']') {
            continue;
        }
        if let Some(span) = token_span_for_name(&tokens, &name) {
            return Some(span);
        }
    }
    None
}

fn token_span_for_name(tokens: &[crate::lexer::SpannedToken], name: &str) -> Option<SourceSpan> {
    for window in tokens.windows(3) {
        if let [left, dot, right] = window
            && matches!(dot.token, Token::Dot)
            && let (Token::Ident(left), Token::Ident(right)) = (&left.token, &right.token)
            && format!("{left}.{right}") == name
        {
            return Some(SourceSpan::new(window[0].span.start, window[2].span.end));
        }
    }

    for token in tokens {
        match &token.token {
            Token::Variable(variable) if variable == name || format!("${variable}") == name => {
                return Some(token.span);
            }
            Token::Ident(ident) if ident == name => return Some(token.span),
            _ => {}
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_parser_errors_with_line_and_column() {
        let error = SourceDiagnostic::new(
            "unexpected character `@`",
            SourceSpan::point(SourcePosition::new(1, 4)),
        );

        assert_eq!(
            format_source_diagnostic(&error),
            "line 2, column 5: unexpected character `@`"
        );
    }

    #[test]
    fn formats_typecheck_errors_with_relevant_token_line_and_column() {
        let source = r#"import std.bytes { missing }
import std.cli { Args }

program main(args: Args) -> exit_code: Int {
    0 -> $exit_code
}
"#;

        assert_eq!(
            format_flowarrow_error(source, "module `std.bytes` does not export `missing`"),
            "line 1, column 20: module `std.bytes` does not export `missing`"
        );
    }
}
