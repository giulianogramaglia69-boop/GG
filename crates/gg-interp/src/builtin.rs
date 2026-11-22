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
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::array::{self, check_shape_size, fmt_shape, make_array, parse_shape};
use crate::env::Environment;
use crate::error::RuntimeError;
use crate::interp::Interpreter;
use crate::value::{ArrayData, ElemType, Value, elem_type_of, values_equal};

/// A builtin function implementation: evaluated arguments, an input
/// reader and an output writer.
pub type BuiltinFn = fn(&[Value], &mut dyn BufRead, &mut dyn Write) -> Result<Value, RuntimeError>;

/// A builtin function implementation that also gets the interpreter,
/// so it can call back into GG code (a frame function, a layout
/// callback, or a generator replay). The `gui` module and `next`
/// use this: every other builtin is `Plain`.
pub type InterpFn =
    fn(&mut Interpreter, &[Value], &mut dyn BufRead, &mut dyn Write) -> Result<Value, RuntimeError>;

/// What is behind a builtin: a plain Rust function, or one that
/// needs the interpreter to call GG code back.
#[derive(Debug, Clone, Copy)]
pub enum BuiltinFunc {
    Plain(BuiltinFn),
    WithInterp(InterpFn),
}

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
    pub func: BuiltinFunc,
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
            func: BuiltinFunc::Plain(abs),
        },
        Builtin {
            name: "append_file",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(append_file),
        },
        Builtin {
            name: "arange",
            arity: Arity::Range(1, 3),
            func: BuiltinFunc::Plain(arange),
        },
        Builtin {
            name: "array",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(array),
        },
        Builtin {
            name: "contains",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(contains),
        },
        Builtin {
            name: "dequeue",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(dequeue),
        },
        Builtin {
            name: "dot",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(dot),
        },
        Builtin {
            name: "enqueue",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(enqueue),
        },
        Builtin {
            name: "ends_with",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(ends_with),
        },
        Builtin {
            name: "file_exists",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(file_exists),
        },
        Builtin {
            name: "float",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(float),
        },
        Builtin {
            name: "input",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(input),
        },
        Builtin {
            name: "int",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(int),
        },
        Builtin {
            name: "is_a",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(is_a),
        },
        Builtin {
            name: "join",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(join),
        },
        Builtin {
            name: "keys",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(keys),
        },
        Builtin {
            name: "len",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(len),
        },
        Builtin {
            name: "lines",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(lines),
        },
        Builtin {
            name: "linspace",
            arity: Arity::Exact(3),
            func: BuiltinFunc::Plain(linspace),
        },
        Builtin {
            name: "list",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(list_of),
        },
        Builtin {
            name: "lower",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(lower),
        },
        Builtin {
            name: "matmul",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(matmul),
        },
        Builtin {
            name: "max",
            arity: Arity::Range(1, 2),
            func: BuiltinFunc::Plain(max),
        },
        Builtin {
            name: "min",
            arity: Arity::Range(1, 2),
            func: BuiltinFunc::Plain(min),
        },
        Builtin {
            name: "ndim",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(ndim),
        },
        Builtin {
            name: "next",
            arity: Arity::Exact(1),
            func: BuiltinFunc::WithInterp(next_gen),
        },
        Builtin {
            name: "ones",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(ones),
        },
        Builtin {
            name: "peek",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(peek),
        },
        Builtin {
            name: "pop",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(pop),
        },
        Builtin {
            name: "push",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(push),
        },
        Builtin {
            name: "queue",
            arity: Arity::Range(0, 1),
            func: BuiltinFunc::Plain(queue_of),
        },
        Builtin {
            name: "range",
            arity: Arity::Range(1, 2),
            func: BuiltinFunc::Plain(range),
        },
        Builtin {
            name: "read_file",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(read_file),
        },
        Builtin {
            name: "replace",
            arity: Arity::Exact(3),
            func: BuiltinFunc::Plain(replace),
        },
        Builtin {
            name: "reshape",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(reshape),
        },
        Builtin {
            name: "shape",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(shape_of),
        },
        Builtin {
            name: "size",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(size_of),
        },
        Builtin {
            name: "split",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(split),
        },
        Builtin {
            name: "stack",
            arity: Arity::Range(0, 1),
            func: BuiltinFunc::Plain(stack_of),
        },
        Builtin {
            name: "starts_with",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(starts_with),
        },
        Builtin {
            name: "str",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(str_of),
        },
        Builtin {
            name: "sum",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(sum),
        },
        Builtin {
            name: "transpose",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(transpose),
        },
        Builtin {
            name: "trim",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(trim),
        },
        Builtin {
            name: "type",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(type_of),
        },
        Builtin {
            name: "upper",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(upper),
        },
        Builtin {
            name: "where",
            arity: Arity::Exact(3),
            func: BuiltinFunc::Plain(where_select),
        },
        Builtin {
            name: "write_file",
            arity: Arity::Exact(2),
            func: BuiltinFunc::Plain(write_file),
        },
        Builtin {
            name: "zeros",
            arity: Arity::Exact(1),
            func: BuiltinFunc::Plain(zeros),
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
/// `"è"` has length 1), or number of elements in any collection
/// (list, tuple, dict, set, array, queue, stack).
fn len(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
        Value::List(list) => Ok(Value::Int(list.borrow().len() as i64)),
        Value::Tuple(items) => Ok(Value::Int(items.len() as i64)),
        Value::Dict(dict) => Ok(Value::Int(dict.borrow().entries.len() as i64)),
        Value::Set(set) => Ok(Value::Int(set.borrow().len() as i64)),
        Value::Array(array) => Ok(Value::Int(array.borrow().items.len() as i64)),
        Value::Queue(queue) => Ok(Value::Int(queue.borrow().len() as i64)),
        Value::Stack(stack) => Ok(Value::Int(stack.borrow().len() as i64)),
        _ => Err(RuntimeError::new(
            "len() expects a Str, List, Tuple, Dict, Set, Array, Queue or Stack",
        )),
    }
}

