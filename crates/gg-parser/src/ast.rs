// SPDX-License-Identifier: GPL-3.0-or-later
//! The GG abstract syntax tree, plus canonical pretty-printing.
//!
//! `Display` reprints a program in canonical form (4-space indent,
//! `end` closing every block) so that parse -> print -> re-parse
//! yields the same AST. Parentheses are added only where the
//! precedence rules need them.

use std::fmt;

/// A whole GG program: a list of top-level statements.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_stmts(f, &self.stmts, 0)
    }
}

/// One statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        value: Expr,
    },
    Assign {
        name: String,
        value: Expr,
    },
    /// Assignment through an index: `xs[0] = 99`, `m[0][1] = 2`.
    IndexAssign {
        target: Expr,
        value: Expr,
    },
    /// Assignment through a field: `p.nome = "Giuliano"`,
    /// `self.eta = eta`. Sets a field on the instance — never a
    /// method (a field with the same name as a method shadows it).
    FieldAssign {
        target: Expr,
        value: Expr,
    },
    Print {
        expr: Expr,
    },
    If {
        cond: Expr,
        then_block: Vec<Stmt>,
        else_block: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    For {
        var: String,
        from: Expr,
        to: Expr,
        body: Vec<Stmt>,
    },
    Fn {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    /// A class definition: `class Persona ... end`, optionally
    /// `class Studente extends Persona ... end`. The body holds
    /// only method definitions.
    Class {
        name: String,
        parent: Option<String>,
        methods: Vec<MethodDef>,
    },
    /// A module import: `import math` loads `math.gg` and binds
    /// the name `math`; `import math as m` binds it as `m`
    /// instead. There is no `from x import y` (yet).
    Import {
        name: String,
        alias: Option<String>,
    },
    Return {
        expr: Option<Expr>,
    },
    ExprStmt {
        expr: Expr,
    },
}

/// One method definition inside a `class` body. The first
/// parameter must be named `self` (enforced by the parser).
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

/// One expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Ident(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// A list literal: `[1, 2, 3]`, `[]`, nested and mixed.
    List(Vec<Expr>),
    /// A tuple literal: `(1, "a")`, `()`, `(1,)` (the trailing
    /// comma is what makes a one-element tuple — `(1)` is just
    /// grouping). Tuples are immutable.
    Tuple(Vec<Expr>),
    /// A dict literal: `{"nome": "Giuliano"}`, `{}`. Keys are
    /// validated at runtime (only Int/Float/Str/Bool).
    Dict(Vec<(Expr, Expr)>),
    /// A set literal: `{1, 2, 3}`. `{}` is the empty dict, not an
    /// empty set (Python convention).
    Set(Vec<Expr>),
    /// Indexing: `xs[0]`, `f()[0]`, `xs[0][1]`. Binds as tightly as
    /// a call (it is a postfix operator, see the parser).
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    /// The `self` keyword: the instance a method was called on.
    SelfExpr,
    /// Field access: `p.nome`, `self.eta`. On an instance it reads
    /// a field first, then a method (bound); on a class it reads an
    /// unbound method.
    Field {
        target: Box<Expr>,
        name: String,
    },
    /// A method call: `p.saluta("ciao")`, `self.nome()`. The
    /// instance is passed as `self` implicitly.
    MethodCall {
        target: Box<Expr>,
        name: String,
        args: Vec<Expr>,
    },
    /// A call into the parent class: `super.init(nome, eta)`,
    /// `super.saluta()`. Only valid inside a method.
    SuperCall {
        name: String,
        args: Vec<Expr>,
    },
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

impl BinaryOp {
    /// Binding strength: higher binds tighter.
    fn prec(&self) -> u8 {
        match self {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::Eq | BinaryOp::NotEq => 3,
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => 4,
            BinaryOp::Add | BinaryOp::Sub => 5,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 6,
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "==",
            BinaryOp::NotEq => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::LtEq => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::GtEq => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }
}

impl Expr {
    /// Binding strength of the expression itself (primaries bind tightest).
    fn prec(&self) -> u8 {
        match self {
            Expr::Binary { op, .. } => op.prec(),
            Expr::Unary { .. } => 7,
            _ => 8,
        }
    }

    /// Print the expression, adding parentheses when its precedence is
    /// lower than the surrounding context requires.
    fn fmt_prec(&self, f: &mut fmt::Formatter<'_>, parent_prec: u8) -> fmt::Result {
        let need_parens = self.prec() < parent_prec;
        if need_parens {
            write!(f, "(")?;
        }
        match self {
            Expr::Int(n) => write!(f, "{n}")?,
            Expr::Float(x) => write!(f, "{}", format_float(*x))?,
            Expr::Str(s) => write!(f, "\"{}\"", escape_str(s))?,
            Expr::Bool(b) => write!(f, "{b}")?,
            Expr::Ident(name) => write!(f, "{name}")?,
            Expr::Unary { op, expr } => {
                match op {
                    UnaryOp::Neg => write!(f, "-")?,
                    UnaryOp::Not => write!(f, "not ")?,
                }
                expr.fmt_prec(f, 7)?;
            }
            Expr::Binary { op, left, right } => {
                // Left-associative: the left operand may share the same
                // precedence, the right one must bind tighter.
                left.fmt_prec(f, op.prec())?;
                write!(f, " {} ", op.symbol())?;
                right.fmt_prec(f, op.prec() + 1)?;
            }
            Expr::Call { name, args } => {
                write!(f, "{name}(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    arg.fmt_prec(f, 0)?;
                }
                write!(f, ")")?;
            }
            Expr::List(elements) => {
                write!(f, "[")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_prec(f, 0)?;
                }
                write!(f, "]")?;
            }
            Expr::Tuple(elements) => {
                write!(f, "(")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_prec(f, 0)?;
                }
                // A one-element tuple keeps its trailing comma, so it
                // re-parses as a tuple and not as grouping.
                if elements.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")?;
            }
            Expr::Dict(entries) => {
                write!(f, "{{")?;
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    key.fmt_prec(f, 0)?;
                    write!(f, ": ")?;
                    value.fmt_prec(f, 0)?;
                }
                write!(f, "}}")?;
            }
            Expr::Set(elements) => {
                write!(f, "{{")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_prec(f, 0)?;
                }
                write!(f, "}}")?;
            }
            Expr::Index { target, index } => {
                target.fmt_prec(f, 8)?;
                write!(f, "[")?;
                index.fmt_prec(f, 0)?;
                write!(f, "]")?;
            }
            Expr::SelfExpr => write!(f, "self")?,
            Expr::Field { target, name } => {
                target.fmt_prec(f, 8)?;
                write!(f, ".{name}")?;
            }
            Expr::MethodCall { target, name, args } => {
                target.fmt_prec(f, 8)?;
                write!(f, ".{name}(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    arg.fmt_prec(f, 0)?;
                }
                write!(f, ")")?;
            }
            Expr::SuperCall { name, args } => {
                write!(f, "super.{name}(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    arg.fmt_prec(f, 0)?;
                }
                write!(f, ")")?;
            }
        }
        if need_parens {
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_prec(f, 0)
    }
}

