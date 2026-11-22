// SPDX-License-Identifier: GPL-3.0-or-later
//! Tree-walking interpreter for the GG programming language.
//!
//! Phase 3: `Interpreter` runs a parsed `Program`; `run_source` is
//! the one-call pipeline (lex + parse + interpret) used by the `gg`
//! binary and by the tests.

pub mod array;
pub mod builtin;
pub mod env;
pub mod error;
#[cfg(feature = "gui")]
pub mod gui;
pub mod interp;
pub mod linalg;
pub mod native;
pub mod repl;
pub mod slice;
pub mod stdlib;
pub mod value;
pub mod venv;

use std::fmt;
use std::io::{BufRead, Write};

pub use builtin::Builtin;
pub use error::RuntimeError;
pub use interp::Interpreter;
pub use repl::{run_repl, run_repl_with_venv};
pub use value::Value;
pub use venv::{create_venv, venv_dirs_from_env, venv_lib_dir};

use gg_lexer::LexerError;
use gg_parser::ParserError;

/// Any error the whole pipeline can produce, with a prefix saying
/// which stage failed.
#[derive(Debug)]
pub enum GgError {
    Lex(LexerError),
    Parse(ParserError),
    Runtime(RuntimeError),
}

impl fmt::Display for GgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GgError::Lex(e) => write!(f, "lex error: {e}"),
            GgError::Parse(e) => write!(f, "parse error: {e}"),
            GgError::Runtime(e) => write!(f, "runtime error: {e}"),
        }
    }
}

impl std::error::Error for GgError {}

impl From<LexerError> for GgError {
    fn from(e: LexerError) -> Self {
        GgError::Lex(e)
    }
}

impl From<ParserError> for GgError {
    fn from(e: ParserError) -> Self {
        GgError::Parse(e)
    }
}

impl From<RuntimeError> for GgError {
    fn from(e: RuntimeError) -> Self {
        GgError::Runtime(e)
    }
}

