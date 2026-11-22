// SPDX-License-Identifier: GPL-3.0-or-later
//! The GG abstract syntax tree, plus canonical pretty-printing.
//!
//! `Display` reprints a program in canonical form (4-space indent,
//! `end` closing every block) so that parse -> print -> re-parse
//! yields the same AST. Parentheses are added only where the
//! precedence rules need them.
//!
//! Phase 16: every node carries the `Span` of the token that opens
//! or identifies it — the `if` keyword for an `if`, the operator
//! for a binary operation, the identifier for a variable, the `[`
//! for an index — so runtime errors can say where they happened.
//! A span is metadata, not semantics: `PartialEq` compares the
//! structure only and ignores spans, so two nodes written in
//! different places are equal when they mean the same thing.
//! Span correctness is covered by dedicated parser tests
//! (`spans_*` in `parser.rs`); `Span::dummy()` is the placeholder
//! for nodes built by hand in tests.

use std::fmt;

use gg_lexer::Span;

/// A whole GG program: a list of top-level statements.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_stmts(f, &self.stmts, 0)
    }
}

/// One pattern in a `case` of a `match`: a literal value, `_`
/// (matches anything), a binding (a bare identifier, bound to the
/// value), or a list/tuple pattern matching the elements one by
/// one (also nested). A literal is the expression as written — a
/// number, a string, `true`/`false`, or a negated number — so it
/// still carries its span; equality ignores spans, like
/// everywhere else in the AST.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Expr),
    Wildcard,
    Binding(String),
    List(Vec<Pattern>),
    Tuple(Vec<Pattern>),
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pattern::Literal(expr) => write!(f, "{expr}"),
            Pattern::Wildcard => write!(f, "_"),
            Pattern::Binding(name) => write!(f, "{name}"),
            Pattern::List(patterns) => {
                write!(f, "[")?;
                for (i, pattern) in patterns.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{pattern}")?;
                }
                write!(f, "]")
            }
            Pattern::Tuple(patterns) => {
                write!(f, "(")?;
                for (i, pattern) in patterns.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{pattern}")?;
                }
                // A one-element tuple keeps its trailing comma,
                // like a one-element tuple literal.
                if patterns.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")
            }
        }
    }
}

/// One `case` of a `match`: a pattern, an optional `if` guard
/// (evaluated with the pattern's bindings in scope) and the body.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
}