/// Print a float so it always re-parses as a float: `1.0`, not `1`.
fn format_float(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

/// Escape a string for re-printing inside double quotes.
fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Write a list of statements, one per line, at the given indent level.
fn write_stmts(f: &mut fmt::Formatter<'_>, stmts: &[Stmt], indent: usize) -> fmt::Result {
    for (i, stmt) in stmts.iter().enumerate() {
        if i > 0 {
            writeln!(f)?;
        }
        stmt.fmt_indented(f, indent)?;
    }
    Ok(())
}

/// Write a block body: a newline, the statements indented one level
/// deeper, and a trailing newline — or just a newline when empty.
fn write_block(f: &mut fmt::Formatter<'_>, stmts: &[Stmt], indent: usize) -> fmt::Result {
    writeln!(f)?;
    if !stmts.is_empty() {
        write_stmts(f, stmts, indent)?;
        writeln!(f)?;
    }
    Ok(())
}

impl Stmt {
    fn fmt_indented(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        let pad = "    ".repeat(indent);
        match self {
            Stmt::Let { name, value } => write!(f, "{pad}let {name} = {value}"),
            Stmt::Assign { name, value } => write!(f, "{pad}{name} = {value}"),
            Stmt::IndexAssign { target, value } => write!(f, "{pad}{target} = {value}"),
            Stmt::FieldAssign { target, value } => write!(f, "{pad}{target} = {value}"),
            Stmt::Print { expr } => write!(f, "{pad}print {expr}"),
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                write!(f, "{pad}if {cond} then")?;
                write_block(f, then_block, indent + 1)?;
                match else_block {
                    Some(else_block) => {
                        write!(f, "{pad}else")?;
                        write_block(f, else_block, indent + 1)?;
                        write!(f, "{pad}end")
                    }
                    None => write!(f, "{pad}end"),
                }
            }
            Stmt::While { cond, body } => {
                write!(f, "{pad}while {cond} then")?;
                write_block(f, body, indent + 1)?;
                write!(f, "{pad}end")
            }
            Stmt::For {
                var,
                from,
                to,
                body,
            } => {
                write!(f, "{pad}for {var} = {from} to {to} then")?;
                write_block(f, body, indent + 1)?;
                write!(f, "{pad}end")
            }
            Stmt::Fn { name, params, body } => {
                write!(f, "{pad}fn {name}({})", params.join(", "))?;
                write_block(f, body, indent + 1)?;
                write!(f, "{pad}end")
            }
            Stmt::Class {
                name,
                parent,
                methods,
            } => {
                write!(f, "{pad}class {name}")?;
                if let Some(parent) = parent {
                    write!(f, " extends {parent}")?;
                }
                writeln!(f)?;
                for (i, method) in methods.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(
                        f,
                        "{pad}    fn {}({})",
                        method.name,
                        method.params.join(", ")
                    )?;
                    write_block(f, &method.body, indent + 2)?;
                    write!(f, "{pad}    end")?;
                }
                if !methods.is_empty() {
                    writeln!(f)?;
                }
                write!(f, "{pad}end")
            }
            Stmt::Import { name, alias } => {
                write!(f, "{pad}import {name}")?;
                if let Some(alias) = alias {
                    write!(f, " as {alias}")?;
                }
                Ok(())
            }
            Stmt::Return { expr } => match expr {
                Some(expr) => write!(f, "{pad}return {expr}"),
                None => write!(f, "{pad}return"),
            },
            Stmt::ExprStmt { expr } => write!(f, "{pad}{expr}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn float_prints_with_a_decimal_point() {
        assert_eq!(Expr::Float(1.0).to_string(), "1.0");
        assert_eq!(Expr::Float(3.15).to_string(), "3.15");
        assert_eq!(Expr::Float(-2.0).to_string(), "-2.0");
    }

    #[test]
    fn strings_are_escaped_when_printed() {
        assert_eq!(
            Expr::Str("a\"b\nc\\d".to_string()).to_string(),
            "\"a\\\"b\\nc\\\\d\""
        );
    }

    #[test]
    fn binary_print_adds_parens_only_where_needed() {
        // (1 + 2) * 3 needs parens on the left.
        let e = bin(BinaryOp::Mul, bin(BinaryOp::Add, int(1), int(2)), int(3));
        assert_eq!(e.to_string(), "(1 + 2) * 3");
        // 1 + (2 * 3) needs none.
        let e = bin(BinaryOp::Add, int(1), bin(BinaryOp::Mul, int(2), int(3)));
        assert_eq!(e.to_string(), "1 + 2 * 3");
        // a - (b - c) needs parens on the right.
        let e = bin(
            BinaryOp::Sub,
            ident("a"),
            bin(BinaryOp::Sub, ident("b"), ident("c")),
        );
        assert_eq!(e.to_string(), "a - (b - c)");
        // (a or b) and c needs parens on the left.
        let e = bin(
            BinaryOp::And,
            bin(BinaryOp::Or, ident("a"), ident("b")),
            ident("c"),
        );
        assert_eq!(e.to_string(), "(a or b) and c");
    }

    #[test]
    fn unary_print_parenthesizes_low_precedence_operands() {
        let e = Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(bin(BinaryOp::Add, ident("a"), ident("b"))),
        };
        assert_eq!(e.to_string(), "-(a + b)");
        let e = Expr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(bin(BinaryOp::And, ident("a"), ident("b"))),
        };
        assert_eq!(e.to_string(), "not (a and b)");
        let e = Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(ident("x")),
        };
        assert_eq!(e.to_string(), "-x");
    }

    #[test]
    fn if_without_else_and_empty_blocks_print_cleanly() {
        let prog = Program {
            stmts: vec![
                Stmt::If {
                    cond: ident("x"),
                    then_block: vec![],
                    else_block: None,
                },
                Stmt::While {
                    cond: Expr::Bool(true),
                    body: vec![],
                },
            ],
        };
        assert_eq!(prog.to_string(), "if x then\nend\nwhile true then\nend");
    }

    #[test]
    fn empty_program_prints_empty() {
        assert_eq!(Program { stmts: vec![] }.to_string(), "");
    }
}
