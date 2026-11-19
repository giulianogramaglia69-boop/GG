// SPDX-License-Identifier: GPL-3.0-or-later
//! `gg` — the GG programming language.
//!
//! Phase 2: the lexer (`gg --tokens file.gg`) and the parser
//! (`gg --ast file.gg`) are available. The interpreter arrives
//! in later phases.

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!("gg {VERSION} — the GG programming language (Phase 2: parser)");
    println!();
    println!("Usage: gg [options] <file.gg>");
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
        _ => {
            eprintln!("Usage: gg --tokens <file.gg> | gg --ast <file.gg>");
            eprintln!("Run `gg --help` for the full usage.");
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
