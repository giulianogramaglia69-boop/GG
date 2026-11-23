// SPDX-License-Identifier: GPL-3.0-or-later
//! Native standard library modules, implemented in Rust.
//!
//! `import math` and `import random` resolve here, after `.gg`
//! files and the embedded GG stdlib (see
//! `Interpreter::import_module`): a user file with the same name
//! wins over the native module. Native modules are built once per
//! interpreter and cached there, so `import math` twice returns
//! the same value.
//!
//! `random` uses a SplitMix64 generator in pure Rust (no external
//! crate): it is fast and good enough for games and sampling, but
//! it is NOT cryptographic, and it is seeded from the wall clock
//! plus the process id, so every run gets a different sequence.

use std::io::{BufRead, Write};
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::builtin::{Arity, Builtin, BuiltinFn, BuiltinFunc};
use crate::env::Environment;
use crate::error::RuntimeError;
use crate::value::{ModuleData, Value};

/// Build a native module by name, or `None` when GG has no native
/// module with that name.
pub fn build(name: &str) -> Option<Value> {
    let env = Environment::new();
    match name {
        "math" => {
            define_functions(&env, MATH_FUNCTIONS);
            for (name, value) in MATH_CONSTANTS {
                env.define(name, Value::Float(*value));
            }
        }
        "random" => define_functions(&env, RANDOM_FUNCTIONS),
        "linalg" => define_functions(&env, LINALG_FUNCTIONS),
        "json" => define_functions(&env, JSON_FUNCTIONS),
        "os" => define_functions(&env, OS_FUNCTIONS),
        "datetime" => define_functions(&env, DATETIME_FUNCTIONS),
        "regex" => define_functions(&env, REGEX_FUNCTIONS),
        "http" => define_functions(&env, HTTP_FUNCTIONS),
        // The GUI module lives in `gui.rs`: it needs the
        // interpreter to call frame functions back, so its
        // functions are `WithInterp` builtins, not plain ones.
        #[cfg(feature = "gui")]
        "gui" => return Some(crate::gui::build()),
        _ => return None,
    }
    Some(Value::Module(Rc::new(ModuleData {
        name: name.to_string(),
        env,
    })))
}

/// One native function: its name, its arity and its implementation.
type NativeFn = (&'static str, Arity, BuiltinFn);

/// Define every function of a native module in its environment.
fn define_functions(env: &Environment, functions: &[NativeFn]) {
    for (name, arity, func) in functions {
        env.define(
            name,
            Value::Builtin(Rc::new(Builtin {
                name,
                arity: *arity,
                func: BuiltinFunc::Plain(*func),
            })),
        );
    }
}

const MATH_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("sqrt", Arity::Exact(1), sqrt),
    ("floor", Arity::Exact(1), floor),
    ("ceil", Arity::Exact(1), ceil),
    ("round", Arity::Exact(1), round),
    ("pow", Arity::Exact(2), pow),
    ("sin", Arity::Exact(1), sin),
    ("cos", Arity::Exact(1), cos),
    ("tan", Arity::Exact(1), tan),
    ("log", Arity::Exact(1), log),
    ("exp", Arity::Exact(1), exp),
];

const MATH_CONSTANTS: &[(&str, f64)] = &[("PI", std::f64::consts::PI), ("E", std::f64::consts::E)];

const RANDOM_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("randint", Arity::Exact(2), randint),
    ("random", Arity::Exact(0), random),
    ("choice", Arity::Exact(1), choice),
    ("shuffle", Arity::Exact(1), shuffle),
];

const LINALG_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("solve", Arity::Exact(2), crate::linalg::solve),
    ("det", Arity::Exact(1), crate::linalg::det),
    ("inv", Arity::Exact(1), crate::linalg::inv),
    ("rank", Arity::Exact(1), crate::linalg::rank),
];

const JSON_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("parse", Arity::Exact(1), crate::json::parse),
    ("stringify", Arity::Range(1, 2), crate::json::stringify),
];

