// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime values for GG.
//!
//! A `Function` captures the environment where it was defined
//! (a closure): calling it later still sees those variables.
//! Collections share the list convention: mutable ones are shared
//! through `Rc<RefCell<...>>` (aliasing, Python style), the tuple
//! is immutable (`Rc` only).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use gg_parser::ast::Stmt;

use crate::builtin::Builtin;
use crate::env::Environment;

/// A value at runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Nil,
    Function(Rc<Function>),
    Builtin(Rc<Builtin>),
    /// A list: shared and mutable, so `let b = a` makes `b` an alias
    /// of `a` (Python style) — a mutation through one name is visible
    /// through the other.
    List(Rc<RefCell<Vec<Value>>>),
    /// A tuple: an immutable, ordered collection — `(1, "a")`.
    Tuple(Rc<Vec<Value>>),
    /// A dictionary: insertion-ordered key/value pairs. Only
    /// Int/Float/Str/Bool values can be keys.
    Dict(Rc<RefCell<DictData>>),
    /// A set: unique hashable elements, insertion-ordered.
    Set(Rc<RefCell<Vec<DictKey>>>),
    /// An array: a fixed-size list whose elements all have the same
    /// type (the type of the first element).
    Array(Rc<RefCell<ArrayData>>),
    /// A FIFO queue.
    Queue(Rc<RefCell<VecDeque<Value>>>),
    /// A LIFO stack.
    Stack(Rc<RefCell<Vec<Value>>>),
    /// A class: `class Persona ... end` defines one. Calling it
    /// creates an instance (its `init` method runs, if any).
    Class(Rc<ClassDef>),
    /// An instance of a class: the class plus its fields, in
    /// insertion order. Instances compare by identity.
    Instance(Rc<RefCell<InstanceData>>),
    /// A method bound to an instance: `p.saluta` without the call.
    /// Calling it passes the instance as `self`.
    BoundMethod(Rc<BoundMethod>),
    /// A module: `import math` loads `math.gg` once and binds the
    /// value. Its top-level names are its environment; attribute
    /// access (`math.pi`) reads them. Modules compare by identity.
    Module(Rc<ModuleData>),
    /// A generator: a function whose body contains `yield`, called
    /// but not yet run. Calling it returns this value instead of
    /// executing the body; `next(gen)` replays the body from the
    /// start up to the next `yield` (see `Interpreter`). Two
    /// generators from the same function are independent: each
    /// keeps its own bindings and count of yielded values.
    Generator(Rc<RefCell<GeneratorData>>),
}

/// A loaded module: its name (as imported) and the environment its
/// top-level code ran in. The environment is a child of the
/// interpreter's globals, so module code sees the builtins — but
/// attribute access only reads the module's own names.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleData {
    pub name: String,
    pub env: Rc<Environment>,
}

/// The state of a generator: the function to replay, the
/// arguments bound at the call (already checked for arity, so a
/// wrong call fails when the generator is created, not at the
/// first `next`), the `super` context to run it in (the defining
/// class for a method, the function's own for a plain function),
/// and how many values have been yielded so far.
///
/// `id` is unique per generator (a process-wide counter): two
/// generators are equal only when they are the same one, even
/// when they come from the same function with the same arguments.
#[derive(Debug, Clone)]
pub struct GeneratorData {
    pub function: Rc<Function>,
    pub bindings: Vec<(String, Value)>,
    pub super_context: Option<Rc<ClassDef>>,
    pub consumed: usize,
    pub id: u64,
}

/// The next generator id: one counter for the whole process, so
/// every generator is unique.
static NEXT_GENERATOR_ID: AtomicU64 = AtomicU64::new(1);

impl GeneratorData {
    pub fn new(
        function: Rc<Function>,
        bindings: Vec<(String, Value)>,
        super_context: Option<Rc<ClassDef>>,
    ) -> Self {
        Self {
            function,
            bindings,
            super_context,
            consumed: 0,
            id: NEXT_GENERATOR_ID.fetch_add(1, Ordering::Relaxed),
        }
    }
}

