// SPDX-License-Identifier: GPL-3.0-or-later
//! The GG virtual machine: a stack-based bytecode interpreter.
//!
//! A frame runs one code object: an operand stack, an
//! index-argument side stack (slices travel there, not on the
//! operand stack), a locals array — one slot per local of the
//! code object, so a variable read is an array index, not a
//! scope-chain walk — a cell array for the locals a nested
//! function captured, the frame's upvalues (cells shared with
//! the defining frame), a handler stack for
//! `try`/`except`/`finally`, and the `super` context of the
//! frame. Calls recurse into `run_frame`, like the tree-walker's
//! `exec_block` recursion, so GG call depth maps to Rust call
//! depth in both engines.
//!
//! A name bound nowhere in any enclosing function is a global:
//! those still resolve by name in the frame's single base
//! environment (the scope the frame was created in), which is
//! also how module attributes and the REPL's persistent top
//! level work. A nested function's code object lists its
//! upvalues: `MakeFunction` fills them from the defining
//! frame's slots and cells, so a closure and its definer share
//! one cell per captured binding — no copies, and later
//! rebindings are visible on both sides.
//!
//! Errors carry the span of the instruction that failed (the
//! loop fills it in with `at`, like `exec_stmt` does for
//! statements), so positions match the tree-walker's exactly.
//! A `finally` runs on every exit path — normal, error, return
//! and yield — with the pending outcome remembered in the frame
//! until `EndFinally` acts on it (Python rule: a `return` or an
//! error inside `finally` replaces the pending outcome).

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

use gg_lexer::Span;
use gg_parser::ast::{Expr, Program};

use crate::array;
use crate::env::Environment;
use crate::error::RuntimeError;
use crate::interp::{
    CallArgs, Interpreter, assign_index_value, binary_values, bind_arguments, bind_comp_slots,
    index_value, iterable_items, match_pattern, spread_items,
};
use crate::slice::{IndexArg, SliceSpec};
use crate::value::{
    BoundMethod, ClassDef, DictData, Function, GeneratorData, InstanceData, ModuleData, Value,
    lookup_method,
};
use crate::vm::chunk::{CodeObject, CompKind, Constant, Instruction, Op, UpvalueKind, VarRef};
use crate::vm::compiler::{compile_expr_code, compile_function_body, compile_program};

/// What running a frame produced: fall off the end (`Normal`),
/// a `return` with its value and span, or a `yield` with its
/// value and span (only while a generator is being replayed).
pub(crate) enum FrameOutcome {
    Normal,
    Return(Value, Span),
    Yield(Value, Span),
}

/// What one instruction did: keep going, or leave the frame.
enum Step {
    Continue,
    Return(Value, Span),
    Yield(Value, Span),
}

/// What an error unwind found: a handler to run, or nothing
/// left to catch it.
enum Unwind {
    Continue,
    Propagate(RuntimeError),
}

/// Which region of a `try` a handler is currently in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandlerPhase {
    Try,
    Except,
    Finally,
}

/// The outcome a `finally` block is waiting to replace or
/// resume: a normal exit, a `return`, an error, or a `yield`.
enum PendingOutcome {
    Normal,
    Return(Value, Span),
    Error(RuntimeError),
    Yield(Value, Span),
}

/// One `try` handler on the frame's handler stack. The stack
/// and index-argument depths are recorded when the handler is
/// pushed, so entering an `except` or `finally` region starts
/// from a clean state. `var` is the slot the error message is
/// bound to on entry to the `except` region.
struct Handler {
    except_pc: Option<usize>,
    finally_pc: Option<usize>,
    var: Option<usize>,
    phase: HandlerPhase,
    stack_len: usize,
    index_len: usize,
}

/// The state of one running code object.
struct Frame {
    code: Rc<CodeObject>,
    pc: usize,
    stack: Vec<Value>,
    index_args: Vec<IndexArg>,
    base_env: Rc<Environment>,
    /// One entry per slot of the code object: the current value
    /// of that local.
    locals: Vec<Value>,
    /// One entry per slot: the shared cell for that local, once
    /// a nested function captured it (lazily created at
    /// `MakeFunction` time).
    cells: Vec<Option<Rc<RefCell<Value>>>>,
    /// The cells this frame's code object captured from the
    /// defining frame, in code-object upvalue order.
    upvalues: Vec<Rc<RefCell<Value>>>,
    handlers: Vec<Handler>,
    super_class: Option<Rc<ClassDef>>,
    yield_target: Option<usize>,
    yield_seen: usize,
    pending: Option<PendingOutcome>,
}

impl Frame {
    fn new(
        code: Rc<CodeObject>,
        env: Rc<Environment>,
        upvalues: Vec<Rc<RefCell<Value>>>,
        locals: Vec<Value>,
        super_class: Option<Rc<ClassDef>>,
        yield_target: Option<usize>,
    ) -> Self {
        let slot_count = code.slot_count;
        let mut locals = locals;
        locals.resize(slot_count, Value::Nil);
        Self {
            code,
            pc: 0,
            stack: Vec::new(),
            index_args: Vec::new(),
            base_env: env,
            locals,
            // Lazily created: a frame whose locals are never
            // captured pays no cell array at all (one fewer
            // allocation per call), and every access already
            // treats a missing entry as "no cell".
            cells: Vec::new(),
            upvalues,
            handlers: Vec::new(),
            super_class,
            yield_target,
            yield_seen: 0,
            pending: None,
        }
    }

    /// The frame's base environment (the scope it was created
    /// in): where globals are defined and looked up.
    fn env(&self) -> &Rc<Environment> {
        &self.base_env
    }

    /// The value of a local slot. When the local was captured,
    /// the cell holds the current value: a nested function may
    /// have written it through the cell (a `StoreUpvalue`
    /// writes the cell only), so the cell is read first and the
    /// slot is only the value before any capture.
    fn load_slot(&self, slot: usize) -> Value {
        if let Some(cell) = self.cells.get(slot).and_then(|c| c.as_ref()) {
            return cell.borrow().clone();
        }
        self.locals.get(slot).cloned().unwrap_or(Value::Nil)
    }

    /// Set a local slot (and its cell, when the local was
    /// captured, so closures see the new value).
    fn store_slot(&mut self, slot: usize, value: Value) {
        if let Some(cell) = self.cells.get(slot).and_then(|c| c.as_ref()) {
            *cell.borrow_mut() = value.clone();
        }
        if slot >= self.locals.len() {
            self.locals.resize(slot + 1, Value::Nil);
        }
        self.locals[slot] = value;
    }

