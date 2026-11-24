// SPDX-License-Identifier: GPL-3.0-or-later
//! The native `json` module: parse and stringify JSON.
//!
//! `import json` gives `json.parse(text)` and
//! `json.stringify(value)` / `json.stringify(value, indent)`.
//! The parser is a hand-written recursive-descent parser in
//! pure Rust — no external crate — following RFC 8259.
//!
//! Mapping to GG values: JSON objects become Dicts (insertion
//! order kept, a duplicate key keeps the last value), arrays
//! become Lists, integers that fit in i64 become Int (anything
//! else numeric becomes Float), `true`/`false` become Bool and
//! `null` becomes Nil. `stringify` goes the other way: Dict
//! keys that are not strings are written as their string form
//! (numbers as numbers, `true`/`false` as words), Lists, Tuples,
//! Sets and Arrays become JSON arrays, and functions, classes,
//! instances, modules, queues, stacks and generators are
//! runtime errors, never silent `nil`.
//!
//! Limits: nesting deeper than `MAX_DEPTH` (128 levels) is an
//! error on both sides — a hand-written recursive parser has
//! no other guard against a hostile `[[[[...`, and the same
//! limit is what makes `stringify` terminate on a cyclic GG
//! value (a list containing itself) instead of recursing
//! forever. Floats print in the shortest form that round-trips
//! (`1.0` prints as `1.0`, not `1`), so `parse(stringify(x))`
//! equals `x` for every finite float.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::value::{ArrayData, DictKey, Value};

/// Maximum nesting depth for parsing and stringifying.
/// Deeper input is an error, not a stack overflow.
const MAX_DEPTH: usize = 128;

/// Parse a JSON document into a GG value (used by the
/// lockfile reader): the same parser as `json.parse(text)`.
pub(crate) fn parse_text(text: &str) -> Result<Value, RuntimeError> {
    Parser::new(text).parse_document()
}

/// Render a GG value as JSON (used by the lockfile writer):
/// the same rendering as `json.stringify(value, indent)`.
pub(crate) fn stringify_value(value: &Value, indent: usize) -> Result<String, RuntimeError> {
    let mut out = String::new();
    write_value(&mut out, value, indent, 0, 0)?;
    Ok(out)
}

/// `json.parse(text)` — parse a JSON document into a GG value.
pub fn parse(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::Str(text) = &args[0] else {
        return Err(RuntimeError::new(format!(
            "json.parse() expects a Str, not {}",
            args[0].type_name()
        )));
    };
    Parser::new(text).parse_document()
}

/// `json.stringify(value)` / `json.stringify(value, indent)` —
/// render a GG value as JSON. Compact when `indent` is absent
/// or 0; otherwise pretty-printed with `indent` spaces per
/// level (empty collections stay on one line, like Python).
pub fn stringify(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let indent = match args.get(1) {
        None => 0,
        Some(Value::Int(n)) if *n >= 0 => *n as usize,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "json.stringify() indent must be a non-negative Int, not {}",
                other.type_name()
            )));
        }
    };
    let mut out = String::new();
    write_value(&mut out, &args[0], indent, 0, 0)?;
    Ok(Value::Str(out))
}

// ---- Parsing ----

/// A JSON parser over a character vector, tracking line and
/// column (both 1-based) so errors point inside the JSON text.
struct Parser {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Parser {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    /// Parse one JSON document: a value, then only whitespace.
    fn parse_document(mut self) -> Result<Value, RuntimeError> {
        self.skip_ws();
        let value = self.parse_value(0)?;
        self.skip_ws();
        if self.pos != self.chars.len() {
            return Err(self.error("trailing characters after JSON value"));
        }
        Ok(value)
    }

    /// An error at the current position, with line and column.
    fn error(&self, message: &str) -> RuntimeError {
        RuntimeError::new(format!(
            "json.parse(): {message} at line {}, column {}",
            self.line, self.col
        ))
    }

    /// The next character, without consuming it.
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Consume and return the next character, tracking line and
    /// column (a newline moves to the next line, column 1).
    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    /// Consume the exact literal, or an error naming it.
    fn expect_literal(&mut self, literal: &str) -> Result<(), RuntimeError> {
        for expected in literal.chars() {
            match self.next() {
                Some(c) if c == expected => {}
                _ => return Err(self.error(&format!("expected '{literal}'"))),
            }
        }
        Ok(())
    }

