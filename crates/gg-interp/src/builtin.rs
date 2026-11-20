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

use crate::env::Environment;
use crate::error::RuntimeError;
use crate::value::{ArrayData, Value, elem_type_of, values_equal};

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
            name: "array",
            arity: Arity::Exact(1),
            func: array,
        },
        Builtin {
            name: "contains",
            arity: Arity::Exact(2),
            func: contains,
        },
        Builtin {
            name: "dequeue",
            arity: Arity::Exact(1),
            func: dequeue,
        },
        Builtin {
            name: "enqueue",
            arity: Arity::Exact(2),
            func: enqueue,
        },
        Builtin {
            name: "ends_with",
            arity: Arity::Exact(2),
            func: ends_with,
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
            name: "is_a",
            arity: Arity::Exact(2),
            func: is_a,
        },
        Builtin {
            name: "join",
            arity: Arity::Exact(2),
            func: join,
        },
        Builtin {
            name: "keys",
            arity: Arity::Exact(1),
            func: keys,
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
            name: "list",
            arity: Arity::Exact(1),
            func: list_of,
        },
        Builtin {
            name: "lower",
            arity: Arity::Exact(1),
            func: lower,
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
            name: "peek",
            arity: Arity::Exact(1),
            func: peek,
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
            name: "queue",
            arity: Arity::Range(0, 1),
            func: queue_of,
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
            name: "replace",
            arity: Arity::Exact(3),
            func: replace,
        },
        Builtin {
            name: "split",
            arity: Arity::Exact(2),
            func: split,
        },
        Builtin {
            name: "stack",
            arity: Arity::Range(0, 1),
            func: stack_of,
        },
        Builtin {
            name: "starts_with",
            arity: Arity::Exact(2),
            func: starts_with,
        },
        Builtin {
            name: "str",
            arity: Arity::Exact(1),
            func: str_of,
        },
        Builtin {
            name: "sum",
            arity: Arity::Exact(1),
            func: sum,
        },
        Builtin {
            name: "trim",
            arity: Arity::Exact(1),
            func: trim,
        },
        Builtin {
            name: "type",
            arity: Arity::Exact(1),
            func: type_of,
        },
        Builtin {
            name: "upper",
            arity: Arity::Exact(1),
            func: upper,
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
        Value::Instance(inst) => inst.borrow().class.name.clone(),
    };
    Ok(Value::Str(name))
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

/// `array([1, 2, 3])` — a fixed-size array with the element type of
/// the list's first element. An empty list has no type to infer and
/// mixed types are an error.
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
fn sum(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let list = as_list("sum", &args[0])?;
    let borrowed = list.borrow();
    if borrowed.is_empty() {
        return Err(RuntimeError::new("sum() of empty list"));
    }
    let mut int_total: i64 = 0;
    let mut float_total: f64 = 0.0;
    let mut is_float = false;
    for item in borrowed.iter() {
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
