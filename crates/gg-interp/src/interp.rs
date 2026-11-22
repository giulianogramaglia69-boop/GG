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
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::rc::Rc;

use gg_lexer::Span;
use gg_parser::ast::{BinaryOp, Expr, Program, Stmt, UnaryOp};

use crate::array::{self, ArrayOp, CompareOp, flat_index, subarray};
use crate::builtin::{self, BuiltinFunc};
use crate::env::Environment;
use crate::error::RuntimeError;
use crate::slice::{AxisSelector, IndexArg, SliceSpec};
use crate::value::{
    BoundMethod, ClassDef, DictData, Function, InstanceData, ModuleData, Value, elem_type_of,
    lookup_method, values_equal,
};

/// What executing a statement produced: either control flows on
/// normally, or a `return` was hit with its value.
enum Flow {
    Normal,
    /// `return` with its value and the span of the `return`.
    Return(Value, Span),
}

/// The items of a value a `for x in ...` loop iterates over, as a
/// snapshot taken once before the loop starts — mutations of the
/// collection inside the body do not change what the loop visits,
/// and a queue or stack is not consumed by iterating it.
/// - List, Tuple: the elements, in order.
/// - Array: the elements of a 1D array; the rows (1D arrays) of
///   a 2D+ array, first axis first.
/// - Set: the elements, in insertion order.
/// - Dict: the keys, in insertion order.
/// - Str: one-character strings, in order (Unicode characters,
///   so `"aè"` gives `"a"` then `"è"`).
/// - Queue, Stack: the elements, front to back / bottom to top.
///
/// Anything else is a runtime error: `Int is not iterable`.
fn iterable_items(value: &Value) -> Result<Vec<Value>, RuntimeError> {
    match value {
        Value::List(list) => Ok(list.borrow().clone()),
        Value::Tuple(items) => Ok(items.as_ref().clone()),
        Value::Array(array) => {
            let data = array.borrow();
            if data.shape.is_empty() {
                Ok(vec![])
            } else if data.shape.len() == 1 {
                Ok(data.items.clone())
            } else {
                (0..data.shape[0])
                    .map(|i| subarray(&data, Value::Int(i as i64)))
                    .collect()
            }
        }
        Value::Set(set) => Ok(set.borrow().iter().map(|k| k.to_value()).collect()),
        Value::Dict(dict) => Ok(dict
            .borrow()
            .keys()
            .into_iter()
            .map(|k| k.to_value())
            .collect()),
        Value::Str(s) => Ok(s.chars().map(|c| Value::Str(c.to_string())).collect()),
        Value::Queue(queue) => Ok(queue.borrow().iter().cloned().collect()),
        Value::Stack(stack) => Ok(stack.borrow().clone()),
        other => Err(RuntimeError::new(format!(
            "{} is not iterable",
            other.type_name()
        ))),
    }
}