/// One statement. The `span` is the token that opens it: the
/// keyword for `let`/`if`/`while`/`for`/`fn`/`class`/`import`/
/// `return`/`print`, the target expression for an assignment, the
/// expression itself for an expression statement.
#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        value: Expr,
        span: Span,
    },
    Assign {
        name: String,
        value: Expr,
        span: Span,
    },
    /// Assignment through an index: `xs[0] = 99`, `m[0][1] = 2`.
    IndexAssign {
        target: Expr,
        value: Expr,
        span: Span,
    },
    /// Assignment through a field: `p.nome = "Giuliano"`,
    /// `self.eta = eta`. Sets a field on the instance — never a
    /// method (a field with the same name as a method shadows it).
    FieldAssign {
        target: Expr,
        value: Expr,
        span: Span,
    },
    Print {
        expr: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_block: Vec<Stmt>,
        else_block: Option<Vec<Stmt>>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    For {
        var: String,
        from: Expr,
        to: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    /// Iteration over a collection: `for x in xs then ... end`.
    /// The iterable is evaluated once; the loop variable is scoped
    /// to the body, like the numeric `for`.
    ForIn {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    Fn {
        name: String,
        params: Vec<ParamDef>,
        rest: Option<String>,
        body: Vec<Stmt>,
        span: Span,
    },
    /// A class definition: `class Persona ... end`, optionally
    /// `class Studente extends Persona ... end`. The body holds
    /// only method definitions.
    Class {
        name: String,
        parent: Option<String>,
        methods: Vec<MethodDef>,
        span: Span,
    },
    /// A module import: `import math` loads `math.gg` and binds
    /// the name `math`; `import math as m` binds it as `m`
    /// instead. There is no `from x import y` (yet).
    Import {
        name: String,
        alias: Option<String>,
        span: Span,
    },
    Return {
        expr: Option<Expr>,
        span: Span,
    },
    /// Error handling: `try then ... except e then ... finally then ... end`.
    /// The `except` clause is optional when a `finally` is present (and
    /// vice versa); at least one of the two is required. The bound
    /// variable receives the error's message as a Str. The `finally`
    /// block runs on every exit path — normal, error, and `return`.
    Try {
        body: Vec<Stmt>,
        except_var: Option<String>,
        except_body: Option<Vec<Stmt>>,
        finally_body: Option<Vec<Stmt>>,
        span: Span,
    },
    /// Raise a runtime error: `raise "message"`. The message must
    /// be a Str; the error points at the `raise`.
    Raise {
        expr: Expr,
        span: Span,
    },
    /// Pattern matching: `match x then case 1 then ... case _ then
    /// ... end`. The first case whose pattern matches (and whose
    /// guard, if any, is true) runs; when none matches, nothing
    /// happens. Match is a statement only (no match expression
    /// yet).
    Match {
        scrutinee: Expr,
        cases: Vec<Case>,
        span: Span,
    },
    /// Yield a value from a generator function: `yield expr`.
    /// A function whose body contains a `yield` is a generator:
    /// calling it returns a generator value instead of running
    /// the body; `next(gen)` resumes it up to the next `yield`.
    /// A bare `yield` yields `nil`, like a bare `return`.
    Yield {
        expr: Option<Expr>,
        span: Span,
    },
    ExprStmt {
        expr: Expr,
        span: Span,
    },
}

impl Stmt {
    /// The position of the token that opens this statement.
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::IndexAssign { span, .. }
            | Stmt::FieldAssign { span, .. }
            | Stmt::Print { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::For { span, .. }
            | Stmt::ForIn { span, .. }
            | Stmt::Fn { span, .. }
            | Stmt::Class { span, .. }
            | Stmt::Import { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Try { span, .. }
            | Stmt::Raise { span, .. }
            | Stmt::Match { span, .. }
            | Stmt::Yield { span, .. }
            | Stmt::ExprStmt { span, .. } => *span,
        }
    }
}

// Spans are metadata: two statements are equal when their
// structure is equal, wherever they were written.
impl PartialEq for Stmt {
    fn eq(&self, other: &Self) -> bool {
        use Stmt::*;
        match (self, other) {
            (
                Let {
                    name: n1,
                    value: v1,
                    ..
                },
                Let {
                    name: n2,
                    value: v2,
                    ..
                },
            ) => n1 == n2 && v1 == v2,
            (
                Assign {
                    name: n1,
                    value: v1,
                    ..
                },
                Assign {
                    name: n2,
                    value: v2,
                    ..
                },
            ) => n1 == n2 && v1 == v2,
            (
                IndexAssign {
                    target: t1,
                    value: v1,
                    ..
                },
                IndexAssign {
                    target: t2,
                    value: v2,
                    ..
                },
            ) => t1 == t2 && v1 == v2,
            (
                FieldAssign {
                    target: t1,
                    value: v1,
                    ..
                },
                FieldAssign {
                    target: t2,
                    value: v2,
                    ..
                },
            ) => t1 == t2 && v1 == v2,
            (Print { expr: e1, .. }, Print { expr: e2, .. }) => e1 == e2,
            (
                If {
                    cond: c1,
                    then_block: t1,
                    else_block: e1,
                    ..
                },
                If {
                    cond: c2,
                    then_block: t2,
                    else_block: e2,
                    ..
                },
            ) => c1 == c2 && t1 == t2 && e1 == e2,
            (
                While {
                    cond: c1, body: b1, ..
                },
                While {
                    cond: c2, body: b2, ..
                },
            ) => c1 == c2 && b1 == b2,
            (
                For {
                    var: v1,
                    from: f1,
                    to: t1,
                    body: b1,
                    ..
                },
                For {
                    var: v2,
                    from: f2,
                    to: t2,
                    body: b2,
                    ..
                },
            ) => v1 == v2 && f1 == f2 && t1 == t2 && b1 == b2,
            (
                ForIn {
                    var: v1,
                    iter: i1,
                    body: b1,
                    ..
                },
                ForIn {
                    var: v2,
                    iter: i2,
                    body: b2,
                    ..
                },
            ) => v1 == v2 && i1 == i2 && b1 == b2,
            (
                Fn {
                    name: n1,
                    params: p1,
                    rest: r1,
                    body: b1,
                    ..
                },
                Fn {
                    name: n2,
                    params: p2,
                    rest: r2,
                    body: b2,
                    ..
                },
            ) => n1 == n2 && p1 == p2 && r1 == r2 && b1 == b2,
            (
                Class {
                    name: n1,
                    parent: p1,
                    methods: m1,
                    ..
                },
                Class {
                    name: n2,
                    parent: p2,
                    methods: m2,
                    ..
                },
            ) => n1 == n2 && p1 == p2 && m1 == m2,
            (
                Import {
                    name: n1,
                    alias: a1,
                    ..
                },
                Import {
                    name: n2,
                    alias: a2,
                    ..
                },
            ) => n1 == n2 && a1 == a2,
            (Return { expr: e1, .. }, Return { expr: e2, .. }) => e1 == e2,
            (
                Try {
                    body: b1,
                    except_var: v1,
                    except_body: e1,
                    finally_body: f1,
                    ..
                },
                Try {
                    body: b2,
                    except_var: v2,
                    except_body: e2,
                    finally_body: f2,
                    ..
                },
            ) => b1 == b2 && v1 == v2 && e1 == e2 && f1 == f2,
            (Raise { expr: e1, .. }, Raise { expr: e2, .. }) => e1 == e2,
            (
                Match {
                    scrutinee: s1,
                    cases: c1,
                    ..
                },
                Match {
                    scrutinee: s2,
                    cases: c2,
                    ..
                },
            ) => s1 == s2 && c1 == c2,
            (Yield { expr: e1, .. }, Yield { expr: e2, .. }) => e1 == e2,
            (ExprStmt { expr: e1, .. }, ExprStmt { expr: e2, .. }) => e1 == e2,
            _ => false,
        }
    }
}

/// One method definition inside a `class` body. The first
/// parameter must be named `self` (enforced by the parser).
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDef {
    pub name: String,
    pub params: Vec<ParamDef>,
    pub rest: Option<String>,
    pub body: Vec<Stmt>,
}

/// One parameter of a function, lambda or method definition:
/// `name`, or `name = default` — a default expression evaluated
/// at definition time (Python rule), never at the call.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDef {
    pub name: String,
    pub default: Option<Expr>,
}

/// One argument in a call: a plain positional (`f(1)`), a spread
/// (`f(*xs)`, expanding a list/tuple/array into positionals), or
/// a keyword argument (`f(k = 2)`). The span is the argument
/// expression itself for a positional, the `*` for a spread, and
/// the keyword name for a keyword argument.
#[derive(Debug, Clone)]
pub enum Arg {
    Positional(Expr),
    Spread {
        expr: Expr,
        span: Span,
    },
    Keyword {
        name: String,
        value: Expr,
        span: Span,
    },
}

impl Arg {
    /// The position that identifies this argument.
    pub fn span(&self) -> Span {
        match self {
            Arg::Positional(expr) => expr.span(),
            Arg::Spread { span, .. } | Arg::Keyword { span, .. } => *span,
        }
    }
}

// Spans are metadata: two arguments are equal when their
// structure is equal, wherever they were written.
impl PartialEq for Arg {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Arg::Positional(e1), Arg::Positional(e2)) => e1 == e2,
            (Arg::Spread { expr: e1, .. }, Arg::Spread { expr: e2, .. }) => e1 == e2,
            (
                Arg::Keyword {
                    name: n1,
                    value: v1,
                    ..
                },
                Arg::Keyword {
                    name: n2,
                    value: v2,
                    ..
                },
            ) => n1 == n2 && v1 == v2,
            _ => false,
        }
    }
}

