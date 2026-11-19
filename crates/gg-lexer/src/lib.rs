// SPDX-License-Identifier: GPL-3.0-or-later
//! Lexer for the GG programming language.
//!
//! Pure: takes a `&str` and returns the list of tokens (or the first
//! lexing error). Every token carries its 1-based line and column.
//!
//! Syntax decisions (final):
//! - English keywords, blocks closed by `end` (Ruby/Lua style).
//! - No significant indentation, no braces.
//! - `#` comments run to the end of the line.
//! - Newlines are real tokens: the parser (Phase 2) uses them as
//!   statement separators. `;` is also accepted as a separator.

use std::fmt;

/// A single token, with its 1-based start position.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

impl Token {
    /// Short type name, e.g. `Let`, `Ident`, `Int`.
    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }

    /// The value carried by the token: the identifier, the number,
    /// the decoded string, or the literal text for keywords/operators.
    /// Empty for `Newline` and `Eof`.
    pub fn value(&self) -> String {
        self.kind.value()
    }
}

/// All token kinds in GG.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Let,
    If,
    Then,
    Else,
    End,
    While,
    For,
    Fn,
    Return,
    Print,
    True,
    False,
    And,
    Or,
    Not,
    // Literals
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Eq,
    // Punctuation
    LParen,
    RParen,
    Comma,
    // Separators
    Newline,
    Semicolon,
    // End of input
    Eof,
}

impl TokenKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            TokenKind::Let => "Let",
            TokenKind::If => "If",
            TokenKind::Then => "Then",
            TokenKind::Else => "Else",
            TokenKind::End => "End",
            TokenKind::While => "While",
            TokenKind::For => "For",
            TokenKind::Fn => "Fn",
            TokenKind::Return => "Return",
            TokenKind::Print => "Print",
            TokenKind::True => "True",
            TokenKind::False => "False",
            TokenKind::And => "And",
            TokenKind::Or => "Or",
            TokenKind::Not => "Not",
            TokenKind::Ident(_) => "Ident",
            TokenKind::Int(_) => "Int",
            TokenKind::Float(_) => "Float",
            TokenKind::Str(_) => "Str",
            TokenKind::Plus => "Plus",
            TokenKind::Minus => "Minus",
            TokenKind::Star => "Star",
            TokenKind::Slash => "Slash",
            TokenKind::Percent => "Percent",
            TokenKind::EqEq => "EqEq",
            TokenKind::NotEq => "NotEq",
            TokenKind::Lt => "Lt",
            TokenKind::LtEq => "LtEq",
            TokenKind::Gt => "Gt",
            TokenKind::GtEq => "GtEq",
            TokenKind::Eq => "Eq",
            TokenKind::LParen => "LParen",
            TokenKind::RParen => "RParen",
            TokenKind::Comma => "Comma",
            TokenKind::Newline => "Newline",
            TokenKind::Semicolon => "Semicolon",
            TokenKind::Eof => "Eof",
        }
    }

    pub fn value(&self) -> String {
        match self {
            TokenKind::Let => "let".to_string(),
            TokenKind::If => "if".to_string(),
            TokenKind::Then => "then".to_string(),
            TokenKind::Else => "else".to_string(),
            TokenKind::End => "end".to_string(),
            TokenKind::While => "while".to_string(),
            TokenKind::For => "for".to_string(),
            TokenKind::Fn => "fn".to_string(),
            TokenKind::Return => "return".to_string(),
            TokenKind::Print => "print".to_string(),
            TokenKind::True => "true".to_string(),
            TokenKind::False => "false".to_string(),
            TokenKind::And => "and".to_string(),
            TokenKind::Or => "or".to_string(),
            TokenKind::Not => "not".to_string(),
            TokenKind::Ident(name) => name.clone(),
            TokenKind::Int(n) => n.to_string(),
            TokenKind::Float(f) => f.to_string(),
            TokenKind::Str(s) => s.clone(),
            TokenKind::Plus => "+".to_string(),
            TokenKind::Minus => "-".to_string(),
            TokenKind::Star => "*".to_string(),
            TokenKind::Slash => "/".to_string(),
            TokenKind::Percent => "%".to_string(),
            TokenKind::EqEq => "==".to_string(),
            TokenKind::NotEq => "!=".to_string(),
            TokenKind::Lt => "<".to_string(),
            TokenKind::LtEq => "<=".to_string(),
            TokenKind::Gt => ">".to_string(),
            TokenKind::GtEq => ">=".to_string(),
            TokenKind::Eq => "=".to_string(),
            TokenKind::LParen => "(".to_string(),
            TokenKind::RParen => ")".to_string(),
            TokenKind::Comma => ",".to_string(),
            TokenKind::Newline | TokenKind::Semicolon | TokenKind::Eof => String::new(),
        }
    }
}

