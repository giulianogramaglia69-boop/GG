// SPDX-License-Identifier: GPL-3.0-or-later
//! Tree-walking interpreter for the GG programming language.
//!
//! Phase 3: `Interpreter` runs a parsed `Program`; `run_source` is
//! the one-call pipeline (lex + parse + interpret) used by the `gg`
//! binary and by the tests.

pub mod builtin;
pub mod env;
pub mod error;
pub mod interp;
pub mod repl;
pub mod value;

use std::fmt;
use std::io::{BufRead, Write};

pub use builtin::Builtin;
pub use error::RuntimeError;
pub use interp::Interpreter;
pub use repl::run_repl;
pub use value::Value;

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
        assert!(run_err("print max(1)\n").contains("max() expects 2 argument(s) but got 1"));
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
        assert!(run_err("print \"abc\"[0]\n").contains("cannot index Str"));
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
        assert!(run_err("print len(5)\n").contains("len() expects a Str or a List"));
    }

    #[test]
    fn push_and_pop() {
        let src = "let xs = []\npush(xs, 1)\npush(xs, 2)\nprint xs\nprint pop(xs)\nprint xs\n";
        assert_eq!(run_ok(src), "[1, 2]\n2\n[1]\n");
        // push returns nil.
        assert_eq!(run_ok("let xs = []\nprint push(xs, 1)\n"), "nil\n");
        assert!(run_err("print pop([])\n").contains("pop() on empty list"));
        assert!(run_err("print push(5, 1)\n").contains("push() expects a List"));
        assert!(run_err("print pop(\"s\")\n").contains("pop() expects a List"));
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
}