/// `type(x)` — the type name as a string. An instance reports
/// its class name (`type(p)` is `"Persona"`); a class reports
/// `"class"`.
fn type_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let name = match &args[0] {
        Value::Int(_) => "int".to_string(),
        Value::Float(_) => "float".to_string(),
        Value::Str(_) => "str".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Nil => "nil".to_string(),
        Value::List(_) => "list".to_string(),
        Value::Tuple(_) => "tuple".to_string(),
        Value::Dict(_) => "dict".to_string(),
        Value::Set(_) => "set".to_string(),
        Value::Array(_) => "array".to_string(),
        Value::Queue(_) => "queue".to_string(),
        Value::Stack(_) => "stack".to_string(),
        Value::Function(_) | Value::Builtin(_) | Value::BoundMethod(_) => "fn".to_string(),
        Value::Class(_) => "class".to_string(),
        Value::Module(_) => "module".to_string(),
        Value::Generator(_) => "generator".to_string(),
        Value::Instance(inst) => inst.borrow().class.name.clone(),
    };
    Ok(Value::Str(name))
}

/// `next(gen)` — pull the next value out of a generator (replay
/// its body up to the next `yield`). When the generator is
/// exhausted, the error is `"generator exhausted"` — catchable
/// with `try`/`except`, and swallowed silently by `for x in gen`.
/// Anything that is not a generator is an error.
fn next_gen(
    interp: &mut Interpreter,
    args: &[Value],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::Generator(generator) = &args[0] else {
        return Err(RuntimeError::new(format!(
            "next() expects a generator, not {}",
            args[0].type_name()
        )));
    };
    match interp.next_generator(generator, input, out)? {
        Some(value) => Ok(value),
        None => Err(RuntimeError::new("generator exhausted")),
    }
}