/// A lexing error: a plain-English message plus 1-based line/column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexerError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl LexerError {
    fn new(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self {
            message: message.into(),
            line,
            col,
        }
    }
}

impl fmt::Display for LexerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, col {}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for LexerError {}

fn keyword_or_ident(word: &str) -> TokenKind {
    match word {
        "let" => TokenKind::Let,
        "if" => TokenKind::If,
        "then" => TokenKind::Then,
        "else" => TokenKind::Else,
        "end" => TokenKind::End,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "fn" => TokenKind::Fn,
        "return" => TokenKind::Return,
        "print" => TokenKind::Print,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "and" => TokenKind::And,
        "or" => TokenKind::Or,
        "not" => TokenKind::Not,
        _ => TokenKind::Ident(word.to_string()),
    }
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    /// Consume one char and advance the position (line/col in characters).
    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn token(&self, kind: TokenKind, line: usize, col: usize) -> Token {
        Token { kind, line, col }
    }

    fn lex_all(mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_eof = token.kind == TokenKind::Eof;
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Token, LexerError> {
        loop {
            let (line, col) = (self.line, self.col);
            let Some(c) = self.peek() else {
                return Ok(self.token(TokenKind::Eof, line, col));
            };
            match c {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Newline, line, col));
                }
                '#' => {
                    // Comment: skip to (not including) the newline.
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                ';' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Semicolon, line, col));
                }
                '(' => {
                    self.advance();
                    return Ok(self.token(TokenKind::LParen, line, col));
                }
                ')' => {
                    self.advance();
                    return Ok(self.token(TokenKind::RParen, line, col));
                }
                ',' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Comma, line, col));
                }
                '+' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Plus, line, col));
                }
                '-' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Minus, line, col));
                }
                '*' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Star, line, col));
                }
                '/' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Slash, line, col));
                }
                '%' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Percent, line, col));
                }
                '=' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        return Ok(self.token(TokenKind::EqEq, line, col));
                    }
                    return Ok(self.token(TokenKind::Eq, line, col));
                }
                '!' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        return Ok(self.token(TokenKind::NotEq, line, col));
                    }
                    return Err(LexerError::new(
                        "unexpected character '!' (did you mean '!='?)",
                        line,
                        col,
                    ));
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        return Ok(self.token(TokenKind::LtEq, line, col));
                    }
                    return Ok(self.token(TokenKind::Lt, line, col));
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        return Ok(self.token(TokenKind::GtEq, line, col));
                    }
                    return Ok(self.token(TokenKind::Gt, line, col));
                }
                '"' => return self.lex_string(),
                c if c.is_ascii_digit() => return self.lex_number(),
                c if c.is_ascii_alphabetic() || c == '_' => return self.lex_ident(),
                _ => {
                    return Err(LexerError::new(
                        format!("unexpected character '{c}'"),
                        line,
                        col,
                    ));
                }
            }
        }
    }

    fn lex_ident(&mut self) -> Result<Token, LexerError> {
        let (line, col) = (self.line, self.col);
        let mut word = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                word.push(c);
                self.advance();
            } else {
                break;
            }
        }
        Ok(self.token(keyword_or_ident(&word), line, col))
    }

    fn lex_number(&mut self) -> Result<Token, LexerError> {
        let (line, col) = (self.line, self.col);
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                text.push(c);
                self.advance();
            } else {
                break;
            }
        }
        let mut is_float = false;
        if self.peek() == Some('.') {
            // A '.' belongs to the number only if a digit follows it:
            // `5.` is not a valid number.
            if self.peek_next().is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                text.push('.');
                self.advance();
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        text.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
            } else {
                return Err(LexerError::new(
                    format!("malformed number '{text}.' (expected a digit after '.')"),
                    line,
                    col,
                ));
            }
        }
        if is_float {
            match text.parse::<f64>() {
                Ok(n) => Ok(self.token(TokenKind::Float(n), line, col)),
                Err(_) => Err(LexerError::new(
                    format!("number '{text}' is out of range"),
                    line,
                    col,
                )),
            }
        } else {
            match text.parse::<i64>() {
                Ok(n) => Ok(self.token(TokenKind::Int(n), line, col)),
                Err(_) => Err(LexerError::new(
                    format!("integer '{text}' is out of range"),
                    line,
                    col,
                )),
            }
        }
    }

    fn lex_string(&mut self) -> Result<Token, LexerError> {
        let (line, col) = (self.line, self.col);
        self.advance(); // opening quote
        let mut value = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(LexerError::new("unterminated string", line, col));
            };
            match c {
                '"' => {
                    self.advance();
                    return Ok(self.token(TokenKind::Str(value), line, col));
                }
                '\n' => {
                    return Err(LexerError::new("unterminated string", line, col));
                }
                '\\' => {
                    self.advance();
                    let Some(esc) = self.peek() else {
                        return Err(LexerError::new("unterminated string", line, col));
                    };
                    let decoded = match esc {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        't' => '\t',
                        _ => {
                            return Err(LexerError::new(
                                format!("unknown escape sequence '\\{esc}'"),
                                self.line,
                                self.col,
                            ));
                        }
                    };
                    value.push(decoded);
                    self.advance();
                }
                _ => {
                    value.push(c);
                    self.advance();
                }
            }
        }
    }
}

