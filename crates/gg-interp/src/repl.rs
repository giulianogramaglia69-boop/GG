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
//! (Ctrl-D) leaves too.
//!
//! The plain loop in `run_repl_with_venv` reads lines from stdin,
//! which is also how the REPL works when stdin is piped (and how the
//! tests drive it). When stdin is a terminal, the `gg` binary drives
//! the same session through `rustyline` instead (`ReplSession`),
//! which adds line editing and arrow-key history.

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
    print_welcome(out)?;
    let mut session = ReplSession::new(venv_dirs);
    session.set_pkg_roots(crate::pkg::default_pkg_roots());
    loop {
        out.write_all(session.prompt().as_bytes())?;
        out.flush()?;
        let mut line = String::new();
        let read = input.read_line(&mut line)?;
        if read == 0 {
            // End of input (Ctrl-D): leave the terminal line clean.
            writeln!(out)?;
            return Ok(());
        }
        if session.handle_line(&line, input, out)? {
            return Ok(());
        }
    }
}

/// The banner printed once at the start of a session, by both the
/// stream loop and the interactive (`rustyline`) driver.
pub fn print_welcome(out: &mut dyn Write) -> std::io::Result<()> {
    writeln!(out, "GG {VERSION} — interactive REPL. Type exit to quit.")
}

/// The state of one REPL session, shared by the plain stream loop
/// above and by the `gg` binary's interactive driver: the
/// interpreter, the lines of the block being typed, and the prompt
/// currently shown (`gg> ` or `... `).
pub struct ReplSession {
    interp: Interpreter,
    buffer: String,
    prompt: &'static str,
}

impl ReplSession {
    /// A fresh session: the interpreter has no definitions yet.
    pub fn new(venv_dirs: Vec<std::path::PathBuf>) -> Self {
        let mut interp = Interpreter::new();
        interp.set_venv_dirs(venv_dirs);
        Self {
            interp,
            buffer: String::new(),
            prompt: PROMPT,
        }
    }

    /// The package roots `import` searches (after the native
    /// modules): the project `gg_packages/` first, then the
    /// global store. Set by the `gg` binary for the interactive
    /// driver; `run_repl_with_venv` sets the defaults itself.
    pub fn set_pkg_roots(&mut self, dirs: Vec<std::path::PathBuf>) {
        self.interp.set_pkg_roots(dirs);
    }

