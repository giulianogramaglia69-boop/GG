// SPDX-License-Identifier: GPL-3.0-or-later
//! `gg` — the GG programming language.
//!
//! Phase 8: the lexer (`gg --tokens file.gg`), the parser
//! (`gg --ast file.gg`), the interpreter (`gg run file.gg`, with
//! builtins and `input` from stdin) and the REPL (`gg` with no
//! arguments, or `gg repl`) are available. Lists, file handling
//! and the other collections (tuples, dicts, sets, arrays, queues,
//! stacks) are in, and so is OOP: classes, methods, single
//! inheritance with `super`, and bound methods.

use std::io::Write;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!("gg {VERSION} — the GG programming language (Phase 8: OOP)");
    println!();
    println!("Usage: gg [options] <file.gg>");
    println!();
    println!("Commands:");
    println!("  run <file.gg>          run a .gg program (stdin is available to `input`)");
    println!("  repl                   start the interactive REPL (also the default: bare `gg`)");
    println!();
    println!("Options:");
    println!("  --tokens <file.gg>   print the tokens of a .gg file, one per line");
    println!("  --ast <file.gg>      parse a .gg file and print its AST (canonical form)");
    println!("  --help, -h           show this help");
    println!("  --version, -V        show the version");
    println!();
    println!("Token output format: <line>:<col> TYPE value");
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => start_repl(),
        [arg] if arg == "repl" => start_repl(),
        [arg] if arg == "--help" || arg == "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        [arg] if arg == "--version" || arg == "-V" => {
            println!("gg {VERSION}");
            ExitCode::SUCCESS
        }
        [flag, file] if flag == "--tokens" => dump_tokens(file),
        [flag, file] if flag == "--ast" => dump_ast(file),
        [cmd, file] if cmd == "run" => run_file(file),
        _ => {
            eprintln!("Usage: gg [repl | run <file.gg> | --tokens <file.gg> | --ast <file.gg>]");
            eprintln!("Run `gg --help` for the full usage.");
            ExitCode::FAILURE
        }
    }
}

fn start_repl() -> ExitCode {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match gg_interp::run_repl(&mut input, &mut out) {
        Ok(()) => {
            let _ = out.flush();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_file(path: &str) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("error: cannot read '{path}': {e}");
            return ExitCode::FAILURE;
        }
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match gg_interp::run_source(&source, &mut input, &mut out) {
        Ok(()) => {
            // Flush stdout so piped output is not lost.
            let _ = out.flush();
            ExitCode::SUCCESS
        }
        Err(gg_interp::GgError::Lex(e)) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
        Err(gg_interp::GgError::Parse(e)) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
        Err(gg_interp::GgError::Runtime(e)) => {
            eprintln!("runtime error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dump_tokens(path: &str) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("error: cannot read '{path}': {e}");
            return ExitCode::FAILURE;
        }
    };
    match gg_lexer::lex(&source) {
        Ok(tokens) => {
            for token in tokens {
                let value = token.value();
                if value.is_empty() {
                    println!("{}:{} {}", token.line, token.col, token.type_name());
                } else {
                    println!(
                        "{}:{} {} {}",
                        token.line,
                        token.col,
                        token.type_name(),
                        value
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dump_ast(path: &str) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("error: cannot read '{path}': {e}");
            return ExitCode::FAILURE;
        }
    };
    let tokens = match gg_lexer::lex(&source) {
        Ok(tokens) => tokens,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match gg_parser::parse(&tokens) {
        Ok(program) => {
            print!("{program}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