    /// Skip JSON whitespace: space, tab, newline, carriage return.
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.next();
        }
    }

    /// Parse any JSON value. `depth` is the nesting level, so a
    /// document of a thousand open brackets is an error, not a
    /// stack overflow.
    fn parse_value(&mut self, depth: usize) -> Result<Value, RuntimeError> {
        if depth > MAX_DEPTH {
            return Err(self.error("JSON nested too deep"));
        }
        match self.peek() {
            Some('{') => self.parse_object(depth),
            Some('[') => self.parse_array(depth),
            Some('"') => Ok(Value::Str(self.parse_string()?)),
            Some('t') => {
                self.expect_literal("true")?;
                Ok(Value::Bool(true))
            }
            Some('f') => {
                self.expect_literal("false")?;
                Ok(Value::Bool(false))
            }
            Some('n') => {
                self.expect_literal("null")?;
                Ok(Value::Nil)
            }
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(self.error(&format!("unexpected character '{c}'"))),
            None => Err(self.error("unexpected end of JSON input")),
        }
    }

    /// Parse an object: `{ "key": value, ... }`. A duplicate
    /// key keeps the last value (Dict insert replaces).
    fn parse_object(&mut self, depth: usize) -> Result<Value, RuntimeError> {
        self.next(); // consume '{'
        let mut dict = crate::value::DictData::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.next();
            return Ok(Value::Dict(Rc::new(RefCell::new(dict))));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.error("expected a string key in object"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.next() != Some(':') {
                return Err(self.error("expected ':' after object key"));
            }
            self.skip_ws();
            let value = self.parse_value(depth + 1)?;
            dict.insert(DictKey::Str(key), value);
            self.skip_ws();
            match self.next() {
                Some(',') => continue,
                Some('}') => break,
                _ => return Err(self.error("expected ',' or '}' in object")),
            }
        }
        Ok(Value::Dict(Rc::new(RefCell::new(dict))))
    }

    /// Parse an array: `[ value, ... ]`.
    fn parse_array(&mut self, depth: usize) -> Result<Value, RuntimeError> {
        self.next(); // consume '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.next();
            return Ok(Value::List(Rc::new(RefCell::new(items))));
        }
        loop {
            self.skip_ws();
            items.push(self.parse_value(depth + 1)?);
            self.skip_ws();
            match self.next() {
                Some(',') => continue,
                Some(']') => break,
                _ => return Err(self.error("expected ',' or ']' in array")),
            }
        }
        Ok(Value::List(Rc::new(RefCell::new(items))))
    }

    /// Parse a JSON string, with escapes. `\uXXXX` handles
    /// surrogate pairs (an emoji is two of them); a lone
    /// surrogate, a raw control character, or a bad escape is
    /// an error.
    fn parse_string(&mut self) -> Result<String, RuntimeError> {
        // Consume the opening quote (callers checked it is there).
        self.next();
        let mut out = String::new();
        loop {
            match self.next() {
                None => return Err(self.error("unterminated string")),
                Some('"') => break,
                Some('\\') => match self.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('/') => out.push('/'),
                    Some('b') => out.push('\u{0008}'),
                    Some('f') => out.push('\u{000C}'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('u') => out.push(self.parse_unicode_escape()?),
                    Some(c) => {
                        return Err(self.error(&format!("invalid escape '\\{c}'")));
                    }
                    None => return Err(self.error("unterminated string")),
                },
                Some(c) if (c as u32) < 0x20 => {
                    return Err(self.error("control character in string"));
                }
                Some(c) => out.push(c),
            }
        }
        Ok(out)
    }

    /// Parse one `\uXXXX` escape, combining a surrogate pair
    /// into a single character when needed.
    fn parse_unicode_escape(&mut self) -> Result<char, RuntimeError> {
        let unit = self.parse_hex4()?;
        if (0xD800..0xDC00).contains(&unit) {
            // A high surrogate must be followed by `\uDC00-\uDFFF`.
            if self.next() != Some('\\') || self.next() != Some('u') {
                return Err(self.error("lone high surrogate in \\u escape"));
            }
            let low = self.parse_hex4()?;
            if !(0xDC00..0xE000).contains(&low) {
                return Err(self.error("lone high surrogate in \\u escape"));
            }
            let combined = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
            return char::from_u32(combined).ok_or_else(|| self.error("invalid \\u escape"));
        }
        if (0xDC00..0xE000).contains(&unit) {
            return Err(self.error("lone low surrogate in \\u escape"));
        }
        char::from_u32(unit).ok_or_else(|| self.error("invalid \\u escape"))
    }

    /// Parse exactly four hex digits after `\u`.
    fn parse_hex4(&mut self) -> Result<u32, RuntimeError> {
        let mut value: u32 = 0;
        for _ in 0..4 {
            match self.next() {
                Some(c) if c.is_ascii_hexdigit() => {
                    value = value * 16 + c.to_digit(16).unwrap();
                }
                _ => return Err(self.error("invalid \\u escape")),
            }
        }
        Ok(value)
    }

    /// Parse a JSON number: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
    /// No fraction or exponent and fitting in i64 gives Int;
    /// anything else gives Float. `NaN` and `Infinity` are not
    /// JSON and are rejected by the grammar itself.
    fn parse_number(&mut self) -> Result<Value, RuntimeError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.next();
        }
        match self.peek() {
            Some('0') => {
                self.next();
            }
            Some(c) if c.is_ascii_digit() => {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.next();
                }
            }
            _ => return Err(self.error("expected a digit in number")),
        }
        let mut is_float = false;
        if self.peek() == Some('.') {
            is_float = true;
            self.next();
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.error("expected a digit after '.' in number"));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.next();
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            is_float = true;
            self.next();
            if matches!(self.peek(), Some('+' | '-')) {
                self.next();
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.error("expected a digit in number exponent"));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.next();
            }
        }
        let literal: String = self.chars[start..self.pos].iter().collect();
        // No fraction or exponent: an Int when it fits in i64,
        // otherwise a Float (rounded) — a huge integer literal
        // is still a valid JSON number.
        if !is_float && let Ok(n) = literal.parse::<i64>() {
            return Ok(Value::Int(n));
        }
        match literal.parse::<f64>() {
            Ok(x) if x.is_finite() => Ok(Value::Float(x)),
            _ => Err(self.error("number out of range")),
        }
    }
}

