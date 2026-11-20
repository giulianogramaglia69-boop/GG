// SPDX-License-Identifier: GPL-3.0-or-later
//! Lexically scoped environments.
//!
//! Environments form a chain: each block (if/while/for body, function
//! call) runs in a child of the enclosing scope. `let` defines in the
//! current scope; assignment walks up the chain and updates the scope
//! where the variable was defined.
//!
//! Each environment may carry a `base_dir`: the directory an `import`
//! in that scope resolves against. The global environment of a script
//! gets the script's directory; a module's environment gets the
//! module's directory. Child scopes inherit it by walking up the
//! chain (`import_base`), so a function defined in a module imports
//! relative to that module even when called from elsewhere.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::value::Value;

/// One scope. Always handled through `Rc`: environments are shared
/// (closures capture them) and mutated through `RefCell`.
#[derive(Debug, PartialEq)]
pub struct Environment {
    vars: RefCell<HashMap<String, Value>>,
    parent: Option<Rc<Environment>>,
    base_dir: RefCell<Option<PathBuf>>,
}

impl Environment {
    /// A fresh top-level (global) environment.
    pub fn new() -> Rc<Environment> {
        Rc::new(Environment {
            vars: RefCell::new(HashMap::new()),
            parent: None,
            base_dir: RefCell::new(None),
        })
    }

    /// A child scope of `parent`.
    pub fn child(parent: &Rc<Environment>) -> Rc<Environment> {
        Rc::new(Environment {
            vars: RefCell::new(HashMap::new()),
            parent: Some(Rc::clone(parent)),
            base_dir: RefCell::new(None),
        })
    }

    /// Set the directory `import` resolves against in this scope
    /// (the script's directory for globals, the module's directory
    /// for a module's environment).
    pub fn set_base_dir(&self, dir: Option<PathBuf>) {
        *self.base_dir.borrow_mut() = dir;
    }

    /// The directory an `import` in this scope resolves against:
    /// this scope's own base directory, or the nearest one up the
    /// chain. `None` means "no importing file" (the REPL): imports
    /// fall back to the venv libraries and the working directory.
    pub fn import_base(&self) -> Option<PathBuf> {
        if let Some(dir) = self.base_dir.borrow().clone() {
            return Some(dir);
        }
        match &self.parent {
            Some(parent) => parent.import_base(),
            None => None,
        }
    }

    /// Define (or redefine) a variable in this scope.
    pub fn define(&self, name: &str, value: Value) {
        self.vars.borrow_mut().insert(name.to_string(), value);
    }

    /// Look a variable up in this scope only (no parent walk):
    /// module attributes are the module's own top-level names, not
    /// the builtins in the global scope.
    pub fn get_local(&self, name: &str) -> Option<Value> {
        self.vars.borrow().get(name).cloned()
    }

    /// Look a variable up, walking up the scope chain.
    pub fn get(&self, name: &str) -> Result<Value, RuntimeError> {
        if let Some(value) = self.vars.borrow().get(name) {
            return Ok(value.clone());
        }
        match &self.parent {
            Some(parent) => parent.get(name),
            None => Err(RuntimeError::new(format!("undefined variable '{name}'"))),
        }
    }

    /// Assign to a variable: update the scope where it was defined.
    /// Assigning to an undefined variable is an error.
    pub fn assign(&self, name: &str, value: Value) -> Result<(), RuntimeError> {
        if self.vars.borrow().contains_key(name) {
            self.vars.borrow_mut().insert(name.to_string(), value);
            return Ok(());
        }
        match &self.parent {
            Some(parent) => parent.assign(name, value),
            None => Err(RuntimeError::new(format!("undefined variable '{name}'"))),
        }
    }
}
