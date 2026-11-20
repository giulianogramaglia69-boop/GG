// SPDX-License-Identifier: GPL-3.0-or-later
//! The GG tree-walking interpreter: AST -> values.
//!
//! Semantics (see README for the full list):
//! - truthiness: only `false` and `nil` are false;
//! - `and`/`or` short-circuit and return operand values;
//! - `/` is always float division, `%` is integer-only;
//! - `for i = a to b` includes both ends and steps +1 or -1;
//! - functions are closures: they capture the defining environment
//!   (and therefore see themselves, so recursion works).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::io::{BufRead, Write};
use std::rc::Rc;

use gg_parser::ast::{BinaryOp, Expr, Program, Stmt, UnaryOp};

use crate::builtin;
use crate::env::Environment;
use crate::error::RuntimeError;
use crate::value::{DictData, Function, Value, elem_type_of, values_equal};

/// What executing a statement produced: either control flows on
/// normally, or a `return` was hit with its value.
enum Flow {
    Normal,
    Return(Value),
}

/// The interpreter. One instance keeps a global environment across
/// `run` calls, so a REPL can reuse it later.
pub struct Interpreter {
    globals: Rc<Environment>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Environment::new();
        builtin::define_all(&globals);
        Self { globals }
    }

    /// Evaluate one expression in the global environment and return
    /// its value. Used by the REPL to echo expression results.
    pub fn eval(
        &mut self,
        expr: &Expr,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let globals = Rc::clone(&self.globals);
        self.eval_expr(expr, &globals, input, out)
    }

    /// Run a whole program. `print` output goes to `out`, `input`
    /// reads from `input`. A `return` at the top level is a runtime
    /// error.
    pub fn run(
        &mut self,
        program: &Program,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<(), RuntimeError> {
        let globals = Rc::clone(&self.globals);
        match self.exec_block(&program.stmts, &globals, input, out)? {
            Flow::Normal => Ok(()),
            Flow::Return(_) => Err(RuntimeError::new("return outside function")),
        }
    }

    /// Execute a block of statements in `env`. Stops early on `return`.
    fn exec_block(
        &mut self,
        stmts: &[Stmt],
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Flow, RuntimeError> {
        for stmt in stmts {
            match self.exec_stmt(stmt, env, input, out)? {
                Flow::Normal => {}
                flow @ Flow::Return(_) => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    /// Execute one statement.
    fn exec_stmt(
        &mut self,
        stmt: &Stmt,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Flow, RuntimeError> {
        match stmt {
            Stmt::Let { name, value } => {
                let value = self.eval_expr(value, env, input, out)?;
                env.define(name, value);
                Ok(Flow::Normal)
            }
            Stmt::Assign { name, value } => {
                let value = self.eval_expr(value, env, input, out)?;
                env.assign(name, value)?;
                Ok(Flow::Normal)
            }
            Stmt::IndexAssign { target, value } => {
                // The right-hand side is evaluated first (like
                // Python), then the index assignment happens.
                let value = self.eval_expr(value, env, input, out)?;
                self.assign_index(target, value, env, input, out)?;
                Ok(Flow::Normal)
            }
            Stmt::Print { expr } => {
                let value = self.eval_expr(expr, env, input, out)?;
                writeln!(out, "{value}")
                    .map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
                Ok(Flow::Normal)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let cond = self.eval_expr(cond, env, input, out)?;
                let branch = if cond.is_truthy() {
                    then_block
                } else {
                    match else_block {
                        Some(block) => block,
                        None => return Ok(Flow::Normal),
                    }
                };
                // Each branch runs in its own child scope.
                let child = Environment::child(env);
                self.exec_block(branch, &child, input, out)
            }
            Stmt::While { cond, body } => loop {
                let cond = self.eval_expr(cond, env, input, out)?;
                if !cond.is_truthy() {
                    return Ok(Flow::Normal);
                }
                let child = Environment::child(env);
                match self.exec_block(body, &child, input, out)? {
                    Flow::Normal => {}
                    flow @ Flow::Return(_) => return Ok(flow),
                }
            },
            Stmt::For {
                var,
                from,
                to,
                body,
            } => {
                let from = self.eval_expr(from, env, input, out)?;
                let to = self.eval_expr(to, env, input, out)?;
                let (Value::Int(from), Value::Int(to)) = (from, to) else {
                    return Err(RuntimeError::new("for loop bounds must be integers"));
                };
                // The loop variable lives in its own scope, visible only
                // to the body; the bounds are inclusive, step is +1 or -1.
                let loop_env = Environment::child(env);
                let mut i = from;
                loop {
                    let done = if from <= to { i > to } else { i < to };
                    if done {
                        break;
                    }
                    loop_env.define(var, Value::Int(i));
                    let child = Environment::child(&loop_env);
                    match self.exec_block(body, &child, input, out)? {
                        Flow::Normal => {}
                        flow @ Flow::Return(_) => return Ok(flow),
                    }
                    i += if from <= to { 1 } else { -1 };
                }
                Ok(Flow::Normal)
            }
            Stmt::Fn { name, params, body } => {
                // The function captures `env` itself, so it can see the
                // name being defined here: recursion works.
                let function = Function {
                    params: params.clone(),
                    body: body.clone(),
                    env: Rc::clone(env),
                };
                env.define(name, Value::Function(Rc::new(function)));
                Ok(Flow::Normal)
            }
            Stmt::Return { expr } => {
                let value = match expr {
                    Some(expr) => self.eval_expr(expr, env, input, out)?,
                    None => Value::Nil,
                };
                Ok(Flow::Return(value))
            }
            Stmt::ExprStmt { expr } => {
                self.eval_expr(expr, env, input, out)?;
                Ok(Flow::Normal)
            }
        }
    }

    /// Evaluate an expression to a value.
    fn eval_expr(
        &mut self,
        expr: &Expr,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Ident(name) => env.get(name),
            Expr::List(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.eval_expr(element, env, input, out)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(values))))
            }
            Expr::Tuple(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.eval_expr(element, env, input, out)?);
                }
                Ok(Value::Tuple(Rc::new(values)))
            }
            Expr::Dict(entries) => {
                let mut data = DictData::new();
                for (key_expr, value_expr) in entries {
                    let key = self.eval_expr(key_expr, env, input, out)?;
                    let Some(key) = key.to_key() else {
                        return Err(RuntimeError::new(format!(
                            "dict key must be Int, Float, Str or Bool, not {}",
                            key.type_name()
                        )));
                    };
                    let value = self.eval_expr(value_expr, env, input, out)?;
                    data.insert(key, value);
                }
                Ok(Value::Dict(Rc::new(RefCell::new(data))))
            }
            Expr::Set(elements) => {
                let mut items = Vec::new();
                for element in elements {
                    let value = self.eval_expr(element, env, input, out)?;
                    let Some(key) = value.to_key() else {
                        return Err(RuntimeError::new(format!(
                            "set element must be Int, Float, Str or Bool, not {}",
                            value.type_name()
                        )));
                    };
                    // Duplicates are ignored silently.
                    if !items.contains(&key) {
                        items.push(key);
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(items))))
            }
            Expr::Index { target, index } => {
                let target = self.eval_expr(target, env, input, out)?;
                let index = self.eval_expr(index, env, input, out)?;
                index_value(&target, index)
            }
            Expr::Unary { op, expr } => {
                let value = self.eval_expr(expr, env, input, out)?;
                match op {
                    UnaryOp::Not => Ok(Value::Bool(!value.is_truthy())),
                    UnaryOp::Neg => match value {
                        Value::Int(n) => n
                            .checked_neg()
                            .map(Value::Int)
                            .ok_or_else(|| RuntimeError::new("integer overflow")),
                        Value::Float(x) => Ok(Value::Float(-x)),
                        other => Err(RuntimeError::new(format!(
                            "cannot negate {}",
                            other.type_name()
                        ))),
                    },
                }
            }
            Expr::Binary { op, left, right } => self.eval_binary(*op, left, right, env, input, out),
            Expr::Call { name, args } => {
                let callee = env.get(name)?;
                match callee {
                    Value::Function(function) => {
                        if args.len() != function.params.len() {
                            return Err(RuntimeError::new(format!(
                                "expected {} argument(s) but got {}",
                                function.params.len(),
                                args.len()
                            )));
                        }
                        let mut values = Vec::with_capacity(args.len());
                        for arg in args {
                            values.push(self.eval_expr(arg, env, input, out)?);
                        }
                        self.call(&function, values, input, out)
                    }
                    Value::Builtin(builtin) => {
                        if !builtin.arity.accepts(args.len()) {
                            return Err(RuntimeError::new(format!(
                                "{}() expects {} argument(s) but got {}",
                                builtin.name,
                                builtin.arity.describe(),
                                args.len()
                            )));
                        }
                        let mut values = Vec::with_capacity(args.len());
                        for arg in args {
                            values.push(self.eval_expr(arg, env, input, out)?);
                        }
                        (builtin.func)(&values, input, out)
                    }
                    other => Err(RuntimeError::new(format!(
                        "cannot call {}",
                        other.type_name()
                    ))),
                }
            }
        }
    }

    /// Evaluate a binary operator. `and`/`or` short-circuit and return
    /// operand values instead of booleans.
    fn eval_binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        match op {
            BinaryOp::And => {
                let left = self.eval_expr(left, env, input, out)?;
                if !left.is_truthy() {
                    return Ok(left);
                }
                self.eval_expr(right, env, input, out)
            }
            BinaryOp::Or => {
                let left = self.eval_expr(left, env, input, out)?;
                if left.is_truthy() {
                    return Ok(left);
                }
                self.eval_expr(right, env, input, out)
            }
            _ => {
                let left = self.eval_expr(left, env, input, out)?;
                let right = self.eval_expr(right, env, input, out)?;
                match op {
                    BinaryOp::Eq => Ok(Value::Bool(values_equal(&left, &right))),
                    BinaryOp::NotEq => Ok(Value::Bool(!values_equal(&left, &right))),
                    BinaryOp::Add => add_values(&left, &right),
                    BinaryOp::Sub => {
                        numeric_op(&left, &right, "subtract", i64::checked_sub, |a, b| a - b)
                    }
                    BinaryOp::Mul => {
                        numeric_op(&left, &right, "multiply", i64::checked_mul, |a, b| a * b)
                    }
                    BinaryOp::Div => divide_values(&left, &right),
                    BinaryOp::Mod => modulo_values(&left, &right),
                    BinaryOp::Lt => compare_values(&left, &right, Ordering::Less, false),
                    BinaryOp::LtEq => compare_values(&left, &right, Ordering::Less, true),
                    BinaryOp::Gt => compare_values(&left, &right, Ordering::Greater, false),
                    BinaryOp::GtEq => compare_values(&left, &right, Ordering::Greater, true),
                    BinaryOp::And | BinaryOp::Or => unreachable!(),
                }
            }
        }
    }

    /// Assign through an index: the target must be an `Index`
    /// expression (`xs[0]`, `m[0][1]`, `d["k"]`). The container is
    /// evaluated first, then the final index, and the element is
    /// replaced — or inserted, for dicts.
    fn assign_index(
        &mut self,
        target: &Expr,
        value: Value,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<(), RuntimeError> {
        let Expr::Index { target, index } = target else {
            return Err(RuntimeError::new("invalid assignment target"));
        };
        let container = self.eval_expr(target, env, input, out)?;
        let index = self.eval_expr(index, env, input, out)?;
        match container {
            Value::List(list) => {
                let pos = resolve_index(list.borrow().len(), index, "list")?;
                list.borrow_mut()[pos] = value;
                Ok(())
            }
            Value::Dict(dict) => {
                let Some(key) = index.to_key() else {
                    return Err(RuntimeError::new(format!(
                        "dict key must be Int, Float, Str or Bool, not {}",
                        index.type_name()
                    )));
                };
                dict.borrow_mut().insert(key, value);
                Ok(())
            }
            Value::Array(array) => {
                let mut array = array.borrow_mut();
                let pos = resolve_index(array.items.len(), index, "array")?;
                let wanted = array.elem;
                match elem_type_of(&value) {
                    Some(elem) if elem == wanted => {
                        array.items[pos] = value;
                        Ok(())
                    }
                    _ => Err(RuntimeError::new(format!(
                        "array element must be {}, not {}",
                        wanted.name(),
                        value.type_name()
                    ))),
                }
            }
            Value::Tuple(_) => Err(RuntimeError::new(
                "cannot assign to tuple (tuples are immutable)",
            )),
            Value::Set(_) => Err(RuntimeError::new(
                "cannot assign to set (sets have no indexes)",
            )),
            other => Err(RuntimeError::new(format!(
                "cannot index {}",
                other.type_name()
            ))),
        }
    }

    /// Call a user function: fresh scope under the captured environment,
    /// parameters bound, `return` caught as the result.
    fn call(
        &mut self,
        function: &Rc<Function>,
        args: Vec<Value>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let call_env = Environment::child(&function.env);
        for (param, value) in function.params.iter().zip(args) {
            call_env.define(param, value);
        }
        match self.exec_block(&function.body, &call_env, input, out)? {
            Flow::Normal => Ok(Value::Nil),
            Flow::Return(value) => Ok(value),
        }
    }
}