/// One part of an f-string: literal text, or an interpolated
/// expression whose value is rendered with the same Display as
/// `print`.
#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Text(String),
    Expr(Expr),
}

/// One expression. The `span` is the token that identifies the
/// node: the literal itself for `Int`/`Float`/`Str`/`Bool`/`Ident`,
/// the operator for `Unary`/`Binary`, the function name for
/// `Call`, the `[` for `Index`, the first `:` for `Slice`, the
/// field or method name for `Field`/`MethodCall`/`SuperCall`,
/// the opening bracket for `List`/`Tuple`/`Dict`/`Set`, the
/// `self`/`super` keyword for `SelfExpr`/`SuperCall`, the `f`
/// for `FString`.
#[derive(Debug, Clone)]
pub enum Expr {
    Int {
        value: i64,
        span: Span,
    },
    Float {
        value: f64,
        span: Span,
    },
    Str {
        value: String,
        span: Span,
    },
    /// An f-string: `f"ciao {nome}"` — literal text parts and
    /// interpolated expressions, evaluated left to right and
    /// concatenated. The span is the `f` that opens it.
    FString {
        parts: Vec<FStringPart>,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    Ident {
        name: String,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    Call {
        name: String,
        args: Vec<Arg>,
        span: Span,
    },
    /// A list literal: `[1, 2, 3]`, `[]`, nested and mixed.
    List {
        elements: Vec<Expr>,
        span: Span,
    },
    /// A tuple literal: `(1, "a")`, `()`, `(1,)` (the trailing
    /// comma is what makes a one-element tuple — `(1)` is just
    /// grouping). Tuples are immutable.
    Tuple {
        elements: Vec<Expr>,
        span: Span,
    },
    /// A dict literal: `{"nome": "Giuliano"}`, `{}`. Keys are
    /// validated at runtime (only Int/Float/Str/Bool).
    Dict {
        entries: Vec<(Expr, Expr)>,
        span: Span,
    },
    /// A set literal: `{1, 2, 3}`. `{}` is the empty dict, not an
    /// empty set (Python convention).
    Set {
        elements: Vec<Expr>,
        span: Span,
    },
    /// A list comprehension: `[x * 2 for x in xs]`, with an
    /// optional filter: `[x for x in xs if x > 0]`. One `for`
    /// clause only (no nested fors in this phase). The loop
    /// variable(s) are scoped to the comprehension; `vars` has
    /// one element, or two when unpacking pairs (`for k, v in
    /// pairs`, each item a 2-tuple / 2-list / 2-element 1D array).
    ListComp {
        elem: Box<Expr>,
        vars: Vec<String>,
        iter: Box<Expr>,
        cond: Option<Box<Expr>>,
        span: Span,
    },
    /// A dict comprehension: `{k: v for k, v in pairs}`, or with
    /// a single variable: `{x: x * 2 for x in xs}`. Duplicate
    /// keys: the last one wins (Python rule).
    DictComp {
        key: Box<Expr>,
        value: Box<Expr>,
        vars: Vec<String>,
        iter: Box<Expr>,
        cond: Option<Box<Expr>>,
        span: Span,
    },
    /// A set comprehension: `{x for x in xs}`, `{x for x in xs
    /// if x > 0}`. Duplicates are dropped, like a set literal.
    SetComp {
        elem: Box<Expr>,
        vars: Vec<String>,
        iter: Box<Expr>,
        cond: Option<Box<Expr>>,
        span: Span,
    },
    /// A lambda: `fn(x) x * 2 end` — `fn` without a name, as an
    /// expression. The body is a single expression; its value is
    /// the return value (no `return` inside, no statements).
    /// Captures the defining environment, like a named function.
    Lambda {
        params: Vec<ParamDef>,
        rest: Option<String>,
        body: Box<Expr>,
        span: Span,
    },
    /// A call of an arbitrary expression: `fn(x) x end(2)`,
    /// `f()(3)`. Plain `name(args)` calls are `Expr::Call`.
    CallExpr {
        callee: Box<Expr>,
        args: Vec<Arg>,
        span: Span,
    },
    /// Indexing: `xs[0]`, `f()[0]`, `xs[0][1]`, and multi-index
    /// `a[i, j]` (one index per dimension of an array). Binds as
    /// tightly as a call (it is a postfix operator, see the parser).
    Index {
        target: Box<Expr>,
        indices: Vec<Expr>,
        span: Span,
    },
    /// A slice: `start:stop:step` inside brackets (`a[1:4]`,
    /// `a[:3]`, `a[::2]`). Each bound is optional (`None` means
    /// "the default for that end"). Only valid as an index item —
    /// evaluating one on its own is a runtime error.
    Slice {
        start: Option<Box<Expr>>,
        stop: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
        span: Span,
    },
    /// The `self` keyword: the instance a method was called on.
    SelfExpr {
        span: Span,
    },
    /// Field access: `p.nome`, `self.eta`. On an instance it reads
    /// a field first, then a method (bound); on a class it reads an
    /// unbound method.
    Field {
        target: Box<Expr>,
        name: String,
        span: Span,
    },
    /// A method call: `p.saluta("ciao")`, `self.nome()`. The
    /// instance is passed as `self` implicitly.
    MethodCall {
        target: Box<Expr>,
        name: String,
        args: Vec<Arg>,
        span: Span,
    },
    /// A call into the parent class: `super.init(nome, eta)`,
    /// `super.saluta()`. Only valid inside a method.
    SuperCall {
        name: String,
        args: Vec<Arg>,
        span: Span,
    },
}

impl Expr {
    /// The position of the token that identifies this node.
    pub fn span(&self) -> Span {
        match self {
            Expr::Int { span, .. }
            | Expr::Float { span, .. }
            | Expr::Str { span, .. }
            | Expr::FString { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Ident { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Call { span, .. }
            | Expr::List { span, .. }
            | Expr::Tuple { span, .. }
            | Expr::Dict { span, .. }
            | Expr::Set { span, .. }
            | Expr::ListComp { span, .. }
            | Expr::DictComp { span, .. }
            | Expr::SetComp { span, .. }
            | Expr::Lambda { span, .. }
            | Expr::CallExpr { span, .. }
            | Expr::Index { span, .. }
            | Expr::Slice { span, .. }
            | Expr::SelfExpr { span, .. }
            | Expr::Field { span, .. }
            | Expr::MethodCall { span, .. }
            | Expr::SuperCall { span, .. } => *span,
        }
    }
}

// Spans are metadata: two expressions are equal when their
// structure is equal, wherever they were written.
impl PartialEq for Expr {
    fn eq(&self, other: &Self) -> bool {
        use Expr::*;
        match (self, other) {
            (Int { value: v1, .. }, Int { value: v2, .. }) => v1 == v2,
            (Float { value: v1, .. }, Float { value: v2, .. }) => v1 == v2,
            (Str { value: v1, .. }, Str { value: v2, .. }) => v1 == v2,
            (FString { parts: p1, .. }, FString { parts: p2, .. }) => p1 == p2,
            (Bool { value: v1, .. }, Bool { value: v2, .. }) => v1 == v2,
            (Ident { name: n1, .. }, Ident { name: n2, .. }) => n1 == n2,
            (
                Unary {
                    op: o1, expr: e1, ..
                },
                Unary {
                    op: o2, expr: e2, ..
                },
            ) => o1 == o2 && e1 == e2,
            (
                Binary {
                    op: o1,
                    left: l1,
                    right: r1,
                    ..
                },
                Binary {
                    op: o2,
                    left: l2,
                    right: r2,
                    ..
                },
            ) => o1 == o2 && l1 == l2 && r1 == r2,
            (
                Call {
                    name: n1, args: a1, ..
                },
                Call {
                    name: n2, args: a2, ..
                },
            ) => n1 == n2 && a1 == a2,
            (List { elements: e1, .. }, List { elements: e2, .. }) => e1 == e2,
            (Tuple { elements: e1, .. }, Tuple { elements: e2, .. }) => e1 == e2,
            (Dict { entries: e1, .. }, Dict { entries: e2, .. }) => e1 == e2,
            (Set { elements: e1, .. }, Set { elements: e2, .. }) => e1 == e2,
            (
                ListComp {
                    elem: e1,
                    vars: v1,
                    iter: i1,
                    cond: c1,
                    ..
                },
                ListComp {
                    elem: e2,
                    vars: v2,
                    iter: i2,
                    cond: c2,
                    ..
                },
            ) => e1 == e2 && v1 == v2 && i1 == i2 && c1 == c2,
            (
                DictComp {
                    key: k1,
                    value: v1,
                    vars: vs1,
                    iter: i1,
                    cond: c1,
                    ..
                },
                DictComp {
                    key: k2,
                    value: v2,
                    vars: vs2,
                    iter: i2,
                    cond: c2,
                    ..
                },
            ) => k1 == k2 && v1 == v2 && vs1 == vs2 && i1 == i2 && c1 == c2,
            (
                SetComp {
                    elem: e1,
                    vars: v1,
                    iter: i1,
                    cond: c1,
                    ..
                },
                SetComp {
                    elem: e2,
                    vars: v2,
                    iter: i2,
                    cond: c2,
                    ..
                },
            ) => e1 == e2 && v1 == v2 && i1 == i2 && c1 == c2,
            (
                Lambda {
                    params: p1,
                    rest: r1,
                    body: b1,
                    ..
                },
                Lambda {
                    params: p2,
                    rest: r2,
                    body: b2,
                    ..
                },
            ) => p1 == p2 && r1 == r2 && b1 == b2,
            (
                CallExpr {
                    callee: c1,
                    args: a1,
                    ..
                },
                CallExpr {
                    callee: c2,
                    args: a2,
                    ..
                },
            ) => c1 == c2 && a1 == a2,
            (
                Index {
                    target: t1,
                    indices: i1,
                    ..
                },
                Index {
                    target: t2,
                    indices: i2,
                    ..
                },
            ) => t1 == t2 && i1 == i2,
            (
                Slice {
                    start: s1,
                    stop: p1,
                    step: t1,
                    ..
                },
                Slice {
                    start: s2,
                    stop: p2,
                    step: t2,
                    ..
                },
            ) => s1 == s2 && p1 == p2 && t1 == t2,
            (SelfExpr { .. }, SelfExpr { .. }) => true,
            (
                Field {
                    target: t1,
                    name: n1,
                    ..
                },
                Field {
                    target: t2,
                    name: n2,
                    ..
                },
            ) => t1 == t2 && n1 == n2,
            (
                MethodCall {
                    target: t1,
                    name: n1,
                    args: a1,
                    ..
                },
                MethodCall {
                    target: t2,
                    name: n2,
                    args: a2,
                    ..
                },
            ) => t1 == t2 && n1 == n2 && a1 == a2,
            (
                SuperCall {
                    name: n1, args: a1, ..
                },
                SuperCall {
                    name: n2, args: a2, ..
                },
            ) => n1 == n2 && a1 == a2,
            _ => false,
        }
    }
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

impl BinaryOp {
    /// Binding strength: higher binds tighter.
    fn prec(&self) -> u8 {
        match self {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::Eq | BinaryOp::NotEq => 3,
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => 4,
            BinaryOp::Add | BinaryOp::Sub => 5,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 6,
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "==",
            BinaryOp::NotEq => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::LtEq => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::GtEq => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }
}

impl Expr {
    /// Binding strength of the expression itself (primaries bind tightest).
    fn prec(&self) -> u8 {
        match self {
            Expr::Binary { op, .. } => op.prec(),
            Expr::Unary { .. } => 7,
            _ => 8,
        }
    }

    /// Print the expression, adding parentheses when its precedence is
    /// lower than the surrounding context requires.
    fn fmt_prec(&self, f: &mut fmt::Formatter<'_>, parent_prec: u8) -> fmt::Result {
        let need_parens = self.prec() < parent_prec;
        if need_parens {
            write!(f, "(")?;
        }
        match self {
            Expr::Int { value, .. } => write!(f, "{value}")?,
            Expr::Float { value, .. } => write!(f, "{}", format_float(*value))?,
            Expr::Str { value, .. } => write!(f, "\"{}\"", escape_str(value))?,
            Expr::FString { parts, .. } => {
                write!(f, "f\"")?;
                for part in parts {
                    match part {
                        FStringPart::Text(text) => write!(f, "{}", escape_fstring_text(text))?,
                        FStringPart::Expr(expr) => write!(f, "{{{expr}}}")?,
                    }
                }
                write!(f, "\"")?;
            }
            Expr::Bool { value, .. } => write!(f, "{value}")?,
            Expr::Ident { name, .. } => write!(f, "{name}")?,
            Expr::Unary { op, expr, .. } => {
                match op {
                    UnaryOp::Neg => write!(f, "-")?,
                    UnaryOp::Not => write!(f, "not ")?,
                }
                expr.fmt_prec(f, 7)?;
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                // Left-associative: the left operand may share the same
                // precedence, the right one must bind tighter.
                left.fmt_prec(f, op.prec())?;
                write!(f, " {} ", op.symbol())?;
                right.fmt_prec(f, op.prec() + 1)?;
            }
            Expr::Call { name, args, .. } => {
                write!(f, "{name}(")?;
                fmt_args(f, args)?;
                write!(f, ")")?;
            }
            Expr::List { elements, .. } => {
                write!(f, "[")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_prec(f, 0)?;
                }
                write!(f, "]")?;
            }
            Expr::Tuple { elements, .. } => {
                write!(f, "(")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_prec(f, 0)?;
                }
                // A one-element tuple keeps its trailing comma, so it
                // re-parses as a tuple and not as grouping.
                if elements.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")?;
            }
            Expr::Dict { entries, .. } => {
                write!(f, "{{")?;
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    key.fmt_prec(f, 0)?;
                    write!(f, ": ")?;
                    value.fmt_prec(f, 0)?;
                }
                write!(f, "}}")?;
            }
            Expr::Set { elements, .. } => {
                write!(f, "{{")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_prec(f, 0)?;
                }
                write!(f, "}}")?;
            }
            Expr::ListComp {
                elem,
                vars,
                iter,
                cond,
                ..
            } => {
                write!(f, "[")?;
                elem.fmt_prec(f, 0)?;
                write_comp_clause(f, vars, iter, cond)?;
                write!(f, "]")?;
            }
            Expr::DictComp {
                key,
                value,
                vars,
                iter,
                cond,
                ..
            } => {
                write!(f, "{{")?;
                key.fmt_prec(f, 0)?;
                write!(f, ": ")?;
                value.fmt_prec(f, 0)?;
                write_comp_clause(f, vars, iter, cond)?;
                write!(f, "}}")?;
            }
            Expr::SetComp {
                elem,
                vars,
                iter,
                cond,
                ..
            } => {
                write!(f, "{{")?;
                elem.fmt_prec(f, 0)?;
                write_comp_clause(f, vars, iter, cond)?;
                write!(f, "}}")?;
            }
            Expr::Lambda {
                params, rest, body, ..
            } => {
                write!(f, "fn(")?;
                fmt_params(f, params, rest)?;
                write!(f, ") ")?;
                body.fmt_prec(f, 0)?;
                write!(f, " end")?;
            }
            Expr::CallExpr { callee, args, .. } => {
                callee.fmt_prec(f, 8)?;
                write!(f, "(")?;
                fmt_args(f, args)?;
                write!(f, ")")?;
            }
            Expr::Index {
                target, indices, ..
            } => {
                target.fmt_prec(f, 8)?;
                write!(f, "[")?;
                for (i, index) in indices.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    index.fmt_prec(f, 0)?;
                }
                write!(f, "]")?;
            }
            Expr::Slice {
                start, stop, step, ..
            } => {
                if let Some(start) = start {
                    start.fmt_prec(f, 0)?;
                }
                write!(f, ":")?;
                if let Some(stop) = stop {
                    stop.fmt_prec(f, 0)?;
                }
                if let Some(step) = step {
                    write!(f, ":")?;
                    step.fmt_prec(f, 0)?;
                }
            }
            Expr::SelfExpr { .. } => write!(f, "self")?,
            Expr::Field { target, name, .. } => {
                target.fmt_prec(f, 8)?;
                write!(f, ".{name}")?;
            }
            Expr::MethodCall {
                target, name, args, ..
            } => {
                target.fmt_prec(f, 8)?;
                write!(f, ".{name}(")?;
                fmt_args(f, args)?;
                write!(f, ")")?;
            }
            Expr::SuperCall { name, args, .. } => {
                write!(f, "super.{name}(")?;
                fmt_args(f, args)?;
                write!(f, ")")?;
            }
        }
        if need_parens {
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_prec(f, 0)
    }
}

/// Print a float so it always re-parses as a float: `1.0`, not `1`.
fn format_float(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

/// Escape a string for re-printing inside double quotes.
fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape literal f-string text for re-printing: the string
/// escapes, plus `{{`/`}}` for literal braces.
fn escape_fstring_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// The shared tail of a comprehension: ` for x in xs` plus an
/// optional ` if cond`.
/// Print a parameter list: `a, b = 1, *rest` — defaults with
/// ` = `, the rest parameter with a leading `*`.
fn fmt_params(
    f: &mut fmt::Formatter<'_>,
    params: &[ParamDef],
    rest: &Option<String>,
) -> fmt::Result {
    for (i, param) in params.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}", param.name)?;
        if let Some(default) = &param.default {
            write!(f, " = ")?;
            default.fmt_prec(f, 0)?;
        }
    }
    if let Some(rest) = rest {
        if !params.is_empty() {
            write!(f, ", ")?;
        }
        write!(f, "*{rest}")?;
    }
    Ok(())
}

/// Print a call's argument list: `1, *xs, k = 2` — positionals
/// plain, spreads with a leading `*`, keywords as `name = value`.
fn fmt_args(f: &mut fmt::Formatter<'_>, args: &[Arg]) -> fmt::Result {
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        match arg {
            Arg::Positional(expr) => expr.fmt_prec(f, 0)?,
            Arg::Spread { expr, .. } => {
                write!(f, "*")?;
                expr.fmt_prec(f, 0)?;
            }
            Arg::Keyword { name, value, .. } => {
                write!(f, "{name} = ")?;
                value.fmt_prec(f, 0)?;
            }
        }
    }
    Ok(())
}