// Two generators are equal only when they are the same one.
impl PartialEq for GeneratorData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// Max collection nesting depth when printing; deeper levels print
/// as `...`, so a self-referential collection cannot blow the stack.
const MAX_DEPTH: usize = 16;

/// Max depth when comparing two collections element by element;
/// distinct cyclic collections are reported as unequal instead of
/// recursing forever.
const MAX_COMPARE_DEPTH: usize = 64;

/// A user-defined function: parameters, body, and the environment
/// captured at definition time.
///
/// `defaults` holds one entry per parameter: the default value
/// for parameters with one (evaluated at definition time, Python
/// rule — a mutable default is shared between calls), `None`
/// for required parameters. `rest` is the `*name` parameter, if
/// any, collecting extra positional arguments into a List.
///
/// `super_class` is the class whose method the function was defined
/// in, if any (a nested function inside a method): `super` inside
/// it resolves against that class — lexical, like Python's
/// `__class__` cell. A plain top-level function has none, so
/// `super` inside it is an error even when called from a method.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub params: Vec<String>,
    pub defaults: Vec<Option<Value>>,
    pub rest: Option<String>,
    pub body: Vec<Stmt>,
    pub env: Rc<Environment>,
    pub super_class: Option<Rc<ClassDef>>,
}

/// A dict/set key: only the scalar values are hashable. Note that
/// `Int(1)` and `Float(1.0)` are *different* keys (unlike `==` on
/// values, where `1 == 1.0` is true) — keys are compared by type
/// and value.
#[derive(Debug, Clone, PartialEq)]
pub enum DictKey {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
}

impl DictKey {
    /// The key back as an ordinary value.
    pub fn to_value(&self) -> Value {
        match self {
            DictKey::Int(n) => Value::Int(*n),
            DictKey::Float(x) => Value::Float(*x),
            DictKey::Str(s) => Value::Str(s.clone()),
            DictKey::Bool(b) => Value::Bool(*b),
        }
    }
}

impl fmt::Display for DictKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DictKey::Int(n) => write!(f, "{n}"),
            DictKey::Float(x) => write!(f, "{x}"),
            // String keys are quoted, like strings inside a list.
            DictKey::Str(s) => write!(f, "\"{}\"", escape_str(s)),
            DictKey::Bool(b) => write!(f, "{b}"),
        }
    }
}

/// The contents of a dict: pairs in insertion order (updating an
/// existing key keeps its original position).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DictData {
    pub entries: Vec<(DictKey, Value)>,
}

impl DictData {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn get(&self, key: &DictKey) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: DictKey, value: Value) {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.entries.push((key, value));
        }
    }

    pub fn keys(&self) -> Vec<DictKey> {
        self.entries.iter().map(|(k, _)| k.clone()).collect()
    }
}

/// A class definition: its name, its parent (single inheritance)
/// and its methods in definition order (a later method with the
/// same name replaces an earlier one).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDef {
    pub name: String,
    pub parent: Option<Rc<ClassDef>>,
    pub methods: Vec<(String, Rc<Function>)>,
}

impl ClassDef {
    /// True when this class is `class` or inherits from it.
    pub fn is_a(&self, class: &Rc<ClassDef>) -> bool {
        let mut current: Option<&ClassDef> = Some(self);
        while let Some(c) = current {
            if std::ptr::eq(c, class.as_ref()) {
                return true;
            }
            current = c.parent.as_deref();
        }
        false
    }
}

/// Find a method by name, walking up the parent chain. Returns the
/// function and the class it was defined in (needed for `super`).
pub fn lookup_method(class: &Rc<ClassDef>, name: &str) -> Option<(Rc<Function>, Rc<ClassDef>)> {
    let mut current = Some(Rc::clone(class));
    while let Some(c) = current {
        if let Some((_, function)) = c.methods.iter().find(|(n, _)| n == name) {
            return Some((Rc::clone(function), Rc::clone(&c)));
        }
        current = c.parent.clone();
    }
    None
}

