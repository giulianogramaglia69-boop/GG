// SPDX-License-Identifier: GPL-3.0-or-later
//! The GG interactive REPL: read a line, evaluate it, print the
//! result (`gg` with no arguments, or `gg repl`).
//!
//! State persists across lines: one `Interpreter` lives for the
//! whole session, so variables and functions defined on one line are
//! visible on the next. Blocks (`if`/`while`/`for`/`fn`) span several
//! lines: after each line the accumulated buffer is parsed, and if
//! the parse fails only because the input is unfinished (missing
//! `end`, missing `then`, unclosed parenthesis) the REPL waits for
//! more lines with the `... ` prompt instead of reporting an error.
//!
//! Expression statements echo their value (`1 + 2` prints `3`);
//! other statements print nothing by themselves, and `nil` is never
//! echoed. Errors are printed and the session goes on with its state
//! intact. `exit` (or `quit`) on its own line leaves; end of input
//! (Ctrl-D) leaves too. No line editing or history: the REPL reads
//! plain lines from stdin (a `rustyline` upgrade can add that later).

use std::io::{BufRead, Write};

use gg_parser::ast::{Program, Stmt};

use crate::error::RuntimeError;
use crate::interp::Interpreter;
use crate::value::Value;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const PROMPT: &str = "gg> ";
const CONTINUATION_PROMPT: &str = "... ";

/// Run the REPL: prompts go to `out`, lines come from `input`.
/// `input` is also the reader `input()` in GG programs reads from,
/// so a program can ask the user for a value mid-session.
pub fn run_repl(input: &mut dyn BufRead, out: &mut dyn Write) -> std::io::Result<()> {
    let mut interp = Interpreter::new();
    writeln!(out, "GG {VERSION} — interactive REPL. Type exit to quit.")?;
    let mut buffer = String::new();
    let mut prompt = PROMPT;
    loop {
        out.write_all(prompt.as_bytes())?;
        out.flush()?;
        let mut line = String::new();
        let read = input.read_line(&mut line)?;
        if read == 0 {
            // End of input (Ctrl-D): leave the terminal line clean.
            writeln!(out)?;
            return Ok(());
        }
        let trimmed = line.trim();
        if buffer.is_empty() && (trimmed == "exit" || trimmed == "quit") {
            return Ok(());
        }
        if buffer.is_empty() && trimmed.is_empty() {
            continue;
        }
        buffer.push_str(&line);
        // Trailing whitespace is stripped before parsing, so an
        // unfinished block fails at end of input (and is detected as
        // incomplete) instead of at the trailing newline.
        let tokens = match gg_lexer::lex(buffer.trim_end()) {
            Ok(tokens) => tokens,
            Err(e) => {
                // A lex error is never "unfinished": strings cannot
                // span lines, so more input cannot fix it.
                writeln!(out, "error: {e}")?;
                buffer.clear();
                prompt = PROMPT;
                continue;
            }
        };
        match gg_parser::parse(&tokens) {
            Ok(program) => {
                buffer.clear();
                prompt = PROMPT;
                if let Err(e) = run_chunk(&mut interp, &program, &mut *input, out) {
                    writeln!(out, "runtime error: {e}")?;
                }
            }
            Err(e) if e.is_incomplete() => {
                // Keep the buffer and wait for more lines.
                prompt = CONTINUATION_PROMPT;
            }
            Err(e) => {
                writeln!(out, "error: {e}")?;
                buffer.clear();
                prompt = PROMPT;
            }
        }
    }
}