/// `str(x)` — convert to a string, same representation as `print`.
/// Note: the interpreter intercepts `str()` on an instance whose
/// class defines a `str` method, so that method wins (see
/// `Interpreter::display`); this function only sees the rest.
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
/// Mixed int/float gives a float result. With a single array
/// argument, `min(a)` / `max(a)` is the smallest/largest element
/// of the whole array.
fn min_max(name: &str, args: &[Value], want_min: bool) -> Result<Value, RuntimeError> {
    if args.len() == 1 {
        let Value::Array(array) = &args[0] else {
            return Err(RuntimeError::new(format!(
                "{name}() expects two numbers or one Array"
            )));
        };
        let borrowed = array.borrow();
        let mut best: Option<Value> = None;
        for item in borrowed.items.iter() {
            let Some(x) = as_f64(item) else {
                return Err(RuntimeError::new(format!(
                    "{name}() expects numbers in the array, not {}",
                    item.type_name()
                )));
            };
            best = Some(match best {
                None => item.clone(),
                Some(current) => {
                    let current_x = as_f64(&current).unwrap_or(f64::NAN);
                    let take = if want_min {
                        x < current_x
                    } else {
                        x > current_x
                    };
                    if take { item.clone() } else { current }
                }
            });
        }
        return best.ok_or_else(|| RuntimeError::new(format!("{name}() of empty array")));
    }
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

/// The argument as a queue, or a clear error naming the builtin.
fn as_queue<'a>(
    name: &str,
    value: &'a Value,
) -> Result<&'a Rc<RefCell<VecDeque<Value>>>, RuntimeError> {
    match value {
        Value::Queue(queue) => Ok(queue),
        _ => Err(RuntimeError::new(format!("{name}() expects a Queue"))),
    }
}

/// The argument as a dict, or a clear error naming the builtin.
fn as_dict<'a>(
    name: &str,
    value: &'a Value,
) -> Result<&'a Rc<RefCell<crate::value::DictData>>, RuntimeError> {
    match value {
        Value::Dict(dict) => Ok(dict),
        _ => Err(RuntimeError::new(format!("{name}() expects a Dict"))),
    }
}

/// `push(list, value)` — append `value` to the list (the list is
/// mutated in place) and return `nil`. Also works on a stack.
fn push(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::List(list) => {
            list.borrow_mut().push(args[1].clone());
            Ok(Value::Nil)
        }
        Value::Stack(stack) => {
            stack.borrow_mut().push(args[1].clone());
            Ok(Value::Nil)
        }
        _ => Err(RuntimeError::new("push() expects a List or a Stack")),
    }
}

/// `pop(list)` — remove and return the last element; an error on
/// an empty list. Also works on a stack.
fn pop(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::List(list) => list
            .borrow_mut()
            .pop()
            .ok_or_else(|| RuntimeError::new("pop() on empty list")),
        Value::Stack(stack) => stack
            .borrow_mut()
            .pop()
            .ok_or_else(|| RuntimeError::new("pop() on empty stack")),
        _ => Err(RuntimeError::new("pop() expects a List or a Stack")),
    }
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

// ---- Phase 7: collections ----

/// The argument as an array, or a clear error naming the builtin.
fn as_array<'a>(name: &str, value: &'a Value) -> Result<&'a Rc<RefCell<ArrayData>>, RuntimeError> {
    match value {
        Value::Array(array) => Ok(array),
        _ => Err(RuntimeError::new(format!(
            "{name}() expects an Array, not {}",
            value.type_name()
        ))),
    }
}

/// The argument as a number (f64), or a clear error naming the
/// builtin.
fn as_f64_named(name: &str, value: &Value) -> Result<f64, RuntimeError> {
    as_f64(value).ok_or_else(|| {
        RuntimeError::new(format!(
            "{name}() expects numbers, not {}",
            value.type_name()
        ))
    })
}

/// `array([1, 2, 3])` — a fixed-size array with the element type of
/// the list's first element. An empty list has no type to infer and
/// mixed types are an error. The array is 1D, with shape `[n]`.
fn array(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let list = as_list("array", &args[0])?;
    let borrowed = list.borrow();
    let Some(first) = borrowed.first() else {
        return Err(RuntimeError::new("cannot infer array type from empty list"));
    };
    let Some(elem) = elem_type_of(first) else {
        return Err(RuntimeError::new(format!(
            "array() elements must be Int, Float, Str or Bool, not {}",
            first.type_name()
        )));
    };
    for item in borrowed.iter() {
        if elem_type_of(item) != Some(elem) {
            return Err(RuntimeError::new(
                "array() elements must all have the same type",
            ));
        }
    }
    Ok(Value::Array(Rc::new(RefCell::new(ArrayData {
        elem,
        shape: vec![borrowed.len()],
        items: borrowed.clone(),
    }))))
}

/// `queue()` — an empty FIFO queue; `queue([1, 2])` — a queue with
/// the list's elements, front first.
fn queue_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Ok(Value::Queue(Rc::new(RefCell::new(VecDeque::new()))));
    }
    let list = as_list("queue", &args[0])?;
    Ok(Value::Queue(Rc::new(RefCell::new(VecDeque::from(
        list.borrow().clone(),
    )))))
}

