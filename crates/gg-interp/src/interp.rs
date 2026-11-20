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

use gg_parser::ast::{BinaryOp, Expr, Program, Stmt, UnaryOp};

use crate::array::{self, ArrayOp, flat_index, subarray};
use crate::builtin;
use crate::env::Environment;
use crate::error::RuntimeError;
use crate::value::{
    BoundMethod, ClassDef, DictData, Function, InstanceData, ModuleData, Value, elem_type_of,
    lookup_method, values_equal,
};

/// What executing a statement produced: either control flows on
/// normally, or a `return` was hit with its value.
enum Flow {
    Normal,
    Return(Value),
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
pub struct Interpreter {
    globals: Rc<Environment>,
    super_stack: Vec<Rc<ClassDef>>,
    venv_dirs: Vec<PathBuf>,
    loading: Vec<PathBuf>,
    cache: HashMap<PathBuf, Value>,
    embedded_cache: HashMap<String, Value>,
    loading_embedded: Vec<String>,
    native_cache: HashMap<String, Value>,
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
            Stmt::FieldAssign { target, value } => {
                let value = self.eval_expr(value, env, input, out)?;
                self.assign_field(target, value, env, input, out)?;
                Ok(Flow::Normal)
            }
            Stmt::Class {
                name,
                parent,
                methods,
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
            Stmt::Import { name, alias } => {
                let module = self.import_module(name, env, input, out)?;
                env.define(alias.as_deref().unwrap_or(name), module);
                Ok(Flow::Normal)
            }
            Stmt::Print { expr } => {
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
            Expr::Index { target, indices } => {
                let target = self.eval_expr(target, env, input, out)?;
                let mut values = Vec::with_capacity(indices.len());
                for index in indices {
                    values.push(self.eval_expr(index, env, input, out)?);
                }
                index_value(&target, values)
            }
            Expr::SelfExpr => env.get("self"),
            Expr::Field { target, name } => {
                let obj = self.eval_expr(target, env, input, out)?;
                self.get_field(&obj, name)
            }
            Expr::MethodCall { target, name, args } => {
                let obj = self.eval_expr(target, env, input, out)?;
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval_expr(arg, env, input, out)?);
                }
                self.call_method_on(&obj, name, values, input, out)
            }
            Expr::SuperCall { name, args } => {
                // `super` only means something inside a method: the
                // defining class is the top of the super stack.
                let Some(defining) = self.super_stack.last() else {
                    return Err(RuntimeError::new("'super' outside a method"));
                };
                let Some(parent) = &defining.parent else {
                    return Err(RuntimeError::new(format!(
                        "class '{}' has no parent (no super)",
                        defining.name
                    )));
                };
                let (function, class) = lookup_method(parent, name).ok_or_else(|| {
                    RuntimeError::new(format!("parent class has no method '{name}'"))
                })?;
                let Value::Instance(instance) = env.get("self")? else {
                    return Err(RuntimeError::new("'self' is not an instance"));
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
                        Value::Array(array) => array::negate_array(&array.borrow()),
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
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval_expr(arg, env, input, out)?);
                }
                self.call_value(&callee, values, input, out)
            }
        }
    }

    /// Call any callable value with already-evaluated arguments:
    /// a user function, a builtin, a bound method, or a class
    /// (which creates an instance).
    fn call_value(
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
                (builtin.func)(&args, input, out)
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
            Flow::Return(value) => Ok(value),
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
            Ok(Flow::Return(_)) => {
                return Err(RuntimeError::new(format!(
                    "module '{name}': return outside function"
                )));
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
        let Expr::Field { target, name } = target else {
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
    /// expression (`xs[0]`, `m[0][1]`, `d["k"]`, `a[0, 1]`). The
    /// container is evaluated first, then the final index, and the
    /// element is replaced — or inserted, for dicts. An array takes
    /// one index per dimension, or a single index when 1D.
    fn assign_index(
        &mut self,
        target: &Expr,
        value: Value,
        env: &Rc<Environment>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<(), RuntimeError> {
        let Expr::Index { target, indices } = target else {
            return Err(RuntimeError::new("invalid assignment target"));
        };
        let container = self.eval_expr(target, env, input, out)?;
        let mut values = Vec::with_capacity(indices.len());
        for index in indices {
            values.push(self.eval_expr(index, env, input, out)?);
        }
        match container {
            Value::List(list) => {
                let index = single_index(values, "list")?;
                let pos = resolve_index(list.borrow().len(), index, "list")?;
                list.borrow_mut()[pos] = value;
                Ok(())
            }
            Value::Dict(dict) => {
                let index = single_index(values, "dict")?;
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
                let pos = if values.len() == 1 && array.ndim() == 1 {
                    resolve_index(
                        array.items.len(),
                        values.into_iter().next().unwrap(),
                        "array",
                    )?
                } else {
                    flat_index(&array.shape, &values)?
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

/// `xs[i]` / `t[i]` / `a[i]` / `d[k]` / `a[i, j]`: read one
/// element. Lists, tuples and dicts take exactly one index (an
/// integer, negative from the end; a hashable key for dicts, failing
/// on a missing key). Arrays take one index per dimension — or a
/// single index, which is a flat element for 1D arrays and a row
/// (a 1D array) for 2D arrays.
fn index_value(target: &Value, indices: Vec<Value>) -> Result<Value, RuntimeError> {
    match target {
        Value::List(list) => {
            let index = single_index(indices, "list")?;
            let borrowed = list.borrow();
            let pos = resolve_index(borrowed.len(), index, "list")?;
            Ok(borrowed[pos].clone())
        }
        Value::Tuple(items) => {
            let index = single_index(indices, "tuple")?;
            let pos = resolve_index(items.len(), index, "tuple")?;
            Ok(items[pos].clone())
        }
        Value::Array(array) => {
            let borrowed = array.borrow();
            if indices.len() == 1 {
                if borrowed.ndim() == 1 {
                    let pos = resolve_index(
                        borrowed.items.len(),
                        indices.into_iter().next().unwrap(),
                        "array",
                    )?;
                    Ok(borrowed.items[pos].clone())
                } else {
                    let row = subarray(&borrowed, indices.into_iter().next().unwrap())?;
                    Ok(row)
                }
            } else {
                let pos = flat_index(&borrowed.shape, &indices)?;
                Ok(borrowed.items[pos].clone())
            }
        }
        Value::Dict(dict) => {
            let index = single_index(indices, "dict")?;
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

/// Exactly one index, or an error: lists, tuples and dicts take a
/// single index.
fn single_index(indices: Vec<Value>, kind: &str) -> Result<Value, RuntimeError> {
    if indices.len() != 1 {
        return Err(RuntimeError::new(format!(
            "{kind} takes one index, not {}",
            indices.len()
        )));
    }
    Ok(indices.into_iter().next().unwrap())
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