const OS_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("args", Arity::Exact(0), crate::os::args),
    ("getenv", Arity::Range(1, 2), crate::os::getenv),
    ("cwd", Arity::Exact(0), crate::os::cwd),
    ("listdir", Arity::Exact(1), crate::os::listdir),
    ("mkdir", Arity::Exact(1), crate::os::mkdir),
    ("remove", Arity::Exact(1), crate::os::remove),
];

const DATETIME_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("now", Arity::Exact(0), crate::datetime::now),
    (
        "from_timestamp",
        Arity::Exact(1),
        crate::datetime::from_timestamp,
    ),
    (
        "to_timestamp",
        Arity::Exact(1),
        crate::datetime::to_timestamp,
    ),
    ("format", Arity::Exact(2), crate::datetime::format),
    ("date", Arity::Exact(3), crate::datetime::date),
];

const REGEX_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("test", Arity::Exact(2), crate::regex::test),
    ("find", Arity::Exact(2), crate::regex::find),
    ("groups", Arity::Exact(2), crate::regex::groups),
    ("replace", Arity::Exact(3), crate::regex::replace),
    ("split", Arity::Exact(2), crate::regex::split),
];

const HTTP_FUNCTIONS: &[(&str, Arity, BuiltinFn)] = &[
    ("get", Arity::Range(1, 2), crate::http::get),
    ("post", Arity::Range(2, 3), crate::http::post),
    ("request", Arity::Exact(4), crate::http::request),
];

/// A number as an f64, or a clear error naming the function.
fn as_number(name: &str, value: &Value) -> Result<f64, RuntimeError> {
    match value {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(x) => Ok(*x),
        other => Err(RuntimeError::new(format!(
            "{name}() expects a number, not {}",
            other.type_name()
        ))),
    }
}

/// An integer argument, or a clear error naming the function.
fn as_int(name: &str, value: &Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(n) => Ok(*n),
        other => Err(RuntimeError::new(format!(
            "{name}() expects an Int, not {}",
            other.type_name()
        ))),
    }
}

/// An f64 as an Int, with a clear error when it is not finite or
/// does not fit in an i64.
fn to_int_checked(name: &str, x: f64) -> Result<Value, RuntimeError> {
    if !x.is_finite() {
        return Err(RuntimeError::new(format!("{name}() result is not finite")));
    }
    if x < i64::MIN as f64 || x >= (i64::MAX as f64) {
        return Err(RuntimeError::new(format!("{name}() result out of range")));
    }
    Ok(Value::Int(x as i64))
}

/// `math.sqrt(x)` — the square root; an error on a negative number.
fn sqrt(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let x = as_number("math.sqrt", &args[0])?;
    if x < 0.0 {
        return Err(RuntimeError::new("math.sqrt() of a negative number"));
    }
    Ok(Value::Float(x.sqrt()))
}

/// `math.floor(x)` — the greatest integer <= x, as an Int.
fn floor(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let x = as_number("math.floor", &args[0])?;
    to_int_checked("math.floor", x.floor())
}

/// `math.ceil(x)` — the smallest integer >= x, as an Int.
fn ceil(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let x = as_number("math.ceil", &args[0])?;
    to_int_checked("math.ceil", x.ceil())
}

/// `math.round(x)` — the nearest integer, as an Int. Halves round
/// away from zero: `round(2.5)` is 3 and `round(-2.5)` is -3.
fn round(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let x = as_number("math.round", &args[0])?;
    to_int_checked("math.round", x.round())
}

/// `math.pow(a, b)` — a to the power of b. Two integers give an
/// exact integer (an overflow is an error); anything else gives a
/// float, and a negative integer exponent gives a float too.
fn pow(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) if *b >= 0 => a
            .checked_pow(*b as u32)
            .map(Value::Int)
            .ok_or_else(|| RuntimeError::new("integer overflow")),
        (a, b) => {
            let (Some(x), Some(y)) = (to_f64(a), to_f64(b)) else {
                return Err(RuntimeError::new("math.pow() expects numbers"));
            };
            Ok(Value::Float(x.powf(y)))
        }
    }
}