/// `stack()` — an empty LIFO stack; `stack([1, 2])` — a stack with
/// the list's elements, the last one on top.
fn stack_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Ok(Value::Stack(Rc::new(RefCell::new(Vec::new()))));
    }
    let list = as_list("stack", &args[0])?;
    Ok(Value::Stack(Rc::new(RefCell::new(list.borrow().clone()))))
}

/// `enqueue(q, x)` — add `x` at the back of the queue.
fn enqueue(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let queue = as_queue("enqueue", &args[0])?;
    queue.borrow_mut().push_back(args[1].clone());
    Ok(Value::Nil)
}

/// `dequeue(q)` — remove and return the front element; an error on
/// an empty queue.
fn dequeue(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let queue = as_queue("dequeue", &args[0])?;
    queue
        .borrow_mut()
        .pop_front()
        .ok_or_else(|| RuntimeError::new("dequeue() on empty queue"))
}

/// `peek(x)` — the top of a stack, the front of a queue, or the
/// last element of a list; an error on an empty one.
fn peek(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match &args[0] {
        Value::Stack(stack) => stack
            .borrow()
            .last()
            .cloned()
            .ok_or_else(|| RuntimeError::new("peek() on empty stack")),
        Value::Queue(queue) => queue
            .borrow()
            .front()
            .cloned()
            .ok_or_else(|| RuntimeError::new("peek() on empty queue")),
        Value::List(list) => list
            .borrow()
            .last()
            .cloned()
            .ok_or_else(|| RuntimeError::new("peek() on empty list")),
        other => Err(RuntimeError::new(format!(
            "peek() expects a List, a Queue or a Stack, not {}",
            other.type_name()
        ))),
    }
}

/// `keys(d)` — the keys of a dict as a list, in insertion order.
fn keys(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let dict = as_dict("keys", &args[0])?;
    let values: Vec<Value> = dict
        .borrow()
        .keys()
        .into_iter()
        .map(|key| key.to_value())
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(values))))
}

/// `contains(coll, x)` — membership: an element of a list, tuple,
/// array or set; a key of a dict; a substring of a string. A value
/// that can never be a member (unhashable, for sets and dicts)
/// simply returns `false`.
fn contains(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let (coll, x) = (&args[0], &args[1]);
    let found = match coll {
        Value::List(list) => list.borrow().iter().any(|item| values_equal(item, x)),
        Value::Tuple(items) => items.iter().any(|item| values_equal(item, x)),
        Value::Array(array) => array
            .borrow()
            .items
            .iter()
            .any(|item| values_equal(item, x)),
        Value::Set(set) => match x.to_key() {
            Some(key) => set.borrow().contains(&key),
            None => false,
        },
        Value::Dict(dict) => match x.to_key() {
            Some(key) => dict.borrow().get(&key).is_some(),
            None => false,
        },
        Value::Str(s) => {
            let Value::Str(needle) = x else {
                return Err(RuntimeError::new(
                    "contains() expects a Str to search in a Str",
                ));
            };
            s.contains(needle.as_str())
        }
        other => {
            return Err(RuntimeError::new(format!(
                "contains() expects a List, Tuple, Dict, Set, Array or Str, not {}",
                other.type_name()
            )));
        }
    };
    Ok(Value::Bool(found))
}

/// `list(x)` — convert a tuple, set, array, queue or stack to a
/// list; a dict gives its keys (like `keys`). A string is an error:
/// use `split` or `lines` to turn text into a list.
fn list_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let values = match &args[0] {
        Value::List(list) => list.borrow().clone(),
        Value::Tuple(items) => items.as_ref().clone(),
        Value::Set(set) => set.borrow().iter().map(|key| key.to_value()).collect(),
        Value::Array(array) => array.borrow().items.clone(),
        Value::Queue(queue) => queue.borrow().iter().cloned().collect(),
        Value::Stack(stack) => stack.borrow().clone(),
        Value::Dict(dict) => dict
            .borrow()
            .keys()
            .into_iter()
            .map(|key| key.to_value())
            .collect(),
        other => {
            return Err(RuntimeError::new(format!(
                "list() cannot convert {} to List",
                other.type_name()
            )));
        }
    };
    Ok(Value::List(Rc::new(RefCell::new(values))))
}

