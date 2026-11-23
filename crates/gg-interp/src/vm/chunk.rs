// SPDX-License-Identifier: GPL-3.0-or-later
//! Bytecode for the GG virtual machine: opcodes, instructions and
//! code objects.
//!
//! A `CodeObject` is one compiled unit: a script, a module, a
//! function, a method or a lambda. It holds the instructions, the
//! constants they load and the names they look up, plus the
//! function metadata the call machinery needs (parameters,
//! defaults, the AST body the tree-walking fallback runs, and
//! whether the body contains a `yield`).
//!
//! Every instruction carries the `Span` of the source token that
//! generated it, so a runtime error points at the same place the
//! tree-walking interpreter would report (see `RuntimeError::at`).

use std::rc::Rc;

use gg_lexer::Span;
use gg_parser::ast::{BinaryOp, Pattern, Stmt};

/// Where a captured variable lives: a slot of the immediately
/// enclosing frame, or one of its upvalues (a variable the
/// enclosing function itself captured). `MakeFunction` resolves
/// each of a nested function's upvalues against the frame that
/// defines it, so the nested frame reads the same shared cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpvalueKind {
    Local(usize),
    Upvalue(usize),
}

/// One variable a code object captures from an enclosing
/// function: its name (for lookup while compiling) and where to
/// find its cell when the function value is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upvalue {
    pub name: String,
    pub kind: UpvalueKind,
}

/// Where a value is read from at runtime: a slot of this frame,
/// one of its upvalues, or a global by name. `CallSuper` reads
/// `self` this way (`None` when `self` is not in scope — the
/// `super` check fails first then, as outside a method), and
/// `MakeClass` reads its parent class this way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarRef {
    Local(usize),
    Upvalue(usize),
    Global(String),
}

/// One literal a `LoadConst` can push. Code objects are not
/// constants: nested functions travel in `MakeFunction` /
/// `MakeClass` operands instead.
#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Nil,
}