/// A value as an f64, if it is a number at all.
fn to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(n) => Some(*n as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

/// `math.sin(x)` / `math.cos(x)` / `math.tan(x)` — in radians.
fn trig(name: &str, args: &[Value], f: fn(f64) -> f64) -> Result<Value, RuntimeError> {
    let x = as_number(name, &args[0])?;
    Ok(Value::Float(f(x)))
}

fn sin(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    trig("math.sin", args, f64::sin)
}

fn cos(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    trig("math.cos", args, f64::cos)
}

fn tan(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    trig("math.tan", args, f64::tan)
}

/// `math.log(x)` — the natural logarithm; an error on x <= 0.
fn log(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let x = as_number("math.log", &args[0])?;
    if x <= 0.0 {
        return Err(RuntimeError::new("math.log() of a non-positive number"));
    }
    Ok(Value::Float(x.ln()))
}

/// `math.exp(x)` — e to the power of x.
fn exp(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let x = as_number("math.exp", &args[0])?;
    Ok(Value::Float(x.exp()))
}

// ---- random: SplitMix64 ----

/// SplitMix64: a small, fast generator with good statistical
/// quality. `next_u64` is the only primitive; the rest is built
/// on top of it.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A float in [0, 1): the top 53 bits of a random u64.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A uniform integer in [0, n): rejection sampling, so there
    /// is no modulo bias.
    fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        let zone = u64::MAX - u64::MAX % n;
        loop {
            let value = self.next_u64();
            if value < zone {
                return value % n;
            }
        }
    }
}

/// The process-wide generator, seeded once from the wall clock
/// (nanoseconds) mixed with the process id. Builtins are plain
/// function pointers with no interpreter state, so the generator
/// lives in a static; a `Mutex` guards it because GG programs
/// are single-threaded but the tests run in parallel.
static RNG: LazyLock<Mutex<SplitMix64>> = LazyLock::new(|| {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x1234_5678_9ABC_DEF0);
    let seed = nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    Mutex::new(SplitMix64 { state: seed })
});

/// `random.randint(a, b)` — a random integer between a and b,
/// both ends included; an error when a > b.
fn randint(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_int("random.randint", &args[0])?;
    let b = as_int("random.randint", &args[1])?;
    if a > b {
        return Err(RuntimeError::new("random.randint() expects a <= b"));
    }
    let mut rng = RNG.lock().unwrap();
    let span = b as i128 - a as i128 + 1;
    let value = if span >= (1i128 << 64) {
        // The whole i64 range: any 64 bits do.
        rng.next_u64() as i64
    } else {
        a + rng.below(span as u64) as i64
    };
    Ok(Value::Int(value))
}

/// `random.random()` — a random float in [0, 1).
fn random(
    _args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let mut rng = RNG.lock().unwrap();
    Ok(Value::Float(rng.next_f64()))
}

/// `random.choice(list)` — a random element of the list; an error
/// on an empty list.
fn choice(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::List(list) = &args[0] else {
        return Err(RuntimeError::new("random.choice() expects a List"));
    };
    let borrowed = list.borrow();
    if borrowed.is_empty() {
        return Err(RuntimeError::new("random.choice() on empty list"));
    }
    let mut rng = RNG.lock().unwrap();
    let index = rng.below(borrowed.len() as u64) as usize;
    Ok(borrowed[index].clone())
}

/// `random.shuffle(list)` — shuffle the list in place
/// (Fisher-Yates) and return `nil`.
fn shuffle(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::List(list) = &args[0] else {
        return Err(RuntimeError::new("random.shuffle() expects a List"));
    };
    let mut rng = RNG.lock().unwrap();
    let mut borrowed = list.borrow_mut();
    let mut i = borrowed.len();
    while i > 1 {
        i -= 1;
        let j = rng.below((i + 1) as u64) as usize;
        borrowed.swap(i, j);
    }
    Ok(Value::Nil)
}
