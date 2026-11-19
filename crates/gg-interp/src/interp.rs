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

use std::cmp::Ordering;
use std::io::Write;
use std::rc::Rc;

use gg_parser::ast::{BinaryOp, Expr, Program, Stmt, UnaryOp};

use crate::env::Environment;
use crate::error::RuntimeError;
use crate::value::{Function, Value, values_equal};

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
        Self {
            globals: Environment::new(),
        }
    }

    /// Run a whole program. `print` output goes to `out`.
    /// A `return` at the top level is a runtime error.
    pub fn run(&mut self, program: &Program, out: &mut dyn Write) -> Result<(), RuntimeError> {
        let globals = Rc::clone(&self.globals);
        match self.exec_block(&program.stmts, &globals, out)? {
            Flow::Normal => Ok(()),
            Flow::Return(_) => Err(RuntimeError::new("return outside function")),
        }
    }

    /// Execute a block of statements in `env`. Stops early on `return`.
    fn exec_block(
        &mut self,
        stmts: &[Stmt],
        env: &Rc<Environment>,
        out: &mut dyn Write,
    ) -> Result<Flow, RuntimeError> {
        for stmt in stmts {
            match self.exec_stmt(stmt, env, out)? {
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
        out: &mut dyn Write,
    ) -> Result<Flow, RuntimeError> {
        match stmt {
            Stmt::Let { name, value } => {
                let value = self.eval_expr(value, env, out)?;
                env.define(name, value);
                Ok(Flow::Normal)
            }
            Stmt::Assign { name, value } => {
                let value = self.eval_expr(value, env, out)?;
                env.assign(name, value)?;
                Ok(Flow::Normal)
            }
            Stmt::Print { expr } => {
                let value = self.eval_expr(expr, env, out)?;
                writeln!(out, "{value}")
                    .map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
                Ok(Flow::Normal)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let cond = self.eval_expr(cond, env, out)?;
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
                self.exec_block(branch, &child, out)
            }
            Stmt::While { cond, body } => loop {
                let cond = self.eval_expr(cond, env, out)?;
                if !cond.is_truthy() {
                    return Ok(Flow::Normal);
                }
                let child = Environment::child(env);
                match self.exec_block(body, &child, out)? {
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
                let from = self.eval_expr(from, env, out)?;
                let to = self.eval_expr(to, env, out)?;
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
                    match self.exec_block(body, &child, out)? {
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
                    Some(expr) => self.eval_expr(expr, env, out)?,
                    None => Value::Nil,
                };
                Ok(Flow::Return(value))
            }
            Stmt::ExprStmt { expr } => {
                self.eval_expr(expr, env, out)?;
                Ok(Flow::Normal)
            }
        }
    }

    /// Evaluate an expression to a value.
    fn eval_expr(
        &mut self,
        expr: &Expr,
        env: &Rc<Environment>,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Ident(name) => env.get(name),
            Expr::Unary { op, expr } => {
                let value = self.eval_expr(expr, env, out)?;
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
            Expr::Binary { op, left, right } => self.eval_binary(*op, left, right, env, out),
            Expr::Call { name, args } => {
                let callee = env.get(name)?;
                let Value::Function(function) = callee else {
                    return Err(RuntimeError::new(format!(
                        "cannot call {}",
                        callee.type_name()
                    )));
                };
                if args.len() != function.params.len() {
                    return Err(RuntimeError::new(format!(
                        "expected {} argument(s) but got {}",
                        function.params.len(),
                        args.len()
                    )));
                }
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval_expr(arg, env, out)?);
                }
                self.call(&function, values, out)
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
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        match op {
            BinaryOp::And => {
                let left = self.eval_expr(left, env, out)?;
                if !left.is_truthy() {
                    return Ok(left);
                }
                self.eval_expr(right, env, out)
            }
            BinaryOp::Or => {
                let left = self.eval_expr(left, env, out)?;
                if left.is_truthy() {
                    return Ok(left);
                }
                self.eval_expr(right, env, out)
            }
            _ => {
                let left = self.eval_expr(left, env, out)?;
                let right = self.eval_expr(right, env, out)?;
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

    /// Call a user function: fresh scope under the captured environment,
    /// parameters bound, `return` caught as the result.
    fn call(
        &mut self,
        function: &Rc<Function>,
        args: Vec<Value>,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let call_env = Environment::child(&function.env);
        for (param, value) in function.params.iter().zip(args) {
            call_env.define(param, value);
        }
        match self.exec_block(&function.body, &call_env, out)? {
            Flow::Normal => Ok(Value::Nil),
            Flow::Return(value) => Ok(value),
        }
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
