// SPDX-License-Identifier: GPL-3.0-or-later
//! The GG bytecode compiler: AST -> `CodeObject`.
//!
//! One `Compiler` compiles one unit (a script, a module, a
//! function, a method, a lambda, or a single expression). Nested
//! functions are compiled eagerly, at the same time as the
//! enclosing unit, and travel in `MakeFunction` / `MakeClass`
//! operands — the CPython model: a function's code object exists
//! before the function value does, and default expressions are
//! compiled to tiny code objects that run at definition time.
//!
//! Variable access is resolved at compile time. A name bound in
//! the current function becomes a frame slot (`LoadSlot` /
//! `StoreSlot` / `DefineSlot`): parameters first, in order, then
//! the other locals, one slot per declaration site. A name bound
//! in an enclosing function becomes an upvalue: a cell shared
//! between the defining frame and the nested one, so a closure
//! sees later rebindings and two closures share one binding.
//! A name bound further up the chain is captured transitively:
//! each intermediate function gains an upvalue for it (recorded
//! in the nested function's `pending` list and provided by the
//! parent right after the nested function is compiled), so a
//! grandchild reads the same cell as its grandparent, with no
//! copies at any level. A name bound nowhere in any enclosing
//! function is a global and stays name-based (`LoadName` /
//! `StoreName` / `DefineName`), which is also how module
//! attributes and the REPL's persistent top level work: only
//! the outermost scope of a script (or a function) uses slots,
//! and only when the unit opts in — a module top level and a
//! REPL chunk keep their names in the environment, so `math.pi`
//! and the next line still find them. Blocks open a
//! compile-time scope; entering one at runtime (`EnterScope`)
//! drops the cells of its slots, so a variable captured there
//! gets a fresh cell per entry (a loop body re-entered, a `try`
//! region re-run). The loop variable itself lives in the
//! loop's scope, entered once per loop: closures over it share
//! one binding and see its last value.

use std::rc::Rc;

use gg_lexer::Span;
use gg_parser::ast::{Arg, BinaryOp, Expr, FStringPart, ParamDef, Pattern, Program, Stmt, UnaryOp};

use crate::interp::body_contains_yield;
use crate::vm::chunk::{
    CodeObject, CompKind, Constant, Instruction, Op, Upvalue, UpvalueKind, VarRef,
};

/// Compile a whole program (a script or a module) to a code
/// object. Falling off the end is a normal exit: the VM loop
/// returns `Normal` when the instructions run out. `top_slots`
/// decides whether the outermost scope uses slots (a script) or
/// keeps its names in the environment (a module, a REPL chunk).
pub(crate) fn compile_program(program: &Program, top_slots: bool) -> Rc<CodeObject> {
    let mut compiler = Compiler::new(
        "<script>",
        Vec::new(),
        None,
        program.stmts.clone(),
        top_slots,
        None,
    );
    compiler.compile_stmts(&program.stmts);
    // A root compiler has no enclosing function, so it records
    // no pending upvalues.
    compiler.finish(false).0
}

/// Compile one expression as a code object that evaluates it and
/// returns the value (what `eval` runs). Names in it are globals.
pub(crate) fn compile_expr_code(expr: &Expr) -> Rc<CodeObject> {
    let mut compiler = Compiler::new(
        "<expr>",
        Vec::new(),
        None,
        vec![Stmt::Return {
            expr: Some(expr.clone()),
            span: expr.span(),
        }],
        false,
        None,
    );
    compiler.compile_expr(expr);
    compiler.emit(Op::Return, expr.span());
    compiler.finish(false).0
}

/// Compile a function or method body to a code object. The body
/// ends with an implicit `return nil`, like the tree-walker.
/// (A method of `Compiler`: nested functions need the enclosing
/// compiler's scopes to resolve their free variables.)
impl Compiler {
    pub(crate) fn compile_function(
        &mut self,
        name: &str,
        params: &[ParamDef],
        rest: Option<&str>,
        body: &[Stmt],
    ) -> Rc<CodeObject> {
        // Defaults are compiled here, in the enclosing context,
        // and run at definition time by `MakeFunction`.
        let mut defaults = Vec::with_capacity(params.len());
        for param in params {
            defaults.push(
                param
                    .default
                    .as_ref()
                    .map(|expr| self.compile_default_expr(expr)),
            );
        }
        let mut child = Compiler::new(
            name,
            params.iter().map(|p| p.name.clone()).collect(),
            rest.map(|r| r.to_string()),
            body.to_vec(),
            true,
            Some(self.snapshot_enclosing()),
        );
        child.defaults = defaults;
        child.compile_stmts(body);
        let nil_idx = child.add_const(Constant::Nil);
        child.emit(Op::LoadConst { idx: nil_idx }, Span::dummy());
        child.emit(Op::Return, Span::dummy());
        let (code, pending) = child.finish(body_contains_yield(body));
        // Provide the upvalues the nested function asked for:
        // each is appended to this function's upvalues, in the
        // order the child recorded them, so the indexes the
        // child computed (the parent's upvalue count at the
        // snapshot, plus the position in the list) line up.
        for name in pending {
            self.provide_upvalue(&name);
        }
        code
    }

    /// Compile a lambda: the body is a single expression, desugared
    /// to a `return` of it — exactly like the tree-walker's lambda.
    pub(crate) fn compile_lambda(
        &mut self,
        params: &[ParamDef],
        rest: Option<&str>,
        body: &Expr,
    ) -> Rc<CodeObject> {
        self.compile_function(
            "<lambda>",
            params,
            rest,
            &[Stmt::Return {
                expr: Some(body.clone()),
                span: body.span(),
            }],
        )
    }

    /// Compile one default expression as a code object that
    /// evaluates it and returns the value. Free variables resolve
    /// against the enclosing function (its slots become upvalues
    /// of the default), so a default can read a local of the
    /// function it belongs to — evaluated at definition time, in
    /// the defining frame.
    fn compile_default_expr(&mut self, expr: &Expr) -> Rc<CodeObject> {
        let mut child = Compiler::new(
            "<expr>",
            Vec::new(),
            None,
            vec![Stmt::Return {
                expr: Some(expr.clone()),
                span: expr.span(),
            }],
            true,
            Some(self.snapshot_enclosing()),
        );
        child.compile_expr(expr);
        child.emit(Op::Return, expr.span());
        let (code, pending) = child.finish(false);
        for name in pending {
            self.provide_upvalue(&name);
        }
        code
    }
}