/// Run one parsed chunk: a lone expression statement echoes its
/// value (unless it is `nil`); anything else runs as a program.
fn run_chunk(
    interp: &mut Interpreter,
    program: &Program,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<(), RuntimeError> {
    if let [Stmt::ExprStmt { expr }] = program.stmts.as_slice() {
        let value = interp.eval(expr, input, out)?;
        if !matches!(value, Value::Nil) {
            writeln!(out, "{value}")
                .map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
        }
        Ok(())
    } else {
        interp.run(program, input, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Run a whole REPL session with `input` as stdin and return the
    /// full transcript (prompts, echoes and errors included).
    fn session(input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes().to_vec());
        let mut out = Vec::new();
        run_repl(&mut input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    const WELCOME: &str = "GG 0.1.0 — interactive REPL. Type exit to quit.\n";

    #[test]
    fn state_persists_across_lines() {
        assert_eq!(
            session("let x = 5\nx\nexit\n"),
            format!("{WELCOME}gg> gg> 5\ngg> ")
        );
    }

    #[test]
    fn expressions_echo_their_value_and_eof_quits() {
        assert_eq!(
            session("1 + 2\nlen(\"ciao\")\n"),
            format!("{WELCOME}gg> 3\ngg> 4\ngg> \n")
        );
    }

    #[test]
    fn no_echo_for_let_and_nil() {
        assert_eq!(
            session("let x = 5\nx\nfn f()\nend\nf()\nexit\n"),
            format!("{WELCOME}gg> gg> 5\ngg> ... gg> gg> ")
        );
    }

    #[test]
    fn multi_line_if_uses_the_continuation_prompt() {
        assert_eq!(
            session("if 1 < 2 then\nprint \"yes\"\nend\nexit\n"),
            format!("{WELCOME}gg> ... ... yes\ngg> ")
        );
    }

    #[test]
    fn fn_defined_across_lines_then_called() {
        assert_eq!(
            session("fn add(a, b)\nreturn a + b\nend\nadd(2, 3)\nexit\n"),
            format!("{WELCOME}gg> ... ... gg> 5\ngg> ")
        );
    }

    #[test]
    fn parse_error_is_reported_and_the_session_recovers() {
        assert_eq!(
            session("let = 5\nlet x = 5\nx\nexit\n"),
            format!(
                "{WELCOME}gg> error: line 1, col 5: expected variable name after 'let' but found '='\ngg> gg> 5\ngg> "
            )
        );
    }

    #[test]
    fn runtime_error_does_not_clear_state() {
        // `x` is 1 here, to make the check unambiguous.
        assert_eq!(
            session("let x = 1\nprint y\nx\nexit\n"),
            format!("{WELCOME}gg> gg> runtime error: undefined variable 'y'\ngg> 1\ngg> ")
        );
    }

    #[test]
    fn exit_and_empty_input_quit() {
        assert_eq!(session("exit\n"), format!("{WELCOME}gg> "));
        assert_eq!(session("quit\n"), format!("{WELCOME}gg> "));
        assert_eq!(session(""), format!("{WELCOME}gg> \n"));
    }

    #[test]
    fn input_inside_the_repl_reads_from_the_same_stdin() {
        // The answer to `input` is the next line in the stream, after
        // the line that runs it.
        assert_eq!(
            session("let name = input(\"Name: \")\nGiuliano\nprint name\nexit\n"),
            format!("{WELCOME}gg> Name: gg> Giuliano\ngg> ")
        );
    }

    #[test]
    fn then_must_stay_on_the_condition_line() {
        // `if x` alone is incomplete, so the REPL waits — but the
        // next line cannot complete it, because GG wants `then` on
        // the same line as the condition. The error is reported and
        // the session recovers.
        assert_eq!(
            session("if x\nthen\nlet a = 1\na\nexit\n"),
            format!(
                "{WELCOME}gg> ... error: line 1, col 5: expected 'then' after if condition but found newline\ngg> gg> 1\ngg> "
            )
        );
    }

    #[test]
    fn lex_error_is_reported_and_the_session_recovers() {
        assert_eq!(
            session("print \"oops\nlet x = 2\nx\nexit\n"),
            format!("{WELCOME}gg> error: line 1, col 7: unterminated string\ngg> gg> 2\ngg> ")
        );
    }
}
