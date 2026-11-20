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
    run_repl_with_venv(input, out, Vec::new())
}

/// Like `run_repl`, but `import` also searches the given venv
/// `lib/` directories (after the working directory fallback — the
/// REPL has no script directory). The `gg` binary passes the
/// `GG_VENV` directories here.
pub fn run_repl_with_venv(
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    venv_dirs: Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    let mut interp = Interpreter::new();
    interp.set_venv_dirs(venv_dirs);
    run_repl_with(&mut interp, input, out)
}

fn run_repl_with(
    interp: &mut Interpreter,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> std::io::Result<()> {
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
                if let Err(e) = run_chunk(interp, &program, &mut *input, out) {
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

    #[test]
    fn lists_work_in_the_repl() {
        assert_eq!(
            session("let xs = [1, 2]\nxs\nxs[1]\npush(xs, 3)\nxs\nexit\n"),
            format!("{WELCOME}gg> gg> [1, 2]\ngg> 2\ngg> gg> [1, 2, 3]\ngg> ")
        );
    }

    #[test]
    fn tuples_and_dicts_work_in_the_repl() {
        assert_eq!(
            session(
                "let t = (1, \"a\")\nt\nt[1]\nlet d = {\"k\": 5}\nd[\"k\"]\nd[\"j\"] = 6\nd\nexit\n"
            ),
            format!(
                "{WELCOME}gg> gg> (1, \"a\")\ngg> a\ngg> gg> 5\ngg> gg> {{\"k\": 5, \"j\": 6}}\ngg> "
            )
        );
    }

    #[test]
    fn classes_work_in_the_repl_across_lines() {
        assert_eq!(
            session(
                "class Persona\nfn init(self, nome)\nself.nome = nome\nend\nfn saluta(self)\nreturn \"Ciao \" + self.nome\nend\nend\nlet p = Persona(\"Giuliano\")\np.saluta()\np\np.nome = \"Fabiana\"\np\nexit\n"
            ),
            format!(
                "{WELCOME}gg> ... ... ... ... ... ... ... gg> gg> Ciao Giuliano\ngg> Persona{{nome: \"Giuliano\"}}\ngg> gg> Persona{{nome: \"Fabiana\"}}\ngg> "
            )
        );
    }

    #[test]
    fn import_from_a_venv_lib_in_the_repl() {
        let dir = std::env::temp_dir().join(format!("gg-test-repl-venv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = dir.join("myproj").join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(
            lib.join("maths.gg"),
            "fn raddoppia(x)\n    return x * 2\nend\n",
        )
        .unwrap();
        let mut input = Cursor::new(
            "import maths\nmaths.raddoppia(21)\nexit\n"
                .as_bytes()
                .to_vec(),
        );
        let mut out = Vec::new();
        run_repl_with_venv(&mut input, &mut out, vec![lib]).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("{WELCOME}gg> gg> 42\ngg> ")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Restores the working directory when dropped, so a test can
    /// change it temporarily without affecting other tests.
    struct RestoreCwd(std::path::PathBuf);

    impl Drop for RestoreCwd {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    #[test]
    fn import_from_the_working_directory_in_the_repl() {
        let dir = std::env::temp_dir().join(format!("gg-test-repl-cwd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("saluti.gg"),
            "fn ciao(nome)\n    return \"Ciao \" + nome\nend\n",
        )
        .unwrap();
        // The REPL has no script directory: imports fall back to
        // the working directory.
        let _guard = RestoreCwd(std::env::current_dir().unwrap());
        std::env::set_current_dir(&dir).unwrap();
        assert_eq!(
            session("import saluti\nsaluti.ciao(\"Giuliano\")\nexit\n"),
            format!("{WELCOME}gg> gg> Ciao Giuliano\ngg> ")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
