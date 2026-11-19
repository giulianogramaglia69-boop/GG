// SPDX-License-Identifier: GPL-3.0-or-later
//! Parser for the GG programming language: tokens -> AST.
//!
//! Phase 2: a recursive-descent parser over `gg-lexer` tokens.
//! Grammar (see README for the full version):
//!
//! ```text
//! program  := statement* EOF
//! block    := statement* ("end" | "else")
//! statement:= "let" ident "=" expr
//!           | ident "=" expr
//!           | "print" expr
//!           | "if" expr "then" block ["else" block] "end"
//!           | "while" expr "then" block "end"
//!           | "for" ident "=" expr "to" expr "then" block "end"
//!           | "fn" ident "(" [ident ("," ident)*] ")" block "end"
//!           | "return" [expr]
//!           | expr
//! expr     := or
//! or       := and ("or" and)*
//! and      := equality ("and" equality)*
//! equality := comparison (("==" | "!=") comparison)*
//! comparison:= term (("<" | "<=" | ">" | ">=") term)*
//! term     := factor (("+" | "-") factor)*
//! factor   := unary (("*" | "/" | "%") unary)*
//! unary    := ("-" | "not") unary | primary
//! primary  := number | string | "true" | "false" | ident ["(" [expr ("," expr)*] ")"]
//!           | "(" expr ")"
//! ```

pub mod ast;
mod parser;

pub use ast::{BinaryOp, Expr, Program, Stmt, UnaryOp};
pub use parser::{ParserError, parse};