// ---- Phase 8: OOP ----

/// `is_a(x, Classe)` — true when `x` is an instance of `Classe`
/// or of one of its subclasses (`is_a(studente, Persona)` is true
/// when `Studente extends Persona`). Anything that is not an
/// instance is false; the second argument must be a class.
fn is_a(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::Class(class) = &args[1] else {
        return Err(RuntimeError::new(
            "is_a() expects a Class as its second argument",
        ));
    };
    let result = match &args[0] {
        Value::Instance(inst) => inst.borrow().class.is_a(class),
        _ => false,
    };
    Ok(Value::Bool(result))
}

// ---- Phase 10: standard library builtins ----

/// `sum(list)` — the sum of a list of numbers. All integers give
/// an integer (an overflow is an error); any float makes the
/// result a float. An empty list or a non-number is an error.
/// Also works on an array: the sum of all its elements.
fn sum(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let items: Vec<Value> = match &args[0] {
        Value::List(list) => list.borrow().clone(),
        Value::Array(array) => array.borrow().items.clone(),
        other => {
            return Err(RuntimeError::new(format!(
                "sum() expects a List or an Array, not {}",
                other.type_name()
            )));
        }
    };
    if items.is_empty() {
        return Err(RuntimeError::new("sum() of empty list"));
    }
    // A Bool array sums to the count of `true` values.
    if let Value::Array(array) = &args[0]
        && array.borrow().elem == ElemType::Bool
    {
        let count = array
            .borrow()
            .items
            .iter()
            .filter(|item| matches!(item, Value::Bool(true)))
            .count();
        return Ok(Value::Int(count as i64));
    }
    let mut int_total: i64 = 0;
    let mut float_total: f64 = 0.0;
    let mut is_float = false;
    for item in items.iter() {
        match item {
            Value::Int(n) => {
                int_total = int_total
                    .checked_add(*n)
                    .ok_or_else(|| RuntimeError::new("integer overflow"))?;
                float_total += *n as f64;
            }
            Value::Float(x) => {
                is_float = true;
                float_total += *x;
            }
            other => {
                return Err(RuntimeError::new(format!(
                    "sum() expects numbers, not {}",
                    other.type_name()
                )));
            }
        }
    }
    if is_float {
        Ok(Value::Float(float_total))
    } else {
        Ok(Value::Int(int_total))
    }
}

/// `join(sep, list)` — the complement of `split`: join the
/// strings of the list with the separator between them.
fn join(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let sep = as_str("join", &args[0])?;
    let list = as_list("join", &args[1])?;
    let borrowed = list.borrow();
    let mut parts: Vec<&str> = Vec::new();
    for item in borrowed.iter() {
        let Value::Str(s) = item else {
            return Err(RuntimeError::new(format!(
                "join() expects a List of Str, not {}",
                item.type_name()
            )));
        };
        parts.push(s.as_str());
    }
    Ok(Value::Str(parts.join(sep)))
}

/// `upper(s)` / `lower(s)` — the string in upper/lower case
/// (Unicode-aware: `upper("è")` is `"È"`).
fn upper(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("upper", &args[0])?;
    Ok(Value::Str(s.to_uppercase()))
}

fn lower(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("lower", &args[0])?;
    Ok(Value::Str(s.to_lowercase()))
}

/// `trim(s)` — the string without leading and trailing
/// whitespace (Unicode whitespace).
fn trim(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("trim", &args[0])?;
    Ok(Value::Str(s.trim().to_string()))
}

/// `replace(s, old, new)` — the string with every occurrence of
/// `old` replaced by `new`. An empty `old` is an error.
fn replace(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("replace", &args[0])?;
    let old = as_str("replace", &args[1])?;
    let new = as_str("replace", &args[2])?;
    if old.is_empty() {
        return Err(RuntimeError::new("replace() old string must not be empty"));
    }
    Ok(Value::Str(s.replace(old, new)))
}

/// `starts_with(s, prefix)` / `ends_with(s, suffix)` — true when
/// the string starts/ends with the given text.
fn starts_with(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("starts_with", &args[0])?;
    let prefix = as_str("starts_with", &args[1])?;
    Ok(Value::Bool(s.starts_with(prefix)))
}