    /// The shared cell for a local slot, creating it on first
    /// capture with the slot's current value.
    fn capture_slot(&mut self, slot: usize) -> Rc<RefCell<Value>> {
        if let Some(cell) = self.cells.get(slot).and_then(|c| c.as_ref()) {
            return Rc::clone(cell);
        }
        let cell = Rc::new(RefCell::new(self.load_slot(slot)));
        if slot >= self.cells.len() {
            self.cells.resize(slot + 1, None);
        }
        self.cells[slot] = Some(Rc::clone(&cell));
        cell
    }

    /// Drop the cell of a slot: the next capture of it starts
    /// from a fresh cell with the slot's current value (a block
    /// re-entered, a loop body re-run, a `try` region re-run).
    fn drop_cell(&mut self, slot: usize) {
        if let Some(cell) = self.cells.get_mut(slot) {
            *cell = None;
        }
    }

    /// Read a variable through a `VarRef`: a slot of this
    /// frame, one of its upvalues, or a global by name.
    fn read_var_ref(&self, r: &VarRef) -> Result<Value, RuntimeError> {
        match r {
            VarRef::Local(slot) => Ok(self.load_slot(*slot)),
            VarRef::Upvalue(idx) => self
                .upvalues
                .get(*idx)
                .map(|cell| cell.borrow().clone())
                .ok_or_else(|| RuntimeError::new("VM upvalue out of range (compiler bug)")),
            VarRef::Global(name) => self.env().get(name),
        }
    }

    /// Reset the operand stack and index-argument stack to
    /// what they were when `handler` was pushed.
    fn truncate_to(&mut self, handler: &Handler) {
        self.stack.truncate(handler.stack_len);
        self.index_args.truncate(handler.index_len);
    }

    fn pop(&mut self) -> Result<Value, RuntimeError> {
        self.stack
            .pop()
            .ok_or_else(|| RuntimeError::new("VM stack underflow (compiler bug)"))
    }
}

/// Leave the frame with a `return`, running the innermost
/// `finally` on the way out first (and the outer ones after it,
/// through `EndFinally`).
fn do_return(frame: &mut Frame, value: Value, span: Span) -> Result<Step, RuntimeError> {
    while let Some(mut handler) = frame.handlers.pop() {
        if handler.phase != HandlerPhase::Finally
            && let Some(finally_pc) = handler.finally_pc
        {
            frame.truncate_to(&handler);
            frame.pending = Some(PendingOutcome::Return(value, span));
            handler.phase = HandlerPhase::Finally;
            frame.handlers.push(handler);
            frame.pc = finally_pc;
            return Ok(Step::Continue);
        }
        // No `finally` (or already inside it): the handler is
        // discarded, and the next outer one is tried.
    }
    Ok(Step::Return(value, span))
}

/// Leave the frame with a `yield`, running the innermost
/// `finally` on the way out first, like `do_return`.
fn do_yield(frame: &mut Frame, value: Value, span: Span) -> Result<Step, RuntimeError> {
    while let Some(mut handler) = frame.handlers.pop() {
        if handler.phase != HandlerPhase::Finally
            && let Some(finally_pc) = handler.finally_pc
        {
            frame.truncate_to(&handler);
            frame.pending = Some(PendingOutcome::Yield(value, span));
            handler.phase = HandlerPhase::Finally;
            frame.handlers.push(handler);
            frame.pc = finally_pc;
            return Ok(Step::Continue);
        }
    }
    Ok(Step::Yield(value, span))
}

/// The compiled form of a function, compiling its body on first
/// use when it was defined by the tree-walker (so it has no
/// compiled code yet). The defaults are already evaluated — they
/// live in the value — so the code object carries none.
pub(crate) fn ensure_compiled(function: &Function) -> Rc<CodeObject> {
    if let Some(code) = &function.code {
        return Rc::clone(code);
    }
    compile_function_body(function)
}

