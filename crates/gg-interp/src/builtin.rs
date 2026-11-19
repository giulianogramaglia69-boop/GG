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

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::env::Environment;
use crate::error::RuntimeError;
use crate::value::Value;

/// A builtin function implementation: evaluated arguments, an input
/// reader and an output writer.
pub type BuiltinFn = fn(&[Value], &mut dyn BufRead, &mut dyn Write) -> Result<Value, RuntimeError>;

/// How many arguments a builtin takes: an exact count, or a range
/// (e.g. `range` takes 1 or 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Exact(usize),
    Range(usize, usize),
}

impl Arity {
    pub fn accepts(&self, got: usize) -> bool {
        match self {
            Arity::Exact(n) => got == *n,
            Arity::Range(min, max) => got >= *min && got <= *max,
        }
    }

    /// The count as it appears in arity errors: `1`, or `1 or 2`.
    pub fn describe(&self) -> String {
        match self {
            Arity::Exact(n) => format!("{n}"),
            Arity::Range(min, max) => format!("{min} or {max}"),
        }
    }
}

/// One builtin: its name, its arity (checked by the interpreter
/// before the call), and the Rust function behind it.
#[derive(Debug, Clone)]
pub struct Builtin {
    pub name: &'static str,
    pub arity: Arity,
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
            arity: Arity::Exact(1),
            func: abs,
        },
        Builtin {
            name: "append_file",
            arity: Arity::Exact(2),
            func: append_file,
        },
        Builtin {
            name: "file_exists",
            arity: Arity::Exact(1),
            func: file_exists,
        },
        Builtin {
            name: "float",
            arity: Arity::Exact(1),
            func: float,
        },
        Builtin {
            name: "input",
            arity: Arity::Exact(1),
            func: input,
        },
        Builtin {
            name: "int",
            arity: Arity::Exact(1),
            func: int,
        },
        Builtin {
            name: "len",
            arity: Arity::Exact(1),
            func: len,
        },
        Builtin {
            name: "lines",
            arity: Arity::Exact(1),
            func: lines,
        },
        Builtin {
            name: "max",
            arity: Arity::Exact(2),
            func: max,
        },
        Builtin {
            name: "min",
            arity: Arity::Exact(2),
            func: min,
        },
        Builtin {
            name: "pop",
            arity: Arity::Exact(1),
            func: pop,
        },
        Builtin {
            name: "push",
            arity: Arity::Exact(2),
            func: push,
        },
        Builtin {
            name: "range",
            arity: Arity::Range(1, 2),
            func: range,
        },
        Builtin {
            name: "read_file",
            arity: Arity::Exact(1),
            func: read_file,
        },
        Builtin {
            name: "split",
            arity: Arity::Exact(2),
            func: split,
        },
        Builtin {
            name: "str",
            arity: Arity::Exact(1),
            func: str_of,
        },
        Builtin {
            name: "type",
            arity: Arity::Exact(1),
            func: type_of,
        },
        Builtin {
            name: "write_file",
            arity: Arity::Exact(2),
            func: write_file,
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
/// `"è"` has length 1), or number of elements in a list.
fn len(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
        Value::List(list) => Ok(Value::Int(list.borrow().len() as i64)),
        _ => Err(RuntimeError::new("len() expects a Str or a List")),
    }
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
        Value::List(_) => "list",
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

/// The argument as a list, or a clear error naming the builtin.
fn as_list<'a>(name: &str, value: &'a Value) -> Result<&'a Rc<RefCell<Vec<Value>>>, RuntimeError> {
    match value {
        Value::List(list) => Ok(list),
        _ => Err(RuntimeError::new(format!("{name}() expects a List"))),
    }
}

/// The argument as a string, or a clear error naming the builtin.
fn as_str<'a>(name: &str, value: &'a Value) -> Result<&'a str, RuntimeError> {
    match value {
        Value::Str(s) => Ok(s),
        _ => Err(RuntimeError::new(format!("{name}() expects a Str"))),
    }
}