/// Compile the body of a function value that has no compiled
/// code (defined by the tree-walker): the parameters and body
/// come from the value, and the defaults are already evaluated
/// (they live in the value), so the code object carries none —
/// `call_vm` binds from the value, never from the code object.
/// Free variables stay globals: the defining environment is on
/// the frame's chain, so the tree-walker's names still resolve.
pub(crate) fn compile_function_body(function: &crate::value::Function) -> Rc<CodeObject> {
    let mut compiler = Compiler::new(
        "<fn>",
        function.params.clone(),
        function.rest.clone(),
        function.body.clone(),
        true,
        None,
    );
    compiler.defaults = vec![None; function.params.len()];
    compiler.compile_stmts(&function.body);
    let nil_idx = compiler.add_const(Constant::Nil);
    compiler.emit(Op::LoadConst { idx: nil_idx }, Span::dummy());
    compiler.emit(Op::Return, Span::dummy());
    compiler.finish(body_contains_yield(&function.body)).0
}

/// The pieces of one comprehension expression, passed to
/// `Compiler::compile_comp` as a single argument.
struct CompSpec<'a> {
    kind: CompKind,
    elem: &'a Expr,
    key: Option<&'a Expr>,
    vars: &'a [String],
    iter: &'a Expr,
    cond: &'a Option<Box<Expr>>,
    span: Span,
}

/// A compile-time scope: the names declared in it, each with
/// the frame slot it got (or none, when this scope keeps its
/// names in the environment). `slot_based` is true for every
/// scope except the outermost one of a module or a REPL chunk,
/// whose names are the module attributes / the persistent state.
#[derive(Clone)]
struct Scope {
    locals: Vec<Local>,
    slot_based: bool,
}

/// One declared local: the name, and the frame slot it maps to.
#[derive(Clone)]
struct Local {
    name: String,
    slot: Option<usize>,
}

/// One level of the enclosing chain: a function's scopes and
/// upvalues, as they were when the nested function was compiled.
#[derive(Clone)]
struct EnclosingLevel {
    scopes: Vec<Scope>,
    upvalues: Vec<Upvalue>,
}

/// The enclosing chain of a function, innermost first: the
/// parent's level, then the grandparent's, and so on. A nested
/// function resolves its free variables against this snapshot —
/// it never mutates the enclosing compiler.
#[derive(Clone)]
struct EnclosingCtx {
    levels: Vec<EnclosingLevel>,
}

/// What resolving a name in the enclosing chain found.
enum ChainFound {
    /// A slot (or upvalue) at `depth` in the chain, with how
    /// the defining frame holds it.
    Captured(usize, UpvalueKind),
    /// A name-based binding: a global, and deeper levels are
    /// shadowed by it.
    Named,
    /// Not found anywhere in the chain.
    Missing,
}

/// How a name resolved at compile time.
enum Resolved {
    /// A slot of this frame.
    Local(usize),
    /// An upvalue of this frame (index into its upvalue array).
    Upvalue(usize),
    /// A global: looked up by name in the environment chain.
    Global,
}

struct Compiler {
    name: String,
    params: Vec<String>,
    rest: Option<String>,
    defaults: Vec<Option<Rc<CodeObject>>>,
    body: Vec<Stmt>,
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    names: Vec<String>,
    hidden: usize,
    scopes: Vec<Scope>,
    upvalues: Vec<Upvalue>,
    next_slot: usize,
    enclosing: Option<EnclosingCtx>,
    /// Names this function needs its parent to provide as
    /// upvalues (they live beyond the parent): recorded in
    /// order of first reference, and appended to the parent's
    /// upvalues by the parent right after this function is
    /// compiled.
    pending: Vec<String>,
}

impl Compiler {
    fn new(
        name: &str,
        params: Vec<String>,
        rest: Option<String>,
        body: Vec<Stmt>,
        slot_based: bool,
        enclosing: Option<EnclosingCtx>,
    ) -> Self {
        let mut compiler = Self {
            name: name.to_string(),
            params: params.clone(),
            rest: rest.clone(),
            defaults: Vec::new(),
            body,
            instructions: Vec::new(),
            constants: Vec::new(),
            names: Vec::new(),
            hidden: 0,
            scopes: vec![Scope {
                locals: Vec::new(),
                slot_based,
            }],
            upvalues: Vec::new(),
            next_slot: 0,
            enclosing,
            pending: Vec::new(),
        };
        // Parameters are slots 0..n, in order, then the rest
        // parameter — so call bindings apply by position.
        for param in params {
            compiler.declare(&param);
        }
        if let Some(rest) = rest {
            compiler.declare(&rest);
        }
        compiler
    }

    fn finish(self, is_generator: bool) -> (Rc<CodeObject>, Vec<String>) {
        (
            Rc::new(CodeObject {
                name: self.name,
                params: self.params,
                rest: self.rest,
                defaults: self.defaults,
                body: self.body,
                instructions: self.instructions,
                constants: self.constants,
                names: self.names,
                slot_count: self.next_slot,
                upvalues: self.upvalues,
                is_generator,
            }),
            self.pending,
        )
    }

    /// The next instruction index (the address an emitted jump
    /// would land on).
    fn here(&self) -> usize {
        self.instructions.len()
    }

    fn emit(&mut self, op: Op, span: Span) -> usize {
        self.instructions.push(Instruction { op, span });
        self.instructions.len() - 1
    }

    /// Emit a jump with a placeholder target, to be patched once
    /// the target is known.
    fn emit_jump(&mut self, op: Op, span: Span) -> usize {
        self.emit(op, span)
    }