/// The fields of an instance, in insertion order (updating a field
/// keeps its original position, like a dict).
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceData {
    pub class: Rc<ClassDef>,
    pub fields: Vec<(String, Value)>,
}

impl InstanceData {
    pub fn new(class: Rc<ClassDef>) -> Self {
        Self {
            class,
            fields: Vec::new(),
        }
    }

    pub fn get_field(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn set_field(&mut self, name: &str, value: Value) {
        if let Some(slot) = self.fields.iter_mut().find(|(n, _)| n == name) {
            slot.1 = value;
        } else {
            self.fields.push((name.to_string(), value));
        }
    }
}

/// A method bound to an instance: `p.saluta` without the call.
/// `class` is where the method was defined (for `super`).
#[derive(Debug, Clone, PartialEq)]
pub struct BoundMethod {
    pub name: String,
    pub instance: Rc<RefCell<InstanceData>>,
    pub function: Rc<Function>,
    pub class: Rc<ClassDef>,
}

/// The element type of an array: the type of its first element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElemType {
    Int,
    Float,
    Str,
    Bool,
}

impl ElemType {
    pub fn name(&self) -> &'static str {
        match self {
            ElemType::Int => "Int",
            ElemType::Float => "Float",
            ElemType::Str => "Str",
            ElemType::Bool => "Bool",
        }
    }
}

/// The element type of a value, if it may be an array element.
pub fn elem_type_of(value: &Value) -> Option<ElemType> {
    match value {
        Value::Int(_) => Some(ElemType::Int),
        Value::Float(_) => Some(ElemType::Float),
        Value::Str(_) => Some(ElemType::Str),
        Value::Bool(_) => Some(ElemType::Bool),
        _ => None,
    }
}

/// The contents of an array: its element type, its shape (one
/// size per dimension) and the items, flat in row-major order.
/// A 1D array of 3 elements has shape `[3]`; a 2x2 matrix has
/// shape `[2, 2]` and its 4 items in row-major order.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayData {
    pub elem: ElemType,
    pub shape: Vec<usize>,
    pub items: Vec<Value>,
}

impl ArrayData {
    /// Total number of elements (the product of the shape).
    pub fn size(&self) -> usize {
        self.items.len()
    }

    /// Number of dimensions.
    pub fn ndim(&self) -> usize {
        self.shape.len()
    }

    /// Row-major strides: the number of flat items to skip when an
    /// index on one axis grows by one.
    pub fn strides(&self) -> Vec<usize> {
        let mut strides = vec![1; self.shape.len()];
        for i in (0..self.shape.len().saturating_sub(1)).rev() {
            strides[i] = strides[i + 1] * self.shape[i + 1];
        }
        strides
    }
}

impl Value {
    /// Short type name used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Str(_) => "Str",
            Value::Bool(_) => "Bool",
            Value::Nil => "Nil",
            Value::Function(_) | Value::Builtin(_) => "Function",
            Value::List(_) => "List",
            Value::Tuple(_) => "Tuple",
            Value::Dict(_) => "Dict",
            Value::Set(_) => "Set",
            Value::Array(_) => "Array",
            Value::Queue(_) => "Queue",
            Value::Stack(_) => "Stack",
            Value::Class(_) => "Class",
            Value::Instance(_) => "Instance",
            Value::BoundMethod(_) => "Function",
            Value::Module(_) => "Module",
            Value::Generator(_) => "Generator",
        }
    }

    /// GG truthiness: only `false` and `nil` are false, everything
    /// else is true (Ruby/Lua style).
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Bool(false) | Value::Nil)
    }

    /// The value as a dict/set key, if it is hashable (Int, Float,
    /// Str or Bool). Anything else can never be a key or a set
    /// element.
    pub fn to_key(&self) -> Option<DictKey> {
        match self {
            Value::Int(n) => Some(DictKey::Int(*n)),
            Value::Float(x) => Some(DictKey::Float(*x)),
            Value::Str(s) => Some(DictKey::Str(s.clone())),
            Value::Bool(b) => Some(DictKey::Bool(*b)),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_depth(f, 0)
    }
}

