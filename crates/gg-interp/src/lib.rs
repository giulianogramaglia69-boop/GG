// SPDX-License-Identifier: GPL-3.0-or-later
//! Tree-walking interpreter for the GG programming language.
//!
//! Phase 3: `Interpreter` runs a parsed `Program`; `run_source` is
//! the one-call pipeline (lex + parse + interpret) used by the `gg`
//! binary and by the tests.

pub mod env;
pub mod error;
pub mod interp;
pub mod value;

use std::fmt;
use std::io::Write;

pub use error::RuntimeError;
pub use interp::Interpreter;
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

/// Lex, parse and run `source`, writing `print` output to `out`.
pub fn run_source(source: &str, out: &mut dyn Write) -> Result<(), GgError> {
    let tokens = gg_lexer::lex(source)?;
    let program = gg_parser::parse(&tokens)?;
    Interpreter::new().run(&program, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `source` and return its printed output, or the error text.
    fn run_ok(source: &str) -> String {
        let mut out = Vec::new();
        run_source(source, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn run_err(source: &str) -> String {
        let mut out = Vec::new();
        let err = run_source(source, &mut out).unwrap_err();
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
}