// ---- Stringifying ----

/// A float as JSON: the shortest form that round-trips, with
/// `.0` appended when it would otherwise look like an integer
/// (`1.0` must stay a float on re-parse, not become Int `1`).
/// Non-finite floats have no JSON form and are an error.
fn fmt_float(x: f64) -> Result<String, RuntimeError> {
    if !x.is_finite() {
        return Err(RuntimeError::new(
            "json.stringify(): cannot stringify non-finite Float",
        ));
    }
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        Ok(s)
    } else {
        Ok(format!("{s}.0"))
    }
}

/// A GG string as a JSON string literal, with the escapes.
/// Non-ASCII (emoji included) goes through as UTF-8.
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Dict key as a JSON object key: strings as-is, numbers as
/// their number form, booleans as `true`/`false`.
fn fmt_key(key: &DictKey) -> Result<String, RuntimeError> {
    match key {
        DictKey::Str(s) => Ok(s.clone()),
        DictKey::Int(n) => Ok(format!("{n}")),
        DictKey::Float(x) => fmt_float(*x),
        DictKey::Bool(b) => Ok(format!("{b}")),
    }
}

/// Write `value` as JSON into `out`. `indent` is 0 for compact
/// output, otherwise the spaces per level for pretty output;
/// `level` is the current nesting level and `depth` guards
/// against cyclic values (a list containing itself).
fn write_value(
    out: &mut String,
    value: &Value,
    indent: usize,
    level: usize,
    depth: usize,
) -> Result<(), RuntimeError> {
    if depth > MAX_DEPTH {
        return Err(RuntimeError::new(
            "json.stringify(): value nested too deep (cyclic values cannot be stringified)",
        ));
    }
    match value {
        Value::Int(n) => out.push_str(&format!("{n}")),
        Value::Float(x) => out.push_str(&fmt_float(*x)?),
        Value::Str(s) => out.push_str(&escape_json_string(s)),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Nil => out.push_str("null"),
        Value::List(list) => {
            write_seq(out, list.borrow().iter(), indent, level, depth)?;
        }
        Value::Tuple(items) => {
            write_seq(out, items.iter(), indent, level, depth)?;
        }
        Value::Set(set) => {
            // A set has no JSON form of its own; like a list,
            // in insertion order.
            let items: Vec<Value> = set.borrow().iter().map(DictKey::to_value).collect();
            write_seq(out, items.iter(), indent, level, depth)?;
        }
        Value::Array(array) => {
            write_array(out, &array.borrow(), indent, level, depth)?;
        }
        Value::Dict(dict) => {
            let borrowed = dict.borrow();
            if borrowed.entries.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            out.push('{');
            let mut first = true;
            for (key, item) in borrowed.entries.iter() {
                if !first {
                    out.push(',');
                }
                first = false;
                if indent > 0 {
                    out.push('\n');
                    out.push_str(&" ".repeat(indent * (level + 1)));
                }
                out.push_str(&escape_json_string(&fmt_key(key)?));
                out.push(':');
                if indent > 0 {
                    out.push(' ');
                }
                write_value(out, item, indent, level + 1, depth + 1)?;
            }
            if indent > 0 {
                out.push('\n');
                out.push_str(&" ".repeat(indent * level));
            }
            out.push('}');
        }
        // No JSON form: an error naming the type, never nil.
        other => {
            return Err(RuntimeError::new(format!(
                "json.stringify(): cannot stringify {}",
                other.type_name()
            )));
        }
    }
    Ok(())
}

