// SPDX-License-Identifier: GPL-3.0-or-later
//! The interactive REPL: the same `ReplSession` the stream loop
//! uses, driven through `rustyline` when stdin is a terminal, so
//! the user gets line editing and arrow-key history.
//!
//! The history persists in `~/.gg_history`, one entry per chunk: a
//! multi-line block is a single entry, so up-arrow recalls the whole
//! block. When stdin is piped, `main::start_repl` uses the plain
//! stream loop instead, so piped sessions behave exactly as before
//! and never touch the history file.
//!
//! Keys: up/down walk the history; Ctrl-C drops the line being typed
//! (and the block in progress, if any) — before this upgrade Ctrl-C
//! killed the process; Ctrl-D on an empty line leaves, even in the
//! middle of a block, like end of input on a pipe. `exit`/`quit`
//! are not recorded in the history.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

/// Run the REPL on a terminal: `rustyline` for input, the shared
/// `ReplSession` for evaluation.
pub fn start_repl_interactive() -> ExitCode {
    let history_path = history_path();
    let mut rl = match DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => {
            eprintln!("error: cannot start the interactive REPL: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(path) = &history_path {
        // A missing file on the first run is fine: there is simply
        // no history yet.
        let _ = rl.load_history(path);
    }
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if let Err(e) = gg_interp::print_welcome(&mut out) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let mut session = gg_interp::ReplSession::new(gg_interp::venv_dirs_from_env());
    session.set_pkg_roots(gg_interp::pkg::default_pkg_roots());
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    // Note: `input` and rustyline both read stdin. While the REPL
    // waits at a prompt, rustyline owns the terminal; while a chunk
    // runs, GG `input()` reads plain lines from the same stdin. If
    // the user types ahead while answering `input()`, those extra
    // lines may sit in the lock's buffer and never reach rustyline.
    let mut pending = PendingChunk::new();
    loop {
        match rl.readline(session.prompt()) {
            Ok(line) => {
                pending.add(&line);
                match session.handle_line(&line, &mut input, &mut out) {
                    Ok(true) => break,
                    Ok(false) => {
                        if session.at_chunk_start()
                            && let Some(entry) = pending.finish()
                        {
                            let _ = rl.add_history_entry(entry);
                        }
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        break;
                    }
                }
                let _ = out.flush();
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: drop the line and the block in progress.
                session.abort_chunk();
                pending.clear();
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D on an empty line: leave the terminal line clean.
                let _ = writeln!(out);
                break;
            }
            Err(e) => {
                eprintln!("error: {e}");
                break;
            }
        }
    }
    if let Some(path) = &history_path {
        let _ = rl.save_history(path);
    }
    let _ = out.flush();
    ExitCode::SUCCESS
}

/// Where the history lives: `~/.gg_history`. `None` when HOME is
/// unset — history still works in memory, it just is not saved.
fn history_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".gg_history"))
}

/// The lines typed for the chunk being entered. A multi-line block
/// is one history entry, not one per line.
struct PendingChunk {
    lines: Vec<String>,
}

impl PendingChunk {
    fn new() -> Self {
        Self { lines: Vec::new() }
    }

    fn add(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }

    fn clear(&mut self) {
        self.lines.clear();
    }

    /// Take the accumulated lines as one history entry (joined with
    /// newlines), or `None` when nothing but blank lines was typed.
    fn finish(&mut self) -> Option<String> {
        let text = self.lines.join("\n");
        self.lines.clear();
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_joins_a_multi_line_block_as_one_entry() {
        let mut pending = PendingChunk::new();
        pending.add("if true then");
        pending.add("print \"x\"");
        pending.add("end");
        assert_eq!(
            pending.finish(),
            Some("if true then\nprint \"x\"\nend".to_string())
        );
        // After finishing, the accumulator is empty again.
        assert_eq!(pending.finish(), None);
    }

    #[test]
    fn finish_skips_blank_lines() {
        let mut pending = PendingChunk::new();
        pending.add("");
        pending.add("   ");
        assert_eq!(pending.finish(), None);
    }

    #[test]
    fn clear_discards_an_aborted_block() {
        let mut pending = PendingChunk::new();
        pending.add("if true then");
        pending.clear();
        assert_eq!(pending.finish(), None);
    }

    #[test]
    fn history_path_is_under_home() {
        // HOME is set in the test environment; the path is just
        // `$HOME/.gg_history`.
        let path = history_path().expect("HOME is set");
        assert_eq!(path.file_name().unwrap(), ".gg_history");
    }
}