fn ends_with(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let s = as_str("ends_with", &args[0])?;
    let suffix = as_str("ends_with", &args[1])?;
    Ok(Value::Bool(s.ends_with(suffix)))
}

// ---- Phase 11: N-dimensional numeric arrays ----

/// `zeros(n)` / `zeros([r, c])` — an Int array of zeros with the
/// given shape (an Int means the 1D shape `[n]`).
fn zeros(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let shape = parse_shape(&args[0], "zeros")?;
    check_shape_size(&shape, "zeros")?;
    let total: usize = shape.iter().product();
    Ok(make_array(
        crate::value::ElemType::Int,
        shape,
        vec![Value::Int(0); total],
    ))
}

/// `ones(n)` / `ones([r, c])` — an Int array of ones with the
/// given shape.
fn ones(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let shape = parse_shape(&args[0], "ones")?;
    check_shape_size(&shape, "ones")?;
    let total: usize = shape.iter().product();
    Ok(make_array(
        crate::value::ElemType::Int,
        shape,
        vec![Value::Int(1); total],
    ))
}

/// `arange(n)` is `[0, 1, ..., n-1]`; `arange(a, b)` is
/// `[a, ..., b-1]`; `arange(a, b, step)` steps by `step`
/// (negative steps count down). A step of 0 is an error, and the
/// result is capped like `range` (max 1000000 elements).
fn arange(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let (start, end, step) = match args.len() {
        1 => (0, as_int("arange", &args[0])?, 1),
        2 => (as_int("arange", &args[0])?, as_int("arange", &args[1])?, 1),
        _ => (
            as_int("arange", &args[0])?,
            as_int("arange", &args[1])?,
            as_int("arange", &args[2])?,
        ),
    };
    if step == 0 {
        return Err(RuntimeError::new("arange() step must not be zero"));
    }
    // Element count in i128 so no overflow is possible.
    let count: i128 = if step > 0 {
        if end <= start {
            0
        } else {
            (end as i128 - start as i128 + step as i128 - 1) / step as i128
        }
    } else if end >= start {
        0
    } else {
        (start as i128 - end as i128 - step as i128 - 1) / (-step) as i128
    };
    if count > MAX_RANGE_LEN {
        return Err(RuntimeError::new(
            "arange() is too large (max 1000000 elements)",
        ));
    }
    let mut values = Vec::with_capacity(count as usize);
    for k in 0..count {
        let value = start as i128 + k * step as i128;
        values.push(Value::Int(value as i64));
    }
    let len = values.len();
    Ok(make_array(crate::value::ElemType::Int, vec![len], values))
}

/// `linspace(a, b, n)` — `n` Float points evenly spaced from `a`
/// to `b`, both ends included. `n` must be at least 1.
fn linspace(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_f64_named("linspace", &args[0])?;
    let b = as_f64_named("linspace", &args[1])?;
    let n = as_int("linspace", &args[2])?;
    if n <= 0 {
        return Err(RuntimeError::new("linspace() needs at least 1 point"));
    }
    if n as i128 > MAX_RANGE_LEN {
        return Err(RuntimeError::new(
            "linspace() is too large (max 1000000 elements)",
        ));
    }
    let mut values = Vec::with_capacity(n as usize);
    if n == 1 {
        values.push(Value::Float(a));
    } else {
        for i in 0..n {
            let t = i as f64 / (n - 1) as f64;
            values.push(Value::Float(a + t * (b - a)));
        }
    }
    Ok(make_array(
        crate::value::ElemType::Float,
        vec![n as usize],
        values,
    ))
}

/// `shape(a)` — the shape of an array as a list of Ints.
fn shape_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let array = as_array("shape", &args[0])?;
    let dims: Vec<Value> = array
        .borrow()
        .shape
        .iter()
        .map(|d| Value::Int(*d as i64))
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(dims))))
}

/// `ndim(a)` — the number of dimensions of an array.
fn ndim(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let array = as_array("ndim", &args[0])?;
    Ok(Value::Int(array.borrow().ndim() as i64))
}

/// `size(a)` — the total number of elements of an array.
fn size_of(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let array = as_array("size", &args[0])?;
    Ok(Value::Int(array.borrow().size() as i64))
}

/// `dot(a, b)` — the scalar product of two 1D vectors.
fn dot(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("dot", &args[0])?;
    let b = as_array("dot", &args[1])?;
    array::dot(&a.borrow(), &b.borrow())
}