fn write_comp_clause(
    f: &mut fmt::Formatter<'_>,
    vars: &[String],
    iter: &Expr,
    cond: &Option<Box<Expr>>,
) -> fmt::Result {
    write!(f, " for {} in ", vars.join(", "))?;
    iter.fmt_prec(f, 0)?;
    if let Some(cond) = cond {
        write!(f, " if ")?;
        cond.fmt_prec(f, 0)?;
    }
    Ok(())
}

/// Write a list of statements, one per line, at the given indent level.
fn write_stmts(f: &mut fmt::Formatter<'_>, stmts: &[Stmt], indent: usize) -> fmt::Result {
    for (i, stmt) in stmts.iter().enumerate() {
        if i > 0 {
            writeln!(f)?;
        }
        stmt.fmt_indented(f, indent)?;
    }
    Ok(())
}

/// Write a block body: a newline, the statements indented one level
/// deeper, and a trailing newline — or just a newline when empty.
fn write_block(f: &mut fmt::Formatter<'_>, stmts: &[Stmt], indent: usize) -> fmt::Result {
    writeln!(f)?;
    if !stmts.is_empty() {
        write_stmts(f, stmts, indent)?;
        writeln!(f)?;
    }
    Ok(())
}

impl Stmt {
    fn fmt_indented(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        let pad = "    ".repeat(indent);
        match self {
            Stmt::Let { name, value, .. } => write!(f, "{pad}let {name} = {value}"),
            Stmt::Assign { name, value, .. } => write!(f, "{pad}{name} = {value}"),
            Stmt::IndexAssign { target, value, .. } => write!(f, "{pad}{target} = {value}"),
            Stmt::FieldAssign { target, value, .. } => write!(f, "{pad}{target} = {value}"),
            Stmt::Print { expr, .. } => write!(f, "{pad}print {expr}"),
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                write!(f, "{pad}if {cond} then")?;
                write_block(f, then_block, indent + 1)?;
                match else_block {
                    Some(else_block) => {
                        write!(f, "{pad}else")?;
                        write_block(f, else_block, indent + 1)?;
                        write!(f, "{pad}end")
                    }
                    None => write!(f, "{pad}end"),
                }
            }
            Stmt::While { cond, body, .. } => {
                write!(f, "{pad}while {cond} then")?;
                write_block(f, body, indent + 1)?;
                write!(f, "{pad}end")
            }
            Stmt::For {
                var,
                from,
                to,
                body,
                ..
            } => {
                write!(f, "{pad}for {var} = {from} to {to} then")?;
                write_block(f, body, indent + 1)?;
                write!(f, "{pad}end")
            }
            Stmt::ForIn {
                var, iter, body, ..
            } => {
                write!(f, "{pad}for {var} in {iter} then")?;
                write_block(f, body, indent + 1)?;
                write!(f, "{pad}end")
            }
            Stmt::Fn {
                name,
                params,
                rest,
                body,
                ..
            } => {
                write!(f, "{pad}fn {name}(")?;
                fmt_params(f, params, rest)?;
                write!(f, ")")?;
                write_block(f, body, indent + 1)?;
                write!(f, "{pad}end")
            }
            Stmt::Class {
                name,
                parent,
                methods,
                ..
            } => {
                write!(f, "{pad}class {name}")?;
                if let Some(parent) = parent {
                    write!(f, " extends {parent}")?;
                }
                writeln!(f)?;
                for (i, method) in methods.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{pad}    fn {}(", method.name)?;
                    fmt_params(f, &method.params, &method.rest)?;
                    write!(f, ")")?;
                    write_block(f, &method.body, indent + 2)?;
                    write!(f, "{pad}    end")?;
                }
                if !methods.is_empty() {
                    writeln!(f)?;
                }
                write!(f, "{pad}end")
            }
            Stmt::Import { name, alias, .. } => {
                write!(f, "{pad}import {name}")?;
                if let Some(alias) = alias {
                    write!(f, " as {alias}")?;
                }
                Ok(())
            }
            Stmt::Return { expr, .. } => match expr {
                Some(expr) => write!(f, "{pad}return {expr}"),
                None => write!(f, "{pad}return"),
            },
            Stmt::Try {
                body,
                except_var,
                except_body,
                finally_body,
                ..
            } => {
                write!(f, "{pad}try then")?;
                write_block(f, body, indent + 1)?;
                if let Some(except_body) = except_body {
                    match except_var {
                        Some(var) => write!(f, "{pad}except {var} then")?,
                        None => write!(f, "{pad}except then")?,
                    }
                    write_block(f, except_body, indent + 1)?;
                }
                if let Some(finally_body) = finally_body {
                    write!(f, "{pad}finally then")?;
                    write_block(f, finally_body, indent + 1)?;
                }
                write!(f, "{pad}end")
            }
            Stmt::Raise { expr, .. } => write!(f, "{pad}raise {expr}"),
            Stmt::Match {
                scrutinee, cases, ..
            } => {
                write!(f, "{pad}match {scrutinee} then")?;
                if cases.is_empty() {
                    writeln!(f)?;
                }
                // `write_block` ends with a newline, so only the
                // first case needs one before it.
                let mut first = true;
                for case in cases {
                    if first {
                        writeln!(f)?;
                        first = false;
                    }
                    write!(f, "{pad}    case {}", case.pattern)?;
                    if let Some(guard) = &case.guard {
                        write!(f, " if {guard}")?;
                    }
                    write!(f, " then")?;
                    write_block(f, &case.body, indent + 2)?;
                }
                write!(f, "{pad}end")
            }
            Stmt::Yield { expr, .. } => match expr {
                Some(expr) => write!(f, "{pad}yield {expr}"),
                None => write!(f, "{pad}yield"),
            },
            Stmt::ExprStmt { expr, .. } => write!(f, "{pad}{expr}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str) -> Expr {
        Expr::Ident {
            name: name.to_string(),
            span: Span::dummy(),
        }
    }

    fn int(n: i64) -> Expr {
        Expr::Int {
            value: n,
            span: Span::dummy(),
        }
    }

    fn bin(op: BinaryOp, left: Expr, right: Expr) -> Expr {
        Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
            span: Span::dummy(),
        }
    }