/// Resolve a (possibly negative) index into a position: `-1` is the
/// last element, `-len` the first. Out of range is an error.
/// `kind` names the collection in the message ("list", "tuple",
/// "array").
fn resolve_index(len: usize, index: Value, kind: &str) -> Result<usize, RuntimeError> {
    let Value::Int(i) = index else {
        return Err(RuntimeError::new(format!(
            "{kind} index must be an integer"
        )));
    };
    let len = len as i64;
    let pos = if i < 0 { len + i } else { i };
    if pos < 0 || pos >= len {
        return Err(RuntimeError::new(format!("{kind} index out of range")));
    }
    Ok(pos as usize)
}

/// `xs[i]` / `t[i]` / `a[i]` / `d[k]`: read one element. Lists,
/// tuples and arrays take an integer index (negative from the end);
/// dicts take a hashable key and fail on a missing key.
fn index_value(target: &Value, index: Value) -> Result<Value, RuntimeError> {
    match target {
        Value::List(list) => {
            let borrowed = list.borrow();
            let pos = resolve_index(borrowed.len(), index, "list")?;
            Ok(borrowed[pos].clone())
        }
        Value::Tuple(items) => {
            let pos = resolve_index(items.len(), index, "tuple")?;
            Ok(items[pos].clone())
        }
        Value::Array(array) => {
            let borrowed = array.borrow();
            let pos = resolve_index(borrowed.items.len(), index, "array")?;
            Ok(borrowed.items[pos].clone())
        }
        Value::Dict(dict) => {
            let Some(key) = index.to_key() else {
                return Err(RuntimeError::new(format!(
                    "dict key must be Int, Float, Str or Bool, not {}",
                    index.type_name()
                )));
            };
            match dict.borrow().get(&key) {
                Some(value) => Ok(value.clone()),
                None => Err(RuntimeError::new("key not found")),
            }
        }
        other => Err(RuntimeError::new(format!(
            "cannot index {}",
            other.type_name()
        ))),
    }
}