    /// Set the target of the jump at `pc`. Only jump-like ops
    /// carry a target; anything else is a compiler bug.
    fn patch(&mut self, pc: usize, target: usize) {
        match &mut self.instructions[pc].op {
            Op::Jump { addr }
            | Op::JumpIfFalse { addr }
            | Op::JumpIfTrue { addr }
            | Op::ForCheck { end: addr, .. }
            | Op::ForNext { end: addr, .. }
            | Op::ForInNext { end: addr, .. }
            | Op::CompNext { end: addr, .. }
            | Op::MatchPattern { fail: addr, .. } => *addr = target,
            Op::PushHandler {
                except, finally, ..
            } => {
                // Handlers are patched by `patch_handler`, not here.
                let _ = (except, finally);
                panic!("patch() cannot patch a handler: use patch_handler()");
            }
            _ => panic!("patch() called on a non-jump instruction"),
        }
    }

    /// Set the slot list of the `EnterScope` at `pc`, once the
    /// block is compiled and its declared slots are known.
    fn patch_scope(&mut self, pc: usize, slots: Vec<usize>) {
        match &mut self.instructions[pc].op {
            Op::EnterScope { slots: s } => *s = Rc::from(slots.as_slice()),
            _ => panic!("patch_scope() called on a non-EnterScope instruction"),
        }
    }

    /// Set the scope-slot list of the `MatchPattern` at `pc`,
    /// once the case body is compiled and its slots are known.
    fn patch_match_scope(&mut self, pc: usize, slots: Vec<usize>) {
        match &mut self.instructions[pc].op {
            Op::MatchPattern { scope_slots: s, .. } => *s = Rc::from(slots.as_slice()),
            _ => panic!("patch_match_scope() called on a non-MatchPattern instruction"),
        }
    }

    /// Push a fresh block scope. Block scopes are always
    /// slot-based: a `let` inside a block is never a module
    /// attribute and never persists across REPL chunks, so a
    /// slot is fine even at a name-based top level.
    fn push_scope(&mut self) {
        self.scopes.push(Scope {
            locals: Vec::new(),
            slot_based: true,
        });
    }

    /// Pop the current scope, returning the slots it declared.
    fn pop_scope(&mut self) -> Vec<usize> {
        self.scopes
            .pop()
            .map(|scope| scope.locals.iter().filter_map(|l| l.slot).collect())
            .unwrap_or_default()
    }

    /// The slots declared in the current scope so far.
    fn current_scope_slots(&self) -> Vec<usize> {
        self.scopes
            .last()
            .map(|scope| scope.locals.iter().filter_map(|l| l.slot).collect())
            .unwrap_or_default()
    }

    /// Declare `name` in the current scope: the slot it gets (a
    /// fresh one, unless the name is already declared in this
    /// scope — a re-declaration reuses the binding, like the
    /// tree-walker's `define` on a child scope), or `None` when
    /// this scope keeps its names in the environment.
    fn declare(&mut self, name: &str) -> Option<usize> {
        let scope = self
            .scopes
            .last_mut()
            .expect("a compiler always has a scope");
        if let Some(local) = scope.locals.iter().find(|l| l.name == name) {
            return local.slot;
        }
        let slot = if scope.slot_based {
            let slot = self.next_slot;
            self.next_slot += 1;
            Some(slot)
        } else {
            None
        };
        scope.locals.push(Local {
            name: name.to_string(),
            slot,
        });
        slot
    }

    /// The enclosing context a nested function compiles
    /// against: this compiler's scopes and upvalues, followed
    /// by the enclosing chain this compiler itself was compiled
    /// against. The nested function's scopes are untouched by
    /// it (it compiles with its own scope stack), so a snapshot
    /// is exact.
    fn snapshot_enclosing(&self) -> EnclosingCtx {
        let mut levels = vec![EnclosingLevel {
            scopes: self.scopes.clone(),
            upvalues: self.upvalues.clone(),
        }];
        if let Some(enclosing) = &self.enclosing {
            levels.extend(enclosing.levels.iter().cloned());
        }
        EnclosingCtx { levels }
    }

