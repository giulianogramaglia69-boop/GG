// SPDX-License-Identifier: GPL-3.0-or-later
//! Lexically scoped environments.
//!
//! Environments form a chain: each block (if/while/for body, function
//! call) runs in a child of the enclosing scope. `let` defines in the
//! current scope; assignment walks up the chain and updates the scope
//! where the variable was defined.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::value::Value;

/// One scope. Always handled through `Rc`: environments are shared
/// (closures capture them) and mutated through `RefCell`.
#[derive(Debug, PartialEq)]
pub struct Environment {
    vars: RefCell<HashMap<String, Value>>,
    parent: Option<Rc<Environment>>,
}

impl Environment {
    /// A fresh top-level (global) environment.
    pub fn new() -> Rc<Environment> {
        Rc::new(Environment {
            vars: RefCell::new(HashMap::new()),
            parent: None,
        })
    }

    /// A child scope of `parent`.
    pub fn child(parent: &Rc<Environment>) -> Rc<Environment> {
        Rc::new(Environment {
            vars: RefCell::new(HashMap::new()),
            parent: Some(Rc::clone(parent)),
        })
    }

    /// Define (or redefine) a variable in this scope.
    pub fn define(&self, name: &str, value: Value) {
        self.vars.borrow_mut().insert(name.to_string(), value);
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