impl Interpreter {
    /// Run a whole program on the VM: compile it, then run the
    /// top-level code object in the global scope.
    pub(crate) fn run_vm(
        &mut self,
        program: &Program,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<(), RuntimeError> {
        // A script's top level uses slots.
        let code = compile_program(program, true);
        let globals = self.globals();
        match self.run_frame(
            &code,
            globals,
            Vec::new(),
            Vec::new(),
            None,
            None,
            input,
            out,
        )? {
            FrameOutcome::Normal => Ok(()),
            FrameOutcome::Return(_, span) => {
                Err(RuntimeError::with_span("return outside function", span))
            }
            FrameOutcome::Yield(_, span) => {
                Err(RuntimeError::with_span("yield outside function", span))
            }
        }
    }

    /// The VM half of `run_repl_chunk`: like `run_vm`, with the
    /// chunk's top level name-based so a `let` defines a
    /// global in the environment, visible to later chunks.
    pub(crate) fn run_repl_chunk_vm(
        &mut self,
        program: &Program,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<(), RuntimeError> {
        let code = compile_program(program, false);
        let globals = self.globals();
        match self.run_frame(
            &code,
            globals,
            Vec::new(),
            Vec::new(),
            None,
            None,
            input,
            out,
        )? {
            FrameOutcome::Normal => Ok(()),
            FrameOutcome::Return(_, span) => {
                Err(RuntimeError::with_span("return outside function", span))
            }
            FrameOutcome::Yield(_, span) => {
                Err(RuntimeError::with_span("yield outside function", span))
            }
        }
    }

    /// Evaluate one expression on the VM, in the global scope.
    pub(crate) fn eval_vm(
        &mut self,
        expr: &Expr,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let code = compile_expr_code(expr);
        let globals = self.globals();
        match self.run_frame(
            &code,
            globals,
            Vec::new(),
            Vec::new(),
            None,
            None,
            input,
            out,
        )? {
            FrameOutcome::Return(value, _) => Ok(value),
            FrameOutcome::Normal => Ok(Value::Nil),
            FrameOutcome::Yield(..) => Err(RuntimeError::new("yield outside a generator")),
        }
    }

    /// Run a module's source on the VM and wrap the result as a
    /// module value (the VM half of `run_module_source`).
    pub(crate) fn run_module_source_vm(
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
        // A module's top level keeps its names in the module
        // environment: they are the module's attributes.
        let code = compile_program(&program, false);
        match self.run_frame(
            &code,
            Rc::clone(module_env),
            Vec::new(),
            Vec::new(),
            None,
            None,
            input,
            out,
        ) {
            Ok(FrameOutcome::Normal) => {}
            Ok(FrameOutcome::Return(_, span)) => {
                return Err(RuntimeError::with_span(
                    format!("module '{name}': return outside function"),
                    span,
                ));
            }
            Ok(FrameOutcome::Yield(_, span)) => {
                return Err(RuntimeError::with_span(
                    format!("module '{name}': yield outside function"),
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

    /// The VM half of `call_value`: user functions run on the VM,
    /// builtins and classes go through the shared paths.
    pub(crate) fn call_value_vm(
        &mut self,
        callee: &Value,
        args: CallArgs,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        match callee {
            Value::Function(function) => self.call_vm(function, args, input, out),
            Value::Builtin(builtin) => self.call_builtin(builtin, args, input, out),
            Value::BoundMethod(bound) => self.call_method_vm(bound, args, input, out),
            Value::Class(class) => self.construct_vm(class, args, input, out),
            other => Err(RuntimeError::new(format!(
                "cannot call {}",
                other.type_name()
            ))),
        }
    }

    /// Call a user function on the VM: bind the arguments (and
    /// fail on a wrong call here, like the tree-walker), then
    /// run the body — or return a generator when the body
    /// contains a `yield`.
    pub(crate) fn call_vm(
        &mut self,
        function: &Rc<Function>,
        args: CallArgs,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        // `ensure_compiled` is pure (it only compiles the body
        // when the function has no code yet), so it may run
        // before the argument check, like the generator check
        // below already did.
        let code = ensure_compiled(function);
        if code.is_generator {
            let bindings = bind_arguments(
                &function.params,
                &function.defaults,
                function.rest.as_deref(),
                &args,
                &[],
                None,
            )?;
            return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorData::new(
                Rc::clone(function),
                bindings,
                function.super_class.clone(),
            )))));
        }
        // Fast path: a positional-only call to a function with
        // no defaults and no rest parameter. The argument list
        // becomes the locals directly — parameter i is slot i —
        // reusing its allocation, with no binding table and no
        // parameter-name clones. The arity error is the same one
        // `bind_arguments` reports for this shape of call.
        let mut locals = if args.kwargs.is_empty()
            && function.rest.is_none()
            && function.defaults.iter().all(|d| d.is_none())
        {
            if args.positional.len() != function.params.len() {
                return Err(RuntimeError::new(format!(
                    "expected {} argument(s) but got {}",
                    function.params.len(),
                    args.positional.len()
                )));
            }
            args.positional
        } else {
            let bindings = bind_arguments(
                &function.params,
                &function.defaults,
                function.rest.as_deref(),
                &args,
                &[],
                None,
            )?;
            // Bind by position: parameter i is slot i, the rest
            // parameter (when present) is the slot right after.
            let mut locals = vec![Value::Nil; code.slot_count];
            for (i, (_, value)) in bindings.iter().enumerate() {
                if i < locals.len() {
                    locals[i] = value.clone();
                }
            }
            locals
        };
        locals.resize(code.slot_count, Value::Nil);
        match self.run_frame(
            &code,
            Rc::clone(&function.env),
            function.upvalues.clone(),
            locals,
            function.super_class.clone(),
            None,
            input,
            out,
        )? {
            FrameOutcome::Normal => Ok(Value::Nil),
            FrameOutcome::Return(value, _) => Ok(value),
            FrameOutcome::Yield(..) => Err(RuntimeError::new("yield outside a generator")),
        }
    }

    /// The VM half of `call_method`: like `call_vm`, with the
    /// instance bound as `self` and the defining class as the
    /// `super` context.
    pub(crate) fn call_method_vm(
        &mut self,
        bound: &BoundMethod,
        args: CallArgs,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Value, RuntimeError> {
        let function = &bound.function;
        // `ensure_compiled` is pure (see `call_vm`).
        let code = ensure_compiled(function);
        if code.is_generator {
            let bindings = bind_arguments(
                &function.params,
                &function.defaults,
                function.rest.as_deref(),
                &args,
                &[("self", Value::Instance(Rc::clone(&bound.instance)))],
                Some(&bound.name),
            )?;
            return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorData::new(
                Rc::clone(function),
                bindings,
                Some(Rc::clone(&bound.class)),
            )))));
        }
        // Fast path, as in `call_vm`: a positional-only call to
        // a method with no defaults and no rest parameter. The
        // parser puts `self` first in a method's parameters, so
        // the instance goes into slot 0 and the argument list
        // becomes the remaining locals, reusing its allocation.
        // The arity error is the same one `bind_arguments`
        // reports for this shape of call.
        let mut locals = if args.kwargs.is_empty()
            && function.rest.is_none()
            && function.defaults.iter().all(|d| d.is_none())
            && function.params.first().is_some_and(|p| p == "self")
        {
            if args.positional.len() != function.params.len() - 1 {
                return Err(RuntimeError::new(format!(
                    "method '{}' expects {} argument(s) but got {}",
                    bound.name,
                    function.params.len() - 1,
                    args.positional.len()
                )));
            }
            let mut locals = args.positional;
            locals.insert(0, Value::Instance(Rc::clone(&bound.instance)));
            locals
        } else {
            let bindings = bind_arguments(
                &function.params,
                &function.defaults,
                function.rest.as_deref(),
                &args,
                &[("self", Value::Instance(Rc::clone(&bound.instance)))],
                Some(&bound.name),
            )?;
            let mut locals = vec![Value::Nil; code.slot_count];
            for (i, (_, value)) in bindings.iter().enumerate() {
                if i < locals.len() {
                    locals[i] = value.clone();
                }
            }
            locals
        };
        locals.resize(code.slot_count, Value::Nil);
        match self.run_frame(
            &code,
            Rc::clone(&function.env),
            function.upvalues.clone(),
            locals,
            Some(Rc::clone(&bound.class)),
            None,
            input,
            out,
        )? {
            FrameOutcome::Normal => Ok(Value::Nil),
            FrameOutcome::Return(value, _) => Ok(value),
            FrameOutcome::Yield(..) => Err(RuntimeError::new("yield outside a generator")),
        }
    }