    #[test]
    fn float_prints_with_a_decimal_point() {
        assert_eq!(
            Expr::Float {
                value: 1.0,
                span: Span::dummy()
            }
            .to_string(),
            "1.0"
        );
        assert_eq!(
            Expr::Float {
                value: 3.15,
                span: Span::dummy()
            }
            .to_string(),
            "3.15"
        );
        assert_eq!(
            Expr::Float {
                value: -2.0,
                span: Span::dummy()
            }
            .to_string(),
            "-2.0"
        );
    }

    #[test]
    fn strings_are_escaped_when_printed() {
        assert_eq!(
            Expr::Str {
                value: "a\"b\nc\\d".to_string(),
                span: Span::dummy()
            }
            .to_string(),
            "\"a\\\"b\\nc\\\\d\""
        );
    }

    #[test]
    fn binary_print_adds_parens_only_where_needed() {
        // (1 + 2) * 3 needs parens on the left.
        let e = bin(BinaryOp::Mul, bin(BinaryOp::Add, int(1), int(2)), int(3));
        assert_eq!(e.to_string(), "(1 + 2) * 3");
        // 1 + (2 * 3) needs none.
        let e = bin(BinaryOp::Add, int(1), bin(BinaryOp::Mul, int(2), int(3)));
        assert_eq!(e.to_string(), "1 + 2 * 3");
        // a - (b - c) needs parens on the right.
        let e = bin(
            BinaryOp::Sub,
            ident("a"),
            bin(BinaryOp::Sub, ident("b"), ident("c")),
        );
        assert_eq!(e.to_string(), "a - (b - c)");
        // (a or b) and c needs parens on the left.
        let e = bin(
            BinaryOp::And,
            bin(BinaryOp::Or, ident("a"), ident("b")),
            ident("c"),
        );
        assert_eq!(e.to_string(), "(a or b) and c");
    }

