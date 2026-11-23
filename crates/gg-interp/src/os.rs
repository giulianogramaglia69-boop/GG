// SPDX-License-Identifier: GPL-3.0-or-later
//! The native `os` module: the process and the filesystem, in
//! pure Rust — no external crate.
//!
//! `import os` gives `os.args()`, `os.getenv(name)` /
//! `os.getenv(name, default)`, `os.cwd()`, `os.listdir(path)`,
//! `os.mkdir(path)` and `os.remove(path)`.
//!
//! `os.args()` is the process command line as a List of Str:
//! the first element is the program name, the rest are the
//! arguments given to `gg` (for `gg run file.gg` that is
//! `["gg", "run", "file.gg"]` — `gg run` takes no script
//! arguments of its own). `os.getenv` returns Nil when the
//! variable is not set, unless a default is given. `listdir`
//! returns the entry names only (not full paths), sorted, and
//! is not recursive. `mkdir` creates one directory only: it
//! errors when the directory already exists or the parent is
//! missing. `remove` deletes a file: it errors on a directory
//! and on a path that does not exist.
//!
//! Out of scope, by design: running commands (`os.system`)
//! and writing environment variables — GG programs observe
//! the environment, they do not change it or spawn processes.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::path::Path;
use std::rc::Rc;

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

/// `os.args()` — the command line as a List of Str, program
/// name first. Non-UTF-8 arguments are lossily converted,
/// like a shell would show them.
pub fn args(
    _args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let items: Vec<Value> = std::env::args_os()
        .map(|a| Value::Str(a.to_string_lossy().into_owned()))
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(items))))
}

/// `os.getenv(name)` / `os.getenv(name, default)` — the value
/// of an environment variable as a Str, Nil when it is not set
/// and no default is given, the default otherwise. A variable
/// set to a non-UTF-8 value is lossily converted.
pub fn getenv(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let name = as_str("os.getenv", &args[0])?;
    match std::env::var_os(&name) {
        Some(value) => Ok(Value::Str(value.to_string_lossy().into_owned())),
        None => match args.get(1) {
            Some(default) => Ok(Value::Str(as_str("os.getenv", default)?)),
            None => Ok(Value::Nil),
        },
    }
}

/// `os.cwd()` — the current working directory as a Str.
pub fn cwd(
    _args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match std::env::current_dir() {
        Ok(path) => Ok(Value::Str(path.to_string_lossy().into_owned())),
        Err(e) => Err(RuntimeError::new(format!("os.cwd(): {e}"))),
    }
}

/// `os.listdir(path)` — the names in a directory as a List of
/// Str, sorted, not recursive. An error when the path does not
/// exist or is not a directory.
pub fn listdir(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("os.listdir", &args[0])?;
    let entries = match std::fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(e) => {
            return Err(RuntimeError::new(format!("os.listdir(\"{path}\"): {e}")));
        }
    };
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => names.push(entry.file_name().to_string_lossy().into_owned()),
            Err(e) => {
                return Err(RuntimeError::new(format!("os.listdir(\"{path}\"): {e}")));
            }
        }
    }
    names.sort();
    let items: Vec<Value> = names.into_iter().map(Value::Str).collect();
    Ok(Value::List(Rc::new(RefCell::new(items))))
}

/// `os.mkdir(path)` — create one directory, returning Nil.
/// An error when it already exists or the parent is missing:
/// no recursive creation.
pub fn mkdir(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("os.mkdir", &args[0])?;
    match std::fs::create_dir(&path) {
        Ok(()) => Ok(Value::Nil),
        Err(e) => Err(RuntimeError::new(format!("os.mkdir(\"{path}\"): {e}"))),
    }
}

/// `os.remove(path)` — delete a file, returning Nil. An error
/// when the path does not exist or is a directory.
pub fn remove(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("os.remove", &args[0])?;
    let target = Path::new(&path);
    if !target.exists() {
        return Err(RuntimeError::new(format!(
            "os.remove(\"{path}\"): no such file or directory"
        )));
    }
    if target.is_dir() {
        return Err(RuntimeError::new(format!(
            "os.remove(\"{path}\"): is a directory"
        )));
    }
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(Value::Nil),
        Err(e) => Err(RuntimeError::new(format!("os.remove(\"{path}\"): {e}"))),
    }
}