/// One bytecode instruction. The operand meanings:
/// - `name` fields are indexes into the code object's name table
///   (globals and module names, looked up by name).
/// - `slot` fields are indexes into the frame's locals array:
///   a local variable of this function, resolved by the compiler.
/// - `idx` fields on the upvalue ops are indexes into the
///   frame's upvalue array (cells captured from enclosing
///   functions).
/// - `addr` fields are absolute instruction indexes (jump targets),
///   patched by the compiler after the target is emitted.
/// - `kwargs` on a call are the keyword names with their spans, in
///   source order; the values sit on the stack above the positional
///   list when the call runs.
/// - `var`/`to`/`step`/`iter`/`idx`/`acc` on the loop instructions
///   are slot indexes for the hidden locals the loop keeps in its
///   own scope.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Push `constants[idx]`.
    LoadConst { idx: usize },
    /// Push the value of `names[name]` (walks the scope chain).
    LoadName { name: usize },
    /// Pop a value and assign it to `names[name]` (walks the scope
    /// chain; an undefined variable is an error).
    StoreName { name: usize },
    /// Pop a value and define `names[name]` in the current scope.
    DefineName { name: usize },
    /// Push the value of frame slot `slot` (through its cell when
    /// the variable is captured by a nested function).
    LoadSlot { slot: usize },
    /// Pop a value and assign it to frame slot `slot` (through
    /// its cell when captured).
    StoreSlot { slot: usize },
    /// Pop a value and define frame slot `slot` (through its cell
    /// when captured: a later `let` of the same name in the same
    /// scope is visible to closures that captured it).
    DefineSlot { slot: usize },
    /// Push the value of upvalue `idx` (a cell shared with the
    /// enclosing function).
    LoadUpvalue { idx: usize },
    /// Pop a value and assign it to upvalue `idx`.
    StoreUpvalue { idx: usize },
    /// Discard the top of the stack.
    Pop,
    /// Duplicate the top of the stack.
    Dup,
    /// Unconditional jump.
    Jump { addr: usize },
    /// Pop a value; jump when it is falsy.
    JumpIfFalse { addr: usize },
    /// Pop a value; jump when it is truthy.
    JumpIfTrue { addr: usize },
    /// Pop a value and return it from the current frame (running
    /// any `finally` blocks on the way out first).
    Return,
    /// Pop a value and yield it from a generator replay (skipped
    /// when the replay has not reached its target `yield` yet).
    Yield,
    /// Pop a value and raise it as an error: it must be a Str
    /// (`expr_span` is where a non-Str value is reported; the
    /// instruction's own span is the `raise` statement).
    Raise { expr_span: Span },
    /// (Re-)enter a block scope: drop the cells of the listed
    /// slots, so a variable captured here gets a fresh cell per
    /// entry (a loop body re-entered, a `try` region re-run).
    /// Patched by the compiler once the block is compiled.
    EnterScope { slots: Rc<[usize]> },
    /// Pop a value, print it with `display` (a custom `str`
    /// method wins) and a newline.
    Print,
    /// Push an empty list.
    BuildList,
    /// Pop a value and append it to the list underneath it.
    ListAppend,
    /// Pop a list/tuple/array and extend the list underneath it
    /// with its items (a spread; `span` is the `*`).
    ListExtend { span: Span },
    /// Pop `n` values and push them as a tuple.
    BuildTuple { n: usize },
    /// Push an empty dict.
    BuildDict,
    /// Pop a value and a key and insert them into the dict
    /// underneath (`key_span` is where a bad key is reported).
    DictInsert { key_span: Span },
    /// Push an empty set.
    BuildSet,
    /// Pop a value and add it to the set underneath (`span` is
    /// where a non-hashable element is reported).
    SetInsert { span: Span },
    /// Call: pops the keyword values (one per entry in `kwargs`,
    /// in order), then the positional list, then the callee, and
    /// calls it.
    Call { kwargs: Rc<[(String, Span)]> },
    /// Method call: pops the keyword values, the positional list
    /// and the target, and calls `target.name(...)`.
    CallMethod {
        name: String,
        kwargs: Rc<[(String, Span)]>,
    },
    /// `super.name(...)`: pops the keyword values and the
    /// positional list; `self` comes from `self_ref` and the
    /// defining class from the current frame.
    CallSuper {
        name: String,
        kwargs: Rc<[(String, Span)]>,
        self_ref: Option<VarRef>,
    },
    /// Pop an object and push its field `name`.
    GetField { name: String },
    /// Pop an object, then a value, and set the object's field
    /// `name` to the value.
    SetField { name: String },
    /// Pop an index argument off the operand stack and push it
    /// onto the frame's index-argument side stack.
    PushIndexValue { span: Span },
    /// Pop three bounds (start, stop, step; `Nil` means absent)
    /// and push a slice onto the index-argument side stack
    /// (`spans` are the bounds' positions, for bad-bound errors).
    PushIndexSlice { spans: [Option<Span>; 3] },
    /// Pop `n` index arguments and the target, and push the
    /// indexed (or sliced) value.
    GetIndex { n: usize },
    /// Pop the target and the assigned value (the value was
    /// evaluated first, so it sits underneath the target) and
    /// assign through `n` index arguments.
    SetIndex { n: usize },
    /// Always an error: a slice outside `[...]`.
    Fail { message: String },
    /// Import a module and bind it as `alias` (or `name`): to the
    /// frame slot `slot` when it is one (a local), or by name in
    /// the current scope when it is not (a module top level).
    Import {
        name: String,
        alias: Option<String>,
        slot: Option<usize>,
    },
    /// Pop a value and push its display string (a custom `str`
    /// method wins, like `print` and f-strings).
    DisplayValue,
    /// Pop two strings and push their concatenation.
    StrConcat,
    /// Pop a value and push its negation.
    Neg,
    /// Pop a value and push `!` of its truthiness.
    Not,
    /// Pop two values and push `left op right`.
    Binary { op: BinaryOp },
    /// Prepare a numeric `for`: pops `to`, then `from` (both must
    /// be integers, reported at `from_span`/`to_span`), drops the
    /// cells of the loop slots and defines `var`, `to` and `step`
    /// in them.
    ForPrep {
        var: usize,
        to: usize,
        step: usize,
        from_span: Span,
        to_span: Span,
    },
    /// Check a numeric `for`: jump to `end` when the loop
    /// variable has passed `to` (the direction comes from `step`).
    ForCheck {
        var: usize,
        to: usize,
        step: usize,
        end: usize,
    },
    /// Advance a numeric `for`: `var += step` — but only when
    /// the next value has not passed `to`; otherwise jump to
    /// `end` without defining it, so the loop variable keeps the
    /// last value the body ran with (like the tree-walker, whose
    /// counter is a local and whose scope is only (re)defined at
    /// the start of each iteration). A closure that captured the
    /// loop scope therefore sees the last iteration's value, not
    /// one past the end.
    ForNext {
        var: usize,
        to: usize,
        step: usize,
        end: usize,
    },
    /// Prepare a `for x in ...`: pops the iterable, drops the
    /// cells of the loop slots (including `var`, so a loop
    /// re-entered starts from a fresh binding) and defines
    /// `iter` (a snapshot list, or the generator itself) and
    /// `idx` (0) in them.
    ForInPrep {
        iter: usize,
        idx: usize,
        var: usize,
        iter_span: Span,
    },
    /// Advance a `for x in ...`: define `var` with the next item
    /// (pulling lazily from a generator), or jump to `end` when
    /// the iterable is exhausted.
    ForInNext {
        iter: usize,
        idx: usize,
        var: usize,
        end: usize,
    },
    /// Prepare a comprehension: pops the iterable (a generator
    /// is an error, like the tree-walker), drops the cells of the
    /// comprehension slots and defines `iter` (a snapshot list),
    /// `idx` (0) and `acc` (the empty result collection of
    /// `kind`) in them.
    CompPrep {
        iter: usize,
        idx: usize,
        acc: usize,
        kind: CompKind,
        iter_span: Span,
    },
    /// Advance a comprehension: push the next item, or jump to
    /// `end` when the snapshot is exhausted.
    CompNext { iter: usize, idx: usize, end: usize },
    /// Pop an item and bind the comprehension variables to it
    /// (unpacking a pair when there are two), like the
    /// tree-walker's `bind_comp_vars`: `vars` are the slots, one
    /// per variable (two slots for an unpack). The cells of those
    /// slots are dropped first, so each item gets fresh cells.
    BindComp { vars: Rc<[usize]>, span: Span },
    /// Pop a value and append it to the list in `acc`.
    CompAppend { acc: usize },
    /// Pop a value and a key and insert them into the dict in
    /// `acc` (`key_span` is where a bad key is reported).
    CompDictInsert { acc: usize, key_span: Span },
    /// Pop a value and add it to the set in `acc` (`span` is
    /// where a non-hashable element is reported).
    CompSetInsert { acc: usize, span: Span },
    /// Define a function: evaluate its default expressions and
    /// push a function value capturing the current scope.
    MakeFunction { code: Rc<CodeObject> },
    /// Define a class: build it from `methods` (each already
    /// compiled) in the current scope, with its parent read
    /// through the `VarRef` (a slot, an upvalue, or a global
    /// name — the name itself is only for the "parent 'x' is
    /// not a class" error).
    MakeClass {
        name: String,
        parent: Option<(String, VarRef)>,
        methods: Vec<(String, Rc<CodeObject>)>,
    },
    /// Try to match the value on top of the stack against
    /// `pattern`: on a match, pop it, drop the cells of the case
    /// scope's slots (`scope_slots`, patched once the case body
    /// is compiled) and define the bindings into `bindings` (the
    /// slots, in the order `match_pattern` returns them), then
    /// continue; otherwise jump to `fail` (the value stays on the
    /// stack for the next case).
    MatchPattern {
        pattern: Pattern,
        fail: usize,
        bindings: Rc<[usize]>,
        scope_slots: Rc<[usize]>,
    },
    /// Push a try handler: `except` is where an error goes
    /// (`None` when there is no `except`), `finally` is where
    /// every exit path goes, `var` is the slot the error message
    /// is bound to in the `except` scope.
    PushHandler {
        except: Option<usize>,
        finally: Option<usize>,
        var: Option<usize>,
    },
    /// The guarded (or `except`) region finished normally: pop the
    /// handler, and when there is a `finally`, run it with a
    /// "normal" pending outcome.
    FinishTry { finally: Option<usize> },
    /// The `finally` region finished: act on the pending outcome
    /// (continue, return, re-raise, or yield).
    EndFinally,
}