/// `+`: integer/float arithmetic, or string concatenation.
fn add_values(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => a
            .checked_add(*b)
            .map(Value::Int)
            .ok_or_else(|| RuntimeError::new("integer overflow")),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
        (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
        _ => Err(RuntimeError::new(format!(
            "cannot add {} and {}",
            left.type_name(),
            right.type_name()
        ))),
    }
}

/// `-` and `*` on numbers (int+int stays int, anything float becomes
/// float). Integer overflow is an error, not a panic.
fn numeric_op(
    left: &Value,
    right: &Value,
    verb: &str,
    int_op: impl Fn(i64, i64) -> Option<i64>,
    float_op: impl Fn(f64, f64) -> f64,
) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => int_op(*a, *b)
            .map(Value::Int)
            .ok_or_else(|| RuntimeError::new("integer overflow")),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(*a, *b))),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(float_op(*a as f64, *b))),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(float_op(*a, *b as f64))),
        _ => Err(RuntimeError::new(format!(
            "cannot {verb} {} and {}",
            left.type_name(),
            right.type_name()
        ))),
    }
}

/// `/`: always float division, even between integers.
fn divide_values(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    let (a, b) = match (left, right) {
        (Value::Int(a), Value::Int(b)) => (*a as f64, *b as f64),
        (Value::Float(a), Value::Float(b)) => (*a, *b),
        (Value::Int(a), Value::Float(b)) => (*a as f64, *b),
        (Value::Float(a), Value::Int(b)) => (*a, *b as f64),
        _ => {
            return Err(RuntimeError::new(format!(
                "cannot divide {} and {}",
                left.type_name(),
                right.type_name()
            )));
        }
    };
    if b == 0.0 {
        return Err(RuntimeError::new("division by zero"));
    }
    Ok(Value::Float(a / b))
}

/// `%`: integers only; the remainder keeps the dividend's sign (Rust).
fn modulo_values(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int(_), Value::Int(0)) => Err(RuntimeError::new("modulo by zero")),
        (Value::Int(a), Value::Int(b)) => a
            .checked_rem(*b)
            .map(Value::Int)
            .ok_or_else(|| RuntimeError::new("integer overflow")),
        _ => Err(RuntimeError::new(format!(
            "cannot apply % to {} and {}",
            left.type_name(),
            right.type_name()
        ))),
    }
}

/// `<`, `<=`, `>`, `>=`: numbers (mixed int/float) or strings
/// (lexicographic order).
fn compare_values(
    left: &Value,
    right: &Value,
    wanted: Ordering,
    allow_equal: bool,
) -> Result<Value, RuntimeError> {
    let ordering = match (left, right) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
        (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
        (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
        _ => None,
    };
    match ordering {
        Some(ord) => Ok(Value::Bool(
            ord == wanted || (allow_equal && ord == Ordering::Equal),
        )),
        None => Err(RuntimeError::new(format!(
            "cannot compare {} and {}",
            left.type_name(),
            right.type_name()
        ))),
    }
}