    /// The prompt to show before the next line.
    pub fn prompt(&self) -> &'static str {
        self.prompt
    }

    /// True when no block is being typed: the next line starts a
    /// fresh chunk. The interactive driver uses this to know when a
    /// chunk is complete (and so is one history entry).
    pub fn at_chunk_start(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Drop the block being typed and go back to the `gg> ` prompt
    /// (Ctrl-C in the interactive REPL).
    pub fn abort_chunk(&mut self) {
        self.buffer.clear();
        self.prompt = PROMPT;
    }

    /// Feed one line of input (with or without its trailing
    /// newline). Returns `true` when the session should end
    /// (`exit`/`quit` on its own line). Errors are printed and the
    /// session goes on with its state intact; `input` is the reader
    /// GG `input()` reads from.
    pub fn handle_line(
        &mut self,
        line: &str,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> std::io::Result<bool> {
        let trimmed = line.trim();
        if self.buffer.is_empty() && (trimmed == "exit" || trimmed == "quit") {
            return Ok(true);
        }
        if self.buffer.is_empty() && trimmed.is_empty() {
            return Ok(false);
        }
        self.buffer.push_str(line);
        if !self.buffer.ends_with('\n') {
            self.buffer.push('\n');
        }
        // Trailing whitespace is stripped before parsing, so an
        // unfinished block fails at end of input (and is detected as
        // incomplete) instead of at the trailing newline.
        let tokens = match gg_lexer::lex(self.buffer.trim_end()) {
            Ok(tokens) => tokens,
            Err(e) => {
                // A lex error is never "unfinished": strings cannot
                // span lines, so more input cannot fix it.
                writeln!(out, "error: {e}")?;
                self.buffer.clear();
                self.prompt = PROMPT;
                return Ok(false);
            }
        };
        match gg_parser::parse(&tokens) {
            Ok(program) => {
                self.buffer.clear();
                self.prompt = PROMPT;
                if let Err(e) = run_chunk(&mut self.interp, &program, input, out) {
                    writeln!(out, "runtime error: {e}")?;
                }
            }
            Err(e) if e.is_incomplete() => {
                // Keep the buffer and wait for more lines.
                self.prompt = CONTINUATION_PROMPT;
            }
            Err(e) => {
                writeln!(out, "error: {e}")?;
                self.buffer.clear();
                self.prompt = PROMPT;
            }
        }
        Ok(false)
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
    if let [Stmt::ExprStmt { expr, .. }] = program.stmts.as_slice() {
        let value = interp.eval(expr, input, out)?;
        if !matches!(value, Value::Nil) {
            writeln!(out, "{value}")
                .map_err(|e| RuntimeError::new(format!("output error: {e}")))?;
        }
        Ok(())
    } else {
        interp.run_repl_chunk(program, input, out)
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
    fn multi_line_for_in_uses_the_continuation_prompt() {
        assert_eq!(
            session("for x in [1, 2] then\nprint x\nend\nexit\n"),
            format!("{WELCOME}gg> ... ... 1\n2\ngg> ")
        );
    }

    #[test]
    fn multi_line_try_except_uses_the_continuation_prompt() {
        assert_eq!(
            session("try then\nprint 1 / 0\nexcept e then\nprint e\nend\nexit\n"),
            format!("{WELCOME}gg> ... ... ... ... division by zero\ngg> ")
        );
    }

    #[test]
    fn raise_in_the_repl_is_caught_by_try() {
        assert_eq!(
            session("try then\nraise \"ciao\"\nexcept e then\nprint e\nend\nexit\n"),
            format!("{WELCOME}gg> ... ... ... ... ciao\ngg> ")
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
        // `x` is 1 here, to make the check unambiguous. The error
        // carries the position of `y` (line 1, col 7).
        assert_eq!(
            session("let x = 1\nprint y\nx\nexit\n"),
            format!("{WELCOME}gg> gg> runtime error: 1:7: undefined variable 'y'\ngg> 1\ngg> ")
        );
    }

    #[test]
    fn runtime_error_position_is_relative_to_the_lines_typed() {
        // A multi-line block: the error is on the second line of the
        // block, at the position of `y` in that line.
        assert_eq!(
            session("if true then\nprint y\nend\nexit\n"),
            format!("{WELCOME}gg> ... ... runtime error: 2:7: undefined variable 'y'\ngg> ")
        );
        // Each line typed on its own is its own chunk, so a
        // single-line statement reports line 1 — even when it is
        // not the first line of the session.
        assert_eq!(
            session("let x = 1\nprint x / 0\nexit\n"),
            format!("{WELCOME}gg> gg> runtime error: 1:9: division by zero\ngg> ")
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
    fn stdlib_modules_work_in_the_repl() {
        assert_eq!(
            session("import math\nmath.sqrt(16)\nimport stats\nstats.mean([1, 2, 3, 4])\nexit\n"),
            format!("{WELCOME}gg> gg> 4\ngg> gg> 2.5\ngg> ")
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

    #[test]
    fn session_tracks_the_continuation_prompt() {
        let mut session = ReplSession::new(Vec::new());
        assert_eq!(session.prompt(), "gg> ");
        assert!(session.at_chunk_start());
        let mut input = Cursor::new(Vec::new());
        let mut out = Vec::new();
        assert!(
            !session
                .handle_line("if true then\n", &mut input, &mut out)
                .unwrap()
        );
        assert_eq!(session.prompt(), "... ");
        assert!(!session.at_chunk_start());
        assert!(
            !session
                .handle_line("print \"x\"\n", &mut input, &mut out)
                .unwrap()
        );
        assert_eq!(session.prompt(), "... ");
        assert!(!session.handle_line("end\n", &mut input, &mut out).unwrap());
        assert_eq!(session.prompt(), "gg> ");
        assert!(session.at_chunk_start());
        assert_eq!(String::from_utf8(out).unwrap(), "x\n");
    }

    #[test]
    fn session_accepts_lines_without_a_trailing_newline() {
        // Lines from rustyline have no trailing newline.
        let mut session = ReplSession::new(Vec::new());
        let mut input = Cursor::new(Vec::new());
        let mut out = Vec::new();
        assert!(
            !session
                .handle_line("let x = 5", &mut input, &mut out)
                .unwrap()
        );
        assert!(!session.handle_line("x", &mut input, &mut out).unwrap());
        assert_eq!(String::from_utf8(out).unwrap(), "5\n");
    }

    #[test]
    fn session_abort_chunk_discards_the_block() {
        let mut session = ReplSession::new(Vec::new());
        let mut input = Cursor::new(Vec::new());
        let mut out = Vec::new();
        assert!(
            !session
                .handle_line("if true then\n", &mut input, &mut out)
                .unwrap()
        );
        session.abort_chunk();
        assert_eq!(session.prompt(), "gg> ");
        assert!(session.at_chunk_start());
        // The aborted block never ran, and the session still works.
        assert!(
            !session
                .handle_line("1 + 1\n", &mut input, &mut out)
                .unwrap()
        );
        assert_eq!(String::from_utf8(out).unwrap(), "2\n");
    }

    #[test]
    fn session_exit_and_empty_lines() {
        let mut session = ReplSession::new(Vec::new());
        let mut input = Cursor::new(Vec::new());
        let mut out = Vec::new();
        assert!(!session.handle_line("\n", &mut input, &mut out).unwrap());
        assert!(!session.handle_line("   \n", &mut input, &mut out).unwrap());
        assert!(session.at_chunk_start());
        assert!(session.handle_line("exit\n", &mut input, &mut out).unwrap());
    }

    #[test]
    fn session_error_lines_finish_the_chunk() {
        // A line that fails is still a finished chunk: the driver
        // records it in the history like any other line.
        let mut session = ReplSession::new(Vec::new());
        let mut input = Cursor::new(Vec::new());
        let mut out = Vec::new();
        assert!(
            !session
                .handle_line("let = 5\n", &mut input, &mut out)
                .unwrap()
        );
        assert!(session.at_chunk_start());
        assert_eq!(session.prompt(), "gg> ");
    }
}
