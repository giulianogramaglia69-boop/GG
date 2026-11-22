// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime errors for the GG interpreter.
//!
//! Phase 16: an error carries the `Span` of the source position
//! that failed — the identifier for an undefined variable, the
//! operator for a division by zero, the index expression for an
//! out-of-range index — and prints as `line:col: message`. An
//! error built without a span (builtins and helpers that do not
//! see the AST) prints just the message; the interpreter fills
//! the span in at the call site or statement level with `at`.
//! `at` never overwrites a span that is already set, so the
//! most precise position wins: an error raised deep inside an
//! expression keeps the operator's position, not the statement's.

use std::fmt;

use gg_lexer::Span;

/// A runtime error: a message, plus (usually) where it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Option<Span>,
}

impl RuntimeError {
    /// An error with a message and no position yet.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
        }
    }

    /// An error with a message and a known position.
    pub fn with_span(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span: Some(span),
        }
    }

    /// Fill in the position, but only when none is set yet: the
    /// first (most precise) position wins. A dummy span (from a
    /// hand-built AST node) is ignored — it is not a real position.
    pub fn at(mut self, span: Span) -> Self {
        if self.span.is_none() && !span.is_dummy() {
            self.span = Some(span);
        }
        self
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(span) => write!(f, "{span}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for RuntimeError {}