/// Write a sequence (list, tuple, set) as a JSON array.
fn write_seq<'a>(
    out: &mut String,
    items: impl Iterator<Item = &'a Value>,
    indent: usize,
    level: usize,
    depth: usize,
) -> Result<(), RuntimeError> {
    let items: Vec<&Value> = items.collect();
    if items.is_empty() {
        out.push_str("[]");
        return Ok(());
    }
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if indent > 0 {
            out.push('\n');
            out.push_str(&" ".repeat(indent * (level + 1)));
        }
        write_value(out, item, indent, level + 1, depth + 1)?;
    }
    if indent > 0 {
        out.push('\n');
        out.push_str(&" ".repeat(indent * level));
    }
    out.push(']');
    Ok(())
}

/// Write an N-d array as nested JSON arrays, one bracket level
/// per dimension, innermost rows holding the flat items in
/// row-major order.
fn write_array(
    out: &mut String,
    data: &ArrayData,
    indent: usize,
    level: usize,
    depth: usize,
) -> Result<(), RuntimeError> {
    if data.shape.is_empty() {
        out.push_str("[]");
        return Ok(());
    }
    write_array_dim(out, data, 0, 0, indent, level, depth)
}

fn write_array_dim(
    out: &mut String,
    data: &ArrayData,
    dim: usize,
    offset: usize,
    indent: usize,
    level: usize,
    depth: usize,
) -> Result<(), RuntimeError> {
    if depth > MAX_DEPTH {
        return Err(RuntimeError::new(
            "json.stringify(): value nested too deep (cyclic values cannot be stringified)",
        ));
    }
    let size = data.shape[dim];
    let stride = data.strides()[dim];
    if size == 0 {
        out.push_str("[]");
        return Ok(());
    }
    out.push('[');
    for i in 0..size {
        if i > 0 {
            out.push(',');
        }
        if indent > 0 {
            out.push('\n');
            out.push_str(&" ".repeat(indent * (level + 1)));
        }
        if dim + 1 == data.shape.len() {
            write_value(
                out,
                &data.items[offset + i * stride],
                indent,
                level + 1,
                depth + 1,
            )?;
        } else {
            write_array_dim(
                out,
                data,
                dim + 1,
                offset + i * stride,
                indent,
                level + 1,
                depth + 1,
            )?;
        }
    }
    if indent > 0 {
        out.push('\n');
        out.push_str(&" ".repeat(indent * level));
    }
    out.push(']');
    Ok(())
}