/// Lex, parse and run `source`: `print` output goes to `out`,
/// `input` reads from `input`.
pub fn run_source(
    source: &str,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(), GgError> {
    let tokens = gg_lexer::lex(source)?;
    let program = gg_parser::parse(&tokens)?;
    Interpreter::new().run(&program, input, out)?;
    Ok(())
}

/// Like `run_source`, but with a module search configured: imports
/// in `source` resolve against `base_dir` (the directory of the
/// script, if any), then the given venv `lib/` directories, then
/// the working directory. Used by the `gg` binary for `gg run` and
/// by the module tests.
pub fn run_source_with(
    source: &str,
    base_dir: Option<std::path::PathBuf>,
    venv_dirs: Vec<std::path::PathBuf>,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(), GgError> {
    let tokens = gg_lexer::lex(source)?;
    let program = gg_parser::parse(&tokens)?;
    let mut interp = Interpreter::new();
    interp.set_base_dir(base_dir);
    interp.set_venv_dirs(venv_dirs);
    interp.run(&program, input, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `source` with empty input and return its printed output,
    /// or the error text.
    fn run_ok(source: &str) -> String {
        run_ok_in(source, "")
    }

    /// Run `source` with `input` as stdin and return its printed
    /// output, or the error text.
    fn run_ok_in(source: &str, input: &str) -> String {
        let mut input = std::io::Cursor::new(input.as_bytes().to_vec());
        let mut out = Vec::new();
        run_source(source, &mut input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn run_err(source: &str) -> String {
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        let err = run_source(source, &mut input, &mut out).unwrap_err();
        err.to_string()
    }

    #[test]
    fn hello_world() {
        assert_eq!(run_ok("print \"hello\"\n"), "hello\n");
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(run_ok("print 1 + 2 * 3\n"), "7\n");
        assert_eq!(run_ok("print (1 + 2) * 3\n"), "9\n");
        assert_eq!(run_ok("print 10 - 4 - 3\n"), "3\n");
        assert_eq!(run_ok("print 2.5 + 1\n"), "3.5\n");
        assert_eq!(run_ok("print -5 + 2\n"), "-3\n");
    }

    #[test]
    fn string_concatenation() {
        assert_eq!(run_ok("print \"foo\" + \"bar\"\n"), "foobar\n");
        assert_eq!(run_ok("print \"a\" + \" \" + \"b\"\n"), "a b\n");
    }

    #[test]
    fn division_is_always_float_and_modulo_is_integer() {
        assert_eq!(run_ok("print 7 / 2\n"), "3.5\n");
        assert_eq!(run_ok("print 6 / 3\n"), "2\n");
        assert_eq!(run_ok("print 7 % 3\n"), "1\n");
        // Rust remainder: the sign follows the dividend.
        assert_eq!(run_ok("print -7 % 3\n"), "-1\n");
    }

    #[test]
    fn if_else_runs_the_right_branch() {
        let src = "let x = 10\nif x > 5 then\n    print \"big\"\nelse\n    print \"small\"\nend\n";
        assert_eq!(run_ok(src), "big\n");
        let src = "let x = 2\nif x > 5 then\n    print \"big\"\nelse\n    print \"small\"\nend\n";
        assert_eq!(run_ok(src), "small\n");
    }

    #[test]
    fn while_counts_down() {
        let src = "let i = 3\nwhile i > 0 then\n    print i\n    i = i - 1\nend\n";
        assert_eq!(run_ok(src), "3\n2\n1\n");
    }

    #[test]
    fn for_is_inclusive_both_ways() {
        assert_eq!(
            run_ok("for i = 1 to 3 then\n    print i\nend\n"),
            "1\n2\n3\n"
        );
        assert_eq!(
            run_ok("for i = 3 to 1 then\n    print i\nend\n"),
            "3\n2\n1\n"
        );
        // A single value still runs once.
        assert_eq!(run_ok("for i = 5 to 5 then\n    print i\nend\n"), "5\n");
    }

    #[test]
    fn for_in_over_a_list_and_range() {
        assert_eq!(
            run_ok("let xs = [1, 2, 3]\nfor x in xs then\n    print x\nend\n"),
            "1\n2\n3\n"
        );
        // `range()` returns a list, so it works directly.
        assert_eq!(
            run_ok("for x in range(3) then\n    print x\nend\n"),
            "0\n1\n2\n"
        );
        // An empty list runs zero times.
        assert_eq!(
            run_ok("for x in [] then\n    print x\nend\nprint \"done\"\n"),
            "done\n"
        );
        // The iterable is evaluated once: reassigning the name
        // inside the body does not change what the loop visits.
        assert_eq!(
            run_ok("let xs = [1, 2]\nfor x in xs then\n    print x\n    xs = [9]\nend\n"),
            "1\n2\n"
        );
    }

    #[test]
    fn for_in_over_tuples_strings_dicts_and_sets() {
        assert_eq!(
            run_ok("for x in (10, \"a\") then\n    print x\nend\n"),
            "10\na\n"
        );
        // Strings iterate one character at a time (Unicode).
        assert_eq!(run_ok("for c in \"aè\" then\n    print c\nend\n"), "a\nè\n");
        // Dicts iterate their keys, in insertion order.
        assert_eq!(
            run_ok("let d = {\"a\": 1, \"b\": 2}\nfor k in d then\n    print k\nend\n"),
            "a\nb\n"
        );
        // Sets iterate in insertion order, duplicates dropped.
        assert_eq!(
            run_ok("for x in {3, 1, 2, 2} then\n    print x\nend\n"),
            "3\n1\n2\n"
        );
    }

    #[test]
    fn for_in_over_arrays() {
        // A 1D array iterates its elements.
        assert_eq!(
            run_ok("let a = array([10, 20])\nfor x in a then\n    print x\nend\n"),
            "10\n20\n"
        );
        // A 2D array iterates its rows (1D arrays).
        assert_eq!(
            run_ok(
                "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nfor r in m then\n    print r[0] + r[1]\nend\n"
            ),
            "3\n7\n"
        );
    }

    #[test]
    fn for_in_does_not_consume_queues_and_stacks() {
        // Iterating a queue or stack only snapshots it: the
        // collection is unchanged afterwards, and pushes inside
        // the body do not extend the loop.
        assert_eq!(
            run_ok(
                "let q = queue([1, 2])\nfor x in q then\n    print x\n    enqueue(q, 99)\nend\nprint q\n"
            ),
            "1\n2\nqueue([1, 2, 99, 99])\n"
        );
        assert_eq!(
            run_ok("let s = stack([1, 2])\nfor x in s then\n    print x\nend\nprint s\n"),
            "1\n2\nstack([1, 2])\n"
        );
    }

    #[test]
    fn for_in_nests_and_works_inside_functions() {
        assert_eq!(
            run_ok(
                "for x in [1, 2] then\n    for y in [10, 20] then\n        print x * 10 + y\n    end\nend\n"
            ),
            "20\n30\n30\n40\n"
        );
        let src = "fn somma(xs)\n    let tot = 0\n    for x in xs then\n        tot = tot + x\n    end\n    return tot\nend\nprint somma([1, 2, 3])\n";
        assert_eq!(run_ok(src), "6\n");
    }

    #[test]
    fn for_in_loop_variable_is_scoped_to_the_body() {
        let src = "for x in [1] then\n    print x\nend\nprint x\n";
        assert!(run_err(src).contains("undefined variable 'x'"));
        // An outer variable with the same name is untouched.
        let src = "let x = 0\nfor x in [7] then\n    print x\nend\nprint x\n";
        assert_eq!(run_ok(src), "7\n0\n");
    }

    #[test]
    fn for_in_over_a_non_iterable_is_an_error() {
        assert!(run_err("for x in 5 then\n    print x\nend\n").contains("Int is not iterable"));
        // A builtin function is a value too — and not iterable.
        assert!(
            run_err("for x in len then\n    print x\nend\n").contains("Function is not iterable")
        );
        assert!(run_err("for x in true then\n    print x\nend\n").contains("Bool is not iterable"));
    }

    #[test]
    fn let_inside_a_block_is_not_visible_outside() {
        let src = "if true then\n    let y = 1\nend\nprint y\n";
        assert!(run_err(src).contains("undefined variable 'y'"));
    }

    #[test]
    fn let_shadows_and_assignment_updates_the_outer_scope() {
        let src = "let x = 1\nif true then\n    let x = 2\n    print x\nend\nprint x\n";
        assert_eq!(run_ok(src), "2\n1\n");
        let src = "let x = 1\nif true then\n    x = 2\nend\nprint x\n";
        assert_eq!(run_ok(src), "2\n");
    }

    #[test]
    fn loop_variable_is_not_visible_after_the_loop() {
        let src = "for i = 1 to 2 then\n    print i\nend\nprint i\n";
        assert!(run_err(src).contains("undefined variable 'i'"));
    }

    #[test]
    fn functions_return_values() {
        let src = "fn add(a, b)\n    return a + b\nend\nprint add(2, 3)\n";
        assert_eq!(run_ok(src), "5\n");
        // No return value means Nil.
        let src = "fn nothing()\nend\nprint nothing()\n";
        assert_eq!(run_ok(src), "nil\n");
        // A bare return means Nil too.
        let src = "fn early()\n    return\nend\nprint early()\n";
        assert_eq!(run_ok(src), "nil\n");
    }

    #[test]
    fn recursion_factorial() {
        let src = "fn fact(n)\n    if n <= 1 then\n        return 1\n    else\n        return n * fact(n - 1)\n    end\nend\nprint fact(5)\n";
        assert_eq!(run_ok(src), "120\n");
    }

    #[test]
    fn closures_capture_the_defining_scope() {
        let src = "fn make_adder(x)\n    fn adder(y)\n        return x + y\n    end\n    return adder\nend\nlet add5 = make_adder(5)\nprint add5(3)\n";
        assert_eq!(run_ok(src), "8\n");
    }

    #[test]
    fn functions_see_globals() {
        let src = "let g = 10\nfn get_g()\n    return g\nend\nprint get_g()\n";
        assert_eq!(run_ok(src), "10\n");
    }

    #[test]
    fn truthiness_and_short_circuit_values() {
        assert_eq!(run_ok("print not false\n"), "true\n");
        assert_eq!(run_ok("print not 0\n"), "false\n");
        assert_eq!(run_ok("print 1 and 2\n"), "2\n");
        assert_eq!(run_ok("print false and 2\n"), "false\n");
        assert_eq!(run_ok("print false or \"x\"\n"), "x\n");
        assert_eq!(run_ok("print 1 or 2\n"), "1\n");
        // Short-circuit: the right side is never evaluated.
        assert_eq!(run_ok("print false and undefined_var\n"), "false\n");
        assert_eq!(run_ok("print true or undefined_var\n"), "true\n");
    }

    #[test]
    fn comparisons_and_mixed_type_equality() {
        assert_eq!(run_ok("print \"abc\" < \"abd\"\n"), "true\n");
        assert_eq!(run_ok("print 1 == 1.0\n"), "true\n");
        assert_eq!(run_ok("print 1 == \"1\"\n"), "false\n");
        assert_eq!(run_ok("print 1 != \"1\"\n"), "true\n");
        assert_eq!(run_ok("print 2 <= 2\n"), "true\n");
        assert_eq!(run_ok("print 3 >= 4\n"), "false\n");
    }

    #[test]
    fn undefined_variable_is_an_error() {
        assert!(run_err("print z\n").contains("undefined variable 'z'"));
        assert!(run_err("x = 5\n").contains("undefined variable 'x'"));
    }

    #[test]
    fn type_errors_have_clear_messages() {
        assert!(run_err("print 1 + \"a\"\n").contains("cannot add Int and Str"));
        assert!(run_err("print \"a\" - 1\n").contains("cannot subtract Str and Int"));
        assert!(run_err("print \"a\" < 1\n").contains("cannot compare Str and Int"));
        assert!(run_err("print -\"a\"\n").contains("cannot negate Str"));
    }

    #[test]
    fn division_and_modulo_by_zero_are_errors() {
        assert!(run_err("print 1 / 0\n").contains("division by zero"));
        assert!(run_err("print 1.0 / 0.0\n").contains("division by zero"));
        assert!(run_err("print 1 % 0\n").contains("modulo by zero"));
    }

    #[test]
    fn calling_a_non_function_is_an_error() {
        assert!(run_err("let x = 1\nprint x()\n").contains("cannot call Int"));
    }

    #[test]
    fn wrong_argument_count_is_an_error() {
        let src = "fn f(a)\n    return a\nend\nprint f(1, 2)\n";
        assert!(run_err(src).contains("expected 1 argument(s) but got 2"));
    }

    #[test]
    fn return_outside_a_function_is_an_error() {
        assert!(run_err("return 1\n").contains("return outside function"));
    }

    #[test]
    fn error_prefixes_name_the_failing_stage() {
        assert!(run_err("print \"oops\n").starts_with("lex error:"));
        assert!(run_err("let = 5\n").starts_with("parse error:"));
        assert!(run_err("print z\n").starts_with("runtime error:"));
    }

    // ---- Phase 4: builtins ----

    #[test]
    fn len_counts_characters_not_bytes() {
        assert_eq!(run_ok("print len(\"ciao\")\n"), "4\n");
        assert_eq!(run_ok("print len(\"\")\n"), "0\n");
        // "è" is one character but two UTF-8 bytes.
        assert_eq!(run_ok("print len(\"è\")\n"), "1\n");
        assert_eq!(run_ok("print len(\"héllo\")\n"), "5\n");
        assert!(run_err("print len(5)\n").contains("len() expects a Str"));
        assert!(run_err("print len()\n").contains("len() expects 1 argument(s) but got 0"));
        assert!(
            run_err("print len(\"a\", \"b\")\n").contains("len() expects 1 argument(s) but got 2")
        );
    }

    #[test]
    fn type_names_every_value() {
        assert_eq!(run_ok("print type(1)\n"), "int\n");
        assert_eq!(run_ok("print type(1.5)\n"), "float\n");
        assert_eq!(run_ok("print type(\"s\")\n"), "str\n");
        assert_eq!(run_ok("print type(true)\n"), "bool\n");
        let src = "fn nothing()\nend\nprint type(nothing())\n";
        assert_eq!(run_ok(src), "nil\n");
        let src = "fn f()\nend\nprint type(f)\n";
        assert_eq!(run_ok(src), "fn\n");
        // Builtins are functions too.
        assert_eq!(run_ok("print type(len)\n"), "fn\n");
        assert!(run_err("print type()\n").contains("type() expects 1 argument(s) but got 0"));
    }

    #[test]
    fn str_converts_like_print() {
        assert_eq!(run_ok("print str(42)\n"), "42\n");
        assert_eq!(run_ok("print str(3.5)\n"), "3.5\n");
        assert_eq!(run_ok("print str(true)\n"), "true\n");
        assert_eq!(run_ok("print str(\"x\")\n"), "x\n");
        let src = "fn nothing()\nend\nprint str(nothing())\n";
        assert_eq!(run_ok(src), "nil\n");
        assert_eq!(run_ok("print str(len)\n"), "<fn>\n");
    }

    #[test]
    fn int_converts_and_truncates_toward_zero() {
        assert_eq!(run_ok("print int(3.9)\n"), "3\n");
        assert_eq!(run_ok("print int(-3.9)\n"), "-3\n");
        assert_eq!(run_ok("print int(\"42\")\n"), "42\n");
        assert_eq!(run_ok("print int(\"  -7  \")\n"), "-7\n");
        assert_eq!(run_ok("print int(true)\n"), "1\n");
        assert_eq!(run_ok("print int(false)\n"), "0\n");
        assert_eq!(run_ok("print int(5)\n"), "5\n");
        assert!(
            run_err("print int(\"ciao\")\n").contains("int() cannot parse \"ciao\" as an integer")
        );
        assert!(
            run_err("print int(\"3.5\")\n").contains("int() cannot parse \"3.5\" as an integer")
        );
        let src = "fn nothing()\nend\nprint int(nothing())\n";
        assert!(run_err(src).contains("int() cannot convert Nil to Int"));
        assert!(run_err("print int(len)\n").contains("int() cannot convert Function to Int"));
    }

    #[test]
    fn float_converts() {
        assert_eq!(run_ok("print type(float(3))\n"), "float\n");
        assert_eq!(run_ok("print float(3)\n"), "3\n");
        assert_eq!(run_ok("print float(\"2.5\")\n"), "2.5\n");
        assert_eq!(run_ok("print float(true)\n"), "1\n");
        assert_eq!(run_ok("print float(false)\n"), "0\n");
        assert_eq!(run_ok("print float(2.5)\n"), "2.5\n");
        assert!(
            run_err("print float(\"ciao\")\n").contains("float() cannot parse \"ciao\" as a float")
        );
        let src = "fn nothing()\nend\nprint float(nothing())\n";
        assert!(run_err(src).contains("float() cannot convert Nil to Float"));
    }

    #[test]
    fn input_reads_a_line_and_writes_the_prompt() {
        let src = "let name = input(\"Name: \")\nprint \"Hi \" + name\n";
        assert_eq!(run_ok_in(src, "Giuliano\n"), "Name: Hi Giuliano\n");
        // The newline is stripped, and a second read gets the next line.
        let src = "print input(\"A: \")\nprint input(\"B: \")\n";
        assert_eq!(run_ok_in(src, "one\ntwo\n"), "A: one\nB: two\n");
        // A CRLF ending is stripped too.
        let src = "print input(\"P: \")\n";
        assert_eq!(run_ok_in(src, "x\r\n"), "P: x\n");
        // At end of input, input returns "".
        let src = "print \"[\" + input(\"P: \") + \"]\"\n";
        assert_eq!(run_ok_in(src, ""), "P: []\n");
        assert!(run_err("print input()\n").contains("input() expects 1 argument(s) but got 0"));
        assert!(run_err("print input(5)\n").contains("input() expects a Str"));
    }

    #[test]
    fn abs_min_max() {
        assert_eq!(run_ok("print abs(-5)\n"), "5\n");
        assert_eq!(run_ok("print abs(5)\n"), "5\n");
        assert_eq!(run_ok("print abs(-2.5)\n"), "2.5\n");
        assert_eq!(run_ok("print abs(0)\n"), "0\n");
        assert!(run_err("print abs(\"a\")\n").contains("abs() expects a number"));
        assert_eq!(run_ok("print min(2, 5)\n"), "2\n");
        assert_eq!(run_ok("print max(2, 5)\n"), "5\n");
        assert_eq!(run_ok("print min(5, 2)\n"), "2\n");
        // Mixed int/float gives a float result.
        assert_eq!(run_ok("print type(min(2.5, 2))\n"), "float\n");
        assert_eq!(run_ok("print min(2.5, 2)\n"), "2\n");
        assert_eq!(run_ok("print max(2, 2.5)\n"), "2.5\n");
        assert!(run_err("print min(\"a\", 1)\n").contains("min() expects numbers"));
        // One argument is the array form: a single number is an error.
        assert!(run_err("print max(1)\n").contains("max() expects two numbers or one Array"));
    }

    #[test]
    fn builtins_are_first_class_values() {
        // Assigned to a variable, then called through it.
        assert_eq!(run_ok("let f = len\nprint f(\"ciao\")\n"), "4\n");
        // Passed to a user function.
        let src = "fn apply(g, x)\n    return g(x)\nend\nprint apply(len, \"abc\")\n";
        assert_eq!(run_ok(src), "3\n");
        // Returned from a user function.
        let src = "fn get_len()\n    return len\nend\nlet g = get_len()\nprint g(\"hi\")\n";
        assert_eq!(run_ok(src), "2\n");
        // A builtin prints like any function.
        assert_eq!(run_ok("print len\n"), "<fn>\n");
        // Builtins compare by identity: the same builtin is equal to itself.
        assert_eq!(run_ok("print len == len\n"), "true\n");
        assert_eq!(run_ok("print len == str\n"), "false\n");
    }

    #[test]
    fn let_shadows_a_builtin_in_the_current_scope() {
        // Shadowing works like any variable: the builtin is hidden.
        assert_eq!(run_ok("let len = 5\nprint len\n"), "5\n");
        assert!(run_err("let len = 5\nprint len(\"a\")\n").contains("cannot call Int"));
        // The shadow is local: outside the block the builtin is back.
        let src = "if true then\n    let len = 1\nend\nprint len(\"ab\")\n";
        assert_eq!(run_ok(src), "2\n");
    }

    // ---- Phase 6: lists ----

    #[test]
    fn list_literals_print() {
        assert_eq!(run_ok("print [1, 2, 3]\n"), "[1, 2, 3]\n");
        assert_eq!(run_ok("print []\n"), "[]\n");
        assert_eq!(
            run_ok("print [1, \"ciao\", true]\n"),
            "[1, \"ciao\", true]\n"
        );
        assert_eq!(run_ok("print [[1, 2], [3]]\n"), "[[1, 2], [3]]\n");
        assert_eq!(run_ok("print [1 + 2, len(\"ab\")]\n"), "[3, 2]\n");
    }

    #[test]
    fn indexing_positive_and_negative() {
        let src = "let xs = [10, 20, 30]\nprint xs[0]\nprint xs[2]\nprint xs[-1]\nprint xs[-3]\n";
        assert_eq!(run_ok(src), "10\n30\n30\n10\n");
        // Indexing a literal and a call result.
        assert_eq!(run_ok("print [7, 8][1]\n"), "8\n");
        let src = "fn get()\n    return [1, 2]\nend\nprint get()[1]\n";
        assert_eq!(run_ok(src), "2\n");
    }

    #[test]
    fn indexing_errors() {
        assert!(run_err("let xs = [1]\nprint xs[1]\n").contains("list index out of range"));
        assert!(run_err("let xs = [1]\nprint xs[-2]\n").contains("list index out of range"));
        assert!(
            run_err("let xs = [1]\nprint xs[\"a\"]\n").contains("list index must be an integer")
        );
        assert!(run_err("let xs = [1]\nprint xs[1.0]\n").contains("list index must be an integer"));
        assert!(run_err("print 5[0]\n").contains("cannot index Int"));
        // A string takes a slice, not a single index (Phase 12).
        assert!(run_err("print \"abc\"[0]\n").contains("cannot index a string with an integer"));
    }

    #[test]
    fn index_assignment_including_nested() {
        let src = "let xs = [1, 2, 3]\nxs[0] = 99\nprint xs\n";
        assert_eq!(run_ok(src), "[99, 2, 3]\n");
        let src = "let xs = [1, 2, 3]\nxs[-1] = 9\nprint xs\n";
        assert_eq!(run_ok(src), "[1, 2, 9]\n");
        let src = "let m = [[1, 2], [3, 4]]\nm[0][1] = 20\nm[1][0] = 30\nprint m\n";
        assert_eq!(run_ok(src), "[[1, 20], [30, 4]]\n");
        // The index may be an expression.
        let src = "let xs = [0, 0]\nxs[1 - 1] = 5\nprint xs\n";
        assert_eq!(run_ok(src), "[5, 0]\n");
        // Errors: out of range, and a non-list target.
        assert!(run_err("let xs = [1]\nxs[5] = 2\n").contains("list index out of range"));
        assert!(run_err("let x = 5\nx[0] = 2\n").contains("cannot index Int"));
        // A call result is not a valid assignment target.
        assert!(run_err("f() = 1\n").contains("invalid assignment target"));
    }

    #[test]
    fn list_aliasing_shares_the_list() {
        let src = "let a = [1, 2]\nlet b = a\npush(b, 3)\nprint a\nprint b\n";
        assert_eq!(run_ok(src), "[1, 2, 3]\n[1, 2, 3]\n");
        // Index assignment through the alias is visible too.
        let src = "let a = [1]\nlet b = a\nb[0] = 9\nprint a[0]\n";
        assert_eq!(run_ok(src), "9\n");
    }

    #[test]
    fn len_on_lists() {
        assert_eq!(run_ok("print len([1, 2, 3])\n"), "3\n");
        assert_eq!(run_ok("print len([])\n"), "0\n");
        assert_eq!(run_ok("print len([[1], [2, 3]])\n"), "2\n");
        assert!(
            run_err("print len(5)\n")
                .contains("len() expects a Str, List, Tuple, Dict, Set, Array, Queue or Stack")
        );
    }

    #[test]
    fn push_and_pop() {
        let src = "let xs = []\npush(xs, 1)\npush(xs, 2)\nprint xs\nprint pop(xs)\nprint xs\n";
        assert_eq!(run_ok(src), "[1, 2]\n2\n[1]\n");
        // push returns nil.
        assert_eq!(run_ok("let xs = []\nprint push(xs, 1)\n"), "nil\n");
        assert!(run_err("print pop([])\n").contains("pop() on empty list"));
        assert!(run_err("print push(5, 1)\n").contains("push() expects a List or a Stack"));
        assert!(run_err("print pop(\"s\")\n").contains("pop() expects a List or a Stack"));
    }

    #[test]
    fn range_forms() {
        assert_eq!(run_ok("print range(3)\n"), "[0, 1, 2]\n");
        assert_eq!(run_ok("print range(0)\n"), "[]\n");
        assert_eq!(run_ok("print range(2, 5)\n"), "[2, 3, 4]\n");
        assert_eq!(run_ok("print range(3, 3)\n"), "[]\n");
        assert_eq!(run_ok("print range(5, 2)\n"), "[]\n");
        assert_eq!(run_ok("print range(-2, 2)\n"), "[-2, -1, 0, 1]\n");
        assert_eq!(run_ok("print len(range(100))\n"), "100\n");
        assert!(
            run_err("print range()\n").contains("range() expects 1 or 2 argument(s) but got 0")
        );
        assert!(
            run_err("print range(1, 2, 3)\n")
                .contains("range() expects 1 or 2 argument(s) but got 3")
        );
        assert!(run_err("print range(\"a\")\n").contains("range() expects an Int"));
        assert!(run_err("print range(0, 1000001)\n").contains("range() is too large"));
    }

    #[test]
    fn split_and_lines() {
        assert_eq!(
            run_ok("print split(\"a,b,c\", \",\")\n"),
            "[\"a\", \"b\", \"c\"]\n"
        );
        assert_eq!(run_ok("print split(\"abc\", \",\")\n"), "[\"abc\"]\n");
        assert_eq!(run_ok("print split(\"\", \",\")\n"), "[\"\"]\n");
        assert_eq!(
            run_ok("print split(\"a--b\", \"--\")\n"),
            "[\"a\", \"b\"]\n"
        );
        assert!(
            run_err("print split(\"a\", \"\")\n").contains("split() separator must not be empty")
        );
        assert!(run_err("print split(5, \",\")\n").contains("split() expects a Str"));
        assert_eq!(
            run_ok("print lines(\"a\\nb\\nc\")\n"),
            "[\"a\", \"b\", \"c\"]\n"
        );
        assert_eq!(run_ok("print lines(\"a\\nb\\n\")\n"), "[\"a\", \"b\"]\n");
        assert_eq!(run_ok("print lines(\"\")\n"), "[]\n");
        assert_eq!(run_ok("print lines(\"solo\")\n"), "[\"solo\"]\n");
    }

    #[test]
    fn list_equality() {
        assert_eq!(run_ok("print [1, 2] == [1, 2]\n"), "true\n");
        assert_eq!(run_ok("print [1, 2] == [1, 3]\n"), "false\n");
        assert_eq!(run_ok("print [1, 2] == [1, 2, 3]\n"), "false\n");
        assert_eq!(run_ok("print [] == []\n"), "true\n");
        assert_eq!(run_ok("print [1, [2]] == [1, [2]]\n"), "true\n");
        assert_eq!(run_ok("print [1] == 1\n"), "false\n");
        // The same list is equal to itself, even when cyclic.
        let src = "let a = [1]\npush(a, a)\nprint a == a\n";
        assert_eq!(run_ok(src), "true\n");
        // Two distinct cyclic lists are reported unequal (and the
        // comparison terminates instead of recursing forever).
        let src = "let a = [1]\npush(a, a)\nlet b = [1]\npush(b, b)\nprint a == b\n";
        assert_eq!(run_ok(src), "false\n");
    }

    #[test]
    fn list_display_quotes_strings_and_caps_depth() {
        assert_eq!(
            run_ok("print [\"a\", \"b\\n c\"]\n"),
            "[\"a\", \"b\\n c\"]\n"
        );
        assert_eq!(run_ok("print type([1])\n"), "list\n");
        assert_eq!(run_ok("print str([1, \"x\"])\n"), "[1, \"x\"]\n");
        // A self-referential list prints with `...` past the depth
        // cap instead of recursing forever.
        let src = "let a = [1]\npush(a, a)\nprint a\n";
        let out = run_ok(src);
        assert!(out.contains("..."), "{out}");
        assert!(out.starts_with("[1, [1,"), "{out}");
    }

    #[test]
    fn iterating_a_list_with_for_and_len() {
        let src = "let xs = [10, 20, 30]\nfor i = 0 to len(xs) - 1 then\n    print xs[i]\nend\n";
        assert_eq!(run_ok(src), "10\n20\n30\n");
    }

    // ---- Phase 6: files ----

    /// A fresh scratch directory for a file test (unique per test
    /// tag, removed when the test is done).
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gg-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_write_read_append_and_exists() {
        let dir = temp_dir("files");
        let path = dir.join("hello.txt");
        let path_str = path.to_str().unwrap();
        let src = format!(
            "print file_exists(\"{path_str}\")\nwrite_file(\"{path_str}\", \"ciao\")\nprint file_exists(\"{path_str}\")\nprint read_file(\"{path_str}\")\nappend_file(\"{path_str}\", \" mondo\")\nprint read_file(\"{path_str}\")\n"
        );
        assert_eq!(run_ok(&src), "false\ntrue\nciao\nciao mondo\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ciao mondo");
        // `lines` strips the `\r` of CRLF endings too (the file is
        // written from Rust here, since GG string literals have no
        // `\r` escape).
        let crlf = dir.join("crlf.txt");
        std::fs::write(&crlf, "a\r\nb\r\n").unwrap();
        let crlf_str = crlf.to_str().unwrap();
        let src = format!("print lines(read_file(\"{crlf_str}\"))\n");
        assert_eq!(run_ok(&src), "[\"a\", \"b\"]\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_errors() {
        let dir = temp_dir("file-errors");
        let missing = dir.join("nope.txt");
        let missing_str = missing.to_str().unwrap();
        let err = run_err(&format!("print read_file(\"{missing_str}\")\n"));
        assert!(
            err.contains(&format!(
                "read_file(\"{missing_str}\"): No such file or directory"
            )),
            "{err}"
        );
        // Writing where the parent directory does not exist fails.
        let bad = dir.join("no-such-dir").join("f.txt");
        let bad_str = bad.to_str().unwrap();
        let err = run_err(&format!("write_file(\"{bad_str}\", \"x\")\n"));
        assert!(
            err.contains(&format!("write_file(\"{bad_str}\"):")),
            "{err}"
        );
        assert!(err.contains("No such file or directory"), "{err}");
        // Non-string arguments are type errors.
        assert!(run_err("print read_file(5)\n").contains("read_file() expects a Str"));
        assert!(run_err("print write_file(\"a\", 5)\n").contains("write_file() expects a Str"));
        assert!(run_err("print file_exists(5)\n").contains("file_exists() expects a Str"));
        assert!(run_err("print append_file(5, \"x\")\n").contains("append_file() expects a Str"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 7: collections ----

    #[test]
    fn tuple_literals_and_grouping() {
        assert_eq!(run_ok("print (1, \"a\", true)\n"), "(1, \"a\", true)\n");
        assert_eq!(run_ok("print ()\n"), "()\n");
        assert_eq!(run_ok("print (1,)\n"), "(1,)\n");
        // `(1)` is grouping, not a tuple.
        assert_eq!(run_ok("print (1)\n"), "1\n");
        assert_eq!(run_ok("print (1 + 2)\n"), "3\n");
        // Nested and mixed tuples.
        assert_eq!(run_ok("print ((1, 2), 3)\n"), "((1, 2), 3)\n");
        assert_eq!(run_ok("print type((1, 2))\n"), "tuple\n");
        assert_eq!(run_ok("print type(())\n"), "tuple\n");
        assert_eq!(run_ok("print len((1, 2, 3))\n"), "3\n");
        assert_eq!(run_ok("print str((1, \"x\"))\n"), "(1, \"x\")\n");
        // Tuples compare by content.
        assert_eq!(run_ok("print (1, 2) == (1, 2)\n"), "true\n");
        assert_eq!(run_ok("print (1, 2) == (1, 3)\n"), "false\n");
        assert_eq!(run_ok("print (1,) == (1,)\n"), "true\n");
        assert_eq!(run_ok("print () == ()\n"), "true\n");
        assert_eq!(run_ok("print (1, 2) == [1, 2]\n"), "false\n");
    }

    #[test]
    fn tuple_indexing_and_immutability() {
        let src = "let t = (10, 20, 30)\nprint t[0]\nprint t[2]\nprint t[-1]\nprint t[-3]\n";
        assert_eq!(run_ok(src), "10\n30\n30\n10\n");
        assert!(run_err("let t = (1,)\nprint t[1]\n").contains("tuple index out of range"));
        assert!(run_err("let t = (1,)\nprint t[-2]\n").contains("tuple index out of range"));
        assert!(
            run_err("let t = (1,)\nprint t[\"a\"]\n").contains("tuple index must be an integer")
        );
        // Tuples are immutable.
        assert!(run_err("let t = (1, 2)\nt[0] = 99\n").contains("cannot assign to tuple"));
    }

    #[test]
    fn dict_literals_access_and_insertion() {
        let src =
            "let d = {\"nome\": \"Giuliano\", \"eta\": 14}\nprint d[\"nome\"]\nprint d[\"eta\"]\n";
        assert_eq!(run_ok(src), "Giuliano\n14\n");
        assert_eq!(run_ok("print {}\n"), "{}\n");
        assert_eq!(run_ok("print {\"a\": 1}\n"), "{\"a\": 1}\n");
        assert_eq!(run_ok("print type({\"a\": 1})\n"), "dict\n");
        assert_eq!(run_ok("print type({})\n"), "dict\n");
        // Insertion of a missing key, and update of an existing one.
        let src = "let d = {}\nd[\"x\"] = 1\nd[\"y\"] = 2\nd[\"x\"] = 9\nprint d\nprint len(d)\n";
        assert_eq!(run_ok(src), "{\"x\": 9, \"y\": 2}\n2\n");
        // Integer, float and bool keys work too.
        let src = "let d = {1: \"a\", 2.5: \"b\", true: \"c\"}\nprint d[1]\nprint d[2.5]\nprint d[true]\n";
        assert_eq!(run_ok(src), "a\nb\nc\n");
        // A missing key is an error.
        assert!(run_err("let d = {}\nprint d[\"x\"]\n").contains("key not found"));
        // Updating keeps the original position.
        let src = "let d = {\"a\": 1, \"b\": 2}\nd[\"a\"] = 9\nprint keys(d)\n";
        assert_eq!(run_ok(src), "[\"a\", \"b\"]\n");
        // Nested dicts.
        let src =
            "let d = {\"a\": {\"b\": 5}}\nprint d[\"a\"][\"b\"]\nd[\"a\"][\"b\"] = 6\nprint d\n";
        assert_eq!(run_ok(src), "5\n{\"a\": {\"b\": 6}}\n");
    }

    #[test]
    fn dict_key_errors() {
        // Lists, dicts, sets and tuples cannot be keys.
        assert!(run_err("print {[1]: 2}\n").contains("dict key must be Int, Float, Str or Bool"));
        assert!(run_err("print {{}: 2}\n").contains("dict key must be Int, Float, Str or Bool"));
        assert!(run_err("print {{1}: 2}\n").contains("dict key must be Int, Float, Str or Bool"));
        assert!(run_err("print {(1,): 2}\n").contains("dict key must be Int, Float, Str or Bool"));
        assert!(
            run_err("let d = {}\nd[[1]] = 2\n")
                .contains("dict key must be Int, Float, Str or Bool")
        );
        // `1` and `1.0` are different keys.
        let src =
            "let d = {1: \"int\"}\nd[1.0] = \"float\"\nprint len(d)\nprint d[1]\nprint d[1.0]\n";
        assert_eq!(run_ok(src), "2\nint\nfloat\n");
    }

    #[test]
    fn dict_equality() {
        assert_eq!(run_ok("print {\"a\": 1} == {\"a\": 1}\n"), "true\n");
        // Order does not matter for dict equality.
        assert_eq!(
            run_ok("print {\"a\": 1, \"b\": 2} == {\"b\": 2, \"a\": 1}\n"),
            "true\n"
        );
        assert_eq!(run_ok("print {\"a\": 1} == {\"a\": 2}\n"), "false\n");
        assert_eq!(run_ok("print {\"a\": 1} == {\"b\": 1}\n"), "false\n");
        assert_eq!(run_ok("print {} == {}\n"), "true\n");
        assert_eq!(run_ok("print {\"a\": 1} == 1\n"), "false\n");
        // Nested dicts compare by content.
        assert_eq!(
            run_ok("print {\"a\": {\"b\": [1]}} == {\"a\": {\"b\": [1]}}\n"),
            "true\n"
        );
    }

    #[test]
    fn set_literals_and_duplicates() {
        assert_eq!(run_ok("print {1, 2, 3}\n"), "{1, 2, 3}\n");
        // Duplicates are ignored silently.
        assert_eq!(run_ok("print {1, 1, 2}\n"), "{1, 2}\n");
        assert_eq!(run_ok("print len({1, 1, 2})\n"), "2\n");
        assert_eq!(run_ok("print type({1})\n"), "set\n");
        // `{}` is the empty dict, not an empty set.
        assert_eq!(run_ok("print type({})\n"), "dict\n");
        assert_eq!(run_ok("print {\"a\", \"b\"} == {\"b\", \"a\"}\n"), "true\n");
        assert_eq!(run_ok("print {1, 2} == {1, 2, 3}\n"), "false\n");
        assert_eq!(run_ok("print {1, 2} == [1, 2]\n"), "false\n");
        // Sets print in insertion order.
        assert_eq!(run_ok("print {3, 1, 2}\n"), "{3, 1, 2}\n");
        // Unhashable elements are an error.
        assert!(
            run_err("print {[1], 2}\n").contains("set element must be Int, Float, Str or Bool")
        );
        assert!(run_err("print {{}, 1}\n").contains("set element must be Int, Float, Str or Bool"));
        // Sets have no indexes and cannot be assigned to.
        assert!(run_err("let s = {1, 2}\nprint s[0]\n").contains("cannot index Set"));
        assert!(run_err("let s = {1, 2}\ns[0] = 9\n").contains("cannot assign to set"));
    }

    #[test]
    fn array_construction_and_type_rules() {
        assert_eq!(run_ok("print array([1, 2, 3])\n"), "array([1, 2, 3])\n");
        assert_eq!(run_ok("print array([\"a\"])\n"), "array([\"a\"])\n");
        assert_eq!(run_ok("print type(array([1]))\n"), "array\n");
        assert_eq!(run_ok("print len(array([1, 2]))\n"), "2\n");
        assert_eq!(
            run_ok("print str(array([1.5, 2.5]))\n"),
            "array([1.5, 2.5])\n"
        );
        // Phase 12: `==` between arrays is element-wise now, so
        // it returns a Bool array instead of a single Bool.
        assert_eq!(
            run_ok("print array([1, 2]) == array([1, 2])\n"),
            "array([true, true])\n"
        );
        assert_eq!(
            run_ok("print array([1]) == array([1.0])\n"),
            "array([true])\n"
        );
        assert_eq!(run_ok("print array([1]) == [1]\n"), "false\n");
        // Empty list: no type to infer.
        assert!(run_err("print array([])\n").contains("cannot infer array type from empty list"));
        // Mixed types are an error.
        assert!(
            run_err("print array([1, \"a\"])\n")
                .contains("array() elements must all have the same type")
        );
        assert!(
            run_err("print array([1, 2.0])\n")
                .contains("array() elements must all have the same type")
        );
        // Lists as elements are not allowed.
        assert!(
            run_err("print array([[1]])\n")
                .contains("array() elements must be Int, Float, Str or Bool")
        );
        // A non-list argument is a type error.
        assert!(run_err("print array(5)\n").contains("array() expects a List"));
    }

    #[test]
    fn array_indexing_and_assignment() {
        let src = "let a = array([10, 20, 30])\nprint a[0]\nprint a[-1]\na[1] = 99\nprint a\n";
        assert_eq!(run_ok(src), "10\n30\narray([10, 99, 30])\n");
        assert!(run_err("let a = array([1])\nprint a[1]\n").contains("array index out of range"));
        assert!(
            run_err("let a = array([1])\nprint a[\"x\"]\n")
                .contains("array index must be an integer")
        );
        // The new value must have the same type.
        assert!(
            run_err("let a = array([1, 2])\na[0] = \"x\"\n")
                .contains("array element must be Int, not Str")
        );
        assert!(
            run_err("let a = array([1.5])\na[0] = 1\n")
                .contains("array element must be Float, not Int")
        );
        // Fixed size: no push or pop.
        assert!(
            run_err("let a = array([1])\npush(a, 2)\n")
                .contains("push() expects a List or a Stack")
        );
        assert!(
            run_err("let a = array([1])\npop(a)\n").contains("pop() expects a List or a Stack")
        );
    }

    // ---- Phase 11: N-dimensional numeric arrays ----

    #[test]
    fn nd_arrays_shape_ndim_size() {
        // 2D display is nested, NumPy style.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m\n";
        assert_eq!(run_ok(src), "array([[1, 2], [3, 4]])\n");
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint shape(m)\nprint ndim(m)\nprint size(m)\nprint len(m)\nprint type(m)\n";
        assert_eq!(run_ok(src), "[2, 2]\n2\n4\n4\narray\n");
        // 1D arrays keep their old shape and display.
        let src =
            "let a = array([1, 2, 3])\nprint shape(a)\nprint ndim(a)\nprint size(a)\nprint a\n";
        assert_eq!(run_ok(src), "[3]\n1\n3\narray([1, 2, 3])\n");
        // zeros and ones take an Int (1D) or a List shape.
        assert_eq!(run_ok("print zeros(3)\n"), "array([0, 0, 0])\n");
        assert_eq!(
            run_ok("print zeros([2, 3])\n"),
            "array([[0, 0, 0], [0, 0, 0]])\n"
        );
        assert_eq!(run_ok("print ones([2, 2])\n"), "array([[1, 1], [1, 1]])\n");
        assert_eq!(run_ok("print shape(zeros([2, 3, 4]))\n"), "[2, 3, 4]\n");
        assert_eq!(run_ok("print ndim(ones([2, 3, 4]))\n"), "3\n");
        assert_eq!(run_ok("print size(ones([2, 3, 4]))\n"), "24\n");
        // Bad shapes are errors.
        assert!(run_err("print zeros(0)\n").contains("zeros() shape must be positive, not 0"));
        assert!(
            run_err("print zeros([2, -1])\n")
                .contains("zeros() shape dimensions must be positive, not -1")
        );
        assert!(
            run_err("print zeros([2, 1.5])\n")
                .contains("zeros() shape dimensions must be integers, not Float")
        );
        assert!(run_err("print zeros([])\n").contains("zeros() shape must not be empty"));
        assert!(
            run_err("print zeros(\"x\")\n")
                .contains("zeros() expects an Int or a List of Ints for the shape, not Str")
        );
        assert!(run_err("print shape([1])\n").contains("shape() expects an Array, not List"));
    }

    #[test]
    fn two_dimensional_indexing() {
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m[0, 0]\nprint m[0, 1]\nprint m[1, 0]\nprint m[1, 1]\nprint m[-1, -1]\nprint m[-2, -2]\n";
        assert_eq!(run_ok(src), "1\n2\n3\n4\n4\n1\n");
        // One index on a 2D array is its row; chained indexing
        // reaches the element.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m[0]\nprint m[1][0]\nprint m[0][-1]\n";
        assert_eq!(run_ok(src), "array([1, 2])\n3\n2\n");
        // Multi-index assignment.
        let src = "let m = zeros([2, 2])\nm[0, 1] = 7\nm[1, 0] = 8\nprint m\n";
        assert_eq!(run_ok(src), "array([[0, 7], [8, 0]])\n");
        // Errors: out of range on either axis, wrong index count.
        let src = "let m = zeros([2, 2])\nprint m[2, 0]\n";
        assert!(run_err(src).contains("array index out of range"));
        let src = "let m = zeros([2, 2])\nprint m[0, 2]\n";
        assert!(run_err(src).contains("array index out of range"));
        let src = "let m = zeros([2, 2])\nprint m[0, 1, 0]\n";
        assert!(run_err(src).contains("array has 2 dimensions but got 3 indices"));
        let src = "let m = zeros([2, 2])\nprint m[\"x\", 0]\n";
        assert!(run_err(src).contains("array index must be an integer"));
        let src = "let m = zeros([2, 2])\nm[0] = 5\n";
        assert!(run_err(src).contains("array has 2 dimensions but got 1 indices"));
        // A list still takes exactly one index.
        assert!(run_err("print [1, 2][0, 1]\n").contains("list takes one index, not 2"));
    }

    #[test]
    fn array_constructors_arange_and_linspace() {
        assert_eq!(run_ok("print arange(4)\n"), "array([0, 1, 2, 3])\n");
        assert_eq!(run_ok("print arange(2, 5)\n"), "array([2, 3, 4])\n");
        assert_eq!(run_ok("print arange(0, 10, 3)\n"), "array([0, 3, 6, 9])\n");
        assert_eq!(run_ok("print arange(5, 0, -2)\n"), "array([5, 3, 1])\n");
        assert_eq!(run_ok("print arange(3, 3)\n"), "array([])\n");
        assert_eq!(run_ok("print linspace(0, 1, 3)\n"), "array([0, 0.5, 1])\n");
        assert_eq!(
            run_ok("print linspace(0, 10, 5)\n"),
            "array([0, 2.5, 5, 7.5, 10])\n"
        );
        assert_eq!(run_ok("print linspace(5, 5, 1)\n"), "array([5])\n");
        assert_eq!(run_ok("print shape(arange(5))\n"), "[5]\n");
        assert!(run_err("print arange(1, 5, 0)\n").contains("arange() step must not be zero"));
        assert!(run_err("print linspace(0, 1, 0)\n").contains("linspace() needs at least 1 point"));
        assert!(run_err("print linspace(0, 1, \"x\")\n").contains("linspace() expects an Int"));
    }

    #[test]
    fn array_elementwise_arithmetic() {
        // Array with array.
        let src = "let a = array([1, 2, 3])\nlet b = array([10, 20, 30])\nprint a + b\nprint b - a\nprint a * b\nprint b / a\n";
        assert_eq!(
            run_ok(src),
            "array([11, 22, 33])\narray([9, 18, 27])\narray([10, 40, 90])\narray([10, 10, 10])\n"
        );
        // Array with scalar, from both sides.
        let src = "let a = array([1, 2, 3])\nprint a + 10\nprint 10 + a\nprint a * 2\nprint 2 * a\nprint a - 1\nprint 10 - a\nprint a / 2\n";
        assert_eq!(
            run_ok(src),
            "array([11, 12, 13])\narray([11, 12, 13])\narray([2, 4, 6])\narray([2, 4, 6])\narray([0, 1, 2])\narray([9, 8, 7])\narray([0.5, 1, 1.5])\n"
        );
        // Int/float promotion, division always float.
        let src = "let a = array([1, 2])\nprint a + 0.5\nprint a * 1.5\nprint type(a + 1)\nprint type(a + 0.5)\n";
        assert_eq!(
            run_ok(src),
            "array([1.5, 2.5])\narray([1.5, 3])\narray\narray\n"
        );
        // 2D arithmetic works the same way.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m + m\nprint m * 10\n";
        assert_eq!(
            run_ok(src),
            "array([[2, 4], [6, 8]])\narray([[10, 20], [30, 40]])\n"
        );
        // Unary minus.
        assert_eq!(run_ok("print -array([1, -2])\n"), "array([-1, 2])\n");
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint -m\n";
        assert_eq!(run_ok(src), "array([[-1, -2], [-3, -4]])\n");
        // Errors: shape mismatch, non-numeric arrays, bad scalar.
        let src = "let a = array([1, 2])\nlet b = array([1, 2, 3])\nprint a + b\n";
        assert!(run_err(src).contains("array shapes [2] and [3] do not match"));
        let src = "let a = array([\"x\"])\nprint a + a\n";
        assert!(run_err(src).contains("array operations require numbers, not array of Str"));
        let src = "let a = array([1])\nprint a + \"x\"\n";
        assert!(run_err(src).contains("cannot add Array and Str"));
        let src = "let a = array([1])\nprint a / 0\n";
        assert!(run_err(src).contains("division by zero"));
    }

    #[test]
    fn dot_and_matmul() {
        assert_eq!(
            run_ok("print dot(array([1, 2, 3]), array([4, 5, 6]))\n"),
            "32\n"
        );
        assert_eq!(
            run_ok("print dot(array([1.5, 2.5]), array([2, 3]))\n"),
            "10.5\n"
        );
        // The worked example: [[1,2],[3,4]] @ [[5,6],[7,8]].
        let src = "let a = reshape(array([1, 2, 3, 4]), [2, 2])\nlet b = reshape(array([5, 6, 7, 8]), [2, 2])\nprint matmul(a, b)\n";
        assert_eq!(run_ok(src), "array([[19, 22], [43, 50]])\n");
        // Vector times matrix and matrix times vector.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint matmul(array([1, 0]), m)\nprint matmul(m, array([1, 0]))\n";
        assert_eq!(run_ok(src), "array([1, 2])\narray([1, 3])\n");
        // Rotation by 90 degrees: (1, 0) becomes (0, 1).
        let src = "let r = reshape(array([0, -1, 1, 0]), [2, 2])\nprint matmul(r, array([1, 0]))\n";
        assert_eq!(run_ok(src), "array([0, 1])\n");
        // Errors.
        assert!(
            run_err("print dot(array([1, 2]), array([1, 2, 3]))\n")
                .contains("dot: vectors of length 2 and 3 do not match")
        );
        assert!(
            run_err("print dot(zeros([2, 2]), zeros([2, 2]))\n")
                .contains("dot() expects two 1D arrays (vectors)")
        );
        assert!(
            run_err("print matmul(array([1, 2]), array([3, 4]))\n")
                .contains("matmul() of two vectors: use dot() instead")
        );
        let src = "let a = reshape(array([1, 2, 3, 4, 5, 6]), [2, 3])\nlet b = reshape(array([1, 2, 3, 4]), [2, 2])\nprint matmul(a, b)\n";
        assert!(run_err(src).contains("matmul: shapes [2, 3] and [2, 2] are incompatible"));
        assert!(
            run_err("print matmul(array([\"a\"]), array([\"b\"]))\n")
                .contains("array operations require numbers, not array of Str")
        );
    }

    #[test]
    fn transpose_and_reshape() {
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint transpose(m)\n";
        assert_eq!(run_ok(src), "array([[1, 3], [2, 4]])\n");
        // A double transpose is the identity. Phase 12: `==`
        // between arrays is element-wise, so the comparison is a
        // Bool array, not a single Bool.
        let src = "let m = reshape(array([1, 2, 3, 4, 5, 6]), [2, 3])\nprint transpose(transpose(m))\nprint m == transpose(transpose(m))\n";
        assert_eq!(
            run_ok(src),
            "array([[1, 2, 3], [4, 5, 6]])\narray([[true, true, true], [true, true, true]])\n"
        );
        // A 1D array transposes to itself.
        assert_eq!(
            run_ok("print transpose(array([1, 2, 3]))\n"),
            "array([1, 2, 3])\n"
        );
        assert_eq!(
            run_ok("print reshape(array([1, 2, 3, 4]), [4, 1])\n"),
            "array([[1], [2], [3], [4]])\n"
        );
        assert_eq!(
            run_ok("print reshape(array([1, 2, 3, 4]), 4)\n"),
            "array([1, 2, 3, 4])\n"
        );
        // Reshape keeps the element type and the items.
        assert_eq!(
            run_ok("print reshape(array([1.5, 2.5]), [1, 2])\n"),
            "array([[1.5, 2.5]])\n"
        );
        assert!(
            run_err("print reshape(array([1, 2, 3]), [2, 2])\n")
                .contains("reshape: cannot reshape array of size 3 into shape [2, 2]")
        );
        assert!(
            run_err("print reshape(array([1]), [0, 1])\n")
                .contains("reshape() shape dimensions must be positive, not 0")
        );
        assert!(
            run_err("print transpose(zeros([2, 2, 2]))\n")
                .contains("transpose() supports 1D and 2D arrays only")
        );
    }

    #[test]
    fn sum_min_max_on_arrays() {
        let src = "let m = reshape(array([3, 1, 4, 2]), [2, 2])\nprint sum(m)\nprint min(m)\nprint max(m)\n";
        assert_eq!(run_ok(src), "10\n1\n4\n");
        assert_eq!(run_ok("print sum(array([1.5, 2.5]))\n"), "4\n");
        assert_eq!(run_ok("print min(array([1.5, 2.5]))\n"), "1.5\n");
        assert_eq!(run_ok("print max(array([1.5, 2.5]))\n"), "2.5\n");
        // The two-argument forms still work.
        assert_eq!(run_ok("print min(3, 7)\n"), "3\n");
        assert_eq!(run_ok("print max(3, 7)\n"), "7\n");
        assert_eq!(run_ok("print min(2.5, 2)\n"), "2\n");
        assert!(run_err("print min(5)\n").contains("min() expects two numbers or one Array"));
        assert!(run_err("print sum(array([\"a\"]))\n").contains("sum() expects numbers, not Str"));
    }

    // ---- Phase 12: slicing and boolean indexing ----

    #[test]
    fn list_slicing() {
        let src = "let xs = [1, 2, 3, 4, 5]\nprint xs[1:3]\nprint xs[:2]\nprint xs[2:]\nprint xs[::2]\nprint xs[::-1]\nprint xs[-3:-1]\nprint xs[10:20]\nprint xs[-100:100]\nprint xs[:]\n";
        assert_eq!(
            run_ok(src),
            "[2, 3]\n[1, 2]\n[3, 4, 5]\n[1, 3, 5]\n[5, 4, 3, 2, 1]\n[3, 4]\n[]\n[1, 2, 3, 4, 5]\n[1, 2, 3, 4, 5]\n"
        );
        // A slice is a copy: mutating it leaves the original alone.
        let src = "let xs = [1, 2, 3]\nlet ys = xs[0:2]\nys[0] = 99\nprint xs\nprint ys\n";
        assert_eq!(run_ok(src), "[1, 2, 3]\n[99, 2]\n");
        // A step of 0 is an error; a non-integer bound is an error.
        assert!(run_err("print [1, 2, 3][::0]\n").contains("slice step must not be zero"));
        assert!(
            run_err("print [1, 2, 3][\"a\":2]\n")
                .contains("slice bounds must be integers, not Str")
        );
    }

    #[test]
    fn string_slicing_is_by_character() {
        assert_eq!(run_ok("print \"ciao\"[1:3]\n"), "ia\n");
        assert_eq!(run_ok("print \"ciao\"[::-1]\n"), "oaic\n");
        assert_eq!(run_ok("print \"ciao\"[:2]\n"), "ci\n");
        // "è" is one character, not two bytes.
        assert_eq!(run_ok("print \"aèb\"[1:2]\n"), "è\n");
        assert_eq!(run_ok("print len(\"aèb\"[1:])\n"), "2\n");
    }

    #[test]
    fn tuple_slicing() {
        assert_eq!(run_ok("print (1, 2, 3, 4)[1:3]\n"), "(2, 3)\n");
        assert_eq!(run_ok("print (1, 2, 3)[::-1]\n"), "(3, 2, 1)\n");
        assert_eq!(run_ok("print (1, 2, 3)[:]\n"), "(1, 2, 3)\n");
        assert_eq!(run_ok("print (1,)[0:1]\n"), "(1,)\n");
    }

    #[test]
    fn array_slicing_1d() {
        let src = "let a = array([10, 20, 30, 40, 50])\nprint a[1:4]\nprint a[:2]\nprint a[::2]\nprint a[::-1]\nprint a[-2:]\n";
        assert_eq!(
            run_ok(src),
            "array([20, 30, 40])\narray([10, 20])\narray([10, 30, 50])\narray([50, 40, 30, 20, 10])\narray([40, 50])\n"
        );
        // A slice of an array is a copy with the same element type.
        let src = "let a = array([1.5, 2.5, 3.5])\nprint a[0:2]\nprint type(a[0:2])\n";
        assert_eq!(run_ok(src), "array([1.5, 2.5])\narray\n");
        assert!(run_err("print array([1, 2, 3])[1:2:0]\n").contains("slice step must not be zero"));
    }

    #[test]
    fn array_slicing_2d() {
        let src = "let m = reshape(array([1, 2, 3, 4, 5, 6]), [2, 3])\nprint m[0:2, 1:3]\nprint m[:, 1]\nprint m[0, :]\nprint m[:, :]\nprint m[1, 0:2]\nprint m[::2, ::2]\n";
        assert_eq!(
            run_ok(src),
            "array([[2, 3], [5, 6]])\narray([2, 5])\narray([1, 2, 3])\narray([[1, 2, 3], [4, 5, 6]])\narray([4, 5])\narray([[1, 3]])\n"
        );
        // A slice on the first axis only leaves the second whole.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m[1:2]\n";
        assert_eq!(run_ok(src), "array([[3, 4]])\n");
        // Reversing both axes.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m[::-1, ::-1]\n";
        assert_eq!(run_ok(src), "array([[4, 3], [2, 1]])\n");
    }

    #[test]
    fn slice_assignment() {
        // Lists: the new list must have the same length as the slice.
        let src = "let xs = [1, 2, 3, 4]\nxs[1:3] = [9, 9]\nprint xs\n";
        assert_eq!(run_ok(src), "[1, 9, 9, 4]\n");
        let src = "let xs = [1, 2, 3, 4]\nxs[::-1] = [4, 3, 2, 1]\nprint xs\n";
        assert_eq!(run_ok(src), "[1, 2, 3, 4]\n");
        assert!(
            run_err("let xs = [1, 2, 3]\nxs[0:2] = [9]\n")
                .contains("slice assignment: the slice has 2 element(s) but the new list has 1")
        );
        assert!(
            run_err("let xs = [1, 2, 3]\nxs[0:2] = 9\n")
                .contains("slice assignment needs a List, not Int")
        );
        // 1D arrays: same length, same element type.
        let src = "let a = array([1, 2, 3, 4])\na[1:3] = array([9, 9])\nprint a\n";
        assert_eq!(run_ok(src), "array([1, 9, 9, 4])\n");
        assert!(
            run_err("let a = array([1, 2, 3])\na[0:2] = array([9])\n")
                .contains("slice assignment: the slice has 2 element(s) but the new array has 1")
        );
        assert!(
            run_err("let a = array([1, 2, 3])\na[0:2] = array([9.5, 9.5])\n")
                .contains("array element must be Int, not Array of Float")
        );
        assert!(
            run_err("let a = array([1, 2, 3])\na[0:2] = [9, 9]\n")
                .contains("slice assignment on an array needs an Array, not List")
        );
        // Slice assignment on a 2D array is not done in this phase.
        assert!(
            run_err("let m = zeros([2, 2])\nm[0:1, 0:1] = array([9])\n")
                .contains("slice assignment works on 1D arrays only")
        );
        // Tuples stay immutable, even for slices.
        assert!(
            run_err("let t = (1, 2, 3)\nt[0:2] = (9, 9)\n")
                .contains("cannot assign to tuple (tuples are immutable)")
        );
    }

    #[test]
    fn array_comparisons_are_elementwise() {
        // Array against a scalar, all six operators.
        let src = "let a = array([1, 2, 3])\nprint a == 2\nprint a != 2\nprint a < 2\nprint a <= 2\nprint a > 2\nprint a >= 2\n";
        assert_eq!(
            run_ok(src),
            "array([false, true, false])\narray([true, false, true])\narray([true, false, false])\narray([true, true, false])\narray([false, false, true])\narray([false, true, true])\n"
        );
        // The scalar may come first.
        assert_eq!(
            run_ok("print 2 < array([1, 2, 3])\n"),
            "array([false, false, true])\n"
        );
        assert_eq!(
            run_ok("print 2 == array([1, 2, 3])\n"),
            "array([false, true, false])\n"
        );
        // Array against array.
        let src = "let a = array([1, 2])\nlet b = array([2, 2])\nprint a == b\nprint a < b\nprint a >= b\n";
        assert_eq!(
            run_ok(src),
            "array([false, true])\narray([true, false])\narray([false, true])\n"
        );
        // 2D comparisons keep the shape.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m > 2\n";
        assert_eq!(run_ok(src), "array([[false, false], [true, true]])\n");
        // Mixed Int/Float compares across types.
        assert_eq!(
            run_ok("print array([1, 2]) < 1.5\n"),
            "array([true, false])\n"
        );
        // Strings compare element-wise too (equality only).
        assert_eq!(
            run_ok("print array([\"a\", \"b\"]) == \"a\"\n"),
            "array([true, false])\n"
        );
        // Errors: shape mismatch, ordering on non-numbers.
        let src = "let a = array([1, 2])\nlet b = array([1, 2, 3])\nprint a == b\n";
        assert!(run_err(src).contains("cannot compare arrays of shapes [2] and [3]"));
        assert!(
            run_err("print array([\"a\"]) < array([\"b\"])\n")
                .contains("cannot compare Str and Str")
        );
        assert!(run_err("print array([\"a\"]) < \"b\"\n").contains("cannot compare Str and Str"));
    }

    #[test]
    fn bool_arrays_and_sum() {
        assert_eq!(
            run_ok("print array([true, false, true])\n"),
            "array([true, false, true])\n"
        );
        assert_eq!(run_ok("print type(array([true]))\n"), "array\n");
        // sum() on a Bool array counts the trues.
        assert_eq!(run_ok("print sum(array([true, false, true]))\n"), "2\n");
        assert_eq!(run_ok("print sum(array([false, false]))\n"), "0\n");
        // A comparison produces a Bool array; sum() counts its trues.
        let src = "let a = array([1, 2, 3, 4])\nprint sum(a > 2)\n";
        assert_eq!(run_ok(src), "2\n");
        // Bool arrays support == and !=, but not ordering.
        assert_eq!(
            run_ok("print array([true, false]) == true\n"),
            "array([true, false])\n"
        );
        assert!(
            run_err("print array([true]) < array([false])\n")
                .contains("cannot compare Bool and Bool")
        );
    }

    #[test]
    fn boolean_indexing() {
        // A 1D mask flattens the selection to 1D.
        let src = "let a = array([1, 2, 3, 4])\nprint a[a > 2]\n";
        assert_eq!(run_ok(src), "array([3, 4])\n");
        let src = "let a = array([1, 2, 3, 4])\nprint a[a == 2]\n";
        assert_eq!(run_ok(src), "array([2])\n");
        let src = "let a = array([1, 2, 3])\nprint a[array([true, false, true])]\n";
        assert_eq!(run_ok(src), "array([1, 3])\n");
        // A 2D mask selects in row-major order, flattened to 1D.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m[m > 2]\n";
        assert_eq!(run_ok(src), "array([3, 4])\n");
        // No match at all gives an empty array.
        let src = "let a = array([1, 2])\nprint a[a > 10]\n";
        assert_eq!(run_ok(src), "array([])\n");
        // The mask keeps the element type.
        let src = "let a = array([1.5, 2.5])\nprint a[a > 2.0]\n";
        assert_eq!(run_ok(src), "array([2.5])\n");
        // Errors: wrong mask shape, a mask on a list, a non-Bool
        // array as an index.
        let src = "let a = array([1, 2, 3])\nprint a[array([true, false])]\n";
        assert!(run_err(src).contains("boolean mask shape [2] does not match array shape [3]"));
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint m[array([true, false])]\n";
        assert!(run_err(src).contains("boolean mask shape [2] does not match array shape [2, 2]"));
        assert!(
            run_err("let xs = [1, 2, 3]\nprint xs[array([true, false, true])]\n")
                .contains("list index must be an integer")
        );
        assert!(
            run_err("let a = array([1, 2])\nprint a[array([0, 1])]\n")
                .contains("array index must be an integer")
        );
        // Assigning through a mask is not supported yet.
        assert!(
            run_err("let a = array([1, 2])\na[a > 1] = 0\n")
                .contains("cannot assign through a boolean mask (not supported yet)")
        );
    }

    #[test]
    fn where_builtin() {
        // Arrays on both sides.
        let src = "let c = array([true, false, true])\nlet a = array([1, 2, 3])\nlet b = array([10, 20, 30])\nprint where(c, a, b)\n";
        assert_eq!(run_ok(src), "array([1, 20, 3])\n");
        // Scalars broadcast.
        let src = "let c = array([true, false])\nprint where(c, 1, 0)\n";
        assert_eq!(run_ok(src), "array([1, 0])\n");
        let src = "let a = array([1, 2, 3])\nprint where(a > 1, a, 0)\n";
        assert_eq!(run_ok(src), "array([0, 2, 3])\n");
        // A scalar and an array mix; Int+Float promotes to Float.
        let src = "let c = array([true, false])\nprint where(c, 1.5, array([1, 2]))\n";
        assert_eq!(run_ok(src), "array([1.5, 2])\n");
        // 2D keeps its shape.
        let src = "let m = reshape(array([1, 2, 3, 4]), [2, 2])\nprint where(m > 2, m, 0)\n";
        assert_eq!(run_ok(src), "array([[0, 0], [3, 4]])\n");
        // Bool and Str results work too.
        let src = "let c = array([true, false])\nprint where(c, array([true, true]), false)\n";
        assert_eq!(run_ok(src), "array([true, false])\n");
        // Errors: non-Bool condition, shape mismatch, bad types.
        assert!(
            run_err("print where(array([1]), 1, 0)\n")
                .contains("where() expects a Bool array as its condition, not Array of Int")
        );
        assert!(
            run_err("print where(array([true]), array([1, 2]), 0)\n")
                .contains("where() argument shape [2] does not match condition shape [1]")
        );
        assert!(
            run_err("print where(array([true]), 1, \"x\")\n")
                .contains("where() arguments have incompatible types: Int and Str")
        );
        assert!(run_err("print where(1, 2, 3)\n").contains("where() expects an Array, not Int"));
    }

    #[test]
    fn queue_is_fifo() {
        assert_eq!(run_ok("print queue()\n"), "queue([])\n");
        assert_eq!(run_ok("print queue([1, 2])\n"), "queue([1, 2])\n");
        assert_eq!(run_ok("print type(queue())\n"), "queue\n");
        assert_eq!(run_ok("print len(queue([1, 2, 3]))\n"), "3\n");
        let src = "let q = queue()\nenqueue(q, 1)\nenqueue(q, 2)\nenqueue(q, 3)\nprint dequeue(q)\nprint dequeue(q)\nprint q\n";
        assert_eq!(run_ok(src), "1\n2\nqueue([3])\n");
        // dequeue on an empty queue is an error.
        assert!(run_err("print dequeue(queue())\n").contains("dequeue() on empty queue"));
        assert!(run_err("print dequeue([1])\n").contains("dequeue() expects a Queue"));
        assert!(run_err("print enqueue([1], 2)\n").contains("enqueue() expects a Queue"));
        assert!(run_err("print queue(5)\n").contains("queue() expects a List"));
        assert!(
            run_err("print queue([1], [2])\n")
                .contains("queue() expects 0 or 1 argument(s) but got 2")
        );
        // peek sees the front without removing it.
        let src = "let q = queue([1, 2])\nprint peek(q)\nprint q\n";
        assert_eq!(run_ok(src), "1\nqueue([1, 2])\n");
        assert!(run_err("print peek(queue())\n").contains("peek() on empty queue"));
    }

    #[test]
    fn stack_is_lifo_and_push_pop_are_extended() {
        assert_eq!(run_ok("print stack()\n"), "stack([])\n");
        assert_eq!(run_ok("print stack([1, 2])\n"), "stack([1, 2])\n");
        assert_eq!(run_ok("print type(stack())\n"), "stack\n");
        assert_eq!(run_ok("print len(stack([1, 2]))\n"), "2\n");
        let src = "let s = stack()\npush(s, 1)\npush(s, 2)\nprint pop(s)\nprint pop(s)\nprint s\n";
        assert_eq!(run_ok(src), "2\n1\nstack([])\n");
        // pop on an empty stack is an error.
        assert!(run_err("print pop(stack())\n").contains("pop() on empty stack"));
        assert!(run_err("print stack(5)\n").contains("stack() expects a List"));
        // peek sees the top without removing it.
        let src = "let s = stack([1, 2])\nprint peek(s)\nprint s\n";
        assert_eq!(run_ok(src), "2\nstack([1, 2])\n");
        assert!(run_err("print peek(stack())\n").contains("peek() on empty stack"));
        // peek on a list gives the last element.
        assert_eq!(run_ok("print peek([1, 2, 3])\n"), "3\n");
        assert!(run_err("print peek([])\n").contains("peek() on empty list"));
        assert!(run_err("print peek(5)\n").contains("peek() expects a List, a Queue or a Stack"));
        // push returns nil, like on lists.
        assert_eq!(run_ok("let s = stack()\nprint push(s, 1)\n"), "nil\n");
    }

    #[test]
    fn keys_in_insertion_order() {
        let src = "let d = {\"b\": 2, \"a\": 1}\nprint keys(d)\n";
        assert_eq!(run_ok(src), "[\"b\", \"a\"]\n");
        assert_eq!(run_ok("print keys({})\n"), "[]\n");
        let src = "let d = {1: \"x\", \"s\": \"y\", true: \"z\"}\nprint keys(d)\n";
        assert_eq!(run_ok(src), "[1, \"s\", true]\n");
        assert!(run_err("print keys([1])\n").contains("keys() expects a Dict"));
    }

    #[test]
    fn contains_on_every_collection() {
        assert_eq!(run_ok("print contains([1, 2], 2)\n"), "true\n");
        assert_eq!(run_ok("print contains([1, 2], 3)\n"), "false\n");
        assert_eq!(run_ok("print contains((1, 2), 2)\n"), "true\n");
        assert_eq!(run_ok("print contains(array([1, 2]), 2)\n"), "true\n");
        assert_eq!(run_ok("print contains({1, 2}, 2)\n"), "true\n");
        assert_eq!(run_ok("print contains({1, 2}, 3)\n"), "false\n");
        // Dicts: the key exists.
        assert_eq!(run_ok("print contains({\"a\": 1}, \"a\")\n"), "true\n");
        assert_eq!(run_ok("print contains({\"a\": 1}, \"b\")\n"), "false\n");
        // Strings: substring search.
        assert_eq!(run_ok("print contains(\"ciao\", \"ia\")\n"), "true\n");
        assert_eq!(run_ok("print contains(\"ciao\", \"z\")\n"), "false\n");
        // Membership uses value equality (1 == 1.0).
        assert_eq!(run_ok("print contains([1], 1.0)\n"), "true\n");
        // An unhashable value can never be a set/dict member: false.
        assert_eq!(run_ok("print contains({1}, [1])\n"), "false\n");
        assert_eq!(run_ok("print contains({\"a\": 1}, [1])\n"), "false\n");
        assert!(
            run_err("print contains(\"ciao\", 1)\n")
                .contains("contains() expects a Str to search in a Str")
        );
        assert!(
            run_err("print contains(5, 1)\n")
                .contains("contains() expects a List, Tuple, Dict, Set, Array or Str")
        );
    }

    #[test]
    fn list_converts_collections() {
        assert_eq!(run_ok("print list((1, 2))\n"), "[1, 2]\n");
        assert_eq!(run_ok("print list({3, 1})\n"), "[3, 1]\n");
        assert_eq!(run_ok("print list(array([1, 2]))\n"), "[1, 2]\n");
        assert_eq!(run_ok("print list(queue([1, 2]))\n"), "[1, 2]\n");
        assert_eq!(run_ok("print list(stack([1, 2]))\n"), "[1, 2]\n");
        assert_eq!(
            run_ok("print list({\"b\": 1, \"a\": 2})\n"),
            "[\"b\", \"a\"]\n"
        );
        // A list converts to a fresh copy: mutating it does not
        // touch the original.
        let src = "let a = [1, 2]\nlet b = list(a)\npush(b, 3)\nprint a\nprint b\n";
        assert_eq!(run_ok(src), "[1, 2]\n[1, 2, 3]\n");
        // A string is an error (use split/lines for text).
        assert!(run_err("print list(\"ciao\")\n").contains("list() cannot convert Str to List"));
        assert!(run_err("print list(5)\n").contains("list() cannot convert Int to List"));
    }

    #[test]
    fn len_str_and_type_on_all_collections() {
        assert_eq!(run_ok("print len((1, 2))\n"), "2\n");
        assert_eq!(run_ok("print len({\"a\": 1, \"b\": 2})\n"), "2\n");
        assert_eq!(run_ok("print len({1, 2, 3})\n"), "3\n");
        assert_eq!(run_ok("print len(array([1]))\n"), "1\n");
        assert_eq!(run_ok("print len(queue([1, 2]))\n"), "2\n");
        assert_eq!(run_ok("print len(stack([1]))\n"), "1\n");
        assert_eq!(run_ok("print type((1,))\n"), "tuple\n");
        assert_eq!(run_ok("print type({\"a\": 1})\n"), "dict\n");
        assert_eq!(run_ok("print type({1})\n"), "set\n");
        assert_eq!(run_ok("print type(array([1]))\n"), "array\n");
        assert_eq!(run_ok("print type(queue())\n"), "queue\n");
        assert_eq!(run_ok("print type(stack())\n"), "stack\n");
        assert_eq!(run_ok("print str({1, 2})\n"), "{1, 2}\n");
        assert_eq!(run_ok("print str({\"a\": 1})\n"), "{\"a\": 1}\n");
        assert_eq!(run_ok("print str(queue([1]))\n"), "queue([1])\n");
        assert_eq!(run_ok("print str(stack([1]))\n"), "stack([1])\n");
        assert_eq!(run_ok("print str(())\n"), "()\n");
    }

    #[test]
    fn nested_collections_equality_and_display_cap() {
        // Nested equality by content.
        assert_eq!(
            run_ok("print {\"a\": (1, [2])} == {\"a\": (1, [2])}\n"),
            "true\n"
        );
        assert_eq!(run_ok("print [{1, 2}] == [{2, 1}]\n"), "true\n");
        // A self-referential list inside a dict prints with `...`
        // instead of recursing forever.
        let src = "let a = [1]\npush(a, a)\nlet d = {\"x\": a}\nprint d\n";
        let out = run_ok(src);
        assert!(out.contains("..."), "{out}");
        assert!(out.starts_with("{\"x\": [1, [1,"), "{out}");
        // A dict containing itself, compared to itself, is equal.
        let src = "let d = {}\nd[\"self\"] = d\nprint d == d\n";
        assert_eq!(run_ok(src), "true\n");
    }

    #[test]
    fn word_count_with_a_dict() {
        // `for x in ...` does not exist yet: count by index instead.
        let src = "let text = \"pane pane latte pane\"\nlet words = split(text, \" \")\nlet counts = {}\nfor i = 0 to len(words) - 1 then\n    let w = words[i]\n    if contains(counts, w) then\n        counts[w] = counts[w] + 1\n    else\n        counts[w] = 1\n    end\nend\nprint counts\nprint counts[\"pane\"]\n";
        assert_eq!(run_ok(src), "{\"pane\": 3, \"latte\": 1}\n3\n");
    }

    // ---- Phase 8: OOP ----

    const PERSONA: &str = "class Persona\n    fn init(self, nome, eta)\n        self.nome = nome\n        self.eta = eta\n    end\n    fn saluta(self)\n        return \"Ciao, sono \" + self.nome\n    end\nend\n";

    #[test]
    fn class_init_fields_and_default_display() {
        let src = format!(
            "{PERSONA}let p = Persona(\"Giuliano\", 14)\nprint p.nome\nprint p.eta\nprint p\n"
        );
        assert_eq!(
            run_ok(&src),
            "Giuliano\n14\nPersona{nome: \"Giuliano\", eta: 14}\n"
        );
        // Fields are created by assignment inside a method: an
        // instance with no init has no fields.
        let src = "class Vuota\nend\nlet v = Vuota()\nprint v\n";
        assert_eq!(run_ok(src), "Vuota{}\n");
        // Fields are added in insertion order, and updating one
        // keeps its position.
        let src = "class C\n    fn init(self)\n        self.a = 1\n        self.b = 2\n        self.a = 9\n    end\nend\nprint C()\n";
        assert_eq!(run_ok(src), "C{a: 9, b: 2}\n");
        // A string field prints quoted inside the instance.
        let src =
            "class C\n    fn init(self)\n        self.nome = \"x\"\n    end\nend\nprint C()\n";
        assert_eq!(run_ok(src), "C{nome: \"x\"}\n");
    }

    #[test]
    fn methods_with_self_and_return_values() {
        let src = format!("{PERSONA}let p = Persona(\"Giuliano\", 14)\nprint p.saluta()\n");
        assert_eq!(run_ok(&src), "Ciao, sono Giuliano\n");
        // A method with arguments, and a method calling another
        // method on the same instance.
        let src = "class Calcolatrice\n    fn somma(self, a, b)\n        return a + b\n    end\n    fn doppio(self, x)\n        return self.somma(x, x)\n    end\nend\nlet c = Calcolatrice()\nprint c.somma(2, 3)\nprint c.doppio(4)\n";
        assert_eq!(run_ok(src), "5\n8\n");
        // A method without a return gives nil.
        let src = "class C\n    fn niente(self)\n    end\nend\nprint C().niente()\n";
        assert_eq!(run_ok(src), "nil\n");
        // Methods see globals, and the class itself.
        let src = "let prefix = \"sig. \"\nclass Persona\n    fn init(self, nome, eta)\n        self.nome = nome\n        self.eta = eta\n    end\n    fn presentazione(self)\n        return prefix + self.nome\n    end\n    fn amico(self, nome)\n        return Persona(nome, 0)\n    end\nend\nlet p = Persona(\"Giuliano\", 14)\nprint p.presentazione()\nprint p.amico(\"Fabiana\").nome\n";
        assert_eq!(run_ok(src), "sig. Giuliano\nFabiana\n");
    }

    #[test]
    fn bound_method_saved_in_a_variable() {
        let src = format!(
            "{PERSONA}let p = Persona(\"Giuliano\", 14)\nlet m = p.saluta\nprint m()\nprint type(m)\n"
        );
        assert_eq!(run_ok(&src), "Ciao, sono Giuliano\nfn\n");
        // The bound method keeps its instance even when the
        // variable is passed around.
        let src = format!(
            "{PERSONA}fn chiama(f)\n    return f()\nend\nlet p = Persona(\"Giuliano\", 14)\nprint chiama(p.saluta)\n"
        );
        assert_eq!(run_ok(&src), "Ciao, sono Giuliano\n");
        // Two accesses to the same method are equal (same instance
        // and same function); different instances are not.
        let src = format!(
            "{PERSONA}let p = Persona(\"A\", 1)\nlet q = Persona(\"B\", 2)\nprint p.saluta == p.saluta\nprint p.saluta == q.saluta\n"
        );
        assert_eq!(run_ok(&src), "true\nfalse\n");
    }

    #[test]
    fn missing_field_and_method_errors() {
        let src = format!("{PERSONA}let p = Persona(\"A\", 1)\nprint p.cognome\n");
        assert!(run_err(&src).contains("has no field or method 'cognome'"));
        let src = format!("{PERSONA}let p = Persona(\"A\", 1)\nprint p.cognome()\n");
        assert!(run_err(&src).contains("has no method 'cognome'"));
        let src = format!("{PERSONA}let p = Persona(\"A\", 1)\nprint p.nome()\n");
        assert!(run_err(&src).contains("cannot call Str"));
        // Assigning the missing field creates it.
        let src =
            format!("{PERSONA}let p = Persona(\"A\", 1)\np.cognome = \"Rossi\"\nprint p.cognome\n");
        assert_eq!(run_ok(&src), "Rossi\n");
        // A field with the same name as a method shadows it.
        let src = format!("{PERSONA}let p = Persona(\"A\", 1)\np.saluta = 5\nprint p.saluta\n");
        assert_eq!(run_ok(&src), "5\n");
        // Fields can only be set on instances.
        assert!(run_err("let x = 5\nx.campo = 1\n").contains("cannot set field 'campo' on Int"));
        assert!(run_err("let x = 5\nprint x.nome\n").contains("cannot access field 'nome' of Int"));
        // Setting a field on a call result works when it returns an
        // instance, and fails otherwise.
        let src = "fn get()\n    return 5\nend\nget().nome = 1\n";
        assert!(run_err(src).contains("cannot set field 'nome' on Int"));
    }

    #[test]
    fn override_and_three_level_chain() {
        let src = "class Animale\n    fn verso(self)\n        return \"...\"\n    end\n    fn zampe(self)\n        return 4\n    end\nend\nclass Cane extends Animale\n    fn verso(self)\n        return \"bau\"\n    end\nend\nclass Cucciolo extends Cane\nend\nlet c = Cucciolo()\nprint c.verso()\nprint c.zampe()\nprint type(c)\n";
        assert_eq!(run_ok(src), "bau\n4\nCucciolo\n");
        // An override in the middle of the chain hides the grandparent.
        let src = "class A\n    fn chi(self)\n        return \"A\"\n    end\nend\nclass B extends A\n    fn chi(self)\n        return \"B\"\n    end\nend\nclass C extends B\nend\nprint C().chi()\nprint B().chi()\nprint A().chi()\n";
        assert_eq!(run_ok(src), "B\nB\nA\n");
        // A subclass inherits the parent's init when it has none.
        let src = "class A\n    fn init(self, x)\n        self.x = x\n    end\nend\nclass B extends A\nend\nprint B(7).x\n";
        assert_eq!(run_ok(src), "7\n");
    }

    #[test]
    fn super_init_and_super_method() {
        let src = "class Persona\n    fn init(self, nome, eta)\n        self.nome = nome\n        self.eta = eta\n    end\n    fn saluta(self)\n        return \"Ciao, sono \" + self.nome\n    end\nend\nclass Studente extends Persona\n    fn init(self, nome, eta, scuola)\n        super.init(nome, eta)\n        self.scuola = scuola\n    end\n    fn saluta(self)\n        return super.saluta() + \" e studio a \" + self.scuola\n    end\nend\nlet s = Studente(\"Giuliano\", 14, \"Leonardo\")\nprint s.nome\nprint s.scuola\nprint s.saluta()\n";
        assert_eq!(
            run_ok(src),
            "Giuliano\nLeonardo\nCiao, sono Giuliano e studio a Leonardo\n"
        );
        // `super` in a grandchild reaches the parent, and the
        // parent's own `super` reaches the grandparent.
        let src = "class A\n    fn chi(self)\n        return \"A\"\n    end\nend\nclass B extends A\n    fn chi(self)\n        return \"B(\" + super.chi() + \")\"\n    end\nend\nclass C extends B\n    fn chi(self)\n        return \"C(\" + super.chi() + \")\"\n    end\nend\nprint C().chi()\n";
        assert_eq!(run_ok(src), "C(B(A))\n");
    }

    #[test]
    fn super_outside_a_method_is_an_error() {
        assert!(run_err("print super.foo()\n").contains("'super' outside a method"));
        // A plain function has no super, even when called from a
        // method — the method's own super does not leak into it.
        let src = "fn helper()\n    return super.foo()\nend\nclass C\n    fn m(self)\n        return helper()\n    end\nend\nprint C().m()\n";
        assert!(run_err(src).contains("'super' outside a method"));
        // A class without a parent has no super.
        let src =
            "class C\n    fn m(self)\n        return super.foo()\n    end\nend\nprint C().m()\n";
        assert!(run_err(src).contains("has no parent (no super)"));
        // A missing method in the parent is an error.
        let src = "class A\nend\nclass B extends A\n    fn m(self)\n        return super.niente()\n    end\nend\nprint B().m()\n";
        assert!(run_err(src).contains("parent class has no method 'niente'"));
    }

    #[test]
    fn super_in_a_nested_function_uses_the_enclosing_method() {
        // A function defined inside a method captures that method's
        // class, so `super` inside it works (like Python's
        // `__class__` cell) — even when the function is called
        // after the method returned.
        let src = "class A\n    fn chi(self)\n        return \"A\"\n    end\nend\nclass B extends A\n    fn crea(self)\n        fn saluto()\n            return super.chi()\n        end\n        return saluto\n    end\nend\nlet f = B().crea()\nprint f()\n";
        assert_eq!(run_ok(src), "A\n");
    }

    #[test]
    fn class_without_init_and_wrong_arguments() {
        let src = "class C\nend\nlet c = C()\nprint type(c)\n";
        assert_eq!(run_ok(src), "C\n");
        assert!(
            run_err("class C\nend\nlet c = C(1)\n").contains("takes no arguments (no init method)")
        );
        // Wrong argument counts: constructor and method.
        let src = format!("{PERSONA}let p = Persona(\"A\")\n");
        assert!(run_err(&src).contains("method 'init' expects 2 argument(s) but got 1"));
        let src = format!("{PERSONA}let p = Persona(\"A\", 1, 2)\n");
        assert!(run_err(&src).contains("method 'init' expects 2 argument(s) but got 3"));
        let src = format!("{PERSONA}let p = Persona(\"A\", 1)\nprint p.saluta(1)\n");
        assert!(run_err(&src).contains("method 'saluta' expects 0 argument(s) but got 1"));
        // Calling something that is not a class or a function.
        assert!(run_err("let x = 1\nprint x()\n").contains("cannot call Int"));
        // The parent must be a class.
        assert!(
            run_err("let x = 5\nclass C extends x\nend\n").contains("parent 'x' is not a class")
        );
        assert!(
            run_err("class C extends Inesistente\nend\n")
                .contains("undefined variable 'Inesistente'")
        );
    }

    #[test]
    fn type_str_and_is_a() {
        let src = format!(
            "{PERSONA}let p = Persona(\"A\", 1)\nprint type(p)\nprint type(Persona)\nprint str(p)\n"
        );
        assert_eq!(
            run_ok(&src),
            "Persona\nclass\nPersona{nome: \"A\", eta: 1}\n"
        );
        // A custom `str` method wins for both `print` and `str()`.
        let src = "class Persona\n    fn init(self, nome)\n        self.nome = nome\n    end\n    fn str(self)\n        return \"Persona: \" + self.nome\n    end\nend\nlet p = Persona(\"Giuliano\")\nprint p\nprint str(p)\nprint type(p)\n";
        assert_eq!(
            run_ok(src),
            "Persona: Giuliano\nPersona: Giuliano\nPersona\n"
        );
        // ...but inside a collection the default form is used (like
        // Python's repr inside a list).
        let src = "class C\n    fn init(self)\n        self.x = 1\n    end\n    fn str(self)\n        return \"custom\"\n    end\nend\nprint [C()]\nprint C()\n";
        assert_eq!(run_ok(src), "[C{x: 1}]\ncustom\n");
        // A `str` method must return a string.
        let src = "class C\n    fn str(self)\n        return 5\n    end\nend\nprint C()\n";
        assert!(run_err(src).contains("str() method must return a Str"));
        assert!(
            run_err("class C\n    fn str(self)\n        return 5\n    end\nend\nprint str(C())\n")
                .contains("str() method must return a Str")
        );
        // `is_a`: ancestors count, anything else is false.
        let src = "class A\nend\nclass B extends A\nend\nclass C extends B\nend\nlet c = C()\nprint is_a(c, C)\nprint is_a(c, B)\nprint is_a(c, A)\nprint is_a(c, 5)\nprint is_a(5, A)\nprint is_a(B(), A)\n";
        assert!(run_err(src).contains("is_a() expects a Class as its second argument"));
        let src = "class A\nend\nclass B extends A\nend\nclass C extends B\nend\nlet c = C()\nprint is_a(c, C)\nprint is_a(c, B)\nprint is_a(c, A)\nprint is_a(5, A)\nprint is_a(B(), A)\nprint is_a(A(), C)\n";
        assert_eq!(run_ok(src), "true\ntrue\ntrue\nfalse\ntrue\nfalse\n");
    }

    #[test]
    fn instances_compare_by_identity() {
        let src = format!(
            "{PERSONA}let p = Persona(\"A\", 1)\nlet q = p\nprint p == q\nprint p == Persona(\"A\", 1)\nprint Persona == Persona\n"
        );
        assert_eq!(run_ok(&src), "true\nfalse\ntrue\n");
        // An instance inside a list compares by identity too.
        let src = format!(
            "{PERSONA}let p = Persona(\"A\", 1)\nprint [p] == [p]\nprint [p] == [Persona(\"A\", 1)]\n"
        );
        assert_eq!(run_ok(&src), "true\nfalse\n");
        // `len` on an instance is an error.
        let src = format!("{PERSONA}print len(Persona(\"A\", 1))\n");
        assert!(run_err(&src).contains("len() expects a Str"));
    }

    #[test]
    fn classes_are_values_and_methods_can_be_unbound() {
        // A class can be stored in a variable and called through it.
        let src =
            format!("{PERSONA}let C = Persona\nlet p = C(\"A\", 1)\nprint p.nome\nprint type(C)\n");
        assert_eq!(run_ok(&src), "A\nclass\n");
        // A class can be defined inside a block (normal scope).
        let src = "if true then\n    class Dentro\n        fn init(self)\n            self.x = 3\n        end\n    end\nend\nprint Dentro()\n";
        assert!(run_err(src).contains("undefined variable 'Dentro'"));
        let src = "fn niente()\nend\nlet C = niente()\nif true then\n    class Dentro\n        fn init(self)\n            self.x = 3\n        end\n    end\n    C = Dentro\nend\nprint C().x\n";
        assert_eq!(run_ok(src), "3\n");
        // An unbound method from the class: pass `self` explicitly.
        let src = format!("{PERSONA}let p = Persona(\"A\", 1)\nprint Persona.saluta(p)\n");
        assert_eq!(run_ok(&src), "Ciao, sono A\n");
        // A missing method on the class is an error.
        let src = format!("{PERSONA}print Persona.niente\n");
        assert!(run_err(&src).contains("class 'Persona' has no method 'niente'"));
    }

    #[test]
    fn field_holding_a_function_is_called_plainly() {
        // A function stored in a field is called without injecting
        // `self`: it is just a value that happens to be callable.
        let src = "fn doppia(x)\n    return x * 2\nend\nclass C\nend\nlet c = C()\nc.doppia = doppia\nprint c.doppia(21)\n";
        assert_eq!(run_ok(src), "42\n");
        // A field holding a non-function cannot be called.
        let src = "class C\nend\nlet c = C()\nc.x = 5\nprint c.x()\n";
        assert!(run_err(src).contains("cannot call Int"));
        // A method name given to a plain function call still goes
        // through the field first: the field shadows the method.
        let src = "class C\n    fn f(self)\n        return \"metodo\"\n    end\nend\nlet c = C()\nc.f = 5\nprint c.f\n";
        assert_eq!(run_ok(src), "5\n");
    }

    #[test]
    fn method_arity_and_self_rules() {
        // `self` outside a method is an undefined variable.
        assert!(run_err("print self\n").contains("undefined variable 'self'"));
        // `self` cannot be a variable name...
        assert!(run_err("let self = 1\n").contains("expected variable name after 'let'"));
        // ...nor `class`, `extends` or `super`.
        assert!(run_err("let class = 1\n").contains("expected variable name after 'let'"));
        assert!(run_err("let super = 1\n").contains("expected variable name after 'let'"));
        // A method whose first parameter is not `self` is a parse
        // error (checked in the parser tests too).
        assert!(
            run_err("class C\n    fn m(x)\n    end\nend\n").contains("methods must take 'self'")
        );
    }

    // ---- Phase 9: modules ----

    /// Write a module file `dir/name.gg` with the given source.
    fn write_module(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
        let path = dir.join(format!("{name}.gg"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, source).unwrap();
        path
    }

    /// Run `source` with the module search configured: `base_dir`
    /// is the importing file's directory, `venv_dirs` the venv
    /// `lib/` directories. Returns the printed output.
    fn run_with(
        source: &str,
        base_dir: Option<std::path::PathBuf>,
        venv_dirs: Vec<std::path::PathBuf>,
    ) -> String {
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        run_source_with(source, base_dir, venv_dirs, &mut input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// Like `run_with`, but returns the error text instead.
    fn run_with_err(
        source: &str,
        base_dir: Option<std::path::PathBuf>,
        venv_dirs: Vec<std::path::PathBuf>,
    ) -> String {
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        run_source_with(source, base_dir, venv_dirs, &mut input, &mut out)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn import_loads_fn_let_and_class() {
        let dir = temp_dir("import-basic");
        write_module(
            &dir,
            "math",
            "let PI = 3.14\nfn doppio(x)\n    return x * 2\nend\nclass Punto\n    fn init(self, x, y)\n        self.x = x\n        self.y = y\n    end\nend\n",
        );
        let src = "import math\nprint math.PI\nprint math.doppio(21)\nlet p = math.Punto(1, 2)\nprint p.x\nprint p.y\nprint type(math)\nprint math\n";
        assert_eq!(
            run_with(src, Some(dir.clone()), vec![]),
            "3.14\n42\n1\n2\nmodule\n<module math>\n"
        );
        // The module value is a first-class value too.
        let src = "import math\nlet m = math\nprint m.doppio(3)\nprint math == m\n";
        assert_eq!(run_with(src, Some(dir.clone()), vec![]), "6\ntrue\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_alias() {
        let dir = temp_dir("import-alias");
        write_module(
            &dir,
            "greeter",
            "fn saluta(nome)\n    return \"Ciao \" + nome\nend\n",
        );
        let src = "import greeter as g\nprint g.saluta(\"Giuliano\")\n";
        assert_eq!(run_with(src, Some(dir.clone()), vec![]), "Ciao Giuliano\n");
        // The original name is not bound when an alias is used.
        let err = run_with_err(
            "import greeter as g\nprint greeter\n",
            Some(dir.clone()),
            vec![],
        );
        assert!(err.contains("undefined variable 'greeter'"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_missing_module_lists_searched_paths() {
        let dir = temp_dir("import-missing");
        let err = run_with_err("import nope\n", Some(dir.clone()), vec![]);
        assert!(err.contains("module 'nope' not found"), "{err}");
        assert!(err.contains("searched:"), "{err}");
        // The importing directory is among the searched paths.
        assert!(
            err.contains(&dir.join("nope.gg").display().to_string()),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_runs_a_module_only_once() {
        let dir = temp_dir("import-cache");
        write_module(&dir, "once", "print \"loaded once\"\nlet x = 1\n");
        write_module(&dir, "other", "import once\nprint \"other\"\n");
        // Imported twice directly, and once through `other`: the
        // module body still runs a single time.
        let src = "import once\nimport once\nimport other\nprint once.x\n";
        assert_eq!(
            run_with(src, Some(dir.clone()), vec![]),
            "loaded once\nother\n1\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_circular_is_an_error() {
        let dir = temp_dir("import-circular");
        write_module(&dir, "a", "import b\nlet x = 1\n");
        write_module(&dir, "b", "import a\nlet y = 2\n");
        let err = run_with_err("import a\n", Some(dir.clone()), vec![]);
        assert!(err.contains("circular import: a -> b -> a"), "{err}");
        // A module importing itself is a cycle too.
        write_module(&dir, "selfish", "import selfish\n");
        let err = run_with_err("import selfish\n", Some(dir.clone()), vec![]);
        assert!(err.contains("circular import: selfish -> selfish"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_nested_resolves_relative_to_the_module() {
        let dir = temp_dir("import-nested");
        let sub = dir.join("sub");
        // `mod` lives in sub/ and imports `helper`, which lives in
        // sub/ too — not next to the main script.
        write_module(&sub, "helper", "fn aiuto()\n    return \"da sub\"\nend\n");
        write_module(
            &sub,
            "mod",
            "import helper\nfn chiama()\n    return helper.aiuto()\nend\n",
        );
        // The entry point runs with sub/ as its base directory, so
        // `import mod` finds sub/mod.gg; mod's own import of
        // `helper` then resolves against sub/ as well.
        let src = "import mod\nprint mod.chiama()\n";
        assert_eq!(run_with(src, Some(sub.clone()), vec![]), "da sub\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_inside_a_function_uses_the_module_directory() {
        let dir = temp_dir("import-in-fn");
        let sub = dir.join("sub");
        write_module(
            &sub,
            "helper",
            "fn chi()\n    return \"helper di sub\"\nend\n",
        );
        // The import happens inside a function defined in the
        // module: it must still resolve against the module's
        // directory, even when the function is called from the
        // main script (whose directory is different).
        write_module(
            &sub,
            "worker",
            "fn lavora()\n    import helper\n    return helper.chi()\nend\n",
        );
        let src = "import worker\nprint worker.lavora()\n";
        assert_eq!(run_with(src, Some(sub.clone()), vec![]), "helper di sub\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_from_a_venv_lib() {
        let dir = temp_dir("import-venv");
        let lib = dir.join("myproj").join("lib");
        write_module(&lib, "maths", "fn raddoppia(x)\n    return x * 2\nend\n");
        // The module lives only in the venv lib: found through the
        // venv search path.
        let src = "import maths\nprint maths.raddoppia(21)\n";
        assert_eq!(run_with(src, Some(dir.clone()), vec![lib.clone()]), "42\n");
        // The script's own directory wins over a venv lib with the
        // same module name.
        write_module(&dir, "maths", "fn raddoppia(x)\n    return x * 100\nend\n");
        assert_eq!(
            run_with(src, Some(dir.clone()), vec![lib.clone()]),
            "2100\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_binds_in_the_current_scope() {
        let dir = temp_dir("import-scope");
        write_module(&dir, "m", "let x = 1\n");
        // An import inside a block binds the name there, like `let`.
        let src = "if true then\n    import m\n    print m.x\nend\n";
        assert_eq!(run_with(src, Some(dir.clone()), vec![]), "1\n");
        let err = run_with_err(
            "if true then\n    import m\nend\nprint m\n",
            Some(dir.clone()),
            vec![],
        );
        assert!(err.contains("undefined variable 'm'"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn module_attribute_errors() {
        let dir = temp_dir("import-attrs");
        write_module(&dir, "m", "let x = 5\nfn f()\n    return 1\nend\n");
        // A missing attribute names the module.
        let err = run_with_err("import m\nprint m.y\n", Some(dir.clone()), vec![]);
        assert!(err.contains("module 'm' has no attribute 'y'"), "{err}");
        let err = run_with_err(
            "import m\nprint m.f()\nprint m.y()\n",
            Some(dir.clone()),
            vec![],
        );
        assert!(err.contains("module 'm' has no attribute 'y'"), "{err}");
        // Builtins are NOT module attributes: the module's
        // environment is a child of the globals, but attribute
        // access only reads the module's own names.
        let err = run_with_err("import m\nprint m.len\n", Some(dir.clone()), vec![]);
        assert!(err.contains("module 'm' has no attribute 'len'"), "{err}");
        // Calling a non-function attribute is a type error.
        let err = run_with_err("import m\nprint m.x()\n", Some(dir.clone()), vec![]);
        assert!(err.contains("cannot call Int"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn module_syntax_errors_name_the_module() {
        let dir = temp_dir("import-syntax");
        write_module(&dir, "bad", "let x =\n");
        let err = run_with_err("import bad\n", Some(dir.clone()), vec![]);
        assert!(err.contains("module 'bad': parse error:"), "{err}");
        write_module(&dir, "badlex", "print \"oops\n");
        let err = run_with_err("import badlex\n", Some(dir.clone()), vec![]);
        assert!(err.contains("module 'badlex': lex error:"), "{err}");
        // A runtime error inside a module names it too.
        write_module(&dir, "badrt", "print undefined_var\n");
        let err = run_with_err("import badrt\n", Some(dir.clone()), vec![]);
        assert!(
            err.contains("module 'badrt': undefined variable 'undefined_var'"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn class_from_a_module_and_is_a() {
        let dir = temp_dir("import-class");
        write_module(
            &dir,
            "animali",
            "class Animale\n    fn init(self, nome)\n        self.nome = nome\n    end\n    fn verso(self)\n        return \"...\"\n    end\nend\nclass Cane extends Animale\n    fn verso(self)\n        return \"bau\"\n    end\nend\n",
        );
        let src = "import animali\nlet c = animali.Cane(\"Fido\")\nprint c.nome\nprint c.verso()\nprint is_a(c, animali.Animale)\nprint is_a(c, animali.Cane)\nprint type(c)\n";
        assert_eq!(
            run_with(src, Some(dir.clone()), vec![]),
            "Fido\nbau\ntrue\ntrue\nCane\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 10: standard library ----

    #[test]
    fn new_string_builtins() {
        assert_eq!(run_ok("print sum([1, 2, 3])\n"), "6\n");
        assert_eq!(run_ok("print sum([1.5, 2])\n"), "3.5\n");
        assert_eq!(run_ok("print type(sum([1, 2]))\n"), "int\n");
        assert_eq!(run_ok("print type(sum([1.5]))\n"), "float\n");
        assert!(run_err("print sum([])\n").contains("sum() of empty list"));
        assert!(run_err("print sum([1, \"a\"])\n").contains("sum() expects numbers"));
        assert!(run_err("print sum(5)\n").contains("sum() expects a List"));
        assert_eq!(run_ok("print join(\", \", [\"a\", \"b\"])\n"), "a, b\n");
        assert_eq!(run_ok("print join(\"-\", [])\n"), "\n");
        assert_eq!(run_ok("print join(\"\", [\"a\", \"b\"])\n"), "ab\n");
        // join is the inverse of split.
        assert_eq!(
            run_ok("print join(\",\", split(\"a,b,c\", \",\"))\n"),
            "a,b,c\n"
        );
        assert!(run_err("print join(\",\", [1, 2])\n").contains("join() expects a List of Str"));
        assert!(run_err("print join(5, [\"a\"])\n").contains("join() expects a Str"));
        assert_eq!(run_ok("print upper(\"ciao\")\n"), "CIAO\n");
        assert_eq!(run_ok("print upper(\"è\")\n"), "È\n");
        assert_eq!(run_ok("print lower(\"CIAO\")\n"), "ciao\n");
        assert_eq!(run_ok("print lower(\"È\")\n"), "è\n");
        assert_eq!(run_ok("print trim(\"  x  \")\n"), "x\n");
        assert_eq!(run_ok("print trim(\"\\n\\tx\\n\")\n"), "x\n");
        assert_eq!(run_ok("print trim(\"xx\")\n"), "xx\n");
        assert_eq!(run_ok("print replace(\"aaa\", \"a\", \"b\")\n"), "bbb\n");
        assert_eq!(run_ok("print replace(\"ciao\", \"z\", \"q\")\n"), "ciao\n");
        assert!(
            run_err("print replace(\"a\", \"\", \"b\")\n")
                .contains("replace() old string must not be empty")
        );
        assert_eq!(run_ok("print starts_with(\"ciao\", \"ci\")\n"), "true\n");
        assert_eq!(run_ok("print starts_with(\"ciao\", \"ao\")\n"), "false\n");
        assert_eq!(run_ok("print ends_with(\"ciao\", \"ao\")\n"), "true\n");
        assert_eq!(run_ok("print ends_with(\"ciao\", \"ci\")\n"), "false\n");
        assert_eq!(run_ok("print starts_with(\"\", \"\")\n"), "true\n");
        assert!(run_err("print upper(5)\n").contains("upper() expects a Str"));
        assert!(run_err("print lower(5)\n").contains("lower() expects a Str"));
        assert!(run_err("print trim(5)\n").contains("trim() expects a Str"));
        assert!(run_err("print starts_with(\"a\", 1)\n").contains("starts_with() expects a Str"));
        assert!(run_err("print ends_with(1, \"a\")\n").contains("ends_with() expects a Str"));
    }

    #[test]
    fn math_module_exact_values() {
        assert_eq!(run_ok("import math\nprint math.sqrt(16)\n"), "4\n");
        assert_eq!(run_ok("import math\nprint math.sqrt(2.25)\n"), "1.5\n");
        assert_eq!(run_ok("import math\nprint math.floor(3.7)\n"), "3\n");
        assert_eq!(run_ok("import math\nprint math.floor(-3.2)\n"), "-4\n");
        assert_eq!(run_ok("import math\nprint math.ceil(3.2)\n"), "4\n");
        assert_eq!(run_ok("import math\nprint math.ceil(-3.7)\n"), "-3\n");
        // Halves round away from zero.
        assert_eq!(run_ok("import math\nprint math.round(2.5)\n"), "3\n");
        assert_eq!(run_ok("import math\nprint math.round(-2.5)\n"), "-3\n");
        assert_eq!(run_ok("import math\nprint math.round(2.4)\n"), "2\n");
        assert_eq!(run_ok("import math\nprint math.round(2.6)\n"), "3\n");
        // floor/ceil/round return Int.
        assert_eq!(
            run_ok("import math\nprint type(math.floor(3.7))\n"),
            "int\n"
        );
        assert_eq!(
            run_ok("import math\nprint type(math.round(2.5))\n"),
            "int\n"
        );
        // pow: two ints give an exact int.
        assert_eq!(run_ok("import math\nprint math.pow(2, 10)\n"), "1024\n");
        assert_eq!(
            run_ok("import math\nprint type(math.pow(2, 10))\n"),
            "int\n"
        );
        assert_eq!(run_ok("import math\nprint math.pow(2, -1)\n"), "0.5\n");
        assert_eq!(run_ok("import math\nprint math.pow(9, 0.5)\n"), "3\n");
        assert_eq!(run_ok("import math\nprint math.sin(0)\n"), "0\n");
        assert_eq!(run_ok("import math\nprint math.cos(0)\n"), "1\n");
        assert_eq!(run_ok("import math\nprint math.tan(0)\n"), "0\n");
        assert_eq!(run_ok("import math\nprint math.log(1)\n"), "0\n");
        assert_eq!(run_ok("import math\nprint math.exp(0)\n"), "1\n");
        // Constants and type.
        assert_eq!(
            run_ok("import math\nprint math.PI\n"),
            "3.141592653589793\n"
        );
        assert_eq!(run_ok("import math\nprint math.E\n"), "2.718281828459045\n");
        assert_eq!(run_ok("import math\nprint type(math)\n"), "module\n");
        assert_eq!(run_ok("import math\nprint math\n"), "<module math>\n");
        // math.PI is between 3.14 and 3.15.
        assert_eq!(
            run_ok("import math\nprint math.PI > 3.14 and math.PI < 3.15\n"),
            "true\n"
        );
        // Errors.
        assert!(
            run_err("import math\nprint math.sqrt(-1)\n")
                .contains("math.sqrt() of a negative number")
        );
        assert!(
            run_err("import math\nprint math.log(0)\n")
                .contains("math.log() of a non-positive number")
        );
        assert!(
            run_err("import math\nprint math.sqrt(\"a\")\n")
                .contains("math.sqrt() expects a number")
        );
        assert!(run_err("import math\nprint math.pow(2, 63)\n").contains("integer overflow"));
        assert!(run_err("import math\nprint math.nope\n").contains("has no attribute 'nope'"));
        // Two imports return the same module value.
        assert_eq!(
            run_ok("import math\nimport math as m2\nprint math == m2\n"),
            "true\n"
        );
    }

    #[test]
    fn random_module_stays_in_range() {
        // 200 draws, every one inside the interval (both ends
        // included: the program prints OUT only on a violation).
        let src = "import random\nfor i = 1 to 200 then\n    let r = random.randint(1, 6)\n    if r < 1 or r > 6 then\n        print \"OUT\"\n    end\nend\nprint \"ok\"\n";
        assert_eq!(run_ok(src), "ok\n");
        // A single-value interval always gives that value.
        assert_eq!(run_ok("import random\nprint random.randint(5, 5)\n"), "5\n");
        // Negative intervals work too.
        let src = "import random\nfor i = 1 to 100 then\n    let r = random.randint(-3, -1)\n    if r < -3 or r > -1 then\n        print \"OUT\"\n    end\nend\nprint \"ok\"\n";
        assert_eq!(run_ok(src), "ok\n");
        assert!(
            run_err("import random\nprint random.randint(5, 1)\n")
                .contains("random.randint() expects a <= b")
        );
        // random() is a float in [0, 1).
        let src = "import random\nfor i = 1 to 100 then\n    let r = random.random()\n    if r < 0 or r >= 1 then\n        print \"OUT\"\n    end\nend\nprint type(random.random())\nprint \"ok\"\n";
        assert_eq!(run_ok(src), "float\nok\n");
        // choice returns an element of the list.
        let src = "import random\nfor i = 1 to 100 then\n    let c = random.choice([10, 20, 30])\n    if c != 10 and c != 20 and c != 30 then\n        print \"OUT\"\n    end\nend\nprint \"ok\"\n";
        assert_eq!(run_ok(src), "ok\n");
        assert!(
            run_err("import random\nprint random.choice([])\n")
                .contains("random.choice() on empty list")
        );
        assert!(
            run_err("import random\nprint random.choice(5)\n")
                .contains("random.choice() expects a List")
        );
        // shuffle keeps the same elements and returns nil.
        let src = "import random\nlet xs = [1, 2, 3, 4, 5]\nprint random.shuffle(xs)\nprint len(xs)\nprint contains(xs, 1)\nprint contains(xs, 2)\nprint contains(xs, 3)\nprint contains(xs, 4)\nprint contains(xs, 5)\n";
        assert_eq!(run_ok(src), "nil\n5\ntrue\ntrue\ntrue\ntrue\ntrue\n");
        assert!(
            run_err("import random\nprint random.shuffle(5)\n")
                .contains("random.shuffle() expects a List")
        );
        assert_eq!(run_ok("import random\nprint type(random)\n"), "module\n");
    }

    #[test]
    fn stats_module_on_known_data() {
        assert_eq!(
            run_ok("import stats\nprint stats.mean([1, 2, 3, 4])\n"),
            "2.5\n"
        );
        assert_eq!(run_ok("import stats\nprint stats.mean([5])\n"), "5\n");
        assert_eq!(
            run_ok("import stats\nprint stats.median([1, 3, 2])\n"),
            "2\n"
        );
        assert_eq!(
            run_ok("import stats\nprint stats.median([1, 2, 3, 4])\n"),
            "2.5\n"
        );
        assert_eq!(run_ok("import stats\nprint stats.median([9])\n"), "9\n");
        // Median sorts: the input order does not matter, and the
        // input list is not modified.
        let src = "import stats\nlet xs = [5, 1, 4]\nprint stats.median(xs)\nprint xs\n";
        assert_eq!(run_ok(src), "4\n[5, 1, 4]\n");
        // Population variance of [1,2,3,4,5] is 2, stdev is sqrt(2).
        assert_eq!(
            run_ok("import stats\nprint stats.variance([1, 2, 3, 4, 5])\n"),
            "2\n"
        );
        assert_eq!(
            run_ok("import stats\nprint stats.stdev([1, 2, 3, 4, 5])\n"),
            "1.4142135623730951\n"
        );
        assert_eq!(
            run_ok("import stats\nprint stats.variance([7, 7, 7])\n"),
            "0\n"
        );
        assert_eq!(run_ok("import stats\nprint type(stats)\n"), "module\n");
        // Empty lists are errors (mean indexes the empty list
        // before it could divide by zero).
        assert!(run_err("import stats\nprint stats.mean([])\n").contains("out of range"));
        assert!(run_err("import stats\nprint stats.median([])\n").contains("out of range"));
        // stats imports the native math module internally.
        assert_eq!(
            run_ok("import stats\nprint stats.stdev([2, 4, 4, 4, 5, 5, 7, 9])\n"),
            "2\n"
        );
    }

    #[test]
    fn user_file_overrides_native_math() {
        let dir = temp_dir("import-override-math");
        write_module(
            &dir,
            "math",
            "let PI = 99\nfn sqrt(x)\n    return -1\nend\n",
        );
        let src = "import math\nprint math.PI\nprint math.sqrt(16)\n";
        assert_eq!(run_with(src, Some(dir.clone()), vec![]), "99\n-1\n");
        let _ = std::fs::remove_dir_all(&dir);
        // And a user file overrides the embedded stats too.
        let dir = temp_dir("import-override-stats");
        write_module(&dir, "stats", "fn mean(xs)\n    return 42\nend\n");
        let src = "import stats\nprint stats.mean([1, 2, 3])\n";
        assert_eq!(run_with(src, Some(dir.clone()), vec![]), "42\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- gui (Phase 13): error paths only — opening a real
    // window is verified by hand under Xvfb, never in `cargo test`.
    #[cfg(feature = "gui")]
    #[test]
    fn gui_module_type_and_widgets_outside_a_frame() {
        assert_eq!(run_ok("import gui\nprint type(gui)\n"), "module\n");
        // A widget outside `gui.run` is a runtime error, not a panic.
        assert!(run_err("import gui\ngui.button(\"x\")\n").contains("inside a frame function"));
        assert!(run_err("import gui\ngui.heading(\"x\")\n").contains("inside a frame function"));
        assert!(run_err("import gui\ngui.label(\"x\")\n").contains("inside a frame function"));
        assert!(run_err("import gui\ngui.separator()\n").contains("inside a frame function"));
        assert!(
            run_err("import gui\ngui.text_input(\"id\", \"v\")\n")
                .contains("inside a frame function")
        );
        assert!(
            run_err("import gui\ngui.slider(\"id\", 1, 0, 2)\n")
                .contains("inside a frame function")
        );
        assert!(
            run_err("import gui\ngui.checkbox(\"id\", \"t\", true)\n")
                .contains("inside a frame function")
        );
    }

    #[cfg(feature = "gui")]
    #[test]
    fn gui_run_and_horizontal_validate_arguments() {
        // `gui.run` with a non-string title.
        assert!(run_err("import gui\ngui.run(1, 2)\n").contains("gui.run()"));
        // `gui.run` with a second argument that is not callable.
        assert!(run_err("import gui\ngui.run(\"T\", 42)\n").contains("expects a function"));
        // `gui.horizontal` outside a frame.
        assert!(
            run_err("import gui\nfn f()\nend\ngui.horizontal(f)\n")
                .contains("inside a frame function")
        );
    }

    // ---- Phase 14: linalg ----

    #[test]
    fn linalg_solve_det_inv() {
        // 2x + y = 5, x + 3y = 10 -> x = 1, y = 3.
        let src = "import linalg\nlet a = reshape(array([2, 1, 1, 3]), [2, 2])\nprint linalg.solve(a, array([5, 10]))\n";
        assert_eq!(run_ok(src), "array([1, 3])\n");
        // A 3x3 triangular system, exact in binary: x + 2y = 5,
        // y + 3z = 7, z = 2 -> x = 3, y = 1, z = 2. (The classic
        // 3x3 from the textbooks gives 3.0000000000000004 for y:
        // Gaussian elimination in f64 rounds a little, like
        // NumPy — the demo shows that instead of hiding it.)
        let src = "import linalg\nlet a = reshape(array([1, 2, 0, 0, 1, 3, 0, 0, 1]), [3, 3])\nprint linalg.solve(a, array([5, 7, 2]))\n";
        assert_eq!(run_ok(src), "array([3, 1, 2])\n");
        // Solving and multiplying back gives b (element-wise ==).
        let src = "import linalg\nlet a = reshape(array([2, 1, 1, 3]), [2, 2])\nlet b = array([5, 10])\nprint matmul(a, linalg.solve(a, b)) == b\n";
        assert_eq!(run_ok(src), "array([true, true])\n");
        // Determinants, including a singular one (det = 0).
        let src = "import linalg\nprint linalg.det(reshape(array([1, 2, 3, 4]), [2, 2]))\n";
        assert_eq!(run_ok(src), "-2\n");
        // A triangular 3x3: the determinant is the product of
        // the diagonal, exactly. (The classic textbook 3x3 gives
        // -0.9999999999999993: partial pivoting divides by 3,
        // which is not exact in binary — same rounding as above.)
        let src = "import linalg\nprint linalg.det(reshape(array([2, 1, -1, 0, -1, 2, 0, 0, 2]), [3, 3]))\n";
        assert_eq!(run_ok(src), "-4\n");
        let src = "import linalg\nprint linalg.det(reshape(array([1, 2, 2, 4]), [2, 2]))\n";
        assert_eq!(run_ok(src), "0\n");
        // Inverses: a diagonal one is exact, and so is a
        // triangular one. (The inverse of [[1, 2], [3, 4]] is
        // [[-2, 1], [1.5, -0.5]] in exact arithmetic, but partial
        // pivoting divides by 3, which is not exact in binary —
        // the round-trip test below checks it within 1e-9.)
        let src = "import linalg\nprint linalg.inv(reshape(array([2, 0, 0, 2]), [2, 2]))\n";
        assert_eq!(run_ok(src), "array([[0.5, 0], [0, 0.5]])\n");
        let src = "import linalg\nprint linalg.inv(reshape(array([1, 2, 0, 1]), [2, 2]))\n";
        assert_eq!(run_ok(src), "array([[1, -2], [0, 1]])\n");
        // A singular matrix has no solution and no inverse:
        //  x + 2y = 3, 2x + 4y = 6 is indeterminate, and
        //  x + 2y = 3, 2x + 4y = 7 is impossible — both are
        // the same runtime error, as in GG's other builtins.
        let singular = "reshape(array([1, 2, 2, 4]), [2, 2])";
        assert!(
            run_err(&format!(
                "import linalg\nprint linalg.solve({singular}, array([3, 6]))\n"
            ))
            .contains("singular matrix")
        );
        assert!(
            run_err(&format!("import linalg\nprint linalg.inv({singular})\n"))
                .contains("singular matrix")
        );
        // Shape and type errors.
        assert!(
            run_err(
                "import linalg\nprint linalg.det(reshape(array([1, 2, 3, 4, 5, 6]), [2, 3]))\n"
            )
            .contains("expects a square 2D Array")
        );
        assert!(
            run_err(
                "import linalg\nprint linalg.solve(reshape(array([2, 1, 1, 3]), [2, 2]), array([5, 10, 1]))\n"
            )
            .contains("expects a vector of length 2")
        );
        assert!(
            run_err(
                "import linalg\nprint linalg.det(reshape(array([\"a\", \"b\", \"c\", \"d\"]), [2, 2]))\n"
            )
            .contains("expects numbers, not array of Str")
        );
    }

    #[test]
    fn linalg_inverse_roundtrip_within_tolerance() {
        // inv([[1, 2], [3, 4]]) times the matrix is the
        // identity, up to f64 rounding (checked here in Rust,
        // where a tolerance is possible — GG `print` has none).
        use crate::array::{make_array, matmul};
        use crate::linalg::inverse;
        use crate::value::ElemType;
        let a = make_array(
            ElemType::Int,
            vec![2, 2],
            vec![Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)],
        );
        let Value::Array(a_data) = &a else {
            unreachable!()
        };
        let inv = inverse(&a_data.borrow()).unwrap();
        let Value::Array(inv_data) = &inv else {
            unreachable!()
        };
        let prod = matmul(&inv_data.borrow(), &a_data.borrow()).unwrap();
        let Value::Array(prod_data) = prod else {
            unreachable!()
        };
        let items = &prod_data.borrow().items;
        for (got, want) in items.iter().zip([1.0, 0.0, 0.0, 1.0]) {
            let Value::Float(x) = got else {
                panic!("not a float: {got}")
            };
            assert!((x - want).abs() < 1e-9, "{x} vs {want}");
        }
    }
}
