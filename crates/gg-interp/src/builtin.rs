// SPDX-License-Identifier: GPL-3.0-or-later
//! Builtin functions for GG.
//!
//! Builtins live in the global environment like ordinary values:
//! they can be assigned to variables, passed as arguments and
//! returned from functions. A `let` with the same name shadows the
//! builtin in the current scope, like any other variable.
//!
//! Builtins that do I/O (`input`) receive the input reader and the
//! output writer as arguments, so programs stay testable: the
//! `gg` binary passes the real stdin/stdout, the tests pass a
//! `Cursor` and a `Vec<u8>`.

use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::env::Environment;
use crate::error::RuntimeError;
use crate::value::Value;

/// A builtin function implementation: evaluated arguments, an input
/// reader and an output writer.
pub type BuiltinFn = fn(&[Value], &mut dyn BufRead, &mut dyn Write) -> Result<Value, RuntimeError>;

/// One builtin: its name, its arity (checked by the interpreter
/// before the call), and the Rust function behind it.
#[derive(Debug, Clone)]
pub struct Builtin {
    pub name: &'static str,
    pub arity: usize,
    pub func: BuiltinFn,
}

// Comparing function pointers is meaningless (their addresses are
// not guaranteed unique), so two builtins are equal when they have
// the same name and arity — builtin names are unique.
impl PartialEq for Builtin {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.arity == other.arity
    }
}

/// All builtins, in alphabetical order.
pub fn all() -> Vec<Builtin> {
    vec![
        Builtin {
            name: "abs",
            arity: 1,
            func: abs,
        },
        Builtin {
            name: "float",
            arity: 1,
            func: float,
        },
        Builtin {
            name: "input",
            arity: 1,
            func: input,
        },
        Builtin {
            name: "int",
            arity: 1,
            func: int,
        },
        Builtin {
            name: "len",
            arity: 1,
            func: len,
        },
        Builtin {
            name: "max",
            arity: 2,
            func: max,
        },
        Builtin {
            name: "min",
            arity: 2,
            func: min,
        },
        Builtin {
            name: "str",
            arity: 1,
            func: str_of,
        },
        Builtin {
            name: "type",
            arity: 1,
            func: type_of,
        },
    ]
}

/// Define every builtin in `env` (the global environment).
pub fn define_all(env: &Environment) {
    for builtin in all() {
        env.define(builtin.name, Value::Builtin(Rc::new(builtin)));
    }
}

/// `len(x)` — number of characters in a string (not bytes:
/// `"è"` has length 1).
fn len(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::Str(s) = &args[0] else {
        return Err(RuntimeError::new("len() expects a Str"));
    };
    Ok(Value::Int(s.chars().count() as i64))
}

/// `type(x)` — the type name as a string.
fn type_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let name = match &args[0] {
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::Bool(_) => "bool",
        Value::Nil => "nil",
        Value::Function(_) | Value::Builtin(_) => "fn",
    };
    Ok(Value::Str(name.to_string()))
}

/// `str(x)` — convert to a string, same representation as `print`.
fn str_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    Ok(Value::Str(args[0].to_string()))
}

/// `int(x)` — truncate a float toward zero, parse a string,
/// `true` is 1 and `false` is 0, an int is itself.
fn int(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(x) => {
            if !x.is_finite() {
                return Err(RuntimeError::new(
                    "int() cannot convert non-finite Float to Int",
                ));
            }
            Ok(Value::Int(x.trunc() as i64))
        }
        Value::Str(s) => s
            .trim()
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| RuntimeError::new(format!("int() cannot parse \"{s}\" as an integer"))),
        Value::Bool(b) => Ok(Value::Int(i64::from(*b))),
        other => Err(RuntimeError::new(format!(
            "int() cannot convert {} to Int",
            other.type_name()
        ))),
    }
}

/// `float(x)` — convert an int, parse a string, `true` is 1.0
/// and `false` is 0.0, a float is itself.
fn float(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::Float(x) => Ok(Value::Float(*x)),
        Value::Int(n) => Ok(Value::Float(*n as f64)),
        Value::Str(s) => s
            .trim()
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| RuntimeError::new(format!("float() cannot parse \"{s}\" as a float"))),
        Value::Bool(b) => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
        other => Err(RuntimeError::new(format!(
            "float() cannot convert {} to Float",
            other.type_name()
        ))),
    }
}

/// `input(prompt)` — print the prompt (no newline), read one line
/// from the input and return it as a string without the newline.
/// At end of input it returns `""`.
fn input(
    args: &[Value],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::Str(prompt) = &args[0] else {
        return Err(RuntimeError::new("input() expects a Str"));
    };
    write!(out, "{prompt}").map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
    out.flush()
        .map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| RuntimeError::new(format!("input error: {e}")))?;
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(Value::Str(line))
}

/// `abs(x)` — absolute value of a number.
fn abs(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::Int(n) => n
            .checked_abs()
            .map(Value::Int)
            .ok_or_else(|| RuntimeError::new("integer overflow")),
        Value::Float(x) => Ok(Value::Float(x.abs())),
        _ => Err(RuntimeError::new("abs() expects a number")),
    }
}

/// `min(a, b)` / `max(a, b)` — the smaller/larger of two numbers.
/// Mixed int/float gives a float result.
fn min_max(name: &str, args: &[Value], want_min: bool) -> Result<Value, RuntimeError> {
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(if want_min {
            (*a).min(*b)
        } else {
            (*a).max(*b)
        })),
        (a, b) => {
            let (Some(x), Some(y)) = (as_f64(a), as_f64(b)) else {
                return Err(RuntimeError::new(format!("{name}() expects numbers")));
            };
            let ord = x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
            let result = if want_min {
                if ord == std::cmp::Ordering::Greater {
                    y
                } else {
                    x
                }
            } else if ord == std::cmp::Ordering::Less {
                y
            } else {
                x
            };
            Ok(Value::Float(result))
        }
    }
}

fn min(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    min_max("min", args, true)
}

fn max(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    min_max("max", args, false)
}

/// A value as an f64, if it is a number at all.
fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(n) => Some(*n as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}
