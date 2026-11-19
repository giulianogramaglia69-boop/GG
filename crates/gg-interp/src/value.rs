// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime values for GG.
//!
//! A `Function` captures the environment where it was defined
//! (a closure): calling it later still sees those variables.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use gg_parser::ast::Stmt;

use crate::builtin::Builtin;
use crate::env::Environment;

/// A value at runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Nil,
    Function(Rc<Function>),
    Builtin(Rc<Builtin>),
    /// A list: shared and mutable, so `let b = a` makes `b` an alias
    /// of `a` (Python style) — a mutation through one name is visible
    /// through the other.
    List(Rc<RefCell<Vec<Value>>>),
}

/// Max list nesting depth when printing; deeper levels print as
/// `...`, so a self-referential list cannot blow the stack.
const MAX_LIST_DEPTH: usize = 16;

/// Max depth when comparing two lists element by element; distinct
/// cyclic lists are reported as unequal instead of recursing forever.
const MAX_LIST_COMPARE_DEPTH: usize = 64;

/// A user-defined function: parameters, body, and the environment
/// captured at definition time.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub env: Rc<Environment>,
}

impl Value {
    /// Short type name used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Str(_) => "Str",
            Value::Bool(_) => "Bool",
            Value::Nil => "Nil",
            Value::Function(_) | Value::Builtin(_) => "Function",
            Value::List(_) => "List",
        }
    }

    /// GG truthiness: only `false` and `nil` are false, everything
    /// else is true (Ruby/Lua style).
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Bool(false) | Value::Nil)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_depth(f, 0)
    }
}

impl Value {
    /// Print with a nesting depth: list elements are one level
    /// deeper than the list itself.
    fn fmt_depth(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            // Plain Rust float formatting: 3.5 prints as "3.5", 1.0 as "1".
            Value::Float(x) => write!(f, "{x}"),
            // Strings print without quotes.
            Value::Str(s) => write!(f, "{s}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Nil => write!(f, "nil"),
            Value::Function(_) | Value::Builtin(_) => write!(f, "<fn>"),
            Value::List(list) => {
                if depth >= MAX_LIST_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "[")?;
                for (i, element) in list.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    // Inside a list, strings are quoted (like the
                    // elements of a Python repr); anything else
                    // prints the same as at the top level.
                    match element {
                        Value::Str(s) => write!(f, "\"{}\"", escape_str(s))?,
                        other => other.fmt_depth(f, depth + 1)?,
                    }
                }
                write!(f, "]")
            }
        }
    }
}

/// Escape a string for printing inside double quotes (only used
/// for strings nested in a list).
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

/// Equality between two values. Numbers compare across Int/Float;
/// `==` between different types is false, never an error.
pub fn values_equal(a: &Value, b: &Value) -> bool {
    values_equal_depth(a, b, 0)
}

fn values_equal_depth(a: &Value, b: &Value, depth: usize) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => (*x as f64) == *y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Nil, Value::Nil) => true,
        (Value::Function(x), Value::Function(y)) => Rc::ptr_eq(x, y),
        (Value::Builtin(x), Value::Builtin(y)) => Rc::ptr_eq(x, y),
        (Value::List(x), Value::List(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            // Distinct cyclic lists would recurse forever: give up
            // and call them unequal.
            if depth >= MAX_LIST_COMPARE_DEPTH {
                return false;
            }
            let x = x.borrow();
            let y = y.borrow();
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|(a, b)| values_equal_depth(a, b, depth + 1))
        }
        _ => false,
    }
}