    /// The VM half of `construct`: create the instance, then run
    /// its `init` through the (backend-dispatched) method call.
    pub(crate) fn construct_vm(
        &mut self,
        class: &Rc<ClassDef>,
        args: CallArgs,
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

    /// The VM half of `next_generator`: replay the generator's
    /// body from the start, stopping at the next `yield` after
    /// those already consumed.
    pub(crate) fn next_generator_vm(
        &mut self,
        generator: &Rc<RefCell<GeneratorData>>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Option<Value>, RuntimeError> {
        let (function, bindings, super_context) = {
            let data = generator.borrow();
            (
                Rc::clone(&data.function),
                data.bindings.clone(),
                data.super_context.clone(),
            )
        };
        let target = generator.borrow().consumed + 1;
        let code = ensure_compiled(&function);
        let mut locals = vec![Value::Nil; code.slot_count];
        for (i, (_, value)) in bindings.iter().enumerate() {
            if i < locals.len() {
                locals[i] = value.clone();
            }
        }
        match self.run_frame(
            &code,
            Rc::clone(&function.env),
            function.upvalues.clone(),
            locals,
            super_context,
            Some(target),
            input,
            out,
        )? {
            FrameOutcome::Yield(value, _) => {
                generator.borrow_mut().consumed += 1;
                Ok(Some(value))
            }
            FrameOutcome::Normal | FrameOutcome::Return(..) => Ok(None),
        }
    }

    /// Run one code object to completion in `env`. `locals`
    /// gives the frame's initial slot values (the call
    /// bindings, by position; empty for a top-level frame).
    /// `yield_target` counts `yield`s for a generator replay
    /// (`None` outside one): the frame stops at the
    /// `yield_target`-th `yield`.
    // Eight arguments is the frame's whole context (code, env,
    // upvalues, locals, super, yield target, input, output):
    // bundling them would only move the arity into a struct.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_frame(
        &mut self,
        code: &Rc<CodeObject>,
        env: Rc<Environment>,
        upvalues: Vec<Rc<RefCell<Value>>>,
        locals: Vec<Value>,
        super_class: Option<Rc<ClassDef>>,
        yield_target: Option<usize>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<FrameOutcome, RuntimeError> {
        let mut frame = Frame::new(
            Rc::clone(code),
            env,
            upvalues,
            locals,
            super_class,
            yield_target,
        );
        // Borrow the instructions through this `Rc`, not through
        // the frame: executing an instruction borrows the frame
        // mutably, and cloning the instruction on every step
        // (its `Op` can carry a `String`) was a heap allocation
        // per instruction — the single biggest cost in a loop.
        let code = Rc::clone(&frame.code);
        loop {
            if frame.pc >= code.instructions.len() {
                return Ok(FrameOutcome::Normal);
            }
            let instr = &code.instructions[frame.pc];
            frame.pc += 1;
            match self.exec_instr(&mut frame, instr, input, out) {
                Ok(Step::Continue) => {}
                Ok(Step::Return(value, span)) => {
                    return Ok(FrameOutcome::Return(value, span));
                }
                Ok(Step::Yield(value, span)) => {
                    return Ok(FrameOutcome::Yield(value, span));
                }
                Err(e) => {
                    // The instruction's span is the fallback
                    // position, like the statement span in
                    // `exec_stmt`: a more precise one already set
                    // (an operator, an index) is never overwritten.
                    let e = e.at(instr.span);
                    match self.unwind_error(&mut frame, e) {
                        Unwind::Continue => {}
                        Unwind::Propagate(e) => return Err(e),
                    }
                }
            }
        }
    }

    /// Unwind the handler stack for an error: run the innermost
    /// `except` that can catch it (then its `finally`), or the
    /// innermost `finally` alone, or propagate the error out of
    /// the frame. An error inside a `finally` replaces the
    /// pending outcome and keeps unwinding.
    fn unwind_error(&mut self, frame: &mut Frame, mut err: RuntimeError) -> Unwind {
        while let Some(mut handler) = frame.handlers.pop() {
            match handler.phase {
                HandlerPhase::Finally => {
                    // The `finally` itself failed: its error
                    // replaces whatever it was waiting for.
                    continue;
                }
                HandlerPhase::Try => {
                    if let Some(except_pc) = handler.except_pc {
                        frame.truncate_to(&handler);
                        if let Some(var) = handler.var {
                            // A fresh cell for the except
                            // variable (the tree-walker defines
                            // it in a child scope, so a closure
                            // over it gets its own binding).
                            frame.drop_cell(var);
                            frame.store_slot(var, Value::Str(err.message.clone()));
                        }
                        handler.phase = HandlerPhase::Except;
                        frame.handlers.push(handler);
                        frame.pc = except_pc;
                        return Unwind::Continue;
                    }
                    if let Some(finally_pc) = handler.finally_pc {
                        frame.truncate_to(&handler);
                        frame.pending = Some(PendingOutcome::Error(err));
                        handler.phase = HandlerPhase::Finally;
                        frame.handlers.push(handler);
                        frame.pc = finally_pc;
                        return Unwind::Continue;
                    }
                    // No `except` and no `finally`: the parser
                    // forbids it, but unwind past it anyway.
                }
                HandlerPhase::Except => {
                    if let Some(finally_pc) = handler.finally_pc {
                        frame.truncate_to(&handler);
                        frame.pending = Some(PendingOutcome::Error(err));
                        handler.phase = HandlerPhase::Finally;
                        frame.handlers.push(handler);
                        frame.pc = finally_pc;
                        return Unwind::Continue;
                    }
                    // No `finally`: the error propagates.
                }
            }
            // The handler was popped and not re-pushed: keep
            // unwinding with the same error. (`err` is only
            // moved into a pending outcome on paths that return.)
            let _ = &mut err;
        }
        Unwind::Propagate(err)
    }

    /// Build a `Function` value from a compiled code object:
    /// evaluate the default expressions now, in the defining
    /// scope (Python rule), and capture the current scope. A
    /// default runs in a frame whose upvalues are captured from
    /// this frame (the defaults resolve against the defining
    /// function), and the new function itself captures the
    /// locals its code object lists as upvalues — each a cell
    /// shared with this frame, so a later rebinding is visible
    /// inside the closure and vice versa.
    fn make_function(
        &mut self,
        code: &Rc<CodeObject>,
        frame: &mut Frame,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Function, RuntimeError> {
        let mut defaults = Vec::with_capacity(code.params.len());
        for default in &code.defaults {
            match default {
                Some(default_code) => {
                    let env = Rc::clone(frame.env());
                    let default_upvalues = Self::capture_upvalues(default_code, frame)?;
                    match self.run_frame(
                        default_code,
                        env,
                        default_upvalues,
                        Vec::new(),
                        frame.super_class.clone(),
                        None,
                        input,
                        out,
                    )? {
                        FrameOutcome::Return(value, _) => defaults.push(Some(value)),
                        FrameOutcome::Normal => defaults.push(Some(Value::Nil)),
                        FrameOutcome::Yield(..) => {
                            return Err(RuntimeError::new("yield outside a generator"));
                        }
                    }
                }
                None => defaults.push(None),
            }
        }
        let upvalues = Self::capture_upvalues(code, frame)?;
        Ok(Function {
            params: code.params.clone(),
            defaults,
            rest: code.rest.clone(),
            body: code.body.clone(),
            env: Rc::clone(frame.env()),
            super_class: frame.super_class.clone(),
            upvalues,
            code: Some(Rc::clone(code)),
        })
    }

    /// Capture the cells a code object's upvalues refer to, from
    /// the frame that defines the function (or evaluates its
    /// defaults): a `Local` upvalue is a cell of the frame's
    /// slot, an `Upvalue` one is one of the frame's own upvalues.
    fn capture_upvalues(
        code: &Rc<CodeObject>,
        frame: &mut Frame,
    ) -> Result<Vec<Rc<RefCell<Value>>>, RuntimeError> {
        let mut upvalues = Vec::with_capacity(code.upvalues.len());
        for upvalue in &code.upvalues {
            upvalues.push(match upvalue.kind {
                UpvalueKind::Local(slot) => frame.capture_slot(slot),
                UpvalueKind::Upvalue(idx) => {
                    frame.upvalues.get(idx).cloned().ok_or_else(|| {
                        RuntimeError::new("VM upvalue out of range (compiler bug)")
                    })?
                }
            });
        }
        Ok(upvalues)
    }

    /// Pop a call's arguments off the stack: the keyword values
    /// (one per entry in `kwargs`), then the positional list.
    fn pop_call_args(
        &mut self,
        frame: &mut Frame,
        kwargs: &[(String, Span)],
    ) -> Result<CallArgs, RuntimeError> {
        let mut kw_values = Vec::with_capacity(kwargs.len());
        for _ in 0..kwargs.len() {
            kw_values.push(frame.pop()?);
        }
        kw_values.reverse();
        let positional = match frame.pop()? {
            // Take the list's buffer when this call owns it (the
            // compiler builds a fresh list per call, so it
            // normally is): cloning it was a heap allocation
            // plus a value clone per argument, per call. An
            // aliased list falls back to the clone.
            Value::List(list) => match Rc::try_unwrap(list) {
                Ok(cell) => cell.into_inner(),
                Err(list) => list.borrow().clone(),
            },
            _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
        };
        Ok(CallArgs {
            positional,
            kwargs: kwargs
                .iter()
                .zip(kw_values)
                .map(|((name, span), value)| (name.clone(), value, *span))
                .collect(),
        })
    }

    /// Execute one instruction. Returns `Continue` to fetch the
    /// next one, or a frame exit (`Return` / `Yield`).
    #[allow(clippy::too_many_lines)]
    fn exec_instr(
        &mut self,
        frame: &mut Frame,
        instr: &Instruction,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> Result<Step, RuntimeError> {
        let span = instr.span;
        match &instr.op {
            Op::LoadConst { idx } => {
                let value = match &frame.code.constants[*idx] {
                    Constant::Int(n) => Value::Int(*n),
                    Constant::Float(x) => Value::Float(*x),
                    Constant::Str(s) => Value::Str(s.clone()),
                    Constant::Bool(b) => Value::Bool(*b),
                    Constant::Nil => Value::Nil,
                };
                frame.stack.push(value);
            }
            Op::LoadName { name } => {
                // Borrow the name from the code object: cloning it
                // here was a heap allocation per variable access.
                let value = frame
                    .env()
                    .get(&frame.code.names[*name])
                    .map_err(|e| e.at(span))?;
                frame.stack.push(value);
            }
            Op::StoreName { name } => {
                let value = frame.pop()?;
                frame
                    .env()
                    .assign(&frame.code.names[*name], value)
                    .map_err(|e| e.at(span))?;
            }
            Op::DefineName { name } => {
                let value = frame.pop()?;
                frame.env().define(&frame.code.names[*name], value);
            }
            Op::LoadSlot { slot } => {
                let value = frame.load_slot(*slot);
                frame.stack.push(value);
            }
            Op::StoreSlot { slot } => {
                let value = frame.pop()?;
                frame.store_slot(*slot, value);
            }
            Op::DefineSlot { slot } => {
                let value = frame.pop()?;
                frame.store_slot(*slot, value);
            }
            Op::LoadUpvalue { idx } => {
                let value = frame
                    .upvalues
                    .get(*idx)
                    .ok_or_else(|| RuntimeError::new("VM upvalue out of range (compiler bug)"))?
                    .borrow()
                    .clone();
                frame.stack.push(value);
            }
            Op::StoreUpvalue { idx } => {
                let value = frame.pop()?;
                let cell = frame
                    .upvalues
                    .get(*idx)
                    .ok_or_else(|| RuntimeError::new("VM upvalue out of range (compiler bug)"))?;
                *cell.borrow_mut() = value;
            }
            Op::Pop => {
                frame.pop()?;
            }
            Op::Dup => {
                let value = frame
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("VM stack underflow (compiler bug)"))?;
                frame.stack.push(value);
            }
            Op::Jump { addr } => frame.pc = *addr,
            Op::JumpIfFalse { addr } => {
                if !frame.pop()?.is_truthy() {
                    frame.pc = *addr;
                }
            }
            Op::JumpIfTrue { addr } => {
                if frame.pop()?.is_truthy() {
                    frame.pc = *addr;
                }
            }
            Op::Return => {
                let value = frame.pop()?;
                return do_return(frame, value, span);
            }
            Op::Yield => {
                let value = frame.pop()?;
                let Some(target) = frame.yield_target else {
                    return Err(RuntimeError::with_span("yield outside a generator", span));
                };
                frame.yield_seen += 1;
                if frame.yield_seen == target {
                    return do_yield(frame, value, span);
                }
                // An earlier `yield`: the replay skips it and
                // keeps running from the start.
            }
            Op::Raise { expr_span } => {
                let value = frame.pop()?;
                let Value::Str(message) = value else {
                    return Err(RuntimeError::with_span(
                        "raise requires a Str message",
                        *expr_span,
                    ));
                };
                return Err(RuntimeError::with_span(message, span));
            }
            Op::EnterScope { slots } => {
                // A fresh cell for each slot of the block, so a
                // variable captured here gets a fresh binding
                // per entry (a loop body re-entered, a `try`
                // region re-run).
                for slot in slots.iter() {
                    frame.drop_cell(*slot);
                }
            }
            Op::Print => {
                let value = frame.pop()?;
                let text = self.display(&value, input, out).map_err(|e| e.at(span))?;
                writeln!(out, "{text}")
                    .map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
            }
            Op::BuildList => {
                frame
                    .stack
                    .push(Value::List(Rc::new(RefCell::new(Vec::new()))));
            }
            Op::ListAppend => {
                let value = frame.pop()?;
                match frame.stack.last() {
                    Some(Value::List(list)) => list.borrow_mut().push(value),
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::ListExtend { span } => {
                let value = frame.pop()?;
                let items = spread_items(&value).map_err(|e| e.at(*span))?;
                match frame.stack.last() {
                    Some(Value::List(list)) => list.borrow_mut().extend(items),
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::BuildTuple { n } => {
                let n = *n;
                if frame.stack.len() < n {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                }
                let items = frame.stack.split_off(frame.stack.len() - n);
                frame.stack.push(Value::Tuple(Rc::new(items)));
            }
            Op::BuildDict => {
                frame
                    .stack
                    .push(Value::Dict(Rc::new(RefCell::new(DictData::new()))));
            }
            Op::DictInsert { key_span } => {
                let value = frame.pop()?;
                let key = frame.pop()?;
                let Some(key) = key.to_key() else {
                    return Err(RuntimeError::new(format!(
                        "dict key must be Int, Float, Str or Bool, not {}",
                        key.type_name()
                    ))
                    .at(*key_span));
                };
                match frame.stack.last() {
                    Some(Value::Dict(dict)) => dict.borrow_mut().insert(key, value),
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::BuildSet => {
                frame
                    .stack
                    .push(Value::Set(Rc::new(RefCell::new(Vec::new()))));
            }
            Op::SetInsert { span } => {
                let value = frame.pop()?;
                let Some(key) = value.to_key() else {
                    return Err(RuntimeError::new(format!(
                        "set element must be Int, Float, Str or Bool, not {}",
                        value.type_name()
                    ))
                    .at(*span));
                };
                match frame.stack.last() {
                    Some(Value::Set(set)) => {
                        let mut set = set.borrow_mut();
                        if !set.contains(&key) {
                            set.push(key);
                        }
                    }
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::Call { kwargs } => {
                let args = self.pop_call_args(frame, kwargs)?;
                let callee = frame.pop()?;
                // Inside the VM the backend is always `Vm`, so a
                // plain function goes straight to `call_vm`
                // instead of through the backend dispatch in
                // `call_value` and the value match in
                // `call_value_vm`; anything else keeps the
                // shared path (builtins, methods, classes).
                let value = match callee {
                    Value::Function(function) => self.call_vm(&function, args, input, out),
                    other => self.call_value(&other, args, input, out),
                }
                .map_err(|e| e.at(span))?;
                frame.stack.push(value);
            }
            Op::CallMethod { name, kwargs } => {
                let args = self.pop_call_args(frame, kwargs)?;
                let target = frame.pop()?;
                let value = self
                    .call_method_on(&target, name, args, input, out)
                    .map_err(|e| e.at(span))?;
                frame.stack.push(value);
            }
            Op::CallSuper {
                name,
                kwargs,
                self_ref,
            } => {
                let args = self.pop_call_args(frame, kwargs)?;
                // `super` only means something inside a method:
                // the defining class is the frame's `super`
                // context. (The arguments were already evaluated
                // before this instruction, unlike the tree-walker
                // — see the README's known differences.)
                let Some(defining) = frame.super_class.clone() else {
                    return Err(RuntimeError::new("'super' outside a method").at(span));
                };
                let Some(parent) = &defining.parent else {
                    return Err(RuntimeError::new(format!(
                        "class '{}' has no parent (no super)",
                        defining.name
                    ))
                    .at(span));
                };
                let (function, class) = lookup_method(parent, name).ok_or_else(|| {
                    RuntimeError::new(format!("parent class has no method '{name}'")).at(span)
                })?;
                let Some(self_ref) = self_ref else {
                    return Err(RuntimeError::new("'super' outside a method").at(span));
                };
                let self_value = frame.read_var_ref(self_ref).map_err(|e| e.at(span))?;
                let Value::Instance(instance) = self_value else {
                    return Err(RuntimeError::new("'self' is not an instance").at(span));
                };
                let bound = BoundMethod {
                    name: name.clone(),
                    instance,
                    function,
                    class,
                };
                let value = self
                    .call_method(&bound, args, input, out)
                    .map_err(|e| e.at(span))?;
                frame.stack.push(value);
            }
            Op::GetField { name } => {
                let obj = frame.pop()?;
                let value = self.get_field(&obj, name).map_err(|e| e.at(span))?;
                frame.stack.push(value);
            }
            Op::SetField { name } => {
                let obj = frame.pop()?;
                let value = frame.pop()?;
                match obj {
                    Value::Instance(inst) => {
                        inst.borrow_mut().set_field(name, value);
                    }
                    other => {
                        return Err(RuntimeError::new(format!(
                            "cannot set field '{}' on {}",
                            name,
                            other.type_name()
                        ))
                        .at(span));
                    }
                }
            }
            Op::PushIndexValue { span } => {
                let value = frame.pop()?;
                frame.index_args.push(IndexArg::Value(value, *span));
            }
            Op::PushIndexSlice { spans } => {
                let mut bounds = [None, None, None];
                for i in (0..3).rev() {
                    let value = frame.pop()?;
                    match value {
                        Value::Nil => bounds[i] = None,
                        Value::Int(n) => bounds[i] = Some(n),
                        other => {
                            return Err(RuntimeError::new(format!(
                                "slice bounds must be integers, not {}",
                                other.type_name()
                            ))
                            .at(spans[i].unwrap_or(span)));
                        }
                    }
                }
                frame.index_args.push(IndexArg::Slice(
                    SliceSpec {
                        start: bounds[0],
                        stop: bounds[1],
                        step: bounds[2],
                    },
                    span,
                ));
            }
            Op::GetIndex { n } => {
                let n = *n;
                if frame.index_args.len() < n {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                }
                let args = frame.index_args.split_off(frame.index_args.len() - n);
                let target = frame.pop()?;
                let value = index_value(&target, args, span)?;
                frame.stack.push(value);
            }
            Op::SetIndex { n } => {
                let n = *n;
                if frame.index_args.len() < n {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                }
                let args = frame.index_args.split_off(frame.index_args.len() - n);
                let target = frame.pop()?;
                let value = frame.pop()?;
                assign_index_value(target, args, value, span)?;
            }
            Op::Fail { message } => {
                return Err(RuntimeError::with_span(message.clone(), span));
            }
            Op::Import { name, alias, slot } => {
                let env = Rc::clone(frame.env());
                let module = self
                    .import_module(name, &env, input, out)
                    .map_err(|e| e.at(span))?;
                match slot {
                    Some(slot) => frame.store_slot(*slot, module),
                    None => frame.env().define(alias.as_deref().unwrap_or(name), module),
                }
            }
            Op::DisplayValue => {
                let value = frame.pop()?;
                let text = self.display(&value, input, out).map_err(|e| e.at(span))?;
                frame.stack.push(Value::Str(text));
            }
            Op::StrConcat => {
                let right = frame.pop()?;
                let left = frame.pop()?;
                let (Value::Str(left), Value::Str(right)) = (left, right) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                frame.stack.push(Value::Str(format!("{left}{right}")));
            }
            Op::Neg => {
                let value = frame.pop()?;
                let result = match value {
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
                }
                .map_err(|e| e.at(span))?;
                frame.stack.push(result);
            }
            Op::Not => {
                let value = frame.pop()?;
                frame.stack.push(Value::Bool(!value.is_truthy()));
            }
            Op::Binary { op } => {
                let right = frame.pop()?;
                let left = frame.pop()?;
                let value = binary_values(*op, &left, &right, span)?;
                frame.stack.push(value);
            }
            Op::ForPrep {
                var,
                to,
                step,
                from_span,
                to_span,
            } => {
                let to_val = frame.pop()?;
                let from_val = frame.pop()?;
                let Value::Int(from_int) = from_val else {
                    return Err(RuntimeError::with_span(
                        "for loop bounds must be integers",
                        *from_span,
                    ));
                };
                let Value::Int(to_int) = to_val else {
                    return Err(RuntimeError::with_span(
                        "for loop bounds must be integers",
                        *to_span,
                    ));
                };
                let step_val: i64 = if from_int <= to_int { 1 } else { -1 };
                // Fresh cells for the loop's slots (a loop
                // re-entered — nested loops share the slot
                // numbering — starts from a fresh binding).
                frame.drop_cell(*var);
                frame.drop_cell(*to);
                frame.drop_cell(*step);
                frame.store_slot(*var, Value::Int(from_int));
                frame.store_slot(*to, Value::Int(to_int));
                frame.store_slot(*step, Value::Int(step_val));
            }
            Op::ForCheck { var, to, step, end } => {
                let Value::Int(i) = frame.load_slot(*var) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let Value::Int(to) = frame.load_slot(*to) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let Value::Int(step) = frame.load_slot(*step) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let done = if step > 0 { i > to } else { i < to };
                if done {
                    frame.pc = *end;
                }
            }
            Op::ForNext { var, to, step, end } => {
                let Value::Int(i) = frame.load_slot(*var) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let Value::Int(to) = frame.load_slot(*to) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let Value::Int(step) = frame.load_slot(*step) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                // The next value, if it would pass `to` (or the
                // addition overflows), ends the loop — without
                // defining it, so the variable keeps the last
                // value the body ran with.
                let done = match i.checked_add(step) {
                    Some(next) => {
                        let done = if step > 0 { next > to } else { next < to };
                        if !done {
                            frame.store_slot(*var, Value::Int(next));
                        }
                        done
                    }
                    None => true,
                };
                if done {
                    frame.pc = *end;
                }
            }
            Op::ForInPrep {
                iter,
                idx,
                var,
                iter_span,
            } => {
                let value = frame.pop()?;
                let iter_value = match &value {
                    Value::Generator(_) => value,
                    _ => {
                        let items = iterable_items(&value).map_err(|e| e.at(*iter_span))?;
                        Value::List(Rc::new(RefCell::new(items)))
                    }
                };
                frame.drop_cell(*iter);
                frame.drop_cell(*idx);
                frame.drop_cell(*var);
                frame.store_slot(*iter, iter_value);
                frame.store_slot(*idx, Value::Int(0));
            }
            Op::ForInNext {
                iter,
                idx,
                var,
                end,
            } => {
                let iter_value = frame.load_slot(*iter);
                match iter_value {
                    Value::Generator(generator) => {
                        match self
                            .next_generator(&generator, input, out)
                            .map_err(|e| e.at(span))?
                        {
                            Some(item) => frame.store_slot(*var, item),
                            None => frame.pc = *end,
                        }
                    }
                    Value::List(list) => {
                        let Value::Int(idx_val) = frame.load_slot(*idx) else {
                            return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                        };
                        let next = {
                            let items = list.borrow();
                            if idx_val as usize >= items.len() {
                                None
                            } else {
                                Some(items[idx_val as usize].clone())
                            }
                        };
                        match next {
                            Some(item) => {
                                frame.store_slot(*var, item);
                                frame.store_slot(*idx, Value::Int(idx_val + 1));
                            }
                            None => frame.pc = *end,
                        }
                    }
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::CompPrep {
                iter,
                idx,
                acc,
                kind,
                iter_span,
            } => {
                let value = frame.pop()?;
                let items = iterable_items(&value).map_err(|e| e.at(*iter_span))?;
                let acc_value = match kind {
                    CompKind::List => Value::List(Rc::new(RefCell::new(Vec::new()))),
                    CompKind::Dict => Value::Dict(Rc::new(RefCell::new(DictData::new()))),
                    CompKind::Set => Value::Set(Rc::new(RefCell::new(Vec::new()))),
                };
                // Fresh cells for the comprehension's slots (a
                // comprehension re-evaluated starts from a fresh
                // binding).
                frame.drop_cell(*iter);
                frame.drop_cell(*idx);
                frame.drop_cell(*acc);
                frame.store_slot(*iter, Value::List(Rc::new(RefCell::new(items))));
                frame.store_slot(*idx, Value::Int(0));
                frame.store_slot(*acc, acc_value);
            }
            Op::CompNext { iter, idx, end } => {
                let Value::List(list) = frame.load_slot(*iter) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let Value::Int(idx_val) = frame.load_slot(*idx) else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let next = {
                    let items = list.borrow();
                    if idx_val as usize >= items.len() {
                        None
                    } else {
                        Some(items[idx_val as usize].clone())
                    }
                };
                match next {
                    Some(item) => {
                        frame.stack.push(item);
                        frame.store_slot(*idx, Value::Int(idx_val + 1));
                    }
                    None => frame.pc = *end,
                }
            }
            Op::BindComp { vars, span } => {
                let item = frame.pop()?;
                let bindings = bind_comp_slots(vars, item, *span)?;
                for (slot, value) in bindings {
                    // A fresh cell per item, so a closure over a
                    // comprehension variable gets its own binding
                    // (the tree-walker binds it in a fresh child
                    // scope per item).
                    frame.drop_cell(slot);
                    frame.store_slot(slot, value);
                }
            }
            Op::CompAppend { acc } => {
                let value = frame.pop()?;
                match frame.load_slot(*acc) {
                    Value::List(list) => list.borrow_mut().push(value),
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::CompDictInsert { acc, key_span } => {
                let value = frame.pop()?;
                let key = frame.pop()?;
                let Some(key) = key.to_key() else {
                    return Err(RuntimeError::new(format!(
                        "dict key must be Int, Float, Str or Bool, not {}",
                        key.type_name()
                    ))
                    .at(*key_span));
                };
                match frame.load_slot(*acc) {
                    Value::Dict(dict) => dict.borrow_mut().insert(key, value),
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::CompSetInsert { acc, span } => {
                let value = frame.pop()?;
                let Some(key) = value.to_key() else {
                    return Err(RuntimeError::new(format!(
                        "set element must be Int, Float, Str or Bool, not {}",
                        value.type_name()
                    ))
                    .at(*span));
                };
                match frame.load_slot(*acc) {
                    Value::Set(set) => {
                        let mut set = set.borrow_mut();
                        if !set.contains(&key) {
                            set.push(key);
                        }
                    }
                    _ => return Err(RuntimeError::new("VM stack underflow (compiler bug)")),
                }
            }
            Op::MakeFunction { code } => {
                let function = self.make_function(code, frame, input, out)?;
                frame.stack.push(Value::Function(Rc::new(function)));
            }
            Op::MakeClass {
                name,
                parent,
                methods,
            } => {
                let parent_class = match parent {
                    Some((parent_name, parent_ref)) => {
                        let value = frame.read_var_ref(parent_ref).map_err(|e| e.at(span))?;
                        let Value::Class(class) = value else {
                            return Err(RuntimeError::new(format!(
                                "parent '{parent_name}' is not a class"
                            ))
                            .at(span));
                        };
                        Some(class)
                    }
                    None => None,
                };
                let mut def_methods: Vec<(String, Rc<Function>)> = Vec::new();
                for (method_name, method_code) in methods {
                    let mut function = self.make_function(method_code, frame, input, out)?;
                    // Methods get their `super` context from the
                    // call, not from where the class is defined.
                    function.super_class = None;
                    if let Some(slot) = def_methods.iter_mut().find(|(n, _)| n == method_name) {
                        slot.1 = Rc::new(function);
                    } else {
                        def_methods.push((method_name.clone(), Rc::new(function)));
                    }
                }
                let class = Rc::new(ClassDef {
                    name: name.clone(),
                    parent: parent_class,
                    methods: def_methods,
                });
                // Push the class like MakeFunction pushes the
                // function: the following DefineName pops it and
                // binds the name.
                frame.stack.push(Value::Class(class));
            }
            Op::MatchPattern {
                pattern,
                fail,
                bindings,
                scope_slots,
            } => {
                let value = frame
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| RuntimeError::new("VM stack underflow (compiler bug)"))?;
                match match_pattern(pattern, &value) {
                    Some(bound) => {
                        frame.pop()?;
                        // Fresh cells for the case scope, so a
                        // closure over a binding (or a body
                        // local) gets its own binding per match.
                        for slot in scope_slots.iter() {
                            frame.drop_cell(*slot);
                        }
                        for (slot, (_, value)) in bindings.iter().zip(bound) {
                            frame.store_slot(*slot, value);
                        }
                    }
                    None => frame.pc = *fail,
                }
            }
            Op::PushHandler {
                except,
                finally,
                var,
            } => {
                frame.handlers.push(Handler {
                    except_pc: *except,
                    finally_pc: *finally,
                    var: *var,
                    phase: HandlerPhase::Try,
                    stack_len: frame.stack.len(),
                    index_len: frame.index_args.len(),
                });
            }
            Op::FinishTry { finally } => {
                let Some(mut handler) = frame.handlers.pop() else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                if let Some(finally_pc) = finally {
                    frame.truncate_to(&handler);
                    frame.pending = Some(PendingOutcome::Normal);
                    handler.phase = HandlerPhase::Finally;
                    frame.handlers.push(handler);
                    frame.pc = *finally_pc;
                }
                // Without a `finally` the handler is just popped.
            }
            Op::EndFinally => {
                let Some(handler) = frame.handlers.pop() else {
                    return Err(RuntimeError::new("VM stack underflow (compiler bug)"));
                };
                let _ = handler;
                match frame.pending.take() {
                    Some(PendingOutcome::Normal) | None => {}
                    Some(PendingOutcome::Return(value, span)) => {
                        return do_return(frame, value, span);
                    }
                    Some(PendingOutcome::Yield(value, span)) => {
                        return do_yield(frame, value, span);
                    }
                    Some(PendingOutcome::Error(err)) => match self.unwind_error(frame, err) {
                        Unwind::Continue => {}
                        Unwind::Propagate(err) => return Err(err),
                    },
                }
            }
        }
        Ok(Step::Continue)
    }
}
