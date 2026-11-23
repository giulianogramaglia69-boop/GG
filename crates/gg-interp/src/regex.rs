// SPDX-License-Identifier: GPL-3.0-or-later
//! The native `regex` module: regular expressions, via the
//! `regex` crate (1.x) from crates.io.
//!
//! `import regex` gives `regex.test(pattern, text)`,
//! `regex.find(pattern, text)`, `regex.groups(pattern, text)`,
//! `regex.replace(pattern, text, replacement)` and
//! `regex.split(pattern, text)`.
//!
//! The pattern syntax is the `regex` crate's: character
//! classes (`[a-z]`, `\d`, `\w`, `\s`), anchors (`^`, `$`),
//! quantifiers (`*`, `+`, `?`, `{n,m}`), groups — numbered
//! `(...)` and named `(?P<nome>...)` — and alternation (`|`).
//! There are no backreferences in patterns (the crate does
//! not support them); backreferences live in the replacement
//! string of `replace` instead (see below). Patterns match on
//! Unicode text: `.` matches one Unicode character, `\d`
//! matches any Unicode decimal digit, and accented letters
//! and emoji match literally.
//!
//! `test` is true when the pattern matches anywhere in the
//! text. `find` returns every non-overlapping match, in
//! order, as a List of Str (empty when nothing matches).
//! `groups` returns the first match as a List: index 0 is the
//! whole match, then one element per capture group — Nil for
//! an optional group that did not participate — and an empty
//! List when there is no match. `replace` replaces every
//! occurrence; the replacement string supports `$1` (a
//! numbered group), `$nome` (a named group), `${nome}`
//! (unambiguous form) and `$$` for a literal `$` — the
//! `regex` crate's syntax, not GG's. `split` cuts the text
//! at every match and returns the pieces as a List of Str.
//!
//! An invalid pattern is a runtime error carrying the
//! crate's message, catchable with `try`/`except`.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

use regex::Regex;

use crate::error::RuntimeError;
use crate::value::Value;

/// A Str argument, or a clear error naming the function.
fn as_str(name: &str, value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(s) => Ok(s.clone()),
        other => Err(RuntimeError::new(format!(
            "{name}() expects a Str, not {}",
            other.type_name()
        ))),
    }
}

/// Compile a pattern, or a runtime error with the crate's
/// message (it names the offending spot in the pattern).
fn compile(name: &str, pattern: &str) -> Result<Regex, RuntimeError> {
    Regex::new(pattern).map_err(|e| RuntimeError::new(format!("{name}(): invalid regex: {e}")))
}

/// `regex.test(pattern, text)` — true when the pattern
/// matches anywhere in the text.
pub fn test(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let pattern = as_str("regex.test", &args[0])?;
    let text = as_str("regex.test", &args[1])?;
    let re = compile("regex.test", &pattern)?;
    Ok(Value::Bool(re.is_match(&text)))
}

/// `regex.find(pattern, text)` — every non-overlapping match,
/// in order, as a List of Str; an empty List when nothing
/// matches.
pub fn find(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let pattern = as_str("regex.find", &args[0])?;
    let text = as_str("regex.find", &args[1])?;
    let re = compile("regex.find", &pattern)?;
    let items: Vec<Value> = re
        .find_iter(&text)
        .map(|m| Value::Str(m.as_str().to_string()))
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(items))))
}

/// `regex.groups(pattern, text)` — the first match as a List:
/// index 0 is the whole match, then one element per capture
/// group, Nil for an optional group that did not participate;
/// an empty List when there is no match.
pub fn groups(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let pattern = as_str("regex.groups", &args[0])?;
    let text = as_str("regex.groups", &args[1])?;
    let re = compile("regex.groups", &pattern)?;
    let Some(caps) = re.captures(&text) else {
        return Ok(Value::List(Rc::new(RefCell::new(Vec::new()))));
    };
    let items: Vec<Value> = (0..caps.len())
        .map(|i| match caps.get(i) {
            Some(m) => Value::Str(m.as_str().to_string()),
            None => Value::Nil,
        })
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(items))))
}

/// `regex.replace(pattern, text, replacement)` — replace every
/// occurrence; the replacement supports `$1`, `$nome`,
/// `${nome}` and `$$` (a literal `$`), the `regex` crate's
/// syntax.
pub fn replace(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let pattern = as_str("regex.replace", &args[0])?;
    let text = as_str("regex.replace", &args[1])?;
    let replacement = as_str("regex.replace", &args[2])?;
    let re = compile("regex.replace", &pattern)?;
    Ok(Value::Str(
        re.replace_all(&text, replacement.as_str()).into_owned(),
    ))
}

/// `regex.split(pattern, text)` — cut the text at every match,
/// as a List of Str. Splitting on a pattern that matches at
/// the edges yields empty strings there, like Python's
/// `re.split`.
pub fn split(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let pattern = as_str("regex.split", &args[0])?;
    let text = as_str("regex.split", &args[1])?;
    let re = compile("regex.split", &pattern)?;
    let items: Vec<Value> = re
        .split(&text)
        .map(|piece| Value::Str(piece.to_string()))
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(items))))
}