    #[test]
    fn unary_print_parenthesizes_low_precedence_operands() {
        let e = Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(bin(BinaryOp::Add, ident("a"), ident("b"))),
            span: Span::dummy(),
        };
        assert_eq!(e.to_string(), "-(a + b)");
        let e = Expr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(bin(BinaryOp::And, ident("a"), ident("b"))),
            span: Span::dummy(),
        };
        assert_eq!(e.to_string(), "not (a and b)");
        let e = Expr::Unary {
            op: UnaryOp::Neg,
            expr: Box::new(ident("x")),
            span: Span::dummy(),
        };
        assert_eq!(e.to_string(), "-x");
    }

    #[test]
    fn if_without_else_and_empty_blocks_print_cleanly() {
        let prog = Program {
            stmts: vec![
                Stmt::If {
                    cond: ident("x"),
                    then_block: vec![],
                    else_block: None,
                    span: Span::dummy(),
                },
                Stmt::While {
                    cond: Expr::Bool {
                        value: true,
                        span: Span::dummy(),
                    },
                    body: vec![],
                    span: Span::dummy(),
                },
            ],
        };
        assert_eq!(prog.to_string(), "if x then\nend\nwhile true then\nend");
    }

    #[test]
    fn empty_program_prints_empty() {
        assert_eq!(Program { stmts: vec![] }.to_string(), "");
    }

    #[test]
    fn spans_are_ignored_by_equality() {
        // The same node written in two places is equal: a span is
        // metadata, not meaning.
        let a = Expr::Ident {
            name: "x".to_string(),
            span: Span::new(1, 1),
        };
        let b = Expr::Ident {
            name: "x".to_string(),
            span: Span::new(9, 4),
        };
        assert_eq!(a, b);
        let c = Expr::Ident {
            name: "y".to_string(),
            span: Span::new(1, 1),
        };
        assert_ne!(a, c);
    }
}
