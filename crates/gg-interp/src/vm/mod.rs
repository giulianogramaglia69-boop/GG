// SPDX-License-Identifier: GPL-3.0-or-later
//! The GG bytecode virtual machine: compiler (AST -> bytecode)
//! and stack machine, behind the same `Interpreter` API as the
//! tree-walker.
//!
//! The backend is selected with the `GG_BACKEND` environment
//! variable (`vm` by default, `tree` for the tree-walking
//! fallback) or with `Interpreter::set_backend`. Both engines
//! run the same programs with the same results: the VM reuses
//! the tree-walker's values, environments, builtins and
//! semantics helpers, and only the dispatch — bytecode
//! instructions instead of AST traversal — differs.

pub mod chunk;
pub mod compiler;
pub(crate) mod machine;
#[cfg(test)]
mod tests;

pub use chunk::CodeObject;

/// Which execution engine an `Interpreter` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// The bytecode VM (the default).
    #[default]
    Vm,
    /// The tree-walking interpreter (the fallback).
    Tree,
}

impl Backend {
    /// Read the backend from the `GG_BACKEND` environment
    /// variable: `tree` selects the tree-walker, anything else
    /// (including unset) selects the VM.
    pub fn from_env() -> Self {
        Self::from_env_value(std::env::var("GG_BACKEND").ok().as_deref())
    }

    /// The same selection, from an already-read value (so tests
    /// need not touch the process environment).
    pub(crate) fn from_env_value(value: Option<&str>) -> Self {
        match value {
            Some("tree") => Backend::Tree,
            _ => Backend::Vm,
        }
    }
}