    /// Resolve a name in this function: a slot of this frame, an
    /// upvalue of it, or a global. A name declared *later* in an
    /// enclosing scope is not seen here — single-pass compilation
    /// has no symtable pass — so it falls through to a global,
    /// unlike the tree-walker (a known limitation, see the
    /// README). A name found beyond the parent is recorded in
    /// `pending`: the parent provides it as an upvalue right
    /// after this function is compiled (and records a pending
    /// entry of its own when the name lives further up, so the
    /// chain is built one level at a time).
    fn resolve(&mut self, name: &str) -> Resolved {
        for scope in self.scopes.iter().rev() {
            if let Some(local) = scope.locals.iter().find(|l| l.name == name) {
                return match local.slot {
                    Some(slot) => Resolved::Local(slot),
                    None => Resolved::Global,
                };
            }
        }
        // What the enclosing chain holds for `name`, found
        // without mutating anything (the borrow ends before the
        // upvalue is recorded below).
        let found = match &self.enclosing {
            Some(enclosing) => {
                let mut found = ChainFound::Missing;
                'levels: for (depth, level) in enclosing.levels.iter().enumerate() {
                    for scope in level.scopes.iter().rev() {
                        if let Some(local) = scope.locals.iter().find(|l| l.name == name) {
                            found = match local.slot {
                                Some(slot) => ChainFound::Captured(depth, UpvalueKind::Local(slot)),
                                None => ChainFound::Named,
                            };
                            break 'levels;
                        }
                    }
                    if let Some(idx) = level.upvalues.iter().position(|u| u.name == name) {
                        found = ChainFound::Captured(depth, UpvalueKind::Upvalue(idx));
                        break 'levels;
                    }
                }
                found
            }
            None => ChainFound::Missing,
        };
        match found {
            ChainFound::Captured(0, kind) => Resolved::Upvalue(self.add_upvalue(name, kind)),
            ChainFound::Captured(_, _) => Resolved::Upvalue(self.add_pending_upvalue(name)),
            ChainFound::Named | ChainFound::Missing => Resolved::Global,
        }
    }

    /// Record an upvalue of this function (deduplicated by name).
    fn add_upvalue(&mut self, name: &str, kind: UpvalueKind) -> usize {
        if let Some(idx) = self.upvalues.iter().position(|u| u.name == name) {
            return idx;
        }
        self.upvalues.push(Upvalue {
            name: name.to_string(),
            kind,
        });
        self.upvalues.len() - 1
    }

    /// Record that the parent must provide `name` as an upvalue
    /// (it lives beyond the parent): this function's upvalue for
    /// it reads the parent's upvalue at the index the parent
    /// will assign — its upvalue count at the snapshot, plus
    /// the name's position in `pending` (deduplicated, so two
    /// references to the same name share one entry).
    fn add_pending_upvalue(&mut self, name: &str) -> usize {
        if let Some(idx) = self.upvalues.iter().position(|u| u.name == name) {
            return idx;
        }
        let parent_count = self
            .enclosing
            .as_ref()
            .and_then(|enclosing| enclosing.levels.first())
            .map(|level| level.upvalues.len())
            .unwrap_or(0);
        let pos = match self.pending.iter().position(|n| n == name) {
            Some(pos) => pos,
            None => {
                self.pending.push(name.to_string());
                self.pending.len() - 1
            }
        };
        self.add_upvalue(name, UpvalueKind::Upvalue(parent_count + pos))
    }

    /// Provide an upvalue a nested function asked for: resolve
    /// `name` as an upvalue of this function (adding it, and
    /// recording a pending entry of this function's own when
    /// the name lives further up the chain). A name the nested
    /// function found in the chain always resolves here — a
    /// global would mean the chain disagrees, which is a
    /// compiler bug.
    fn provide_upvalue(&mut self, name: &str) {
        if let Resolved::Global = self.resolve(name) {
            panic!("pending upvalue '{name}' did not resolve (compiler bug)");
        }
    }

    /// Resolve a name to how it is read at runtime: a slot of
    /// this frame, an upvalue of it, or a global by name.
    fn resolve_var_ref(&mut self, name: &str) -> VarRef {
        match self.resolve(name) {
            Resolved::Local(slot) => VarRef::Local(slot),
            Resolved::Upvalue(idx) => VarRef::Upvalue(idx),
            Resolved::Global => VarRef::Global(name.to_string()),
        }
    }

    /// Emit the load of a variable, resolved at compile time.
    fn load_var(&mut self, name: &str, span: Span) {
        match self.resolve(name) {
            Resolved::Local(slot) => self.emit(Op::LoadSlot { slot }, span),
            Resolved::Upvalue(idx) => self.emit(Op::LoadUpvalue { idx }, span),
            Resolved::Global => {
                let name = self.name_idx(name);
                self.emit(Op::LoadName { name }, span)
            }
        };
    }

    /// Emit the assignment of a variable (the value is already on
    /// the stack).
    fn store_var(&mut self, name: &str, span: Span) {
        match self.resolve(name) {
            Resolved::Local(slot) => self.emit(Op::StoreSlot { slot }, span),
            Resolved::Upvalue(idx) => self.emit(Op::StoreUpvalue { idx }, span),
            Resolved::Global => {
                let name = self.name_idx(name);
                self.emit(Op::StoreName { name }, span)
            }
        };
    }

    /// Emit the definition of a variable (the value is already on
    /// the stack): a slot, or an environment name.
    fn define_var(&mut self, name: &str, span: Span) {
        match self.declare(name) {
            Some(slot) => self.emit(Op::DefineSlot { slot }, span),
            None => {
                let name = self.name_idx(name);
                self.emit(Op::DefineName { name }, span)
            }
        };
    }

    /// Compile `stmts` in a fresh block scope, with an
    /// `EnterScope` first (patched with the scope's slots once
    /// the block is compiled).
    fn compile_block(&mut self, stmts: &[Stmt], span: Span) {
        self.push_scope();
        let enter_pc = self.emit(Op::EnterScope { slots: Rc::new([]) }, span);
        self.compile_stmts(stmts);
        let slots = self.pop_scope();
        self.patch_scope(enter_pc, slots);
    }

    /// The binding names of a pattern, in the order
    /// `interp::match_pattern` returns them (left to right,
    /// depth first).
    fn pattern_binding_names(pattern: &Pattern, out: &mut Vec<String>) {
        match pattern {
            Pattern::Literal(_) | Pattern::Wildcard => {}
            Pattern::Binding(name) => out.push(name.clone()),
            Pattern::List(patterns) | Pattern::Tuple(patterns) => {
                for pattern in patterns {
                    Self::pattern_binding_names(pattern, out);
                }
            }
        }
    }

    /// Set the `except` / `finally` targets of the handler at `pc`.
    fn patch_handler(&mut self, pc: usize, except: Option<usize>, finally: Option<usize>) {
        match &mut self.instructions[pc].op {
            Op::PushHandler {
                except: e,
                finally: f,
                ..
            } => {
                *e = except;
                *f = finally;
            }
            _ => panic!("patch_handler() called on a non-handler instruction"),
        }
    }

    /// Set the `finally` target of the `FinishTry` at `pc`.
    fn patch_finish_try(&mut self, pc: usize, finally: Option<usize>) {
        match &mut self.instructions[pc].op {
            Op::FinishTry { finally: f } => *f = finally,
            _ => panic!("patch_finish_try() called on a non-FinishTry instruction"),
        }
    }

    /// Add a constant, deduplicating the scalar ones.
    fn add_const(&mut self, constant: Constant) -> usize {
        if let Some(idx) = self.constants.iter().position(|c| c == &constant) {
            idx
        } else {
            self.constants.push(constant);
            self.constants.len() - 1
        }
    }

    /// Intern a name in the name table.
    fn name_idx(&mut self, name: &str) -> usize {
        if let Some(idx) = self.names.iter().position(|n| n == name) {
            idx
        } else {
            self.names.push(name.to_string());
            self.names.len() - 1
        }
    }

    /// A hidden local name: `$` cannot appear in a user-written
    /// identifier, so it never collides with a real variable.
    fn hidden_name(&mut self, what: &str) -> String {
        let name = format!("${what}_{}", self.hidden);
        self.hidden += 1;
        name
    }

    fn load_const(&mut self, constant: Constant, span: Span) {
        let idx = self.add_const(constant);
        self.emit(Op::LoadConst { idx }, span);
    }

    fn compile_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.compile_stmt(stmt);
        }
    }

    fn compile_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, value, .. } => {
                self.compile_expr(value);
                self.define_var(name, stmt.span());
            }
            Stmt::Assign { name, value, .. } => {
                self.compile_expr(value);
                self.store_var(name, stmt.span());
            }
            Stmt::IndexAssign { target, value, .. } => {
                // The right-hand side is evaluated first (like
                // Python), then the container, then the indices.
                self.compile_expr(value);
                let Expr::Index {
                    target, indices, ..
                } = target
                else {
                    // Unreachable through the parser (it only
                    // builds IndexAssign from an Index target).
                    self.emit(
                        Op::Fail {
                            message: "invalid assignment target".to_string(),
                        },
                        stmt.span(),
                    );
                    return;
                };
                self.compile_expr(target);
                let n = indices.len();
                for index in indices {
                    self.compile_index_item(index);
                }
                self.emit(Op::SetIndex { n }, stmt.span());
            }
            Stmt::FieldAssign { target, value, .. } => {
                self.compile_expr(value);
                let Expr::Field { target, name, .. } = target else {
                    self.emit(
                        Op::Fail {
                            message: "invalid assignment target".to_string(),
                        },
                        stmt.span(),
                    );
                    return;
                };
                self.compile_expr(target);
                self.emit(Op::SetField { name: name.clone() }, stmt.span());
            }
            Stmt::Print { expr, .. } => {
                self.compile_expr(expr);
                self.emit(Op::Print, stmt.span());
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                self.compile_expr(cond);
                let else_jump = self.emit_jump(Op::JumpIfFalse { addr: 0 }, stmt.span());
                self.compile_block(then_block, stmt.span());
                match else_block {
                    Some(else_block) => {
                        let end_jump = self.emit_jump(Op::Jump { addr: 0 }, stmt.span());
                        self.patch(else_jump, self.here());
                        self.compile_block(else_block, stmt.span());
                        self.patch(end_jump, self.here());
                    }
                    None => self.patch(else_jump, self.here()),
                }
            }
            Stmt::While { cond, body, .. } => {
                let cond_pc = self.here();
                self.compile_expr(cond);
                let exit_jump = self.emit_jump(Op::JumpIfFalse { addr: 0 }, stmt.span());
                self.compile_block(body, stmt.span());
                self.emit(Op::Jump { addr: cond_pc }, stmt.span());
                self.patch(exit_jump, self.here());
            }
            Stmt::For {
                var,
                from,
                to,
                body,
                ..
            } => {
                self.compile_expr(from);
                self.compile_expr(to);
                // The loop scope holds the loop variable and the
                // hidden bounds; the body runs in a fresh child
                // scope per iteration, and the loop variable
                // itself is one shared binding across iterations.
                self.push_scope();
                let var_slot = self.declare(var).expect("loop scopes are slot-based");
                let to_name = self.hidden_name("for_to");
                let to_slot = self.declare(&to_name).expect("loop scopes are slot-based");
                let step_name = self.hidden_name("for_step");
                let step_slot = self
                    .declare(&step_name)
                    .expect("loop scopes are slot-based");
                self.emit(
                    Op::ForPrep {
                        var: var_slot,
                        to: to_slot,
                        step: step_slot,
                        from_span: from.span(),
                        to_span: to.span(),
                    },
                    stmt.span(),
                );
                let head_pc = self.here();
                let check_pc = self.emit(
                    Op::ForCheck {
                        var: var_slot,
                        to: to_slot,
                        step: step_slot,
                        end: 0,
                    },
                    stmt.span(),
                );
                self.compile_block(body, stmt.span());
                let next_pc = self.emit(
                    Op::ForNext {
                        var: var_slot,
                        to: to_slot,
                        step: step_slot,
                        end: 0,
                    },
                    stmt.span(),
                );
                self.emit(Op::Jump { addr: head_pc }, stmt.span());
                let after = self.here();
                self.patch(check_pc, after);
                self.patch(next_pc, after);
                self.pop_scope();
            }
            Stmt::ForIn {
                var, iter, body, ..
            } => {
                self.compile_expr(iter);
                self.push_scope();
                let iter_name = self.hidden_name("for_iter");
                let iter_slot = self
                    .declare(&iter_name)
                    .expect("loop scopes are slot-based");
                let idx_name = self.hidden_name("for_idx");
                let idx_slot = self.declare(&idx_name).expect("loop scopes are slot-based");
                let var_slot = self.declare(var).expect("loop scopes are slot-based");
                self.emit(
                    Op::ForInPrep {
                        iter: iter_slot,
                        idx: idx_slot,
                        var: var_slot,
                        iter_span: iter.span(),
                    },
                    stmt.span(),
                );
                let head_pc = self.here();
                let next_pc = self.emit(
                    Op::ForInNext {
                        iter: iter_slot,
                        idx: idx_slot,
                        var: var_slot,
                        end: 0,
                    },
                    stmt.span(),
                );
                self.compile_block(body, stmt.span());
                self.emit(Op::Jump { addr: head_pc }, stmt.span());
                self.patch(next_pc, self.here());
                self.pop_scope();
            }
            Stmt::Fn {
                name,
                params,
                rest,
                body,
                ..
            } => {
                // Declare the name first: the function can call
                // itself (its own name resolves to an upvalue of
                // the enclosing frame, or to a global).
                self.declare(name);
                let code = self.compile_function(name, params, rest.as_deref(), body);
                self.emit(Op::MakeFunction { code }, stmt.span());
                self.define_var(name, stmt.span());
            }
            Stmt::Class {
                name,
                parent,
                methods,
                ..
            } => {
                // Like `fn`: the name is declared first, so a
                // method can refer to its own class.
                self.declare(name);
                let parent = parent
                    .as_ref()
                    .map(|parent| (parent.clone(), self.resolve_var_ref(parent)));
                let methods = methods
                    .iter()
                    .map(|method| {
                        (
                            method.name.clone(),
                            self.compile_function(
                                &method.name,
                                &method.params,
                                method.rest.as_deref(),
                                &method.body,
                            ),
                        )
                    })
                    .collect();
                self.emit(
                    Op::MakeClass {
                        name: name.clone(),
                        parent: parent.clone(),
                        methods,
                    },
                    stmt.span(),
                );
                self.define_var(name, stmt.span());
            }
            Stmt::Import { name, alias, .. } => {
                let bound = alias.as_deref().unwrap_or(name);
                let slot = self.declare(bound);
                self.emit(
                    Op::Import {
                        name: name.clone(),
                        alias: alias.clone(),
                        slot,
                    },
                    stmt.span(),
                );
            }
            Stmt::Return { expr, .. } => {
                match expr {
                    Some(expr) => self.compile_expr(expr),
                    None => self.load_const(Constant::Nil, stmt.span()),
                }
                self.emit(Op::Return, stmt.span());
            }
            Stmt::Try {
                body,
                except_var,
                except_body,
                finally_body,
                ..
            } => {
                // The except variable lives in its own scope,
                // which the try body must not see: declare it
                // now (its slot goes into the handler), stash
                // the scope, and push it back when compiling
                // the except region.
                let mut except_scope = None;
                let except_var_slot = if except_body.is_some() {
                    self.push_scope();
                    let slot = except_var.as_ref().and_then(|var| self.declare(var));
                    except_scope = Some(self.scopes.pop().expect("the except scope"));
                    slot
                } else {
                    None
                };
                let push_pc = self.emit(
                    Op::PushHandler {
                        except: None,
                        finally: None,
                        var: except_var_slot,
                    },
                    stmt.span(),
                );
                self.compile_stmts(body);
                let finish_try = self.emit(Op::FinishTry { finally: None }, stmt.span());
                let jump_after_try = self.emit_jump(Op::Jump { addr: 0 }, stmt.span());
                // The `except` region, entered by the VM on an
                // error (with the error message bound to `var`).
                let except_pc = except_body.as_ref().map(|_| self.here());
                let mut finish_except = None;
                let mut jump_after_except = None;
                if let Some(except_body) = except_body {
                    self.scopes.push(except_scope.expect("the except scope"));
                    let enter_pc = self.emit(Op::EnterScope { slots: Rc::new([]) }, stmt.span());
                    self.compile_stmts(except_body);
                    let slots = self.pop_scope();
                    self.patch_scope(enter_pc, slots);
                    finish_except = Some(self.emit(Op::FinishTry { finally: None }, stmt.span()));
                    jump_after_except = Some(self.emit_jump(Op::Jump { addr: 0 }, stmt.span()));
                }
                let finally_pc = finally_body.as_ref().map(|_| self.here());
                if let Some(finally_body) = finally_body {
                    self.compile_block(finally_body, stmt.span());
                    self.emit(Op::EndFinally, stmt.span());
                }
                let after = self.here();
                self.patch_handler(push_pc, except_pc, finally_pc);
                // Both `FinishTry`s share the same `finally`.
                self.patch_finish_try(finish_try, finally_pc);
                if let Some(finish_except) = finish_except {
                    self.patch_finish_try(finish_except, finally_pc);
                }
                self.patch(jump_after_try, after);
                if let Some(jump_after_except) = jump_after_except {
                    self.patch(jump_after_except, after);
                }
            }
            Stmt::Raise { expr, .. } => {
                self.compile_expr(expr);
                self.emit(
                    Op::Raise {
                        expr_span: expr.span(),
                    },
                    stmt.span(),
                );
            }
            Stmt::Match {
                scrutinee, cases, ..
            } => {
                self.compile_expr(scrutinee);
                let mut end_jumps = Vec::new();
                for case in cases {
                    self.emit(Op::Dup, stmt.span());
                    // The case scope holds the pattern bindings
                    // (and the body's locals); its slots are
                    // patched into the `MatchPattern` once the
                    // body is compiled.
                    self.push_scope();
                    let mut binding_names = Vec::new();
                    Self::pattern_binding_names(&case.pattern, &mut binding_names);
                    let bindings: Vec<usize> = binding_names
                        .iter()
                        .map(|name| self.declare(name).expect("match scopes are slot-based"))
                        .collect();
                    let match_pc = self.emit(
                        Op::MatchPattern {
                            pattern: case.pattern.clone(),
                            fail: 0,
                            bindings: Rc::from(bindings.as_slice()),
                            scope_slots: Rc::new([]),
                        },
                        stmt.span(),
                    );
                    // A false guard means "try the next case".
                    let mut guard_jump = None;
                    if let Some(guard) = &case.guard {
                        self.compile_expr(guard);
                        guard_jump = Some(self.emit_jump(Op::JumpIfFalse { addr: 0 }, stmt.span()));
                    }
                    self.compile_stmts(&case.body);
                    let scope_slots = self.current_scope_slots();
                    self.patch_match_scope(match_pc, scope_slots);
                    self.pop_scope();
                    end_jumps.push(self.emit_jump(Op::Jump { addr: 0 }, stmt.span()));
                    // The next case starts here: the guard-false
                    // path and the no-match path both land here.
                    let next_case = self.here();
                    if let Some(guard_jump) = guard_jump {
                        self.patch(guard_jump, next_case);
                    }
                    self.patch(match_pc, next_case);
                }
                let end = self.here();
                for jump in end_jumps {
                    self.patch(jump, end);
                }
                self.emit(Op::Pop, stmt.span());
            }
            Stmt::Yield { expr, .. } => {
                match expr {
                    Some(expr) => self.compile_expr(expr),
                    None => self.load_const(Constant::Nil, stmt.span()),
                }
                self.emit(Op::Yield, stmt.span());
            }
            Stmt::ExprStmt { expr, .. } => {
                self.compile_expr(expr);
                self.emit(Op::Pop, stmt.span());
            }
        }
    }

    /// Compile one index item of an `Index` node: a plain
    /// expression becomes an index value, a slice becomes its
    /// evaluated bounds.
    fn compile_index_item(&mut self, index: &Expr) {
        match index {
            Expr::Slice {
                start,
                stop,
                step,
                span,
            } => {
                let mut spans = [None, None, None];
                for (i, bound) in [start, stop, step].iter().enumerate() {
                    match bound {
                        Some(expr) => {
                            self.compile_expr(expr);
                            spans[i] = Some(expr.span());
                        }
                        None => self.load_const(Constant::Nil, *span),
                    }
                }
                self.emit(Op::PushIndexSlice { spans }, *span);
            }
            other => {
                self.compile_expr(other);
                self.emit(Op::PushIndexValue { span: other.span() }, other.span());
            }
        }
    }

    /// Compile a call's argument list. The callee is already on
    /// the stack; this appends the positional list, then the
    /// keyword values (the parser guarantees positionals and
    /// spreads come before keywords, so source evaluation order
    /// is preserved). Returns the keyword names with their spans,
    /// for the `Call` instruction.
    fn compile_args(&mut self, args: &[Arg]) -> Rc<[(String, Span)]> {
        self.emit(Op::BuildList, Span::dummy());
        let mut kwargs = Vec::new();
        for arg in args {
            match arg {
                Arg::Positional(expr) => {
                    self.compile_expr(expr);
                    self.emit(Op::ListAppend, expr.span());
                }
                Arg::Spread { expr, span } => {
                    self.compile_expr(expr);
                    self.emit(Op::ListExtend { span: *span }, *span);
                }
                Arg::Keyword { name, value, span } => {
                    self.compile_expr(value);
                    kwargs.push((name.clone(), *span));
                }
            }
        }
        Rc::from(kwargs.as_slice())
    }

    fn compile_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Int { value, span } => self.load_const(Constant::Int(*value), *span),
            Expr::Float { value, span } => self.load_const(Constant::Float(*value), *span),
            Expr::Str { value, span } => self.load_const(Constant::Str(value.clone()), *span),
            Expr::FString { parts, span } => {
                self.load_const(Constant::Str(String::new()), *span);
                for part in parts {
                    match part {
                        FStringPart::Text(text) => {
                            self.load_const(Constant::Str(text.clone()), *span);
                        }
                        FStringPart::Expr(expr) => {
                            self.compile_expr(expr);
                            self.emit(Op::DisplayValue, expr.span());
                        }
                    }
                    self.emit(Op::StrConcat, *span);
                }
            }
            Expr::Bool { value, span } => self.load_const(Constant::Bool(*value), *span),
            Expr::Ident { name, span } => {
                self.load_var(name, *span);
            }
            Expr::Unary { op, expr, span } => {
                self.compile_expr(expr);
                match op {
                    UnaryOp::Neg => self.emit(Op::Neg, *span),
                    UnaryOp::Not => self.emit(Op::Not, *span),
                };
            }
            Expr::Binary {
                op,
                left,
                right,
                span,
            } => match op {
                // `and` / `or` short-circuit and return operand
                // values: duplicate the left operand, jump past
                // the right one when it already decides the
                // result, and drop the duplicate otherwise.
                BinaryOp::And => {
                    self.compile_expr(left);
                    self.emit(Op::Dup, *span);
                    let jump = self.emit_jump(Op::JumpIfFalse { addr: 0 }, *span);
                    self.emit(Op::Pop, *span);
                    self.compile_expr(right);
                    self.patch(jump, self.here());
                }
                BinaryOp::Or => {
                    self.compile_expr(left);
                    self.emit(Op::Dup, *span);
                    let jump = self.emit_jump(Op::JumpIfTrue { addr: 0 }, *span);
                    self.emit(Op::Pop, *span);
                    self.compile_expr(right);
                    self.patch(jump, self.here());
                }
                op => {
                    self.compile_expr(left);
                    self.compile_expr(right);
                    self.emit(Op::Binary { op: *op }, *span);
                }
            },
            Expr::Call { name, args, span } => {
                self.load_var(name, *span);
                let kwargs = self.compile_args(args);
                self.emit(Op::Call { kwargs }, *span);
            }
            Expr::List { elements, span } => {
                self.emit(Op::BuildList, *span);
                for element in elements {
                    self.compile_expr(element);
                    self.emit(Op::ListAppend, element.span());
                }
            }
            Expr::Tuple { elements, .. } => {
                for element in elements {
                    self.compile_expr(element);
                }
                self.emit(Op::BuildTuple { n: elements.len() }, expr.span());
            }
            Expr::Dict { entries, span } => {
                self.emit(Op::BuildDict, *span);
                for (key_expr, value_expr) in entries {
                    self.compile_expr(key_expr);
                    self.compile_expr(value_expr);
                    self.emit(
                        Op::DictInsert {
                            key_span: key_expr.span(),
                        },
                        *span,
                    );
                }
            }
            Expr::Set { elements, span } => {
                self.emit(Op::BuildSet, *span);
                for element in elements {
                    self.compile_expr(element);
                    self.emit(
                        Op::SetInsert {
                            span: element.span(),
                        },
                        *span,
                    );
                }
            }
            Expr::ListComp {
                elem,
                vars,
                iter,
                cond,
                span,
            } => self.compile_comp(&CompSpec {
                kind: CompKind::List,
                elem,
                key: None,
                vars,
                iter,
                cond,
                span: *span,
            }),
            Expr::DictComp {
                key,
                value,
                vars,
                iter,
                cond,
                span,
            } => self.compile_comp(&CompSpec {
                kind: CompKind::Dict,
                elem: value,
                key: Some(key),
                vars,
                iter,
                cond,
                span: *span,
            }),
            Expr::SetComp {
                elem,
                vars,
                iter,
                cond,
                span,
            } => self.compile_comp(&CompSpec {
                kind: CompKind::Set,
                elem,
                key: None,
                vars,
                iter,
                cond,
                span: *span,
            }),
            Expr::Lambda {
                params, rest, body, ..
            } => {
                let code = self.compile_lambda(params, rest.as_deref(), body);
                self.emit(Op::MakeFunction { code }, expr.span());
            }
            Expr::CallExpr { callee, args, span } => {
                self.compile_expr(callee);
                let kwargs = self.compile_args(args);
                self.emit(Op::Call { kwargs }, *span);
            }
            Expr::Index {
                target,
                indices,
                span,
            } => {
                self.compile_expr(target);
                let n = indices.len();
                for index in indices {
                    self.compile_index_item(index);
                }
                self.emit(Op::GetIndex { n }, *span);
            }
            Expr::Slice { span, .. } => {
                self.emit(
                    Op::Fail {
                        message: "a slice is only valid inside '[...]'".to_string(),
                    },
                    *span,
                );
            }
            Expr::SelfExpr { span } => {
                self.load_var("self", *span);
            }
            Expr::Field { target, name, span } => {
                self.compile_expr(target);
                self.emit(Op::GetField { name: name.clone() }, *span);
            }
            Expr::MethodCall {
                target,
                name,
                args,
                span,
            } => {
                self.compile_expr(target);
                let kwargs = self.compile_args(args);
                self.emit(
                    Op::CallMethod {
                        name: name.clone(),
                        kwargs,
                    },
                    *span,
                );
            }
            Expr::SuperCall { name, args, span } => {
                let kwargs = self.compile_args(args);
                // `self` resolves like any variable: a slot of
                // this method, an upvalue of a nested function,
                // or nothing (outside a method — the VM reports
                // "'super' outside a method" then).
                let self_ref = match self.resolve_var_ref("self") {
                    VarRef::Local(slot) => Some(VarRef::Local(slot)),
                    VarRef::Upvalue(idx) => Some(VarRef::Upvalue(idx)),
                    VarRef::Global(_) => None,
                };
                self.emit(
                    Op::CallSuper {
                        name: name.clone(),
                        kwargs,
                        self_ref,
                    },
                    *span,
                );
            }
        }
    }

    /// Compile a comprehension: snapshot the iterable, then loop
    /// over it with a hidden index, binding the loop variable(s)
    /// fresh per item (like the tree-walker's `bind_comp_vars`)
    /// and accumulating into a hidden result collection. The
    /// comprehension's helpers live in one scope (entered once),
    /// the loop variables in a per-item scope (re-bound each
    /// item by `BindComp`).
    fn compile_comp(&mut self, spec: &CompSpec<'_>) {
        self.compile_expr(spec.iter);
        self.push_scope();
        let iter_name = self.hidden_name("comp_iter");
        let iter_slot = self
            .declare(&iter_name)
            .expect("comprehension scopes are slot-based");
        let idx_name = self.hidden_name("comp_idx");
        let idx_slot = self
            .declare(&idx_name)
            .expect("comprehension scopes are slot-based");
        let acc_name = self.hidden_name("comp_acc");
        let acc_slot = self
            .declare(&acc_name)
            .expect("comprehension scopes are slot-based");
        self.emit(
            Op::CompPrep {
                iter: iter_slot,
                idx: idx_slot,
                acc: acc_slot,
                kind: spec.kind,
                iter_span: spec.iter.span(),
            },
            spec.span,
        );
        let head_pc = self.here();
        let next_pc = self.emit(
            Op::CompNext {
                iter: iter_slot,
                idx: idx_slot,
                end: 0,
            },
            spec.span,
        );
        self.push_scope();
        let var_slots: Vec<usize> = spec
            .vars
            .iter()
            .map(|var| {
                self.declare(var)
                    .expect("comprehension scopes are slot-based")
            })
            .collect();
        self.emit(
            Op::BindComp {
                vars: Rc::from(var_slots.as_slice()),
                span: spec.span,
            },
            spec.span,
        );
        // A false filter skips the item: go back to the head.
        if let Some(cond) = spec.cond {
            self.compile_expr(cond);
            let skip_jump = self.emit_jump(Op::JumpIfFalse { addr: 0 }, spec.span);
            match spec.kind {
                CompKind::List | CompKind::Set => {
                    self.compile_expr(spec.elem);
                    self.emit(
                        match spec.kind {
                            CompKind::List => Op::CompAppend { acc: acc_slot },
                            CompKind::Set => Op::CompSetInsert {
                                acc: acc_slot,
                                span: spec.elem.span(),
                            },
                            CompKind::Dict => unreachable!(),
                        },
                        spec.span,
                    );
                }
                CompKind::Dict => {
                    let key = spec.key.expect("a dict comprehension has a key");
                    self.compile_expr(key);
                    self.compile_expr(spec.elem);
                    self.emit(
                        Op::CompDictInsert {
                            acc: acc_slot,
                            key_span: key.span(),
                        },
                        spec.span,
                    );
                }
            }
            self.emit(Op::Jump { addr: head_pc }, spec.span);
            let skip_pc = self.here();
            self.emit(Op::Jump { addr: head_pc }, spec.span);
            self.patch(skip_jump, skip_pc);
        } else {
            match spec.kind {
                CompKind::List | CompKind::Set => {
                    self.compile_expr(spec.elem);
                    self.emit(
                        match spec.kind {
                            CompKind::List => Op::CompAppend { acc: acc_slot },
                            CompKind::Set => Op::CompSetInsert {
                                acc: acc_slot,
                                span: spec.elem.span(),
                            },
                            CompKind::Dict => unreachable!(),
                        },
                        spec.span,
                    );
                }
                CompKind::Dict => {
                    let key = spec.key.expect("a dict comprehension has a key");
                    self.compile_expr(key);
                    self.compile_expr(spec.elem);
                    self.emit(
                        Op::CompDictInsert {
                            acc: acc_slot,
                            key_span: key.span(),
                        },
                        spec.span,
                    );
                }
            }
            self.emit(Op::Jump { addr: head_pc }, spec.span);
        }
        self.pop_scope();
        self.patch(next_pc, self.here());
        self.emit(Op::LoadSlot { slot: acc_slot }, spec.span);
        self.pop_scope();
    }
}