/// Tokenize a GG source string.
///
/// Returns the token list (always ending with `Eof`) or the first
/// lexing error, with its 1-based line and column.
pub fn lex(source: &str) -> Result<Vec<Token>, LexerError> {
    Lexer::new(source).lex_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source).unwrap().into_iter().map(|t| t.kind).collect()
    }

    fn kinds_no_eof(source: &str) -> Vec<TokenKind> {
        let mut ks = kinds(source);
        assert_eq!(ks.pop(), Some(TokenKind::Eof));
        ks
    }

    #[test]
    fn canonical_example_exact_sequence() {
        let source =
            "let x = 10\nif x > 5 then\n    print \"big\"\nelse\n    print \"small\"\nend\n";
        let tokens = lex(source).unwrap();
        let expected = vec![
            TokenKind::Let,
            TokenKind::Ident("x".to_string()),
            TokenKind::Eq,
            TokenKind::Int(10),
            TokenKind::Newline,
            TokenKind::If,
            TokenKind::Ident("x".to_string()),
            TokenKind::Gt,
            TokenKind::Int(5),
            TokenKind::Then,
            TokenKind::Newline,
            TokenKind::Print,
            TokenKind::Str("big".to_string()),
            TokenKind::Newline,
            TokenKind::Else,
            TokenKind::Newline,
            TokenKind::Print,
            TokenKind::Str("small".to_string()),
            TokenKind::Newline,
            TokenKind::End,
            TokenKind::Newline,
            TokenKind::Eof,
        ];
        assert_eq!(
            tokens.into_iter().map(|t| t.kind).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn canonical_example_positions() {
        let source = "let x = 10\nif x > 5 then\n    print \"big\"\nend\n";
        let tokens = lex(source).unwrap();
        // `print` is on line 3, column 5 (after 4 spaces).
        let print = tokens.iter().find(|t| t.kind == TokenKind::Print).unwrap();
        assert_eq!((print.line, print.col), (3, 5));
        // `end` is on line 4, column 1.
        let end = tokens.iter().find(|t| t.kind == TokenKind::End).unwrap();
        assert_eq!((end.line, end.col), (4, 1));
        // The final Eof sits on line 5, column 1.
        assert_eq!(
            (tokens.last().unwrap().line, tokens.last().unwrap().col),
            (5, 1)
        );
    }

    #[test]
    fn keywords_are_not_identifiers_but_prefixes_are() {
        assert_eq!(
            kinds_no_eof("let letter endif endless truex not_now"),
            vec![
                TokenKind::Let,
                TokenKind::Ident("letter".to_string()),
                TokenKind::Ident("endif".to_string()),
                TokenKind::Ident("endless".to_string()),
                TokenKind::Ident("truex".to_string()),
                TokenKind::Ident("not_now".to_string()),
            ]
        );
    }

    #[test]
    fn all_keywords() {
        assert_eq!(
            kinds_no_eof("let if then else end while for fn return print true false and or not"),
            vec![
                TokenKind::Let,
                TokenKind::If,
                TokenKind::Then,
                TokenKind::Else,
                TokenKind::End,
                TokenKind::While,
                TokenKind::For,
                TokenKind::Fn,
                TokenKind::Return,
                TokenKind::Print,
                TokenKind::True,
                TokenKind::False,
                TokenKind::And,
                TokenKind::Or,
                TokenKind::Not,
            ]
        );
    }

    #[test]
    fn numbers_int_and_float() {
        assert_eq!(
            kinds_no_eof("42 0 3.15 100.001"),
            vec![
                TokenKind::Int(42),
                TokenKind::Int(0),
                TokenKind::Float(3.15),
                TokenKind::Float(100.001),
            ]
        );
    }

    #[test]
    fn numbers_invalid() {
        // `5.` — a dot with no digit after it.
        assert!(lex("5.").is_err());
        // `.5` — a leading dot is not a number.
        let err = lex(".5").unwrap_err();
        assert!(err.message.contains("unexpected character '.'"));
        // `1.2.3` — the second dot is unexpected.
        assert!(lex("1.2.3").is_err());
    }

    #[test]
    fn strings_with_escapes() {
        assert_eq!(
            kinds_no_eof(r#""hello" "a\"b" "c\\d" "e\nf" "g\th""#),
            vec![
                TokenKind::Str("hello".to_string()),
                TokenKind::Str("a\"b".to_string()),
                TokenKind::Str("c\\d".to_string()),
                TokenKind::Str("e\nf".to_string()),
                TokenKind::Str("g\th".to_string()),
            ]
        );
    }

    #[test]
    fn string_errors() {
        let err = lex("\"never closed").unwrap_err();
        assert_eq!(err.message, "unterminated string");
        assert_eq!((err.line, err.col), (1, 1));

        let err = lex("\"bad \\q escape\"").unwrap_err();
        assert!(err.message.contains("unknown escape sequence"));
    }

    #[test]
    fn comments_run_to_end_of_line() {
        let tokens = lex("let x = 1 # this is a comment\nprint x # trailing\n").unwrap();
        let expected = vec![
            TokenKind::Let,
            TokenKind::Ident("x".to_string()),
            TokenKind::Eq,
            TokenKind::Int(1),
            TokenKind::Newline,
            TokenKind::Print,
            TokenKind::Ident("x".to_string()),
            TokenKind::Newline,
            TokenKind::Eof,
        ];
        assert_eq!(
            tokens.into_iter().map(|t| t.kind).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn two_char_operators() {
        assert_eq!(
            kinds_no_eof("== != <= >= < > ="),
            vec![
                TokenKind::EqEq,
                TokenKind::NotEq,
                TokenKind::LtEq,
                TokenKind::GtEq,
                TokenKind::Lt,
                TokenKind::Gt,
                TokenKind::Eq,
            ]
        );
    }

    #[test]
    fn arithmetic_operators_and_punctuation() {
        assert_eq!(
            kinds_no_eof("+ - * / % ( ) ,"),
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Percent,
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::Comma,
            ]
        );
    }

    #[test]
    fn semicolons_and_newlines_as_separators() {
        assert_eq!(
            kinds_no_eof("let a = 1; let b = 2\nlet c = 3"),
            vec![
                TokenKind::Let,
                TokenKind::Ident("a".to_string()),
                TokenKind::Eq,
                TokenKind::Int(1),
                TokenKind::Semicolon,
                TokenKind::Let,
                TokenKind::Ident("b".to_string()),
                TokenKind::Eq,
                TokenKind::Int(2),
                TokenKind::Newline,
                TokenKind::Let,
                TokenKind::Ident("c".to_string()),
                TokenKind::Eq,
                TokenKind::Int(3),
            ]
        );
    }

    #[test]
    fn unexpected_characters() {
        let err = lex("@").unwrap_err();
        assert_eq!(err.message, "unexpected character '@'");
        assert_eq!((err.line, err.col), (1, 1));

        // A lone `!` is not an operator (only `!=` is).
        let err = lex("!").unwrap_err();
        assert!(err.message.contains("unexpected character '!'"));

        // Position on a later line: `@` is the 3rd char of line 2.
        let err = lex("let a = 1\nx @ y").unwrap_err();
        assert_eq!((err.line, err.col), (2, 3));
    }

    #[test]
    fn empty_source_is_just_eof() {
        let tokens = lex("").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Eof);
        assert_eq!((tokens[0].line, tokens[0].col), (1, 1));
    }
}