/// The result collection a comprehension accumulates into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompKind {
    List,
    Dict,
    Set,
}

/// One instruction: an opcode and the span of the source token
/// that generated it.
#[derive(Debug, Clone, PartialEq)]
pub struct Instruction {
    pub op: Op,
    pub span: Span,
}

/// A compiled unit: a script, a module, a function, a method, a
/// lambda, or a single expression (defaults and `eval`).
///
/// `defaults` holds one compiled expression per parameter: the
/// default value for parameters with one (evaluated at definition
/// time, Python rule), `None` for required parameters — mirroring
/// `Function::defaults`, which holds the evaluated values.
///
/// `body` is the AST the tree-walking fallback runs for the same
/// unit, so a function keeps working when the backend is switched
/// to the tree-walker after it was defined.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeObject {
    /// `<script>`, a function/method name, `<lambda>`, `<expr>`.
    pub name: String,
    pub params: Vec<String>,
    pub rest: Option<String>,
    pub defaults: Vec<Option<Rc<CodeObject>>>,
    pub body: Vec<Stmt>,
    pub instructions: Vec<Instruction>,
    pub constants: Vec<Constant>,
    pub names: Vec<String>,
    /// How many frame slots this unit uses (parameters first, in
    /// order, then the rest parameter, then the other locals).
    pub slot_count: usize,
    /// The variables this unit captures from enclosing functions,
    /// in the order the frame's upvalue array holds them.
    pub upvalues: Vec<Upvalue>,
    /// True when the body contains a `yield` (a function with
    /// one is a generator: calling it returns a generator value
    /// instead of running the body).
    pub is_generator: bool,
}
