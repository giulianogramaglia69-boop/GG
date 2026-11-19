// SPDX-License-Identifier: GPL-3.0-or-later
//! Recursive-descent parser for GG: tokens -> AST.
//!
//! Statements are separated by newlines and/or `;` (any number of
//! separators is allowed, including at the start and end of a program
//! or block). Blocks end with `end` — or with `else` in the `then`
//! branch of an `if`.

use crate::ast::{BinaryOp, Expr, Program, Stmt, UnaryOp};
use gg_lexer::{Token, TokenKind};
use std::fmt;

/// A parse error: a plain-English message plus the 1-based
/// line/column of the token that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl ParserError {
    fn new(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self {
            message: message.into(),
            line,
            col,
        }
    }
}

impl fmt::Display for ParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, col {}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParserError {}

/// A synthetic `Eof` token used when the token list is empty or the
/// parser has run past its end.
const EOF_TOKEN: Token = Token {
    kind: TokenKind::Eof,
    line: 1,
    col: 1,
};

/// Parse a token list into a program.
pub fn parse(tokens: &[Token]) -> Result<Program, ParserError> {
    Parser::new(tokens).parse_program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&EOF_TOKEN)
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn peek_next_kind(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos + 1).map(|t| &t.kind)
    }

    /// Consume the current token (never past `Eof`) and return it.
    fn advance(&mut self) -> Token {
        let token = self.peek().clone();
        if token.kind != TokenKind::Eof {
            self.pos += 1;
        }
        token
    }

    /// True when the current token is of the given kind
    /// (data-carrying kinds are compared by discriminant only).
    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind)
    }

    fn error_here(&self, message: impl Into<String>) -> ParserError {
        let token = self.peek();
        ParserError::new(message, token.line, token.col)
    }

    /// Consume the current token if it is of the given kind, else error.
    fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<Token, ParserError> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            Err(self.error_here(format!(
                "expected {what} but found {}",
                describe(self.peek())
            )))
        }
    }

    /// Consume the current token if it is an identifier, else error.
    fn expect_ident(&mut self, what: &str) -> Result<String, ParserError> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(self.error_here(format!(
                "expected {what} but found {}",
                describe(self.peek())
            ))),
        }
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek_kind(), TokenKind::Newline | TokenKind::Semicolon) {
            self.advance();
        }
    }

    fn is_separator(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Newline | TokenKind::Semicolon)
    }

    fn is_block_end(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::End | TokenKind::Else | TokenKind::Eof
        )
    }

    // ----- top level -----

    fn parse_program(&mut self) -> Result<Program, ParserError> {
        let stmts = self.parse_block()?;
        match self.peek_kind() {
            TokenKind::End => Err(self.error_here("unexpected 'end'")),
            TokenKind::Else => Err(self.error_here("unexpected 'else'")),
            TokenKind::Eof => Ok(Program { stmts }),
            _ => Err(self.error_here("expected end of input")),
        }
    }

    /// Parse statements until `end`, `else` or end of input.
    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParserError> {
        let mut stmts = Vec::new();
        loop {
            self.skip_separators();
            if self.is_block_end() {
                break;
            }
            let stmt = self.parse_stmt()?;
            stmts.push(stmt);
            if !self.is_separator() && !self.is_block_end() {
                return Err(self.error_here("expected newline or ';' after statement"));
            }
        }
        Ok(stmts)
    }

    // ----- statements -----

    fn parse_stmt(&mut self) -> Result<Stmt, ParserError> {
        match self.peek_kind() {
            TokenKind::Let => self.parse_let(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Fn => self.parse_fn(),
            TokenKind::Return => self.parse_return(),
            TokenKind::Print => self.parse_print(),
            TokenKind::Ident(_) => {
                // `x = ...` is an assignment; anything else starting
                // with an identifier is an expression statement.
                if matches!(self.peek_next_kind(), Some(TokenKind::Eq)) {
                    self.parse_assign()
                } else {
                    self.parse_expr_stmt()
                }
            }
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_let(&mut self) -> Result<Stmt, ParserError> {
        self.advance(); // let
        let name = self.expect_ident("variable name after 'let'")?;
        self.expect(&TokenKind::Eq, "'=' after variable name")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Let { name, value })
    }

    fn parse_assign(&mut self) -> Result<Stmt, ParserError> {
        let name = self.expect_ident("variable name")?;
        self.expect(&TokenKind::Eq, "'=' in assignment")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Assign { name, value })
    }

    fn parse_print(&mut self) -> Result<Stmt, ParserError> {
        self.advance(); // print
        let expr = self.parse_expr()?;
        Ok(Stmt::Print { expr })
    }

    fn parse_if(&mut self) -> Result<Stmt, ParserError> {
        self.advance(); // if
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Then, "'then' after if condition")?;
        let then_block = self.parse_block()?;
        let else_block = if self.check(&TokenKind::Else) {
            self.advance(); // else
            Some(self.parse_block()?)
        } else {
            None
        };
        self.expect(&TokenKind::End, "'end' to close 'if' block")?;
        Ok(Stmt::If {
            cond,
            then_block,
            else_block,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParserError> {
        self.advance(); // while
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Then, "'then' after while condition")?;
        let body = self.parse_block()?;
        self.expect(&TokenKind::End, "'end' to close 'while' block")?;
        Ok(Stmt::While { cond, body })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParserError> {
        self.advance(); // for
        let var = self.expect_ident("variable name after 'for'")?;
        self.expect(&TokenKind::Eq, "'=' in for loop")?;
        let from = self.parse_expr()?;
        self.expect(&TokenKind::To, "'to' in for loop")?;
        let to = self.parse_expr()?;
        self.expect(&TokenKind::Then, "'then' after for range")?;
        let body = self.parse_block()?;
        self.expect(&TokenKind::End, "'end' to close 'for' block")?;
        Ok(Stmt::For {
            var,
            from,
            to,
            body,
        })
    }

    fn parse_fn(&mut self) -> Result<Stmt, ParserError> {
        self.advance(); // fn
        let name = self.expect_ident("function name after 'fn'")?;
        self.expect(&TokenKind::LParen, "'(' after function name")?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            params.push(self.expect_ident("parameter name")?);
            while self.check(&TokenKind::Comma) {
                self.advance(); // ,
                params.push(self.expect_ident("parameter name")?);
            }
        }
        self.expect(&TokenKind::RParen, "')' after parameter list")?;
        let body = self.parse_block()?;
        self.expect(&TokenKind::End, "'end' to close function body")?;
        Ok(Stmt::Fn { name, params, body })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParserError> {
        self.advance(); // return
        let expr = match self.peek_kind() {
            TokenKind::Newline
            | TokenKind::Semicolon
            | TokenKind::End
            | TokenKind::Else
            | TokenKind::Eof => None,
            _ => Some(self.parse_expr()?),
        };
        Ok(Stmt::Return { expr })
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt, ParserError> {
        let expr = self.parse_expr()?;
        Ok(Stmt::ExprStmt { expr })
    }

    // ----- expressions (lowest precedence first) -----

    fn parse_expr(&mut self) -> Result<Expr, ParserError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            self.advance();
            let right = self.parse_and()?;
            expr = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_equality()?;
        while self.check(&TokenKind::And) {
            self.advance();
            let right = self.parse_equality()?;
            expr = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_comparison()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::EqEq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_term()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::GtEq => BinaryOp::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_term()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_term(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_factor()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_factor()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_factor(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParserError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                })
            }
            TokenKind::Not => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParserError> {
        match self.peek_kind().clone() {
            TokenKind::Int(n) => {
                self.advance();
                Ok(Expr::Int(n))
            }
            TokenKind::Float(x) => {
                self.advance();
                Ok(Expr::Float(x))
            }
            TokenKind::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            TokenKind::Ident(name) => {
                self.advance();
                if self.check(&TokenKind::LParen) {
                    self.advance(); // (
                    let mut args = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        args.push(self.parse_expr()?);
                        while self.check(&TokenKind::Comma) {
                            self.advance(); // ,
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(&TokenKind::RParen, "')' after call arguments")?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            TokenKind::LParen => {
                self.advance(); // (
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "')' after expression")?;
                Ok(expr)
            }
            _ => Err(self.error_here(format!(
                "expected expression but found {}",
                describe(self.peek())
            ))),
        }
    }
}

/// A short human-readable description of a token for error messages.
fn describe(token: &Token) -> String {
    match &token.kind {
        TokenKind::Eof => "end of input".to_string(),
        TokenKind::Newline => "newline".to_string(),
        TokenKind::Str(_) => "a string".to_string(),
        TokenKind::Int(_) | TokenKind::Float(_) => "a number".to_string(),
        TokenKind::Ident(name) => format!("'{name}'"),
        _ => format!("'{}'", token.value()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BinaryOp::*;

    fn parse_ok(source: &str) -> Program {
        let tokens = gg_lexer::lex(source).unwrap();
        parse(&tokens).unwrap()
    }

    fn parse_err(source: &str) -> ParserError {
        let tokens = gg_lexer::lex(source).unwrap();
        parse(&tokens).unwrap_err()
    }

    fn ident(name: &str) -> Expr {
        Expr::Ident(name.to_string())
    }

    fn int(n: i64) -> Expr {
        Expr::Int(n)
    }

    fn bin(op: BinaryOp, left: Expr, right: Expr) -> Expr {
        Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    #[test]
    fn canonical_example_exact_ast() {
        let prog = parse_ok(
            "let x = 10\nif x > 5 then\n    print \"big\"\nelse\n    print \"small\"\nend\n",
        );
        assert_eq!(
            prog,
            Program {
                stmts: vec![
                    Stmt::Let {
                        name: "x".to_string(),
                        value: int(10),
                    },
                    Stmt::If {
                        cond: bin(Gt, ident("x"), int(5)),
                        then_block: vec![Stmt::Print {
                            expr: Expr::Str("big".to_string()),
                        }],
                        else_block: Some(vec![Stmt::Print {
                            expr: Expr::Str("small".to_string()),
                        }]),
                    },
                ]
            }
        );
    }

    #[test]
    fn precedence() {
        // 1 + 2 * 3 = 1 + (2 * 3)
        assert_eq!(
            parse_ok("print 1 + 2 * 3").stmts,
            vec![Stmt::Print {
                expr: bin(Add, int(1), bin(Mul, int(2), int(3))),
            }]
        );
        // not a and b = (not a) and b
        assert_eq!(
            parse_ok("print not a and b").stmts,
            vec![Stmt::Print {
                expr: bin(
                    And,
                    Expr::Unary {
                        op: UnaryOp::Not,
                        expr: Box::new(ident("a")),
                    },
                    ident("b"),
                ),
            }]
        );
        // a == b and c = (a == b) and c
        assert_eq!(
            parse_ok("print a == b and c").stmts,
            vec![Stmt::Print {
                expr: bin(And, bin(Eq, ident("a"), ident("b")), ident("c")),
            }]
        );
        // a or b and c = a or (b and c)
        assert_eq!(
            parse_ok("print a or b and c").stmts,
            vec![Stmt::Print {
                expr: bin(Or, ident("a"), bin(And, ident("b"), ident("c"))),
            }]
        );
        // -x * y = (-x) * y
        assert_eq!(
            parse_ok("print -x * y").stmts,
            vec![Stmt::Print {
                expr: bin(
                    Mul,
                    Expr::Unary {
                        op: UnaryOp::Neg,
                        expr: Box::new(ident("x")),
                    },
                    ident("y"),
                ),
            }]
        );
        // a - b - c is left-associative: (a - b) - c
        assert_eq!(
            parse_ok("print a - b - c").stmts,
            vec![Stmt::Print {
                expr: bin(Sub, bin(Sub, ident("a"), ident("b")), ident("c")),
            }]
        );
    }

    #[test]
    fn parentheses_group_without_a_node() {
        assert_eq!(
            parse_ok("print (1 + 2) * 3").stmts,
            vec![Stmt::Print {
                expr: bin(Mul, bin(Add, int(1), int(2)), int(3)),
            }]
        );
    }

    #[test]
    fn nested_if_with_else() {
        let prog = parse_ok(
            "if a then\n    if b then\n        print 1\n    else\n        print 2\n    end\nelse\n    print 3\nend\n",
        );
        assert_eq!(
            prog,
            Program {
                stmts: vec![Stmt::If {
                    cond: ident("a"),
                    then_block: vec![Stmt::If {
                        cond: ident("b"),
                        then_block: vec![Stmt::Print { expr: int(1) }],
                        else_block: Some(vec![Stmt::Print { expr: int(2) }]),
                    }],
                    else_block: Some(vec![Stmt::Print { expr: int(3) }]),
                }],
            }
        );
    }

    #[test]
    fn while_loop() {
        assert_eq!(
            parse_ok("while x < 10 then\n    x = x + 1\nend").stmts,
            vec![Stmt::While {
                cond: bin(Lt, ident("x"), int(10)),
                body: vec![Stmt::Assign {
                    name: "x".to_string(),
                    value: bin(Add, ident("x"), int(1)),
                }],
            }]
        );
    }

    #[test]
    fn fn_with_zero_and_two_params_and_bare_return() {
        assert_eq!(
            parse_ok("fn greet()\n    print \"hi\"\nend").stmts,
            vec![Stmt::Fn {
                name: "greet".to_string(),
                params: vec![],
                body: vec![Stmt::Print {
                    expr: Expr::Str("hi".to_string()),
                }],
            }]
        );
        assert_eq!(
            parse_ok("fn add(a, b)\n    return a + b\nend").stmts,
            vec![Stmt::Fn {
                name: "add".to_string(),
                params: vec!["a".to_string(), "b".to_string()],
                body: vec![Stmt::Return {
                    expr: Some(bin(Add, ident("a"), ident("b"))),
                }],
            }]
        );
        assert_eq!(
            parse_ok("fn f()\n    return\nend").stmts,
            vec![Stmt::Fn {
                name: "f".to_string(),
                params: vec![],
                body: vec![Stmt::Return { expr: None }],
            }]
        );
    }

    #[test]
    fn for_to_loop() {
        assert_eq!(
            parse_ok("for i = 1 to 10 then\n    print i\nend").stmts,
            vec![Stmt::For {
                var: "i".to_string(),
                from: int(1),
                to: int(10),
                body: vec![Stmt::Print { expr: ident("i") }],
            }]
        );
        // Bounds may be expressions.
        assert_eq!(
            parse_ok("for i = 1 + 1 to n * 2 then\nend").stmts,
            vec![Stmt::For {
                var: "i".to_string(),
                from: bin(Add, int(1), int(1)),
                to: bin(Mul, ident("n"), int(2)),
                body: vec![],
            }]
        );
    }

    #[test]
    fn calls_with_zero_and_three_args() {
        assert_eq!(
            parse_ok("f()").stmts,
            vec![Stmt::ExprStmt {
                expr: Expr::Call {
                    name: "f".to_string(),
                    args: vec![],
                },
            }]
        );
        assert_eq!(
            parse_ok("f(1, x, \"s\")").stmts,
            vec![Stmt::ExprStmt {
                expr: Expr::Call {
                    name: "f".to_string(),
                    args: vec![int(1), ident("x"), Expr::Str("s".to_string()),],
                },
            }]
        );
    }

    #[test]
    fn assignment_is_a_statement_not_an_expression() {
        assert_eq!(
            parse_ok("x = 1 + 2").stmts,
            vec![Stmt::Assign {
                name: "x".to_string(),
                value: bin(Add, int(1), int(2)),
            }]
        );
        // `==` is a comparison, not an assignment.
        assert_eq!(
            parse_ok("x == 1").stmts,
            vec![Stmt::ExprStmt {
                expr: bin(Eq, ident("x"), int(1)),
            }]
        );
    }

    #[test]
    fn separators_and_empty_programs() {
        assert_eq!(parse_ok("").stmts, vec![]);
        assert_eq!(parse_ok("\n\n;;;").stmts, vec![]);
        assert_eq!(
            parse_ok("let a = 1; let b = 2\nlet c = 3").stmts,
            vec![
                Stmt::Let {
                    name: "a".to_string(),
                    value: int(1),
                },
                Stmt::Let {
                    name: "b".to_string(),
                    value: int(2),
                },
                Stmt::Let {
                    name: "c".to_string(),
                    value: int(3),
                },
            ]
        );
        // A block may close on the same line as its last statement.
        assert_eq!(
            parse_ok("if x then print 1 end").stmts,
            vec![Stmt::If {
                cond: ident("x"),
                then_block: vec![Stmt::Print { expr: int(1) }],
                else_block: None,
            }]
        );
    }

    #[test]
    fn literals() {
        assert_eq!(
            parse_ok("print 42\nprint 3.15\nprint \"s\"\nprint true\nprint false").stmts,
            vec![
                Stmt::Print {
                    expr: Expr::Int(42),
                },
                Stmt::Print {
                    expr: Expr::Float(3.15),
                },
                Stmt::Print {
                    expr: Expr::Str("s".to_string()),
                },
                Stmt::Print {
                    expr: Expr::Bool(true),
                },
                Stmt::Print {
                    expr: Expr::Bool(false),
                },
            ]
        );
    }

    #[test]
    fn missing_end() {
        let err = parse_err("if x then\n    print 1\n");
        assert!(err.message.contains("expected 'end'"), "{}", err.message);
        assert!(err.message.contains("end of input"), "{}", err.message);
        assert_eq!((err.line, err.col), (3, 1));
    }

    #[test]
    fn missing_then() {
        let err = parse_err("if x\n    print 1\nend");
        assert!(err.message.contains("expected 'then'"), "{}", err.message);
        assert!(err.message.contains("found newline"), "{}", err.message);
        assert_eq!((err.line, err.col), (1, 5));
    }

    #[test]
    fn unclosed_paren() {
        let err = parse_err("print (1 + 2");
        assert!(err.message.contains("expected ')'"), "{}", err.message);
    }

    #[test]
    fn let_without_name() {
        let err = parse_err("let = 5");
        assert!(
            err.message.contains("expected variable name"),
            "{}",
            err.message
        );
        assert!(err.message.contains("found '='"), "{}", err.message);
    }

    #[test]
    fn if_without_condition() {
        let err = parse_err("if then\nend");
        assert!(
            err.message.contains("expected expression"),
            "{}",
            err.message
        );
        assert!(err.message.contains("found 'then'"), "{}", err.message);
    }

    #[test]
    fn stray_end_and_missing_separator() {
        let err = parse_err("end");
        assert!(err.message.contains("unexpected 'end'"), "{}", err.message);

        let err = parse_err("let a = 1 let b = 2");
        assert!(
            err.message.contains("expected newline or ';'"),
            "{}",
            err.message
        );
    }

    #[test]
    fn display_round_trip_on_canonical_example() {
        let source =
            "let x = 10\nif x > 5 then\n    print \"big\"\nelse\n    print \"small\"\nend\n";
        let prog = parse_ok(source);
        let printed = prog.to_string();
        assert_eq!(
            printed,
            "let x = 10\nif x > 5 then\n    print \"big\"\nelse\n    print \"small\"\nend"
        );
        let reparsed = parse_ok(&printed);
        assert_eq!(reparsed, prog);
    }

    #[test]
    fn display_round_trip_with_parens_and_floats() {
        let source = "let a = (1 + 2) * 3\nlet b = -(a + 1)\nlet c = not (a and true)\nlet d = 1.0\nprint a - (b - c)\n";
        let prog = parse_ok(source);
        let reparsed = parse_ok(&prog.to_string());
        assert_eq!(reparsed, prog);
    }
}