/// `matmul(a, b)` — the matrix product: 2D x 2D, 1D x 2D and
/// 2D x 1D. Two 1D vectors are an error (use `dot()`).
fn matmul(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("matmul", &args[0])?;
    let b = as_array("matmul", &args[1])?;
    array::matmul(&a.borrow(), &b.borrow())
}

/// `transpose(a)` — swap the two axes of a 2D array; a 1D array
/// is returned unchanged.
fn transpose(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("transpose", &args[0])?;
    array::transpose_array(&a.borrow())
}

/// `reshape(a, shape)` — the same items with a new shape (an Int
/// means the 1D shape `[n]`); the total number of elements must
/// stay the same.
fn reshape(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("reshape", &args[0])?;
    let shape = parse_shape(&args[1], "reshape")?;
    array::reshape_array(&a.borrow(), shape)
}

/// `where(cond, a, b)` — element-by-element selection: where the
/// Bool array `cond` is true take the element from `a`, otherwise
/// from `b`. `a` and `b` are arrays of the same shape as `cond`,
/// or scalars (broadcast). The result's element type promotes
/// Int+Float to Float; Bool+Bool stays Bool, Str+Str stays Str.
fn where_select(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let cond = as_array("where", &args[0])?;
    let cond = cond.borrow();
    if cond.elem != ElemType::Bool {
        return Err(RuntimeError::new(format!(
            "where() expects a Bool array as its condition, not Array of {}",
            cond.elem.name()
        )));
    }
    let shape = cond.shape.clone();
    // Each of a/b: an array of the condition's shape (its items),
    // or a scalar (broadcast). Returns the element type and the
    // items, when it is an array.
    let pick = |value: &Value| -> Result<(ElemType, Option<Vec<Value>>), RuntimeError> {
        match value {
            Value::Array(array) => {
                let array = array.borrow();
                if array.shape != shape {
                    return Err(RuntimeError::new(format!(
                        "where() argument shape {} does not match condition shape {}",
                        fmt_shape(&array.shape),
                        fmt_shape(&shape)
                    )));
                }
                Ok((array.elem, Some(array.items.clone())))
            }
            Value::Int(_) => Ok((ElemType::Int, None)),
            Value::Float(_) => Ok((ElemType::Float, None)),
            Value::Str(_) => Ok((ElemType::Str, None)),
            Value::Bool(_) => Ok((ElemType::Bool, None)),
            other => Err(RuntimeError::new(format!(
                "where() expects an Array or a scalar, not {}",
                other.type_name()
            ))),
        }
    };
    let (elem_a, items_a) = pick(&args[1])?;
    let (elem_b, items_b) = pick(&args[2])?;
    let elem = promote_elem(elem_a, elem_b)?;
    let items = cond
        .items
        .iter()
        .enumerate()
        .map(|(i, flag)| {
            let value = match (matches!(flag, Value::Bool(true)), &items_a, &items_b) {
                (true, Some(items), _) => items[i].clone(),
                (true, None, _) => args[1].clone(),
                (false, _, Some(items)) => items[i].clone(),
                (false, _, None) => args[2].clone(),
            };
            // Keep the result homogeneous: an Int scalar picked
            // into a Float result becomes a Float.
            match (elem, value) {
                (ElemType::Float, Value::Int(n)) => Value::Float(n as f64),
                (_, value) => value,
            }
        })
        .collect();
    Ok(make_array(elem, shape, items))
}

/// The element type of a `where()` result: Int+Int stays Int,
/// anything with a Float becomes Float, Bool+Bool stays Bool,
/// Str+Str stays Str; anything else is an error.
fn promote_elem(a: ElemType, b: ElemType) -> Result<ElemType, RuntimeError> {
    match (a, b) {
        (ElemType::Int, ElemType::Int) => Ok(ElemType::Int),
        (ElemType::Float, _) | (_, ElemType::Float) => Ok(ElemType::Float),
        (ElemType::Bool, ElemType::Bool) => Ok(ElemType::Bool),
        (ElemType::Str, ElemType::Str) => Ok(ElemType::Str),
        _ => Err(RuntimeError::new(format!(
            "where() arguments have incompatible types: {} and {}",
            a.name(),
            b.name()
        ))),
    }
}