/// The interpreter. One instance keeps a global environment across
/// `run` calls, so a REPL can reuse it later.
///
/// `super_stack` tracks the class whose method is currently
/// executing (innermost first): `super.foo()` looks the method up
/// in that class's parent. It is empty outside methods.
///
/// `venv_dirs` are the `lib/` directories of the active virtual
/// environments, in search order. `cache` holds the modules already
/// loaded in this process (by canonical path: each module runs
/// once). `loading` is the stack of modules being loaded right now
/// (by canonical path): an import of one of them is a cycle.
/// `embedded_cache` and `loading_embedded` are the same pair for
/// the embedded stdlib modules (by name: they have no file), and
/// `native_cache` holds the native modules (`math`, `random`),
/// built once and shared by every import.
///
/// `gui_running` is true while a `gui.run()` window is open: a
/// frame function calling `gui.run()` again is an error (eframe
/// cannot nest event loops), and the flag is cleared when the
/// window closes, success or failure.
pub struct Interpreter {
    globals: Rc<Environment>,
    super_stack: Vec<Rc<ClassDef>>,
    venv_dirs: Vec<PathBuf>,
    loading: Vec<PathBuf>,
    cache: HashMap<PathBuf, Value>,
    embedded_cache: HashMap<String, Value>,
    loading_embedded: Vec<String>,
    native_cache: HashMap<String, Value>,
    #[cfg(feature = "gui")]
    pub(crate) gui_running: bool,
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
        Self {
            globals,
            super_stack: Vec::new(),
            venv_dirs: Vec::new(),
            loading: Vec::new(),
            cache: HashMap::new(),
            embedded_cache: HashMap::new(),
            loading_embedded: Vec::new(),
            native_cache: HashMap::new(),
            #[cfg(feature = "gui")]
            gui_running: false,
        }
    }

    /// The `lib/` directories `import` searches (after the
    /// importing file's directory, before the working directory),
    /// in order. Set by the `gg` binary from `--venv` flags, or
    /// from the `GG_VENV` environment variable when no flag is
    /// given.
    pub fn set_venv_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.venv_dirs = dirs;
    }

    /// The directory the main script lives in: `import` in the
    /// script (and in any function it defines) resolves against
    /// it. `None` means "no script" (the REPL): imports fall back
    /// to the venv libraries and the working directory.
    pub fn set_base_dir(&mut self, dir: Option<PathBuf>) {
        self.globals.set_base_dir(dir);
    }

    /// The string to show for a value: an instance whose class
    /// defines a `str` method uses it; anything else uses its
    /// Display form. Used by `print`, by the REPL echo, and by
    /// `str()` on instances (see `call_value`).
    pub fn display(
        &mut self,
        value: &Value,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<String, RuntimeError> {
        let custom = match value {
            Value::Instance(inst) => lookup_method(&inst.borrow().class, "str")
                .map(|(function, class)| (Rc::clone(inst), function, class)),
            _ => None,
        };
        if let Some((inst, function, class)) = custom {
            let bound = BoundMethod {
                name: "str".to_string(),
                instance: inst,
                function,
                class,
            };
            let result = self.call_method(&bound, vec![], input, out)?;
            let Value::Str(s) = result else {
                return Err(RuntimeError::new("str() method must return a Str"));
            };
            return Ok(s);
        }
        Ok(format!("{value}"))
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
            Flow::Return(_, span) => Err(RuntimeError::with_span("return outside function", span)),
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
                flow @ Flow::Return(..) => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    /// Execute one statement. Errors without a position yet are
    /// reported at the statement: the most precise position (an
    /// operator, an identifier) wins over this fallback, because
    /// `at` never overwrites a span that is already set.
    fn exec_stmt(
        &mut self,
        stmt: &Stmt,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Flow, RuntimeError> {
        let span = stmt.span();
        self.exec_stmt_inner(stmt, env, input, out)
            .map_err(|e| e.at(span))
    }

    /// Execute one statement (the body; see `exec_stmt`).
    fn exec_stmt_inner(
        &mut self,
        stmt: &Stmt,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Flow, RuntimeError> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let value = self.eval_expr(value, env, input, out)?;
                env.define(name, value);
                Ok(Flow::Normal)
            }
            Stmt::Assign { name, value, .. } => {
                let value = self.eval_expr(value, env, input, out)?;
                env.assign(name, value)?;
                Ok(Flow::Normal)
            }
            Stmt::IndexAssign { target, value, .. } => {
                // The right-hand side is evaluated first (like
                // Python), then the index assignment happens.
                let value = self.eval_expr(value, env, input, out)?;
                self.assign_index(target, value, env, input, out)?;
                Ok(Flow::Normal)
            }
            Stmt::FieldAssign { target, value, .. } => {
                let value = self.eval_expr(value, env, input, out)?;
                self.assign_field(target, value, env, input, out)?;
                Ok(Flow::Normal)
            }
            Stmt::Class {
                name,
                parent,
                methods,
                ..
            } => {
                let parent_class = match parent {
                    Some(parent_name) => {
                        let value = env.get(parent_name)?;
                        let Value::Class(class) = value else {
                            return Err(RuntimeError::new(format!(
                                "parent '{parent_name}' is not a class"
                            )));
                        };
                        Some(class)
                    }
                    None => None,
                };
                // Methods capture the environment where the class is
                // defined, so they see globals — and the class name
                // itself, for creating more instances inside a
                // method. A later method with the same name replaces
                // an earlier one (like Python).
                let mut def_methods: Vec<(String, Rc<Function>)> = Vec::new();
                for method in methods {
                    let function = Rc::new(Function {
                        params: method.params.clone(),
                        body: method.body.clone(),
                        env: Rc::clone(env),
                        // Methods get their `super` context from the
                        // call, not from where the class is defined.
                        super_class: None,
                    });
                    if let Some(slot) = def_methods.iter_mut().find(|(n, _)| *n == method.name) {
                        slot.1 = Rc::clone(&function);
                    } else {
                        def_methods.push((method.name.clone(), function));
                    }
                }
                let class = Rc::new(ClassDef {
                    name: name.clone(),
                    parent: parent_class,
                    methods: def_methods,
                });
                env.define(name, Value::Class(class));
                Ok(Flow::Normal)
            }
            Stmt::Import { name, alias, .. } => {
                let module = self.import_module(name, env, input, out)?;
                env.define(alias.as_deref().unwrap_or(name), module);
                Ok(Flow::Normal)
            }
            Stmt::Print { expr, .. } => {
                let value = self.eval_expr(expr, env, input, out)?;
                let text = self.display(&value, input, out)?;
                writeln!(out, "{text}")
                    .map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
                Ok(Flow::Normal)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
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
            Stmt::While { cond, body, .. } => loop {
                let cond = self.eval_expr(cond, env, input, out)?;
                if !cond.is_truthy() {
                    return Ok(Flow::Normal);
                }
                let child = Environment::child(env);
                match self.exec_block(body, &child, input, out)? {
                    Flow::Normal => {}
                    flow @ Flow::Return(..) => return Ok(flow),
                }
            },
            Stmt::For {
                var,
                from,
                to,
                body,
                ..
            } => {
                let from_value = self.eval_expr(from, env, input, out)?;
                let to_value = self.eval_expr(to, env, input, out)?;
                // A non-integer bound points at the bound itself.
                let Value::Int(from) = from_value else {
                    return Err(RuntimeError::with_span(
                        "for loop bounds must be integers",
                        from.span(),
                    ));
                };
                let Value::Int(to) = to_value else {
                    return Err(RuntimeError::with_span(
                        "for loop bounds must be integers",
                        to.span(),
                    ));
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
                        flow @ Flow::Return(..) => return Ok(flow),
                    }
                    i += if from <= to { 1 } else { -1 };
                }
                Ok(Flow::Normal)
            }
            Stmt::ForIn {
                var, iter, body, ..
            } => {
                // The iterable is evaluated once, then snapshotted:
                // the loop visits exactly those items, in order.
                let value = self.eval_expr(iter, env, input, out)?;
                let items = iterable_items(&value).map_err(|e| e.at(iter.span()))?;
                // Same scoping as the numeric for: the loop variable
                // lives in its own scope, visible only to the body.
                let loop_env = Environment::child(env);
                for item in items {
                    loop_env.define(var, item);
                    let child = Environment::child(&loop_env);
                    match self.exec_block(body, &child, input, out)? {
                        Flow::Normal => {}
                        flow @ Flow::Return(..) => return Ok(flow),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Fn {
                name, params, body, ..
            } => {
                // The function captures `env` itself, so it can see the
                // name being defined here: recursion works. It also
                // captures the class of the method it is defined in
                // (if any), so `super` inside it resolves lexically.
                let function = Function {
                    params: params.clone(),
                    body: body.clone(),
                    env: Rc::clone(env),
                    super_class: self.super_stack.last().cloned(),
                };
                env.define(name, Value::Function(Rc::new(function)));
                Ok(Flow::Normal)
            }
            Stmt::Return { expr, .. } => {
                let value = match expr {
                    Some(expr) => self.eval_expr(expr, env, input, out)?,
                    None => Value::Nil,
                };
                Ok(Flow::Return(value, stmt.span()))
            }
            Stmt::ExprStmt { expr, .. } => {
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
            Expr::Int { value, .. } => Ok(Value::Int(*value)),
            Expr::Float { value, .. } => Ok(Value::Float(*value)),
            Expr::Str { value, .. } => Ok(Value::Str(value.clone())),
            Expr::Bool { value, .. } => Ok(Value::Bool(*value)),
            Expr::Ident { name, span } => env.get(name).map_err(|e| e.at(*span)),
            Expr::List { elements, .. } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.eval_expr(element, env, input, out)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(values))))
            }
            Expr::Tuple { elements, .. } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.eval_expr(element, env, input, out)?);
                }
                Ok(Value::Tuple(Rc::new(values)))
            }
            Expr::Dict { entries, .. } => {
                let mut data = DictData::new();
                for (key_expr, value_expr) in entries {
                    let key = self.eval_expr(key_expr, env, input, out)?;
                    let Some(key) = key.to_key() else {
                        return Err(RuntimeError::new(format!(
                            "dict key must be Int, Float, Str or Bool, not {}",
                            key.type_name()
                        ))
                        .at(key_expr.span()));
                    };
                    let value = self.eval_expr(value_expr, env, input, out)?;
                    data.insert(key, value);
                }
                Ok(Value::Dict(Rc::new(RefCell::new(data))))
            }
            Expr::Set { elements, .. } => {
                let mut items = Vec::new();
                for element in elements {
                    let value = self.eval_expr(element, env, input, out)?;
                    let Some(key) = value.to_key() else {
                        return Err(RuntimeError::new(format!(
                            "set element must be Int, Float, Str or Bool, not {}",
                            value.type_name()
                        ))
                        .at(element.span()));
                    };
                    // Duplicates are ignored silently.
                    if !items.contains(&key) {
                        items.push(key);
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(items))))
            }
            Expr::Index {
                target,
                indices,
                span,
            } => {
                let target = self.eval_expr(target, env, input, out)?;
                let args = self.eval_indices(indices, env, input, out)?;
                index_value(&target, args, *span)
            }
            Expr::Slice { span, .. } => {
                Err(RuntimeError::new("a slice is only valid inside '[...]'").at(*span))
            }
            Expr::SelfExpr { span } => env.get("self").map_err(|e| e.at(*span)),
            Expr::Field { target, name, span } => {
                let obj = self.eval_expr(target, env, input, out)?;
                self.get_field(&obj, name).map_err(|e| e.at(*span))
            }
            Expr::MethodCall {
                target,
                name,
                args,
                span,
            } => {
                let obj = self.eval_expr(target, env, input, out)?;
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval_expr(arg, env, input, out)?);
                }
                self.call_method_on(&obj, name, values, input, out)
                    .map_err(|e| e.at(*span))
            }
            Expr::SuperCall { name, args, span } => {
                // `super` only means something inside a method: the
                // defining class is the top of the super stack.
                let Some(defining) = self.super_stack.last() else {
                    return Err(RuntimeError::new("'super' outside a method").at(*span));
                };
                let Some(parent) = &defining.parent else {
                    return Err(RuntimeError::new(format!(
                        "class '{}' has no parent (no super)",
                        defining.name
                    ))
                    .at(*span));
                };
                let (function, class) = lookup_method(parent, name).ok_or_else(|| {
                    RuntimeError::new(format!("parent class has no method '{name}'")).at(*span)
                })?;
                let Value::Instance(instance) = env.get("self").map_err(|e| e.at(*span))? else {
                    return Err(RuntimeError::new("'self' is not an instance").at(*span));
                };
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval_expr(arg, env, input, out)?);
                }
                let bound = BoundMethod {
                    name: name.to_string(),
                    instance,
                    function,
                    class,
                };
                self.call_method(&bound, values, input, out)
                    .map_err(|e| e.at(*span))
            }
            Expr::Unary { op, expr, span } => {
                let value = self.eval_expr(expr, env, input, out)?;
                let result = match op {
                    UnaryOp::Not => Ok(Value::Bool(!value.is_truthy())),
                    UnaryOp::Neg => match value {
                        Value::Int(n) => n
                            .checked_neg()
                            .map(Value::Int)
                            .ok_or_else(|| RuntimeError::new("integer overflow")),
                        Value::Float(x) => Ok(Value::Float(-x)),
                        Value::Array(array) => array::negate_array(&array.borrow()),
                        other => Err(RuntimeError::new(format!(
                            "cannot negate {}",
                            other.type_name()
                        ))),
                    },
                };
                result.map_err(|e| e.at(*span))
            }
            Expr::Binary { .. } => self.eval_binary(expr, env, input, out),
            Expr::Call { name, args, span } => {
                let callee = env.get(name).map_err(|e| e.at(*span))?;
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval_expr(arg, env, input, out)?);
                }
                self.call_value(&callee, values, input, out)
                    .map_err(|e| e.at(*span))
            }
        }
    }

    /// Evaluate the index expressions of an `Index` node: a plain
    /// expression becomes a value, a slice becomes its evaluated
    /// bounds (each must be an integer, or absent). Each argument
    /// keeps the span of the expression it came from, so an
    /// out-of-range index points at that expression.
    fn eval_indices(
        &mut self,
        indices: &[Expr],
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Vec<IndexArg>, RuntimeError> {
        let mut args = Vec::with_capacity(indices.len());
        for index in indices {
            match index {
                Expr::Slice {
                    start,
                    stop,
                    step,
                    span,
                } => {
                    let mut bounds = [None, None, None];
                    for (i, bound) in [start, stop, step].iter().enumerate() {
                        if let Some(expr) = bound {
                            let value = self.eval_expr(expr, env, input, out)?;
                            match value {
                                Value::Int(n) => bounds[i] = Some(n),
                                other => {
                                    return Err(RuntimeError::new(format!(
                                        "slice bounds must be integers, not {}",
                                        other.type_name()
                                    ))
                                    .at(expr.span()));
                                }
                            }
                        }
                    }
                    args.push(IndexArg::Slice(
                        SliceSpec {
                            start: bounds[0],
                            stop: bounds[1],
                            step: bounds[2],
                        },
                        *span,
                    ));
                }
                other => {
                    args.push(IndexArg::Value(
                        self.eval_expr(other, env, input, out)?,
                        other.span(),
                    ));
                }
            }
        }
        Ok(args)
    }

    /// Call any callable value with already-evaluated arguments:
    /// a user function, a builtin, a bound method, or a class
    /// (which creates an instance).
    /// Call any callable value (a GG function, a builtin, a bound
    /// method, a class): the single entry point the GUI module uses
    /// to run frame functions and layout callbacks.
    pub(crate) fn call_value(
        &mut self,
        callee: &Value,
        args: Vec<Value>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        match callee {
            Value::Function(function) => {
                if args.len() != function.params.len() {
                    return Err(RuntimeError::new(format!(
                        "expected {} argument(s) but got {}",
                        function.params.len(),
                        args.len()
                    )));
                }
                self.call(function, args, input, out)
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
                // `str` on an instance with a custom `str` method
                // uses it (see `display`); anything else goes to
                // the builtin itself.
                if builtin.name == "str" && matches!(args.first(), Some(Value::Instance(_))) {
                    return Ok(Value::Str(self.display(&args[0], input, out)?));
                }
                // Plain builtins are Rust functions; `WithInterp`
                // builtins (the `gui` module) get the interpreter
                // so they can call GG code back.
                match &builtin.func {
                    BuiltinFunc::Plain(func) => func(&args, input, out),
                    BuiltinFunc::WithInterp(func) => func(self, &args, input, out),
                }
            }
            Value::BoundMethod(bound) => self.call_method(bound, args, input, out),
            Value::Class(class) => self.construct(class, args, input, out),
            other => Err(RuntimeError::new(format!(
                "cannot call {}",
                other.type_name()
            ))),
        }
    }

    /// Read a field or method from a value. On an instance, fields
    /// win over methods; on a class, a method is returned unbound
    /// (call it with the instance as its first argument).
    fn get_field(&mut self, obj: &Value, name: &str) -> Result<Value, RuntimeError> {
        match obj {
            Value::Instance(inst) => {
                if let Some(value) = inst.borrow().get_field(name) {
                    return Ok(value.clone());
                }
                match lookup_method(&inst.borrow().class, name) {
                    Some((function, class)) => Ok(Value::BoundMethod(Rc::new(BoundMethod {
                        name: name.to_string(),
                        instance: Rc::clone(inst),
                        function,
                        class,
                    }))),
                    None => Err(RuntimeError::new(format!(
                        "instance of '{}' has no field or method '{}'",
                        inst.borrow().class.name,
                        name
                    ))),
                }
            }
            Value::Class(class) => match lookup_method(class, name) {
                Some((function, _)) => Ok(Value::Function(function)),
                None => Err(RuntimeError::new(format!(
                    "class '{}' has no method '{}'",
                    class.name, name
                ))),
            },
            Value::Module(module) => match module.env.get_local(name) {
                // Only the module's own top-level names: builtins
                // live in the parent (global) scope and are NOT
                // module attributes (`m.len` is an error).
                Some(value) => Ok(value),
                None => Err(RuntimeError::new(format!(
                    "module '{}' has no attribute '{}'",
                    module.name, name
                ))),
            },
            other => Err(RuntimeError::new(format!(
                "cannot access field '{}' of {}",
                name,
                other.type_name()
            ))),
        }
    }

    /// Call a method on a value: `obj.name(args)`. On an instance,
    /// a field holding a function wins over a method (fields are
    /// checked first, like for plain field access); on a class the
    /// method is unbound, so the caller passes `self` explicitly.
    fn call_method_on(
        &mut self,
        obj: &Value,
        name: &str,
        args: Vec<Value>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        match obj {
            Value::Instance(inst) => {
                if let Some(value) = inst.borrow().get_field(name) {
                    let value = value.clone();
                    return self.call_value(&value, args, input, out);
                }
                match lookup_method(&inst.borrow().class, name) {
                    Some((function, class)) => {
                        let bound = BoundMethod {
                            name: name.to_string(),
                            instance: Rc::clone(inst),
                            function,
                            class,
                        };
                        self.call_method(&bound, args, input, out)
                    }
                    None => Err(RuntimeError::new(format!(
                        "instance of '{}' has no method '{}'",
                        inst.borrow().class.name,
                        name
                    ))),
                }
            }
            Value::Class(class) => match lookup_method(class, name) {
                Some((function, defining)) => {
                    // Unbound: the caller passes `self` explicitly.
                    // When it is an instance, go through the method
                    // call so `super` works inside; otherwise call
                    // the function plainly and let it fail on `self`.
                    if let Some(Value::Instance(inst)) = args.first() {
                        let rest: Vec<Value> = args.iter().skip(1).cloned().collect();
                        let bound = BoundMethod {
                            name: name.to_string(),
                            instance: Rc::clone(inst),
                            function,
                            class: defining,
                        };
                        self.call_method(&bound, rest, input, out)
                    } else {
                        if args.len() != function.params.len() {
                            return Err(RuntimeError::new(format!(
                                "expected {} argument(s) but got {}",
                                function.params.len(),
                                args.len()
                            )));
                        }
                        self.call(&function, args, input, out)
                    }
                }
                None => Err(RuntimeError::new(format!(
                    "class '{}' has no method '{}'",
                    class.name, name
                ))),
            },
            // A module attribute that is callable: a function, a
            // builtin, or a class (calling it constructs an
            // instance). `m.len` is not an attribute (see
            // `get_field`), so this never sees a builtin through
            // the module.
            Value::Module(module) => match module.env.get_local(name) {
                Some(value) => self.call_value(&value, args, input, out),
                None => Err(RuntimeError::new(format!(
                    "module '{}' has no attribute '{}'",
                    module.name, name
                ))),
            },
            other => Err(RuntimeError::new(format!(
                "cannot call method '{}' on {}",
                name,
                other.type_name()
            ))),
        }
    }

    /// Call a method on an instance: bind the instance as `self`
    /// (the method's first parameter) and run the body with the
    /// defining class as the `super` context, so `super` works
    /// inside. The previous context is restored afterwards, so a
    /// plain function called from the method does not inherit it.
    fn call_method(
        &mut self,
        bound: &BoundMethod,
        args: Vec<Value>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let function = &bound.function;
        if args.len() + 1 != function.params.len() {
            return Err(RuntimeError::new(format!(
                "method '{}' expects {} argument(s) but got {}",
                bound.name,
                function.params.len() - 1,
                args.len()
            )));
        }
        let call_env = Environment::child(&function.env);
        call_env.define(
            &function.params[0],
            Value::Instance(Rc::clone(&bound.instance)),
        );
        for (param, value) in function.params.iter().skip(1).zip(args) {
            call_env.define(param, value);
        }
        let saved = std::mem::replace(&mut self.super_stack, vec![Rc::clone(&bound.class)]);
        let result = self.exec_block(&function.body, &call_env, input, out);
        self.super_stack = saved;
        match result? {
            Flow::Normal => Ok(Value::Nil),
            Flow::Return(value, _) => Ok(value),
        }
    }

    /// Create an instance of a class: run its `init` method (looked
    /// up the inheritance chain) with the given arguments, or fail
    /// when arguments are given but there is no `init`.
    fn construct(
        &mut self,
        class: &Rc<ClassDef>,
        args: Vec<Value>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let instance = Rc::new(RefCell::new(InstanceData::new(Rc::clone(class))));
        match lookup_method(class, "init") {
            Some((function, defining)) => {
                let bound = BoundMethod {
                    name: "init".to_string(),
                    instance: Rc::clone(&instance),
                    function,
                    class: defining,
                };
                self.call_method(&bound, args, input, out)?;
            }
            None => {
                if !args.is_empty() {
                    return Err(RuntimeError::new(format!(
                        "class '{}' takes no arguments (no init method)",
                        class.name
                    )));
                }
            }
        }
        Ok(Value::Instance(instance))
    }

    /// Load a module by name (`import math` looks for `math.gg`).
    ///
    /// Search order: (1) a `.gg` file — the directory of the file
    /// doing the import (the importing module's directory, or the
    /// main script's), then the `lib/` directories of the active
    /// venvs, in order, then the working directory; (2) an
    /// embedded stdlib module written in GG (`stats`); (3) a native
    /// module written in Rust (`math`, `random`). The first match
    /// wins, so a user file with the same name overrides a stdlib
    /// or native module.
    ///
    /// Each module runs exactly once per process: the second import
    /// returns the cached value. A module that (directly or
    /// indirectly) imports itself is a circular import.
    fn import_module(
        &mut self,
        name: &str,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let file_name = format!("{name}.gg");
        // Search order: importing file's directory, venv libs,
        // working directory. Duplicates (e.g. the script's
        // directory is the working directory) are tried once.
        let mut bases: Vec<PathBuf> = Vec::new();
        if let Some(base) = env.import_base() {
            bases.push(base);
        }
        bases.extend(self.venv_dirs.iter().cloned());
        if let Ok(cwd) = std::env::current_dir() {
            bases.push(cwd);
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        for base in bases {
            let candidate = base.join(&file_name);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        if let Some(path) = candidates.iter().find(|p| p.is_file()) {
            return self.import_file_module(name, path, input, out);
        }
        // No file: an embedded stdlib module, then a native one.
        if crate::stdlib::get(name).is_some() {
            return self.import_embedded_module(name, input, out);
        }
        if let Some(module) = self.native_module(name) {
            return Ok(module);
        }
        let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
        Err(RuntimeError::new(format!(
            "module '{name}' not found (searched: {})",
            tried.join(", ")
        )))
    }

    /// Load a module from a `.gg` file: run it once in its own
    /// environment (a child of the globals, so it sees the
    /// builtins) with its own directory as the import base.
    fn import_file_module(
        &mut self,
        name: &str,
        path: &PathBuf,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let canonical = std::fs::canonicalize(path).map_err(|e| {
            RuntimeError::new(format!(
                "module '{name}': cannot read {}: {e}",
                path.display()
            ))
        })?;
        // A module that is already being loaded (higher up the
        // import chain) is a cycle: name the whole chain.
        if self.loading.contains(&canonical) {
            let mut chain: Vec<String> = self
                .loading
                .iter()
                .map(|p| {
                    p.file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .collect();
            chain.push(name.to_string());
            return Err(RuntimeError::new(format!(
                "circular import: {}",
                chain.join(" -> ")
            )));
        }
        if let Some(module) = self.cache.get(&canonical) {
            return Ok(module.clone());
        }
        let source = std::fs::read_to_string(&canonical).map_err(|e| {
            RuntimeError::new(format!(
                "module '{name}': cannot read {}: {e}",
                canonical.display()
            ))
        })?;
        let module_env = Environment::child(&self.globals);
        module_env.set_base_dir(canonical.parent().map(|p| p.to_path_buf()));
        // Mark the module as loading while it runs, so an import
        // of itself (directly or through another module) is a
        // cycle; pop it again afterwards, success or failure.
        self.loading.push(canonical.clone());
        let result = self.run_module_source(name, &source, &module_env, input, out);
        self.loading.pop();
        let module = result?;
        self.cache.insert(canonical, module.clone());
        Ok(module)
    }

    /// Load an embedded stdlib module (GG source compiled into the
    /// binary). It runs in its own environment with no base
    /// directory — it has no file — so its own imports resolve
    /// through the venv libraries, the working directory, and then
    /// the stdlib and native modules again. Cached by name, with
    /// the same cycle detection as file modules.
    fn import_embedded_module(
        &mut self,
        name: &str,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        if self.loading_embedded.contains(&name.to_string()) {
            let mut chain = self.loading_embedded.clone();
            chain.push(name.to_string());
            return Err(RuntimeError::new(format!(
                "circular import: {}",
                chain.join(" -> ")
            )));
        }
        if let Some(module) = self.embedded_cache.get(name) {
            return Ok(module.clone());
        }
        // `import_module` checked that the source exists.
        let source = crate::stdlib::get(name).unwrap();
        let module_env = Environment::child(&self.globals);
        self.loading_embedded.push(name.to_string());
        let result = self.run_module_source(name, source, &module_env, input, out);
        self.loading_embedded.pop();
        let module = result?;
        self.embedded_cache.insert(name.to_string(), module.clone());
        Ok(module)
    }

    /// Run a module's source in `module_env` and wrap the result
    /// as a module value. Lex/parse/runtime errors are prefixed
    /// with the module name; a `return` at the top level is an
    /// error.
    fn run_module_source(
        &mut self,
        name: &str,
        source: &str,
        module_env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let tokens = gg_lexer::lex(source)
            .map_err(|e| RuntimeError::new(format!("module '{name}': lex error: {e}")))?;
        let program = gg_parser::parse(&tokens)
            .map_err(|e| RuntimeError::new(format!("module '{name}': parse error: {e}")))?;
        match self.exec_block(&program.stmts, module_env, input, out) {
            Ok(Flow::Normal) => {}
            Ok(Flow::Return(_, span)) => {
                return Err(RuntimeError::with_span(
                    format!("module '{name}': return outside function"),
                    span,
                ));
            }
            Err(e) => {
                return Err(RuntimeError::new(format!("module '{name}': {e}")));
            }
        }
        Ok(Value::Module(Rc::new(ModuleData {
            name: name.to_string(),
            env: Rc::clone(module_env),
        })))
    }

    /// A native module (`math`, `random`), built once and cached,
    /// or `None` when GG has no native module with that name.
    fn native_module(&mut self, name: &str) -> Option<Value> {
        if let Some(module) = self.native_cache.get(name) {
            return Some(module.clone());
        }
        let module = crate::native::build(name)?;
        self.native_cache.insert(name.to_string(), module.clone());
        Some(module)
    }

    /// Assign to a field: the target is a `Field` expression
    /// (`p.nome`, `self.eta`) and the object must be an instance.
    /// The field is created when it does not exist yet.
    fn assign_field(
        &mut self,
        target: &Expr,
        value: Value,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<(), RuntimeError> {
        let Expr::Field { target, name, .. } = target else {
            return Err(RuntimeError::new("invalid assignment target"));
        };
        let obj = self.eval_expr(target, env, input, out)?;
        match obj {
            Value::Instance(inst) => {
                inst.borrow_mut().set_field(name, value);
                Ok(())
            }
            other => Err(RuntimeError::new(format!(
                "cannot set field '{}' on {}",
                name,
                other.type_name()
            ))),
        }
    }

    /// Evaluate a binary operator. `and`/`or` short-circuit and return
    /// operand values instead of booleans. Arithmetic and comparison
    /// errors point at the operator's span.
    fn eval_binary(
        &mut self,
        binary: &Expr,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let Expr::Binary {
            op,
            left,
            right,
            span,
        } = binary
        else {
            unreachable!("eval_binary is only called with a Binary node");
        };
        let op = *op;
        let span = *span;
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
                // Comparisons with an array are element-wise
                // (Phase 12): they return a Bool array, never a
                // single Bool. `==`/`!=` against a List or a Dict
                // still falls back to whole-value equality.
                if matches!(left, Value::Array(_)) || matches!(right, Value::Array(_)) {
                    let op = match op {
                        BinaryOp::Eq => Some(CompareOp::Eq),
                        BinaryOp::NotEq => Some(CompareOp::NotEq),
                        BinaryOp::Lt => Some(CompareOp::Lt),
                        BinaryOp::LtEq => Some(CompareOp::LtEq),
                        BinaryOp::Gt => Some(CompareOp::Gt),
                        BinaryOp::GtEq => Some(CompareOp::GtEq),
                        _ => None,
                    };
                    if let Some(op) = op {
                        return array::compare_arrays(op, &left, &right).map_err(|e| e.at(span));
                    }
                }
                let result = match op {
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
                };
                result.map_err(|e| e.at(span))
            }
        }
    }

    /// Assign through an index: the target must be an `Index`
    /// expression (`xs[0]`, `m[0][1]`, `d["k"]`, `a[0, 1]`,
    /// `xs[1:3]`). The container is evaluated first, then the
    /// index arguments, and the element is replaced — or inserted,
    /// for dicts. An array takes one index per dimension, or a
    /// single index when 1D. A slice assigns a whole span at once
    /// (lists and 1D arrays only, same length required).
    ///
    /// Errors about one index (out of range, step zero) point at
    /// that index expression; errors about the assignment as a
    /// whole point at the `Index` node (the `[`).
    fn assign_index(
        &mut self,
        target: &Expr,
        value: Value,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<(), RuntimeError> {
        let node_span = target.span();
        let Expr::Index {
            target, indices, ..
        } = target
        else {
            return Err(RuntimeError::new("invalid assignment target").at(node_span));
        };
        let container = self.eval_expr(target, env, input, out)?;
        let args = self.eval_indices(indices, env, input, out)?;
        match container {
            Value::List(list) => {
                let arg = single_arg(args, "list").map_err(|e| e.at(node_span))?;
                match arg {
                    IndexArg::Value(index, span) => {
                        let pos = resolve_index(list.borrow().len(), index, "list")
                            .map_err(|e| e.at(span))?;
                        list.borrow_mut()[pos] = value;
                        Ok(())
                    }
                    IndexArg::Slice(spec, span) => {
                        let Value::List(new_items) = value else {
                            return Err(RuntimeError::new(format!(
                                "slice assignment needs a List, not {}",
                                value.type_name()
                            ))
                            .at(span));
                        };
                        let positions = crate::slice::slice_positions(list.borrow().len(), &spec)
                            .map_err(|e| e.at(span))?;
                        let new_items = new_items.borrow();
                        if new_items.len() != positions.len() {
                            return Err(RuntimeError::new(format!(
                                "slice assignment: the slice has {} element(s) but the new list has {}",
                                positions.len(),
                                new_items.len()
                            ))
                            .at(span));
                        }
                        let mut borrowed = list.borrow_mut();
                        for (pos, item) in positions.iter().zip(new_items.iter()) {
                            borrowed[*pos] = item.clone();
                        }
                        Ok(())
                    }
                }
            }
            Value::Dict(dict) => {
                let arg = single_arg(args, "dict").map_err(|e| e.at(node_span))?;
                let (index, span) = match arg {
                    IndexArg::Value(index, span) => (index, span),
                    IndexArg::Slice(_, span) => {
                        return Err(RuntimeError::with_span("cannot slice a dict", span));
                    }
                };
                let Some(key) = index.to_key() else {
                    return Err(RuntimeError::new(format!(
                        "dict key must be Int, Float, Str or Bool, not {}",
                        index.type_name()
                    ))
                    .at(span));
                };
                dict.borrow_mut().insert(key, value);
                Ok(())
            }
            Value::Array(array) => {
                // A slice anywhere in the index means slice
                // assignment: 1D arrays only, same length required.
                if args.iter().any(|a| matches!(a, IndexArg::Slice(..))) {
                    if args.len() != 1 || array.borrow().ndim() != 1 {
                        return Err(RuntimeError::with_span(
                            "slice assignment works on 1D arrays only",
                            node_span,
                        ));
                    }
                    let IndexArg::Slice(spec, span) = args.into_iter().next().unwrap() else {
                        unreachable!()
                    };
                    let Value::Array(new_arr) = value else {
                        return Err(RuntimeError::new(format!(
                            "slice assignment on an array needs an Array, not {}",
                            value.type_name()
                        ))
                        .at(span));
                    };
                    let wanted = array.borrow().elem;
                    let new_arr = new_arr.borrow();
                    if new_arr.elem != wanted {
                        return Err(RuntimeError::new(format!(
                            "array element must be {}, not Array of {}",
                            wanted.name(),
                            new_arr.elem.name()
                        ))
                        .at(span));
                    }
                    let positions = crate::slice::slice_positions(array.borrow().size(), &spec)
                        .map_err(|e| e.at(span))?;
                    if new_arr.items.len() != positions.len() {
                        return Err(RuntimeError::new(format!(
                            "slice assignment: the slice has {} element(s) but the new array has {}",
                            positions.len(),
                            new_arr.items.len()
                        ))
                        .at(span));
                    }
                    let mut borrowed = array.borrow_mut();
                    for (pos, item) in positions.iter().zip(new_arr.items.iter()) {
                        borrowed.items[*pos] = item.clone();
                    }
                    return Ok(());
                }
                // A boolean mask cannot be assigned through.
                if args.len() == 1
                    && let IndexArg::Value(Value::Array(mask), span) = &args[0]
                    && mask.borrow().elem == crate::value::ElemType::Bool
                {
                    return Err(RuntimeError::with_span(
                        "cannot assign through a boolean mask (not supported yet)",
                        *span,
                    ));
                }
                let indices: Vec<(Value, Span)> = args
                    .into_iter()
                    .map(|arg| match arg {
                        IndexArg::Value(value, span) => (value, span),
                        IndexArg::Slice(..) => unreachable!(),
                    })
                    .collect();
                let mut array = array.borrow_mut();
                let pos = if indices.len() == 1 && array.ndim() == 1 {
                    let (index, span) = indices.into_iter().next().unwrap();
                    resolve_index(array.items.len(), index, "array").map_err(|e| e.at(span))?
                } else {
                    flat_index(&array.shape, &indices).map_err(|e| e.at(node_span))?
                };
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
            Value::Tuple(_) => Err(RuntimeError::with_span(
                "cannot assign to tuple (tuples are immutable)",
                node_span,
            )),
            Value::Set(_) => Err(RuntimeError::with_span(
                "cannot assign to set (sets have no indexes)",
                node_span,
            )),
            other => Err(RuntimeError::with_span(
                format!("cannot index {}", other.type_name()),
                node_span,
            )),
        }
    }

    /// Call a user function: fresh scope under the captured environment,
    /// parameters bound, `return` caught as the result. The `super`
    /// context is the class the function was defined in (if any): a
    /// plain function called from a method does not inherit the
    /// method's `super`.
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
        let saved = std::mem::replace(
            &mut self.super_stack,
            match &function.super_class {
                Some(class) => vec![Rc::clone(class)],
                None => Vec::new(),
            },
        );
        let result = self.exec_block(&function.body, &call_env, input, out);
        self.super_stack = saved;
        match result? {
            Flow::Normal => Ok(Value::Nil),
            Flow::Return(value, _) => Ok(value),
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

/// `xs[i]` / `xs[1:3]` / `t[i]` / `t[1:3]` / `s[1:3]` / `a[i]` /
/// `a[i, j]` / `a[0:2, 1:3]` / `a[:, 1]` / `a[mask]` / `d[k]`:
/// read one element, or a slice (always a copy). Lists, tuples
/// and dicts take exactly one argument; strings take a slice only
/// (a single index on a string is an error). Arrays take one
/// selector per axis — missing axes are full slices — or a single
/// Bool array as a mask, which selects the true elements,
/// flattened to 1D.
///
/// `node_span` is the span of the `Index` node (the `[`): errors
/// about the indexing as a whole (wrong number of indices, cannot
/// index this type) point at it; errors about one index (out of
/// range, wrong key, step zero) point at that index expression,
/// which each `IndexArg` carries.
fn index_value(
    target: &Value,
    args: Vec<IndexArg>,
    node_span: Span,
) -> Result<Value, RuntimeError> {
    match target {
        Value::List(list) => {
            let arg = single_arg(args, "list").map_err(|e| e.at(node_span))?;
            match arg {
                IndexArg::Value(index, span) => {
                    let borrowed = list.borrow();
                    let pos =
                        resolve_index(borrowed.len(), index, "list").map_err(|e| e.at(span))?;
                    Ok(borrowed[pos].clone())
                }
                IndexArg::Slice(spec, span) => {
                    let items = crate::slice::slice_values(&list.borrow(), &spec)
                        .map_err(|e| e.at(span))?;
                    Ok(Value::List(Rc::new(RefCell::new(items))))
                }
            }
        }
        Value::Tuple(items) => {
            let arg = single_arg(args, "tuple").map_err(|e| e.at(node_span))?;
            match arg {
                IndexArg::Value(index, span) => {
                    let pos = resolve_index(items.len(), index, "tuple").map_err(|e| e.at(span))?;
                    Ok(items[pos].clone())
                }
                IndexArg::Slice(spec, span) => {
                    let items = crate::slice::slice_values(items, &spec).map_err(|e| e.at(span))?;
                    Ok(Value::Tuple(Rc::new(items)))
                }
            }
        }
        Value::Str(s) => {
            let arg = single_arg(args, "string").map_err(|e| e.at(node_span))?;
            match arg {
                IndexArg::Value(_, span) => Err(RuntimeError::new(
                    "cannot index a string with an integer (use a slice, e.g. s[1:3])",
                )
                .at(span)),
                IndexArg::Slice(spec, span) => Ok(Value::Str(
                    crate::slice::slice_str(s, &spec).map_err(|e| e.at(span))?,
                )),
            }
        }
        Value::Array(array) => {
            let borrowed = array.borrow();
            // Boolean indexing: a single Bool array mask.
            if args.len() == 1
                && let IndexArg::Value(Value::Array(mask), span) = &args[0]
                && mask.borrow().elem == crate::value::ElemType::Bool
            {
                return crate::slice::mask_select(&borrowed, &mask.borrow())
                    .map_err(|e| e.at(*span));
            }
            // One selector per axis; missing axes are full slices.
            if args.len() > borrowed.ndim() {
                return Err(RuntimeError::with_span(
                    format!(
                        "array has {} dimensions but got {} indices",
                        borrowed.ndim(),
                        args.len()
                    ),
                    node_span,
                ));
            }
            let mut selectors: Vec<AxisSelector> = args
                .iter()
                .map(|arg| match arg {
                    IndexArg::Value(value, span) => AxisSelector::Index(value.clone(), *span),
                    IndexArg::Slice(spec, span) => AxisSelector::Slice(spec.clone(), *span),
                })
                .collect();
            while selectors.len() < borrowed.ndim() {
                selectors.push(AxisSelector::Slice(SliceSpec::full(), node_span));
            }
            if selectors
                .iter()
                .all(|s| matches!(s, AxisSelector::Index(..)))
            {
                // All indexes: the Phase 11 behavior — one index
                // is an element for 1D arrays and a row for 2D
                // arrays (and a sub-array for 3D+), several are one
                // element per dimension.
                let indices: Vec<(Value, Span)> = selectors
                    .into_iter()
                    .map(|s| match s {
                        AxisSelector::Index(value, span) => (value, span),
                        AxisSelector::Slice(..) => unreachable!(),
                    })
                    .collect();
                if indices.len() == 1 {
                    let (index, span) = indices.into_iter().next().unwrap();
                    if borrowed.ndim() == 1 {
                        let pos = resolve_index(borrowed.items.len(), index, "array")
                            .map_err(|e| e.at(span))?;
                        Ok(borrowed.items[pos].clone())
                    } else {
                        subarray(&borrowed, index).map_err(|e| e.at(span))
                    }
                } else {
                    let pos = flat_index(&borrowed.shape, &indices).map_err(|e| e.at(node_span))?;
                    Ok(borrowed.items[pos].clone())
                }
            } else {
                crate::slice::slice_array(&borrowed, &selectors)
            }
        }
        Value::Dict(dict) => {
            let arg = single_arg(args, "dict").map_err(|e| e.at(node_span))?;
            let IndexArg::Value(index, span) = arg else {
                return Err(RuntimeError::with_span("cannot slice a dict", node_span));
            };
            let Some(key) = index.to_key() else {
                return Err(RuntimeError::new(format!(
                    "dict key must be Int, Float, Str or Bool, not {}",
                    index.type_name()
                ))
                .at(span));
            };
            match dict.borrow().get(&key) {
                Some(value) => Ok(value.clone()),
                None => Err(RuntimeError::with_span("key not found", span)),
            }
        }
        other => Err(RuntimeError::with_span(
            format!("cannot index {}", other.type_name()),
            node_span,
        )),
    }
}

/// Exactly one index argument, or an error: lists, tuples,
/// strings and dicts take a single index.
fn single_arg(args: Vec<IndexArg>, kind: &str) -> Result<IndexArg, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(format!(
            "{kind} takes one index, not {}",
            args.len()
        )));
    }
    Ok(args.into_iter().next().unwrap())
}

/// `+`: integer/float arithmetic, string concatenation, or
/// element-wise array arithmetic (array + array, array + scalar,
/// scalar + array).
fn add_values(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    if matches!(left, Value::Array(_)) || matches!(right, Value::Array(_)) {
        return array::elementwise(ArrayOp::Add, left, right);
    }
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
/// float). Integer overflow is an error, not a panic. Arrays work
/// element by element (array op array, array op scalar, scalar op
/// array).
fn numeric_op(
    left: &Value,
    right: &Value,
    verb: &str,
    int_op: impl Fn(i64, i64) -> Option<i64>,
    float_op: impl Fn(f64, f64) -> f64,
) -> Result<Value, RuntimeError> {
    if matches!(left, Value::Array(_)) || matches!(right, Value::Array(_)) {
        let op = if verb == "subtract" {
            ArrayOp::Sub
        } else {
            ArrayOp::Mul
        };
        return array::elementwise(op, left, right);
    }
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

/// `/`: always float division, even between integers. Arrays work
/// element by element (array / array, array / scalar, scalar /
/// array).
fn divide_values(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    if matches!(left, Value::Array(_)) || matches!(right, Value::Array(_)) {
        return array::elementwise(ArrayOp::Div, left, right);
    }
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