/// The argument as an integer, or a clear error naming the builtin.
fn as_int(name: &str, value: &Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(n) => Ok(*n),
        _ => Err(RuntimeError::new(format!("{name}() expects an Int"))),
    }
}

/// `push(list, value)` — append `value` to the list (the list is
/// mutated in place) and return `nil`.
fn push(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let list = as_list("push", &args[0])?;
    list.borrow_mut().push(args[1].clone());
    Ok(Value::Nil)
}

/// `pop(list)` — remove and return the last element; an error on
/// an empty list.
fn pop(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let list = as_list("pop", &args[0])?;
    list.borrow_mut()
        .pop()
        .ok_or_else(|| RuntimeError::new("pop() on empty list"))
}

/// Max number of elements `range` may produce, so a huge range
/// cannot exhaust memory by accident.
const MAX_RANGE_LEN: i128 = 1_000_000;

/// `range(n)` is `[0, 1, ..., n-1]`; `range(a, b)` is
/// `[a, ..., b-1]`. Either form is empty when the start is not
/// below the end.
fn range(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let (start, end) = if args.len() == 1 {
        (0, as_int("range", &args[0])?)
    } else {
        (as_int("range", &args[0])?, as_int("range", &args[1])?)
    };
    let len = end as i128 - start as i128;
    if len > MAX_RANGE_LEN {
        return Err(RuntimeError::new(
            "range() is too large (max 1000000 elements)",
        ));
    }
    let mut values = Vec::new();
    let mut i = start;
    while i < end {
        values.push(Value::Int(i));
        i += 1;
    }
    Ok(Value::List(Rc::new(RefCell::new(values))))
}

/// `split(s, sep)` — split the string on the separator: every
/// occurrence of `sep` cuts the string. An empty separator is an
/// error.
fn split(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("split", &args[0])?;
    let sep = as_str("split", &args[1])?;
    if sep.is_empty() {
        return Err(RuntimeError::new("split() separator must not be empty"));
    }
    let parts: Vec<Value> = s
        .split(sep)
        .map(|part| Value::Str(part.to_string()))
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(parts))))
}

/// `lines(s)` — the lines of the string, split on `\n` (a trailing
/// `\r` is stripped, and a trailing newline adds no empty line).
fn lines(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("lines", &args[0])?;
    let parts: Vec<Value> = s.lines().map(|line| Value::Str(line.to_string())).collect();
    Ok(Value::List(Rc::new(RefCell::new(parts))))
}

// File handling: whole-file helpers only — GG has no open file
// handles. Paths are strings, relative to the current directory.
// Errors carry the operating system's message.

/// `read_file(path)` — the whole file as a string.
fn read_file(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("read_file", &args[0])?;
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Value::Str(content)),
        Err(e) => Err(RuntimeError::new(format!("read_file(\"{path}\"): {e}"))),
    }
}

/// `write_file(path, text)` — create or overwrite the file.
fn write_file(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("write_file", &args[0])?;
    let text = as_str("write_file", &args[1])?;
    match std::fs::write(path, text) {
        Ok(()) => Ok(Value::Nil),
        Err(e) => Err(RuntimeError::new(format!("write_file(\"{path}\"): {e}"))),
    }
}

/// `append_file(path, text)` — append to the file, creating it if
/// it does not exist.
fn append_file(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("append_file", &args[0])?;
    let text = as_str("append_file", &args[1])?;
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()));
    match result {
        Ok(()) => Ok(Value::Nil),
        Err(e) => Err(RuntimeError::new(format!("append_file(\"{path}\"): {e}"))),
    }
}

/// `file_exists(path)` — true when the path exists.
fn file_exists(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("file_exists", &args[0])?;
    Ok(Value::Bool(std::path::Path::new(path).exists()))
}
