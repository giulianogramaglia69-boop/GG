// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime values for GG.
//!
//! A `Function` captures the environment where it was defined
//! (a closure): calling it later still sees those variables.

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
}

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
        match self {
            Value::Int(n) => write!(f, "{n}"),
            // Plain Rust float formatting: 3.5 prints as "3.5", 1.0 as "1".
            Value::Float(x) => write!(f, "{x}"),
            // Strings print without quotes.
            Value::Str(s) => write!(f, "{s}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Nil => write!(f, "nil"),
            Value::Function(_) | Value::Builtin(_) => write!(f, "<fn>"),
        }
    }
}

/// Equality between two values. Numbers compare across Int/Float;
/// `==` between different types is false, never an error.
pub fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => (*x as f64) == *y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Nil, Value::Nil) => true,
        (Value::Function(x), Value::Function(y)) => Rc::ptr_eq(x, y),
        (Value::Builtin(x), Value::Builtin(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}