impl Value {
    /// Print with a nesting depth: collection elements are one level
    /// deeper than the collection itself.
    fn fmt_depth(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{n}"),
            // Plain Rust float formatting: 3.5 prints as "3.5", 1.0 as "1".
            Value::Float(x) => write!(f, "{x}"),
            // Strings print without quotes.
            Value::Str(s) => write!(f, "{s}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Nil => write!(f, "nil"),
            Value::Function(_) | Value::Builtin(_) => write!(f, "<fn>"),
            Value::List(list) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "[")?;
                for (i, element) in list.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_nested(f, depth + 1)?;
                }
                write!(f, "]")
            }
            Value::Tuple(items) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "(")?;
                for (i, element) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_nested(f, depth + 1)?;
                }
                // A one-element tuple keeps its trailing comma, so it
                // still reads as a tuple.
                if items.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")
            }
            Value::Dict(dict) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "{{")?;
                for (i, (key, value)) in dict.borrow().entries.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{key}: ")?;
                    value.fmt_nested(f, depth + 1)?;
                }
                write!(f, "}}")
            }
            Value::Set(set) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "{{")?;
                for (i, key) in set.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{key}")?;
                }
                write!(f, "}}")
            }
            Value::Array(array) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "array(")?;
                fmt_array(f, &array.borrow(), depth)?;
                write!(f, ")")
            }
            Value::Queue(queue) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "queue([")?;
                for (i, element) in queue.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_nested(f, depth + 1)?;
                }
                write!(f, "])")
            }
            Value::Stack(stack) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                write!(f, "stack([")?;
                for (i, element) in stack.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    element.fmt_nested(f, depth + 1)?;
                }
                write!(f, "])")
            }
            Value::Class(class) => write!(f, "<class {}>", class.name),
            Value::Instance(inst) => {
                if depth >= MAX_DEPTH {
                    return write!(f, "...");
                }
                let inst = inst.borrow();
                write!(f, "{}{{", inst.class.name)?;
                for (i, (name, value)) in inst.fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: ")?;
                    value.fmt_nested(f, depth + 1)?;
                }
                write!(f, "}}")
            }
            Value::BoundMethod(bound) => {
                write!(f, "<bound method {}.{}>", bound.class.name, bound.name)
            }
            Value::Module(module) => write!(f, "<module {}>", module.name),
            Value::Generator(_) => write!(f, "<generator>"),
        }
    }

    /// Print a value as an element of a collection: strings are
    /// quoted (like the elements of a Python repr); anything else
    /// prints the same as at the top level.
    fn fmt_nested(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "\"{}\"", escape_str(s)),
            other => other.fmt_depth(f, depth),
        }
    }
}

/// Print an array in NumPy style: 1D `array([1, 2, 3])`, 2D
/// `array([[1, 2], [3, 4]])`. One bracket level per dimension,
/// innermost rows holding the flat items in row-major order.
fn fmt_array(f: &mut fmt::Formatter<'_>, data: &ArrayData, depth: usize) -> fmt::Result {
    if data.shape.is_empty() {
        return write!(f, "[]");
    }
    fmt_array_dim(f, data, 0, 0, depth)
}

