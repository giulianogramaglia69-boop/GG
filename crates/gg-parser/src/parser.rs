// SPDX-License-Identifier: GPL-3.0-or-later
//! Recursive-descent parser for GG: tokens -> AST.
//!
//! Statements are separated by newlines and/or `;` (any number of
//! separators is allowed, including at the start and end of a program
//! or block). Blocks end with `end` — or with `else` in the `then`
//! branch of an `if`.

use crate::ast::{BinaryOp, Expr, MethodDef, Program, Stmt, UnaryOp};
use gg_lexer::{Span, Token, TokenKind};
use std::fmt;

/// A parse error: a plain-English message plus the 1-based
/// line/column of the token that caused it.
///
/// `incomplete` is true when the error was caused by unexpected end
/// of input (e.g. a missing `end`): the source could become valid if
/// more lines were added. The REPL uses it to decide whether to keep
/// reading instead of reporting the error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserError {
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub incomplete: bool,
}

impl ParserError {
    fn new(message: impl Into<String>, line: usize, col: usize, incomplete: bool) -> Self {
        Self {
            message: message.into(),
            line,
            col,
            incomplete,
        }
    }

    /// True when this error is only due to unexpected end of input.
    pub fn is_incomplete(&self) -> bool {
        self.incomplete
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
        // An error at end of input means the source may still become
        // valid with more lines (e.g. a block missing its `end`).
        let incomplete = token.kind == TokenKind::Eof;
        ParserError::new(message, token.line, token.col, incomplete)
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

    /// Like `expect_ident`, but also returns the identifier's span:
    /// field and method names carry the span of the name itself.
    fn expect_ident_span(&mut self, what: &str) -> Result<(String, Span), ParserError> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                let span = self.advance().span();
                Ok((name, span))
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
            TokenKind::Class => self.parse_class(),
            TokenKind::Import => self.parse_import(),
            TokenKind::Return => self.parse_return(),
            TokenKind::Print => self.parse_print(),
            _ => {
                // An assignment starts with an expression (a plain
                // name, an index like `xs[0]`, or a field like
                // `p.nome` / `self.eta`): if an `=` follows it, this
                // is an assignment; otherwise it is an expression
                // statement (`x == 1` is a comparison, not an
                // assignment).
                let expr = self.parse_expr()?;
                if self.check(&TokenKind::Eq) {
                    self.advance(); // =
                    let value = self.parse_expr()?;
                    // The statement starts where its target starts:
                    // the variable name, the `[` of an index, the
                    // field name of a field assignment.
                    let span = expr.span();
                    match expr {
                        Expr::Ident { name, .. } => Ok(Stmt::Assign { name, value, span }),
                        Expr::Index { .. } => Ok(Stmt::IndexAssign {
                            target: expr,
                            value,
                            span,
                        }),
                        Expr::Field { .. } => Ok(Stmt::FieldAssign {
                            target: expr,
                            value,
                            span,
                        }),
                        _ => Err(self.error_here("invalid assignment target")),
                    }
                } else {
                    let span = expr.span();
                    Ok(Stmt::ExprStmt { expr, span })
                }
            }
        }
    }

    fn parse_let(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // let
        let name = self.expect_ident("variable name after 'let'")?;
        self.expect(&TokenKind::Eq, "'=' after variable name")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Let { name, value, span })
    }

    fn parse_print(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // print
        let expr = self.parse_expr()?;
        Ok(Stmt::Print { expr, span })
    }

    fn parse_if(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // if
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
            span,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // while
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Then, "'then' after while condition")?;
        let body = self.parse_block()?;
        self.expect(&TokenKind::End, "'end' to close 'while' block")?;
        Ok(Stmt::While { cond, body, span })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // for
        let var = self.expect_ident("variable name after 'for'")?;
        if self.check(&TokenKind::In) {
            // for-in: `for x in xs then ... end`.
            self.advance(); // in
            let iter = self.parse_expr()?;
            self.expect(&TokenKind::Then, "'then' after for-in iterable")?;
            let body = self.parse_block()?;
            self.expect(&TokenKind::End, "'end' to close 'for' block")?;
            return Ok(Stmt::ForIn {
                var,
                iter,
                body,
                span,
            });
        }
        self.expect(&TokenKind::Eq, "'=' or 'in' in for loop")?;
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
            span,
        })
    }

    fn parse_fn(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // fn
        let name = self.expect_ident("function name after 'fn'")?;
        self.expect(&TokenKind::LParen, "'(' after function name")?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            params.push(self.expect_param_name()?);
            while self.check(&TokenKind::Comma) {
                self.advance(); // ,
                params.push(self.expect_param_name()?);
            }
        }
        self.expect(&TokenKind::RParen, "')' after parameter list")?;
        let body = self.parse_block()?;
        self.expect(&TokenKind::End, "'end' to close function body")?;
        Ok(Stmt::Fn {
            name,
            params,
            body,
            span,
        })
    }

    /// A parameter name: a plain identifier, or `self` (methods
    /// take `self` as their first parameter, like Python).
    fn expect_param_name(&mut self) -> Result<String, ParserError> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            TokenKind::SelfKw => {
                self.advance();
                Ok("self".to_string())
            }
            _ => Err(self.error_here(format!(
                "expected parameter name but found {}",
                describe(self.peek())
            ))),
        }
    }

    fn parse_class(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // class
        let name = self.expect_ident("class name after 'class'")?;
        let parent = if self.check(&TokenKind::Extends) {
            self.advance(); // extends
            Some(self.expect_ident("parent class name after 'extends'")?)
        } else {
            None
        };
        // The class body holds only method definitions.
        let mut methods = Vec::new();
        loop {
            self.skip_separators();
            match self.peek_kind() {
                TokenKind::End => {
                    self.advance(); // end
                    break;
                }
                TokenKind::Eof => {
                    return Err(self.error_here("expected 'end' to close class body"));
                }
                TokenKind::Fn => {
                    let fn_token = self.peek().clone();
                    let Stmt::Fn {
                        name: method_name,
                        params,
                        body,
                        ..
                    } = self.parse_fn()?
                    else {
                        unreachable!("parse_fn always returns Stmt::Fn")
                    };
                    if params.is_empty() || params[0] != "self" {
                        return Err(ParserError::new(
                            "methods must take 'self' as their first parameter",
                            fn_token.line,
                            fn_token.col,
                            false,
                        ));
                    }
                    methods.push(MethodDef {
                        name: method_name,
                        params,
                        body,
                    });
                }
                _ => {
                    return Err(
                        self.error_here("only method definitions (fn) are allowed inside a class")
                    );
                }
            }
        }
        Ok(Stmt::Class {
            name,
            parent,
            methods,
            span,
        })
    }

    fn parse_import(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // import
        let name = self.expect_ident("module name after 'import'")?;
        let alias = if self.check(&TokenKind::As) {
            self.advance(); // as
            Some(self.expect_ident("alias after 'as'")?)
        } else {
            None
        };
        Ok(Stmt::Import { name, alias, span })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParserError> {
        let span = self.advance().span(); // return
        let expr = match self.peek_kind() {
            TokenKind::Newline
            | TokenKind::Semicolon
            | TokenKind::End
            | TokenKind::Else
            | TokenKind::Eof => None,
            _ => Some(self.parse_expr()?),
        };
        Ok(Stmt::Return { expr, span })
    }

    // ----- expressions (lowest precedence first) -----

    fn parse_expr(&mut self) -> Result<Expr, ParserError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            let span = self.advance().span();
            let right = self.parse_and()?;
            expr = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(expr),
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_equality()?;
        while self.check(&TokenKind::And) {
            let span = self.advance().span();
            let right = self.parse_equality()?;
            expr = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(expr),
                right: Box::new(right),
                span,
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
            let span = self.advance().span();
            let right = self.parse_comparison()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
                span,
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
            let span = self.advance().span();
            let right = self.parse_term()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
                span,
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
            let span = self.advance().span();
            let right = self.parse_factor()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
                span,
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
            let span = self.advance().span();
            let right = self.parse_unary()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParserError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                let span = self.advance().span();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                    span,
                })
            }
            TokenKind::Not => {
                let span = self.advance().span();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                    span,
                })
            }
            _ => self.parse_postfix(),
        }
    }

    /// Postfix operators, as tight as a call: indexing (`xs[0]`,
    /// `f()[0]`, `xs[0][1]`) and field access / method calls
    /// (`p.nome`, `p.saluta("ciao")`). Applies after any primary.
    fn parse_postfix(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.check(&TokenKind::LBracket) {
                let span = self.advance().span(); // [
                let mut indices = vec![self.parse_index_item()?];
                while self.check(&TokenKind::Comma) {
                    self.advance(); // ,
                    indices.push(self.parse_index_item()?);
                }
                self.expect(&TokenKind::RBracket, "']' after index")?;
                expr = Expr::Index {
                    target: Box::new(expr),
                    indices,
                    span,
                };
            } else if self.check(&TokenKind::Dot) {
                self.advance(); // .
                let (name, span) = self.expect_ident_span("field name after '.'")?;
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
                    self.expect(&TokenKind::RParen, "')' after method arguments")?;
                    expr = Expr::MethodCall {
                        target: Box::new(expr),
                        name,
                        args,
                        span,
                    };
                } else {
                    expr = Expr::Field {
                        target: Box::new(expr),
                        name,
                        span,
                    };
                }
            } else {
                break;
            }
        }
        Ok(expr)
    }

    /// One item inside `[...]`: a plain index expression, or a
    /// slice `start:stop:step` where each bound is optional
    /// (`a[1:4]`, `a[:3]`, `a[2:]`, `a[::2]`, `a[::-1]`, `a[:]`).
    /// A `:` after the first expression (or at the start) makes it
    /// a slice; otherwise it is a plain index.
    fn parse_index_item(&mut self) -> Result<Expr, ParserError> {
        if self.check(&TokenKind::Colon) {
            return self.parse_slice(None);
        }
        let first = self.parse_expr()?;
        if self.check(&TokenKind::Colon) {
            return self.parse_slice(Some(first));
        }
        Ok(first)
    }

    /// The `:` and the bounds after it: `start` was already parsed
    /// (or is `None` when the slice starts with `:`). A bound is
    /// missing when the next token is `:`, `,` or `]`.
    fn parse_slice(&mut self, start: Option<Expr>) -> Result<Expr, ParserError> {
        let span = self.expect(&TokenKind::Colon, "':' in slice")?.span();
        let stop = if self.at_slice_bound_end() {
            None
        } else {
            Some(self.parse_expr()?)
        };
        let step = if self.check(&TokenKind::Colon) {
            self.advance(); // :
            if self.at_slice_bound_end() {
                None
            } else {
                Some(self.parse_expr()?)
            }
        } else {
            None
        };
        Ok(Expr::Slice {
            start: start.map(Box::new),
            stop: stop.map(Box::new),
            step: step.map(Box::new),
            span,
        })
    }

    /// True when a slice bound ends here: the next token closes the
    /// bound (`:` starts the step, `,` the next index, `]` the
    /// brackets).
    fn at_slice_bound_end(&self) -> bool {
        self.check(&TokenKind::Colon)
            || self.check(&TokenKind::Comma)
            || self.check(&TokenKind::RBracket)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParserError> {
        match self.peek_kind().clone() {
            TokenKind::Int(n) => {
                let span = self.advance().span();
                Ok(Expr::Int { value: n, span })
            }
            TokenKind::Float(x) => {
                let span = self.advance().span();
                Ok(Expr::Float { value: x, span })
            }
            TokenKind::Str(s) => {
                let span = self.advance().span();
                Ok(Expr::Str { value: s, span })
            }
            TokenKind::True => {
                let span = self.advance().span();
                Ok(Expr::Bool { value: true, span })
            }
            TokenKind::False => {
                let span = self.advance().span();
                Ok(Expr::Bool { value: false, span })
            }
            TokenKind::SelfKw => {
                let span = self.advance().span();
                Ok(Expr::SelfExpr { span })
            }
            TokenKind::Super => {
                self.advance(); // super
                self.expect(&TokenKind::Dot, "'.' after 'super'")?;
                let (name, span) = self.expect_ident_span("method name after 'super.'")?;
                self.expect(&TokenKind::LParen, "'(' after super method name")?;
                let mut args = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    args.push(self.parse_expr()?);
                    while self.check(&TokenKind::Comma) {
                        self.advance(); // ,
                        args.push(self.parse_expr()?);
                    }
                }
                self.expect(&TokenKind::RParen, "')' after super method arguments")?;
                Ok(Expr::SuperCall { name, args, span })
            }
            TokenKind::Ident(name) => {
                let span = self.advance().span();
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
                    Ok(Expr::Call { name, args, span })
                } else {
                    Ok(Expr::Ident { name, span })
                }
            }
            TokenKind::LParen => {
                let span = self.advance().span(); // (
                if self.check(&TokenKind::RParen) {
                    self.advance(); // )
                    return Ok(Expr::Tuple {
                        elements: vec![],
                        span,
                    });
                }
                let first = self.parse_expr()?;
                if self.check(&TokenKind::Comma) {
                    // Tuple: one or more elements, trailing comma
                    // allowed (`(1,)` is a one-element tuple).
                    let mut elements = vec![first];
                    while self.check(&TokenKind::Comma) {
                        self.advance(); // ,
                        if self.check(&TokenKind::RParen) {
                            break;
                        }
                        elements.push(self.parse_expr()?);
                    }
                    self.expect(&TokenKind::RParen, "')' after tuple elements")?;
                    Ok(Expr::Tuple { elements, span })
                } else {
                    self.expect(&TokenKind::RParen, "')' after expression")?;
                    Ok(first)
                }
            }
            TokenKind::LBrace => self.parse_brace(),
            TokenKind::LBracket => {
                let span = self.advance().span(); // [
                let mut elements = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    elements.push(self.parse_expr()?);
                    while self.check(&TokenKind::Comma) {
                        self.advance(); // ,
                        elements.push(self.parse_expr()?);
                    }
                }
                self.expect(&TokenKind::RBracket, "']' after list elements")?;
                Ok(Expr::List { elements, span })
            }
            _ => Err(self.error_here(format!(
                "expected expression but found {}",
                describe(self.peek())
            ))),
        }
    }

    /// `{...}`: a dict when the entries have `:`, a set otherwise.
    /// `{}` is the empty dict (Python convention).
    fn parse_brace(&mut self) -> Result<Expr, ParserError> {
        let span = self.advance().span(); // {
        if self.check(&TokenKind::RBrace) {
            self.advance(); // }
            return Ok(Expr::Dict {
                entries: vec![],
                span,
            });
        }
        let first = self.parse_expr()?;
        if self.check(&TokenKind::Colon) {
            // Dict: key: value, key: value, ... (trailing comma ok).
            let mut entries = Vec::new();
            self.advance(); // :
            let value = self.parse_expr()?;
            entries.push((first, value));
            while self.check(&TokenKind::Comma) {
                self.advance(); // ,
                if self.check(&TokenKind::RBrace) {
                    break;
                }
                let key = self.parse_expr()?;
                self.expect(&TokenKind::Colon, "':' after dict key")?;
                let value = self.parse_expr()?;
                entries.push((key, value));
            }
            self.expect(&TokenKind::RBrace, "'}' after dict entries")?;
            Ok(Expr::Dict { entries, span })
        } else {
            // Set: elements separated by commas (trailing comma ok).
            let mut elements = vec![first];
            while self.check(&TokenKind::Comma) {
                self.advance(); // ,
                if self.check(&TokenKind::RBrace) {
                    break;
                }
                elements.push(self.parse_expr()?);
            }
            self.expect(&TokenKind::RBrace, "'}' after set elements")?;
            Ok(Expr::Set { elements, span })
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
        Expr::Ident {
            name: name.to_string(),
            span: Span::dummy(),
        }
    }

    fn int(n: i64) -> Expr {
        Expr::Int {
            value: n,
            span: Span::dummy(),
        }
    }

    fn bin(op: BinaryOp, left: Expr, right: Expr) -> Expr {
        Expr::Binary {
            span: Span::dummy(),
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
                        span: Span::dummy(),
                        name: "x".to_string(),
                        value: int(10),
                    },
                    Stmt::If {
                        span: Span::dummy(),
                        cond: bin(Gt, ident("x"), int(5)),
                        then_block: vec![Stmt::Print {
                            span: Span::dummy(),
                            expr: Expr::Str {
                                value: "big".to_string(),
                                span: Span::dummy()
                            },
                        }],
                        else_block: Some(vec![Stmt::Print {
                            span: Span::dummy(),
                            expr: Expr::Str {
                                value: "small".to_string(),
                                span: Span::dummy()
                            },
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
                span: Span::dummy(),
                expr: bin(Add, int(1), bin(Mul, int(2), int(3))),
            }]
        );
        // not a and b = (not a) and b
        assert_eq!(
            parse_ok("print not a and b").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: bin(
                    And,
                    Expr::Unary {
                        span: Span::dummy(),
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
                span: Span::dummy(),
                expr: bin(And, bin(Eq, ident("a"), ident("b")), ident("c")),
            }]
        );
        // a or b and c = a or (b and c)
        assert_eq!(
            parse_ok("print a or b and c").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: bin(Or, ident("a"), bin(And, ident("b"), ident("c"))),
            }]
        );
        // -x * y = (-x) * y
        assert_eq!(
            parse_ok("print -x * y").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: bin(
                    Mul,
                    Expr::Unary {
                        span: Span::dummy(),
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
                span: Span::dummy(),
                expr: bin(Sub, bin(Sub, ident("a"), ident("b")), ident("c")),
            }]
        );
    }

    #[test]
    fn parentheses_group_without_a_node() {
        assert_eq!(
            parse_ok("print (1 + 2) * 3").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
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
                    span: Span::dummy(),
                    cond: ident("a"),
                    then_block: vec![Stmt::If {
                        span: Span::dummy(),
                        cond: ident("b"),
                        then_block: vec![Stmt::Print {
                            span: Span::dummy(),
                            expr: int(1)
                        }],
                        else_block: Some(vec![Stmt::Print {
                            span: Span::dummy(),
                            expr: int(2)
                        }]),
                    }],
                    else_block: Some(vec![Stmt::Print {
                        span: Span::dummy(),
                        expr: int(3)
                    }]),
                }],
            }
        );
    }

    #[test]
    fn while_loop() {
        assert_eq!(
            parse_ok("while x < 10 then\n    x = x + 1\nend").stmts,
            vec![Stmt::While {
                span: Span::dummy(),
                cond: bin(Lt, ident("x"), int(10)),
                body: vec![Stmt::Assign {
                    span: Span::dummy(),
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
                span: Span::dummy(),
                name: "greet".to_string(),
                params: vec![],
                body: vec![Stmt::Print {
                    span: Span::dummy(),
                    expr: Expr::Str {
                        value: "hi".to_string(),
                        span: Span::dummy()
                    },
                }],
            }]
        );
        assert_eq!(
            parse_ok("fn add(a, b)\n    return a + b\nend").stmts,
            vec![Stmt::Fn {
                span: Span::dummy(),
                name: "add".to_string(),
                params: vec!["a".to_string(), "b".to_string()],
                body: vec![Stmt::Return {
                    span: Span::dummy(),
                    expr: Some(bin(Add, ident("a"), ident("b"))),
                }],
            }]
        );
        assert_eq!(
            parse_ok("fn f()\n    return\nend").stmts,
            vec![Stmt::Fn {
                span: Span::dummy(),
                name: "f".to_string(),
                params: vec![],
                body: vec![Stmt::Return {
                    span: Span::dummy(),
                    expr: None
                }],
            }]
        );
    }

    #[test]
    fn for_to_loop() {
        assert_eq!(
            parse_ok("for i = 1 to 10 then\n    print i\nend").stmts,
            vec![Stmt::For {
                span: Span::dummy(),
                var: "i".to_string(),
                from: int(1),
                to: int(10),
                body: vec![Stmt::Print {
                    span: Span::dummy(),
                    expr: ident("i")
                }],
            }]
        );
        // Bounds may be expressions.
        assert_eq!(
            parse_ok("for i = 1 + 1 to n * 2 then\nend").stmts,
            vec![Stmt::For {
                span: Span::dummy(),
                var: "i".to_string(),
                from: bin(Add, int(1), int(1)),
                to: bin(Mul, ident("n"), int(2)),
                body: vec![],
            }]
        );
    }

    #[test]
    fn for_in_loop() {
        assert_eq!(
            parse_ok("for x in xs then\n    print x\nend").stmts,
            vec![Stmt::ForIn {
                span: Span::dummy(),
                var: "x".to_string(),
                iter: ident("xs"),
                body: vec![Stmt::Print {
                    span: Span::dummy(),
                    expr: ident("x")
                }],
            }]
        );
        // The iterable may be any expression, and the Display
        // prints back in canonical form (round-trip).
        for source in [
            "for x in xs then\n    print x\nend",
            "for x in range(5) then\n    print x\nend",
            "for x in [1, 2] then\nend",
            "for k in d then\n    print k\nend",
        ] {
            let prog = parse_ok(source);
            assert_eq!(prog.to_string(), source);
            assert_eq!(parse_ok(&prog.to_string()), prog);
        }
        // Nested for-in loops.
        assert_eq!(
            parse_ok("for x in xs then\n    for y in ys then\n        print x\n    end\nend").stmts,
            vec![Stmt::ForIn {
                span: Span::dummy(),
                var: "x".to_string(),
                iter: ident("xs"),
                body: vec![Stmt::ForIn {
                    span: Span::dummy(),
                    var: "y".to_string(),
                    iter: ident("ys"),
                    body: vec![Stmt::Print {
                        span: Span::dummy(),
                        expr: ident("x")
                    }],
                }],
            }]
        );
        // The numeric for is untouched by the new form.
        assert_eq!(
            parse_ok("for i = 1 to 3 then\n    print i\nend").stmts,
            vec![Stmt::For {
                span: Span::dummy(),
                var: "i".to_string(),
                from: int(1),
                to: int(3),
                body: vec![Stmt::Print {
                    span: Span::dummy(),
                    expr: ident("i")
                }],
            }]
        );
    }

    #[test]
    fn for_needs_equals_or_in() {
        let err = parse_err("for x then\nend");
        assert!(
            err.message.contains("expected '=' or 'in'"),
            "{}",
            err.message
        );
        // A missing `end` on a for-in is incomplete, like any block.
        let err = parse_err("for x in xs then\n    print x\n");
        assert!(err.message.contains("expected 'end'"), "{}", err.message);
        assert!(err.is_incomplete());
    }

    #[test]
    fn calls_with_zero_and_three_args() {
        assert_eq!(
            parse_ok("f()").stmts,
            vec![Stmt::ExprStmt {
                span: Span::dummy(),
                expr: Expr::Call {
                    span: Span::dummy(),
                    name: "f".to_string(),
                    args: vec![],
                },
            }]
        );
        assert_eq!(
            parse_ok("f(1, x, \"s\")").stmts,
            vec![Stmt::ExprStmt {
                span: Span::dummy(),
                expr: Expr::Call {
                    span: Span::dummy(),
                    name: "f".to_string(),
                    args: vec![
                        int(1),
                        ident("x"),
                        Expr::Str {
                            value: "s".to_string(),
                            span: Span::dummy()
                        },
                    ],
                },
            }]
        );
    }

    #[test]
    fn assignment_is_a_statement_not_an_expression() {
        assert_eq!(
            parse_ok("x = 1 + 2").stmts,
            vec![Stmt::Assign {
                span: Span::dummy(),
                name: "x".to_string(),
                value: bin(Add, int(1), int(2)),
            }]
        );
        // `==` is a comparison, not an assignment.
        assert_eq!(
            parse_ok("x == 1").stmts,
            vec![Stmt::ExprStmt {
                span: Span::dummy(),
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
                    span: Span::dummy(),
                    name: "a".to_string(),
                    value: int(1),
                },
                Stmt::Let {
                    span: Span::dummy(),
                    name: "b".to_string(),
                    value: int(2),
                },
                Stmt::Let {
                    span: Span::dummy(),
                    name: "c".to_string(),
                    value: int(3),
                },
            ]
        );
        // A block may close on the same line as its last statement.
        assert_eq!(
            parse_ok("if x then print 1 end").stmts,
            vec![Stmt::If {
                span: Span::dummy(),
                cond: ident("x"),
                then_block: vec![Stmt::Print {
                    span: Span::dummy(),
                    expr: int(1)
                }],
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
                    span: Span::dummy(),
                    expr: Expr::Int {
                        value: 42,
                        span: Span::dummy()
                    },
                },
                Stmt::Print {
                    span: Span::dummy(),
                    expr: Expr::Float {
                        value: 3.15,
                        span: Span::dummy()
                    },
                },
                Stmt::Print {
                    span: Span::dummy(),
                    expr: Expr::Str {
                        value: "s".to_string(),
                        span: Span::dummy()
                    },
                },
                Stmt::Print {
                    span: Span::dummy(),
                    expr: Expr::Bool {
                        value: true,
                        span: Span::dummy()
                    },
                },
                Stmt::Print {
                    span: Span::dummy(),
                    expr: Expr::Bool {
                        value: false,
                        span: Span::dummy()
                    },
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

    #[test]
    fn list_literals() {
        assert_eq!(
            parse_ok("print [1, 2, 3]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::List {
                    elements: vec![int(1), int(2), int(3)],
                    span: Span::dummy()
                },
            }]
        );
        // Empty, mixed and nested lists.
        assert_eq!(
            parse_ok("print []").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::List {
                    elements: vec![],
                    span: Span::dummy()
                },
            }]
        );
        assert_eq!(
            parse_ok("print [1, \"ciao\", true]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::List {
                    elements: vec![
                        int(1),
                        Expr::Str {
                            value: "ciao".to_string(),
                            span: Span::dummy()
                        },
                        Expr::Bool {
                            value: true,
                            span: Span::dummy()
                        },
                    ],
                    span: Span::dummy()
                },
            }]
        );
        assert_eq!(
            parse_ok("print [[1, 2], [3]]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::List {
                    elements: vec![
                        Expr::List {
                            elements: vec![int(1), int(2)],
                            span: Span::dummy()
                        },
                        Expr::List {
                            elements: vec![int(3)],
                            span: Span::dummy()
                        },
                    ],
                    span: Span::dummy()
                },
            }]
        );
        // Elements may be any expression.
        assert_eq!(
            parse_ok("print [1 + 2, f(3)]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::List {
                    elements: vec![
                        bin(Add, int(1), int(2)),
                        Expr::Call {
                            span: Span::dummy(),
                            name: "f".to_string(),
                            args: vec![int(3)],
                        },
                    ],
                    span: Span::dummy()
                },
            }]
        );
    }

    #[test]
    fn index_is_a_tight_postfix() {
        // xs[0]
        assert_eq!(
            parse_ok("print xs[0]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(ident("xs")),
                    indices: vec![int(0)],
                },
            }]
        );
        // Negative index is just a unary minus inside the brackets.
        assert_eq!(
            parse_ok("print xs[-1]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(ident("xs")),
                    indices: vec![Expr::Unary {
                        span: Span::dummy(),
                        op: UnaryOp::Neg,
                        expr: Box::new(int(1)),
                    }],
                },
            }]
        );
        // Chained indexes and indexing a call result.
        assert_eq!(
            parse_ok("print xs[0][1]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(Expr::Index {
                        span: Span::dummy(),
                        target: Box::new(ident("xs")),
                        indices: vec![int(0)],
                    }),
                    indices: vec![int(1)],
                },
            }]
        );
        assert_eq!(
            parse_ok("print f()[0]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(Expr::Call {
                        span: Span::dummy(),
                        name: "f".to_string(),
                        args: vec![],
                    }),
                    indices: vec![int(0)],
                },
            }]
        );
        // Indexing binds tighter than `+`: xs[0] + 1, not xs[0 + 1].
        assert_eq!(
            parse_ok("print xs[0] + 1").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: bin(
                    Add,
                    Expr::Index {
                        span: Span::dummy(),
                        target: Box::new(ident("xs")),
                        indices: vec![int(0)],
                    },
                    int(1),
                ),
            }]
        );
        // Multi-index: a[0, 1] is one Index with two indices, and
        // it prints back the same way (round-trip).
        assert_eq!(
            parse_ok("print a[0, 1]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(ident("a")),
                    indices: vec![int(0), int(1)],
                },
            }]
        );
        assert_eq!(parse_ok("print a[0, 1]").to_string(), "print a[0, 1]");
        assert_eq!(
            parse_ok("print a[-1, -2]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(ident("a")),
                    indices: vec![
                        Expr::Unary {
                            span: Span::dummy(),
                            op: UnaryOp::Neg,
                            expr: Box::new(int(1)),
                        },
                        Expr::Unary {
                            span: Span::dummy(),
                            op: UnaryOp::Neg,
                            expr: Box::new(int(2)),
                        },
                    ],
                },
            }]
        );
    }

    #[test]
    fn slice_parsing_and_display() {
        // A slice is one index item with optional bounds.
        assert_eq!(
            parse_ok("print xs[1:4]").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(ident("xs")),
                    indices: vec![Expr::Slice {
                        span: Span::dummy(),
                        start: Some(Box::new(int(1))),
                        stop: Some(Box::new(int(4))),
                        step: None,
                    }],
                },
            }]
        );
        // Missing bounds print back empty; the step prints only
        // when present (round-trip through Display + re-parse).
        for source in [
            "print xs[1:4]",
            "print xs[:3]",
            "print xs[2:]",
            "print xs[::2]",
            "print xs[::-1]",
            "print xs[:]",
            "print xs[1:4:2]",
            "print xs[-3:-1]",
            "print a[0:2, 1:3]",
            "print a[:, 1]",
            "print a[0, :]",
            "print a[1:4, 0]",
        ] {
            let prog = parse_ok(source);
            assert_eq!(prog.to_string(), source);
            assert_eq!(parse_ok(&prog.to_string()), prog);
        }
        // Slice bounds are expressions, not just literals.
        assert_eq!(
            parse_ok("print xs[n + 1:len(xs)]").to_string(),
            "print xs[n + 1:len(xs)]"
        );
        // A slice assignment parses like any index assignment.
        assert_eq!(
            parse_ok("xs[1:3] = [9, 9]").stmts,
            vec![Stmt::IndexAssign {
                span: Span::dummy(),
                target: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(ident("xs")),
                    indices: vec![Expr::Slice {
                        span: Span::dummy(),
                        start: Some(Box::new(int(1))),
                        stop: Some(Box::new(int(3))),
                        step: None,
                    }],
                },
                value: Expr::List {
                    elements: vec![int(9), int(9)],
                    span: Span::dummy()
                },
            }]
        );
    }

    #[test]
    fn index_assignment() {
        assert_eq!(
            parse_ok("xs[0] = 99").stmts,
            vec![Stmt::IndexAssign {
                span: Span::dummy(),
                target: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(ident("xs")),
                    indices: vec![int(0)],
                },
                value: int(99),
            }]
        );
        // Nested index assignment, with an expression on the right.
        assert_eq!(
            parse_ok("m[0][1] = 1 + 2").stmts,
            vec![Stmt::IndexAssign {
                span: Span::dummy(),
                target: Expr::Index {
                    span: Span::dummy(),
                    target: Box::new(Expr::Index {
                        span: Span::dummy(),
                        target: Box::new(ident("m")),
                        indices: vec![int(0)],
                    }),
                    indices: vec![int(1)],
                },
                value: bin(Add, int(1), int(2)),
            }]
        );
        // A call result is not a valid assignment target.
        let err = parse_err("f() = 1");
        assert!(
            err.message.contains("invalid assignment target"),
            "{}",
            err.message
        );
        // A plain variable assignment still works, and `==` is still
        // a comparison.
        assert_eq!(
            parse_ok("x = 5").stmts,
            vec![Stmt::Assign {
                span: Span::dummy(),
                name: "x".to_string(),
                value: int(5),
            }]
        );
        assert_eq!(
            parse_ok("xs[0] == 5").stmts,
            vec![Stmt::ExprStmt {
                span: Span::dummy(),
                expr: bin(
                    Eq,
                    Expr::Index {
                        span: Span::dummy(),
                        target: Box::new(ident("xs")),
                        indices: vec![int(0)],
                    },
                    int(5),
                ),
            }]
        );
    }

    #[test]
    fn list_display_round_trip() {
        let source = "let xs = [1, \"a\", [2, 3]]\nprint xs[0]\nxs[1] = 9\n";
        let prog = parse_ok(source);
        assert_eq!(
            prog.to_string(),
            "let xs = [1, \"a\", [2, 3]]\nprint xs[0]\nxs[1] = 9"
        );
        let reparsed = parse_ok(&prog.to_string());
        assert_eq!(reparsed, prog);
    }

    #[test]
    fn tuple_literals_and_grouping() {
        // A two-element tuple.
        assert_eq!(
            parse_ok("print (1, \"a\")").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Tuple {
                    elements: vec![
                        int(1),
                        Expr::Str {
                            value: "a".to_string(),
                            span: Span::dummy()
                        }
                    ],
                    span: Span::dummy()
                },
            }]
        );
        // The empty tuple.
        assert_eq!(
            parse_ok("print ()").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Tuple {
                    elements: vec![],
                    span: Span::dummy()
                },
            }]
        );
        // One element with a trailing comma is a tuple...
        assert_eq!(
            parse_ok("print (1,)").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Tuple {
                    elements: vec![int(1)],
                    span: Span::dummy()
                },
            }]
        );
        // ...but without the comma it is just grouping.
        assert_eq!(
            parse_ok("print (1)").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: int(1)
            }]
        );
        assert_eq!(
            parse_ok("print (1 + 2)").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: bin(Add, int(1), int(2)),
            }]
        );
        // Trailing comma after several elements is fine.
        assert_eq!(
            parse_ok("print (1, 2,)").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Tuple {
                    elements: vec![int(1), int(2)],
                    span: Span::dummy()
                },
            }]
        );
        // Elements may be any expression, including nested tuples.
        assert_eq!(
            parse_ok("print ((1, 2), f(3))").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Tuple {
                    elements: vec![
                        Expr::Tuple {
                            elements: vec![int(1), int(2)],
                            span: Span::dummy()
                        },
                        Expr::Call {
                            span: Span::dummy(),
                            name: "f".to_string(),
                            args: vec![int(3)],
                        },
                    ],
                    span: Span::dummy()
                },
            }]
        );
    }

    #[test]
    fn dict_literals() {
        assert_eq!(
            parse_ok("print {\"nome\": \"Giuliano\", \"eta\": 14}").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Dict {
                    entries: vec![
                        (
                            Expr::Str {
                                value: "nome".to_string(),
                                span: Span::dummy()
                            },
                            Expr::Str {
                                value: "Giuliano".to_string(),
                                span: Span::dummy()
                            }
                        ),
                        (
                            Expr::Str {
                                value: "eta".to_string(),
                                span: Span::dummy()
                            },
                            int(14)
                        ),
                    ],
                    span: Span::dummy()
                },
            }]
        );
        // The empty braces are the empty dict.
        assert_eq!(
            parse_ok("print {}").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Dict {
                    entries: vec![],
                    span: Span::dummy()
                },
            }]
        );
        // Keys may be any expression (validated at runtime).
        assert_eq!(
            parse_ok("print {1: \"a\", true: 2}").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Dict {
                    entries: vec![
                        (
                            int(1),
                            Expr::Str {
                                value: "a".to_string(),
                                span: Span::dummy()
                            }
                        ),
                        (
                            Expr::Bool {
                                value: true,
                                span: Span::dummy()
                            },
                            int(2)
                        ),
                    ],
                    span: Span::dummy()
                },
            }]
        );
        // A trailing comma is fine.
        assert_eq!(
            parse_ok("print {\"a\": 1,}").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Dict {
                    entries: vec![(
                        Expr::Str {
                            value: "a".to_string(),
                            span: Span::dummy()
                        },
                        int(1)
                    )],
                    span: Span::dummy()
                },
            }]
        );
    }

    #[test]
    fn set_literals() {
        assert_eq!(
            parse_ok("print {1, 2, 3}").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Set {
                    elements: vec![int(1), int(2), int(3)],
                    span: Span::dummy()
                },
            }]
        );
        assert_eq!(
            parse_ok("print {\"a\"}").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Set {
                    elements: vec![Expr::Str {
                        value: "a".to_string(),
                        span: Span::dummy()
                    }],
                    span: Span::dummy()
                },
            }]
        );
        // A trailing comma is fine.
        assert_eq!(
            parse_ok("print {1,}").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Set {
                    elements: vec![int(1)],
                    span: Span::dummy()
                },
            }]
        );
        // A missing colon in a dict entry is an error...
        let err = parse_err("print {\"a\": 1, 2}");
        assert!(
            err.message.contains("':' after dict key"),
            "{}",
            err.message
        );
        // ...and a colon inside a set is an error too.
        let err = parse_err("print {1, \"a\": 2}");
        assert!(
            err.message.contains("expected '}' after set elements"),
            "{}",
            err.message
        );
        // An unclosed brace is incomplete (the REPL keeps reading).
        assert!(parse_err("print {1, 2").is_incomplete());
    }

    #[test]
    fn collections_display_round_trip() {
        let source = "let t = (1, \"a\")\nlet e = ()\nlet s = (1,)\nlet d = {\"k\": [1, 2]}\nlet u = {1, 2}\nprint t\nprint e\nprint s\nprint d\nprint u\n";
        let prog = parse_ok(source);
        assert_eq!(
            prog.to_string(),
            "let t = (1, \"a\")\nlet e = ()\nlet s = (1,)\nlet d = {\"k\": [1, 2]}\nlet u = {1, 2}\nprint t\nprint e\nprint s\nprint d\nprint u"
        );
        let reparsed = parse_ok(&prog.to_string());
        assert_eq!(reparsed, prog);
    }

    #[test]
    fn class_with_init_and_methods() {
        let src = "class Persona\n    fn init(self, nome, eta)\n        self.nome = nome\n        self.eta = eta\n    end\n    fn saluta(self)\n        print self.nome\n    end\nend\n";
        assert_eq!(
            parse_ok(src).stmts,
            vec![Stmt::Class {
                span: Span::dummy(),
                name: "Persona".to_string(),
                parent: None,
                methods: vec![
                    MethodDef {
                        name: "init".to_string(),
                        params: vec!["self".to_string(), "nome".to_string(), "eta".to_string()],
                        body: vec![
                            Stmt::FieldAssign {
                                span: Span::dummy(),
                                target: Expr::Field {
                                    span: Span::dummy(),
                                    target: Box::new(Expr::SelfExpr {
                                        span: Span::dummy()
                                    }),
                                    name: "nome".to_string(),
                                },
                                value: ident("nome"),
                            },
                            Stmt::FieldAssign {
                                span: Span::dummy(),
                                target: Expr::Field {
                                    span: Span::dummy(),
                                    target: Box::new(Expr::SelfExpr {
                                        span: Span::dummy()
                                    }),
                                    name: "eta".to_string(),
                                },
                                value: ident("eta"),
                            },
                        ],
                    },
                    MethodDef {
                        name: "saluta".to_string(),
                        params: vec!["self".to_string()],
                        body: vec![Stmt::Print {
                            span: Span::dummy(),
                            expr: Expr::Field {
                                span: Span::dummy(),
                                target: Box::new(Expr::SelfExpr {
                                    span: Span::dummy()
                                }),
                                name: "nome".to_string(),
                            },
                        }],
                    },
                ],
            }]
        );
    }

    #[test]
    fn class_extends_and_empty_class() {
        assert_eq!(
            parse_ok("class Studente extends Persona\nend").stmts,
            vec![Stmt::Class {
                span: Span::dummy(),
                name: "Studente".to_string(),
                parent: Some("Persona".to_string()),
                methods: vec![],
            }]
        );
        assert_eq!(
            parse_ok("class Vuota\nend").stmts,
            vec![Stmt::Class {
                span: Span::dummy(),
                name: "Vuota".to_string(),
                parent: None,
                methods: vec![],
            }]
        );
    }

    #[test]
    fn field_access_method_calls_and_super() {
        // p.nome
        assert_eq!(
            parse_ok("print p.nome").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Field {
                    span: Span::dummy(),
                    target: Box::new(ident("p")),
                    name: "nome".to_string(),
                },
            }]
        );
        // p.saluta("ciao")
        assert_eq!(
            parse_ok("print p.saluta(\"ciao\")").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::MethodCall {
                    span: Span::dummy(),
                    target: Box::new(ident("p")),
                    name: "saluta".to_string(),
                    args: vec![Expr::Str {
                        value: "ciao".to_string(),
                        span: Span::dummy()
                    }],
                },
            }]
        );
        // Chained: self.amico.nome, a.b.c()
        assert_eq!(
            parse_ok("print self.amico.nome").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: Expr::Field {
                    span: Span::dummy(),
                    target: Box::new(Expr::Field {
                        span: Span::dummy(),
                        target: Box::new(Expr::SelfExpr {
                            span: Span::dummy()
                        }),
                        name: "amico".to_string(),
                    }),
                    name: "nome".to_string(),
                },
            }]
        );
        assert_eq!(
            parse_ok("a.b.c()").stmts,
            vec![Stmt::ExprStmt {
                span: Span::dummy(),
                expr: Expr::MethodCall {
                    span: Span::dummy(),
                    target: Box::new(Expr::Field {
                        span: Span::dummy(),
                        target: Box::new(ident("a")),
                        name: "b".to_string(),
                    }),
                    name: "c".to_string(),
                    args: vec![],
                },
            }]
        );
        // super.init(nome, eta)
        assert_eq!(
            parse_ok("super.init(nome, eta)").stmts,
            vec![Stmt::ExprStmt {
                span: Span::dummy(),
                expr: Expr::SuperCall {
                    span: Span::dummy(),
                    name: "init".to_string(),
                    args: vec![ident("nome"), ident("eta")],
                },
            }]
        );
        // A field access binds as tightly as a call: p.nome + 1.
        assert_eq!(
            parse_ok("print p.nome + 1").stmts,
            vec![Stmt::Print {
                span: Span::dummy(),
                expr: bin(
                    Add,
                    Expr::Field {
                        span: Span::dummy(),
                        target: Box::new(ident("p")),
                        name: "nome".to_string(),
                    },
                    int(1),
                ),
            }]
        );
    }

    #[test]
    fn field_assignment() {
        assert_eq!(
            parse_ok("p.nome = \"Giuliano\"").stmts,
            vec![Stmt::FieldAssign {
                span: Span::dummy(),
                target: Expr::Field {
                    span: Span::dummy(),
                    target: Box::new(ident("p")),
                    name: "nome".to_string(),
                },
                value: Expr::Str {
                    value: "Giuliano".to_string(),
                    span: Span::dummy()
                },
            }]
        );
        assert_eq!(
            parse_ok("self.eta = eta + 1").stmts,
            vec![Stmt::FieldAssign {
                span: Span::dummy(),
                target: Expr::Field {
                    span: Span::dummy(),
                    target: Box::new(Expr::SelfExpr {
                        span: Span::dummy()
                    }),
                    name: "eta".to_string(),
                },
                value: bin(Add, ident("eta"), int(1)),
            }]
        );
        // A field access is not an assignment without `=`.
        assert_eq!(
            parse_ok("p.nome == \"x\"").stmts,
            vec![Stmt::ExprStmt {
                span: Span::dummy(),
                expr: bin(
                    Eq,
                    Expr::Field {
                        span: Span::dummy(),
                        target: Box::new(ident("p")),
                        name: "nome".to_string(),
                    },
                    Expr::Str {
                        value: "x".to_string(),
                        span: Span::dummy()
                    },
                ),
            }]
        );
        // A field of a call result is a valid target: the call is
        // evaluated, then the field is set on what it returns.
        assert_eq!(
            parse_ok("f().nome = 1").stmts,
            vec![Stmt::FieldAssign {
                span: Span::dummy(),
                target: Expr::Field {
                    span: Span::dummy(),
                    target: Box::new(Expr::Call {
                        span: Span::dummy(),
                        name: "f".to_string(),
                        args: vec![],
                    }),
                    name: "nome".to_string(),
                },
                value: int(1),
            }]
        );
        // But a bare call is not a valid assignment target.
        let err = parse_err("f() = 1");
        assert!(
            err.message.contains("invalid assignment target"),
            "{}",
            err.message
        );
    }

    #[test]
    fn class_body_errors() {
        // A method without `self` as first parameter.
        let err = parse_err("class C\n    fn m(x)\n        return x\n    end\nend\n");
        assert!(
            err.message.contains("methods must take 'self'"),
            "{}",
            err.message
        );
        // A method with no parameters at all.
        let err = parse_err("class C\n    fn m()\n    end\nend\n");
        assert!(
            err.message.contains("methods must take 'self'"),
            "{}",
            err.message
        );
        // Anything that is not a method definition.
        let err = parse_err("class C\n    let x = 1\nend\n");
        assert!(
            err.message.contains("only method definitions"),
            "{}",
            err.message
        );
        let err = parse_err("class C\n    print 1\nend\n");
        assert!(
            err.message.contains("only method definitions"),
            "{}",
            err.message
        );
        // `super` without a method call.
        let err = parse_err("print super.nome");
        assert!(
            err.message.contains("expected '(' after super method name"),
            "{}",
            err.message
        );
        let err = parse_err("print super");
        assert!(
            err.message.contains("expected '.' after 'super'"),
            "{}",
            err.message
        );
        // A class without a name, or without a parent name.
        let err = parse_err("class\nend");
        assert!(
            err.message.contains("expected class name"),
            "{}",
            err.message
        );
        let err = parse_err("class C extends\nend");
        assert!(
            err.message.contains("expected parent class name"),
            "{}",
            err.message
        );
    }

    #[test]
    fn class_display_round_trip() {
        let source = "class Studente extends Persona\n    fn init(self, nome)\n        super.init(nome)\n        self.nome = nome\n    end\n    fn saluta(self)\n        print self.nome\n    end\nend\nlet s = Studente(\"Giuliano\")\nprint s.saluta()\n";
        let prog = parse_ok(source);
        assert_eq!(
            prog.to_string(),
            "class Studente extends Persona\n    fn init(self, nome)\n        super.init(nome)\n        self.nome = nome\n    end\n    fn saluta(self)\n        print self.nome\n    end\nend\nlet s = Studente(\"Giuliano\")\nprint s.saluta()"
        );
        let reparsed = parse_ok(&prog.to_string());
        assert_eq!(reparsed, prog);
        // An empty class prints cleanly too.
        let prog = parse_ok("class Vuota\nend");
        assert_eq!(prog.to_string(), "class Vuota\nend");
        assert_eq!(parse_ok(&prog.to_string()), prog);
    }

    #[test]
    fn class_missing_end_is_incomplete() {
        assert!(parse_err("class C\n    fn m(self)\n    end\n").is_incomplete());
        assert!(parse_err("class C").is_incomplete());
    }

    #[test]
    fn import_with_and_without_alias() {
        assert_eq!(
            parse_ok("import math").stmts,
            vec![Stmt::Import {
                span: Span::dummy(),
                name: "math".to_string(),
                alias: None,
            }]
        );
        assert_eq!(
            parse_ok("import math as m").stmts,
            vec![Stmt::Import {
                span: Span::dummy(),
                name: "math".to_string(),
                alias: Some("m".to_string()),
            }]
        );
        // An import is a statement like any other: it can sit
        // inside a block, after other statements, separated by `;`.
        assert_eq!(
            parse_ok("let x = 1; import a\nimport b as c").stmts,
            vec![
                Stmt::Let {
                    span: Span::dummy(),
                    name: "x".to_string(),
                    value: int(1),
                },
                Stmt::Import {
                    span: Span::dummy(),
                    name: "a".to_string(),
                    alias: None,
                },
                Stmt::Import {
                    span: Span::dummy(),
                    name: "b".to_string(),
                    alias: Some("c".to_string()),
                },
            ]
        );
    }

    #[test]
    fn import_errors() {
        // No module name.
        let err = parse_err("import\n");
        assert!(
            err.message.contains("expected module name after 'import'"),
            "{}",
            err.message
        );
        // `as` without an alias.
        let err = parse_err("import math as\n");
        assert!(
            err.message.contains("expected alias after 'as'"),
            "{}",
            err.message
        );
        // A keyword cannot be a module name.
        let err = parse_err("import class");
        assert!(
            err.message.contains("expected module name after 'import'"),
            "{}",
            err.message
        );
        // `from` is not a thing in GG (yet): `import` takes a bare
        // module name, nothing else.
        let err = parse_err("import math from x");
        assert!(
            err.message
                .contains("expected newline or ';' after statement"),
            "{}",
            err.message
        );
    }

    #[test]
    fn import_display_round_trip() {
        let source = "import math\nimport greeter as g\nprint math.pi\n";
        let prog = parse_ok(source);
        assert_eq!(
            prog.to_string(),
            "import math\nimport greeter as g\nprint math.pi"
        );
        let reparsed = parse_ok(&prog.to_string());
        assert_eq!(reparsed, prog);
    }

    #[test]
    fn incomplete_errors_are_flagged_when_input_ends_early() {
        // A block missing its `end`.
        assert!(parse_err("if x then\n    print 1\n").is_incomplete());
        // A `then` missing at end of input.
        assert!(parse_err("if x").is_incomplete());
        // An unclosed parenthesis at end of input.
        assert!(parse_err("print (1 + 2").is_incomplete());
        // An expression missing at end of input.
        assert!(parse_err("let x =").is_incomplete());
        // But an error at a real token is not incomplete, even when
        // the same keyword is involved: `then` missing before a
        // newline means the line is wrong, not unfinished.
        assert!(!parse_err("if x\n    print 1\nend").is_incomplete());
        assert!(!parse_err("let = 5").is_incomplete());
        assert!(!parse_err("end").is_incomplete());
        assert!(!parse_err("let x = 1 let y = 2").is_incomplete());
    }

    // ----- Phase 16: spans -----

    #[test]
    fn spans_point_at_the_opening_token() {
        let prog = parse_ok("let x = 10\nif x > 5 then\n    print \"big\"\nend\n");
        let [let_stmt, if_stmt] = prog.stmts.as_slice() else {
            panic!("expected two statements");
        };
        let Stmt::Let { span, value, .. } = let_stmt else {
            panic!("expected a let");
        };
        assert_eq!(*span, Span::new(1, 1));
        assert_eq!(value.span(), Span::new(1, 9));
        let Stmt::If {
            span,
            cond,
            then_block,
            ..
        } = if_stmt
        else {
            panic!("expected an if");
        };
        assert_eq!(*span, Span::new(2, 1));
        // The `>` operator, not the `if`.
        assert_eq!(cond.span(), Span::new(2, 6));
        let [Stmt::Print { span, expr, .. }] = then_block.as_slice() else {
            panic!("expected one print");
        };
        assert_eq!(*span, Span::new(3, 5));
        assert_eq!(expr.span(), Span::new(3, 11));
    }

    #[test]
    fn binary_and_unary_spans_are_the_operator() {
        let prog = parse_ok("print 1 + 2\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 9));
        let prog = parse_ok("print -x\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 7));
        // A nested binary: the outer `*` is at col 15, the inner `+` at 10.
        let prog = parse_ok("print (1 + 2) * 3\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 15));
        let Expr::Binary { left, .. } = expr else {
            panic!("expected a binary");
        };
        assert_eq!(left.span(), Span::new(1, 10));
    }

    #[test]
    fn call_index_field_and_method_spans() {
        let prog = parse_ok("print f(1)\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 7));
        let prog = parse_ok("print xs[0]\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 9));
        let prog = parse_ok("print p.nome\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 9));
        let prog = parse_ok("print p.saluta()\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 9));
        // A slice's span is its first `:`.
        let prog = parse_ok("print a[1:3]\n");
        let [Stmt::Print { expr, .. }] = prog.stmts.as_slice() else {
            panic!("expected a print");
        };
        assert_eq!(expr.span(), Span::new(1, 8));
        let Expr::Index { indices, .. } = expr else {
            panic!("expected an index");
        };
        assert_eq!(indices[0].span(), Span::new(1, 10));
    }

    #[test]
    fn spans_follow_the_source_lines() {
        let prog = parse_ok("fn f(a)\n    return a + 1\nend\n");
        let [Stmt::Fn { span, body, .. }] = prog.stmts.as_slice() else {
            panic!("expected a fn");
        };
        assert_eq!(*span, Span::new(1, 1));
        let [Stmt::Return { span, expr, .. }] = body.as_slice() else {
            panic!("expected a return");
        };
        assert_eq!(*span, Span::new(2, 5));
        let Some(expr) = expr else {
            panic!("expected a return value");
        };
        assert_eq!(expr.span(), Span::new(2, 14));
    }
}
