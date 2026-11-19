// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime errors for the GG interpreter.
//!
//! The Phase 2 AST carries no source positions, so runtime errors in
//! this phase have no line or column — that is a known limitation, and
//! positions will arrive in a later phase (see README).

use std::fmt;

/// A runtime error: a message, nothing else (no position yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RuntimeError {}