fn fmt_array_dim(
    f: &mut fmt::Formatter<'_>,
    data: &ArrayData,
    dim: usize,
    offset: usize,
    depth: usize,
) -> fmt::Result {
    write!(f, "[")?;
    let size = data.shape[dim];
    let stride = data.strides()[dim];
    for i in 0..size {
        if i > 0 {
            write!(f, ", ")?;
        }
        if dim + 1 == data.shape.len() {
            data.items[offset + i * stride].fmt_nested(f, depth + 1)?;
        } else {
            fmt_array_dim(f, data, dim + 1, offset + i * stride, depth + 1)?;
        }
    }
    write!(f, "]")
}

/// Escape a string for printing inside double quotes (only used
/// for strings nested in a collection).
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

/// Equality between two values. Numbers compare across Int/Float;
/// `==` between different types is false, never an error.
pub fn values_equal(a: &Value, b: &Value) -> bool {
    values_equal_depth(a, b, 0)
}

fn values_equal_depth(a: &Value, b: &Value, depth: usize) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => (*x as f64) == *y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Nil, Value::Nil) => true,
        (Value::Function(x), Value::Function(y)) => Rc::ptr_eq(x, y),
        (Value::Builtin(x), Value::Builtin(y)) => Rc::ptr_eq(x, y),
        (Value::List(x), Value::List(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            // Distinct cyclic lists would recurse forever: give up
            // and call them unequal.
            if depth >= MAX_COMPARE_DEPTH {
                return false;
            }
            let x = x.borrow();
            let y = y.borrow();
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|(a, b)| values_equal_depth(a, b, depth + 1))
        }
        (Value::Tuple(x), Value::Tuple(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            if depth >= MAX_COMPARE_DEPTH {
                return false;
            }
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|(a, b)| values_equal_depth(a, b, depth + 1))
        }
        (Value::Dict(x), Value::Dict(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            if depth >= MAX_COMPARE_DEPTH {
                return false;
            }
            let x = x.borrow();
            let y = y.borrow();
            x.entries.len() == y.entries.len()
                && x.entries.iter().all(|(key, value)| match y.get(key) {
                    Some(other) => values_equal_depth(value, other, depth + 1),
                    None => false,
                })
        }
        (Value::Set(x), Value::Set(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let x = x.borrow();
            let y = y.borrow();
            x.len() == y.len() && x.iter().all(|key| y.contains(key))
        }
        (Value::Array(x), Value::Array(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            if depth >= MAX_COMPARE_DEPTH {
                return false;
            }
            let x = x.borrow();
            let y = y.borrow();
            x.elem == y.elem
                && x.shape == y.shape
                && x.items.len() == y.items.len()
                && x.items
                    .iter()
                    .zip(y.items.iter())
                    .all(|(a, b)| values_equal_depth(a, b, depth + 1))
        }
        (Value::Queue(x), Value::Queue(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            if depth >= MAX_COMPARE_DEPTH {
                return false;
            }
            let x = x.borrow();
            let y = y.borrow();
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|(a, b)| values_equal_depth(a, b, depth + 1))
        }
        (Value::Stack(x), Value::Stack(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            if depth >= MAX_COMPARE_DEPTH {
                return false;
            }
            let x = x.borrow();
            let y = y.borrow();
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|(a, b)| values_equal_depth(a, b, depth + 1))
        }
        (Value::Class(x), Value::Class(y)) => Rc::ptr_eq(x, y),
        // Instances compare by identity: two calls to the same
        // class make two different instances, even with the same
        // fields. Use `is_a` to test the class instead.
        (Value::Instance(x), Value::Instance(y)) => Rc::ptr_eq(x, y),
        (Value::BoundMethod(x), Value::BoundMethod(y)) => {
            Rc::ptr_eq(&x.instance, &y.instance) && Rc::ptr_eq(&x.function, &y.function)
        }
        // Modules compare by identity: importing the same file
        // twice returns the same value, so this is almost always
        // true for two imports of one module.
        (Value::Module(x), Value::Module(y)) => Rc::ptr_eq(x, y),
        // Generators compare by identity: two generators from the
        // same function are different generators.
        (Value::Generator(x), Value::Generator(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}
