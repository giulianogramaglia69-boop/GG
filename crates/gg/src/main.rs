// SPDX-License-Identifier: GPL-3.0-or-later
//! `gg` — the GG programming language.
//!
//! Phase 9: the lexer (`gg --tokens file.gg`), the parser
//! (`gg --ast file.gg`), the interpreter (`gg run file.gg`, with
//! builtins and `input` from stdin) and the REPL (`gg` with no
//! arguments, or `gg repl`) are available. Lists, file handling
//! and the other collections (tuples, dicts, sets, arrays, queues,
//! stacks) are in, and so is OOP: classes, methods, single
//! inheritance with `super`, and bound methods. Modules (`import`)
//! and virtual environments (`gg venv`, `gg run --venv <dir>`,
//! `GG_VENV`) are in too.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!("gg {VERSION} — the GG programming language (Phase 9: modules + venv)");
    println!();
    println!("Usage: gg [options] <file.gg>");
    println!();
    println!("Commands:");
    println!(
        "  run [--venv <dir>]... <file.gg>  run a .gg program (stdin is available to `input`)"
    );
    println!(
        "  repl                              start the interactive REPL (also the default: bare `gg`)"
    );
    println!(
        "  venv <name>                       create a virtual environment <name>/ with lib/ and gg.toml"
    );
    println!();
    println!("Options:");
    println!("  --tokens <file.gg>   print the tokens of a .gg file, one per line");
    println!("  --ast <file.gg>      parse a .gg file and print its AST (canonical form)");
    println!("  --help, -h           show this help");
    println!("  --version, -V        show the version");
    println!();
    println!("Modules: `import math` loads math.gg from the script's directory,");
    println!("then the venv lib/ directories (--venv, or GG_VENV), then the working");
    println!("directory. Each module runs once per process.");
    println!();
    println!("Token output format: <line>:<col> TYPE value");
}

/// One parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Repl,
    Run {
        file: String,
        venv_dirs: Vec<PathBuf>,
    },
    Tokens {
        file: String,
    },
    Ast {
        file: String,
    },
    Venv {
        name: String,
    },
    Help,
    Version,
}

/// Parse the command line (without the program name) into a
/// `Command`, or an error message for `main` to print.
fn parse_args(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Repl),
        [arg] if arg == "repl" => Ok(Command::Repl),
        [arg] if arg == "--help" || arg == "-h" => Ok(Command::Help),
        [arg] if arg == "--version" || arg == "-V" => Ok(Command::Version),
        [flag, file] if flag == "--tokens" => Ok(Command::Tokens {
            file: file.clone(),
        }),
        [flag, file] if flag == "--ast" => Ok(Command::Ast {
            file: file.clone(),
        }),
        [cmd, name] if cmd == "venv" => Ok(Command::Venv {
            name: name.clone(),
        }),
        [cmd] if cmd == "venv" => Err("gg venv needs a name: gg venv <name>".to_string()),
        [cmd, ..] if cmd == "venv" => {
            Err("gg venv takes exactly one name: gg venv <name>".to_string())
        }
        [cmd, rest @ ..] if cmd == "run" => parse_run(rest),
        _ => Err("Usage: gg [repl | run [--venv <dir>]... <file.gg> | venv <name> | --tokens <file.gg> | --ast <file.gg>]".to_string()),
    }
}

/// Parse the arguments after `run`: `--venv <dir>` flags
/// (repeatable) and exactly one file.
fn parse_run(rest: &[String]) -> Result<Command, String> {
    let mut venv_dirs = Vec::new();
    let mut file: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--venv" => {
                i += 1;
                let Some(dir) = rest.get(i) else {
                    return Err("expected a directory after '--venv'".to_string());
                };
                venv_dirs.push(PathBuf::from(dir));
            }
            other if other.starts_with('-') => {
                return Err(format!(
                    "unknown option '{other}' (did you mean '--venv <dir>'?)"
                ));
            }
            other => {
                if file.is_some() {
                    return Err("gg run takes exactly one file".to_string());
                }
                file = Some(other.to_string());
            }
        }
        i += 1;
    }
    match file {
        Some(file) => Ok(Command::Run { file, venv_dirs }),
        None => Err("gg run needs a file: gg run [--venv <dir>] <file.gg>".to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Command::Repl) => start_repl(),
        Ok(Command::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("gg {VERSION}");
            ExitCode::SUCCESS
        }
        Ok(Command::Tokens { file }) => dump_tokens(&file),
        Ok(Command::Ast { file }) => dump_ast(&file),
        Ok(Command::Run { file, venv_dirs }) => run_file(&file, venv_dirs),
        Ok(Command::Venv { name }) => create_venv_cmd(&name),
        Err(msg) => {
            eprintln!("error: {msg}");
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
    // The REPL has no script directory: imports fall back to the
    // venv libraries (from GG_VENV) and the working directory.
    match gg_interp::run_repl_with_venv(&mut input, &mut out, gg_interp::venv_dirs_from_env()) {
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

fn run_file(path: &str, flag_venv_dirs: Vec<PathBuf>) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("error: cannot read '{path}': {e}");
            return ExitCode::FAILURE;
        }
    };
    // `--venv` wins over `GG_VENV` when both are given.
    let from_flags = !flag_venv_dirs.is_empty();
    // A `--venv` directory must exist; `GG_VENV` pointing nowhere
    // is silently ignored (the search just finds nothing there).
    if from_flags {
        for dir in &flag_venv_dirs {
            if !dir.is_dir() {
                eprintln!("error: venv directory '{}' does not exist", dir.display());
                return ExitCode::FAILURE;
            }
        }
    }
    // `--venv` takes venv roots; the interpreter searches their
    // `lib/` directories (`GG_VENV` already resolves to `lib/`).
    let venv_dirs = if from_flags {
        flag_venv_dirs
            .iter()
            .map(|d| gg_interp::venv_lib_dir(d))
            .collect()
    } else {
        gg_interp::venv_dirs_from_env()
    };
    // Imports in the script resolve against the script's directory
    // first: canonicalize the path so the base is absolute.
    let base_dir = std::fs::canonicalize(path)
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match gg_interp::run_source_with(&source, base_dir, venv_dirs, &mut input, &mut out) {
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

fn create_venv_cmd(name: &str) -> ExitCode {
    match gg_interp::create_venv(name) {
        Ok(()) => {
            println!("Created venv '{name}':");
            println!("  {name}/");
            println!("  {name}/lib/");
            println!("  {name}/gg.toml");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn parse_repl_help_and_version() {
        assert_eq!(parse_args(&args(&[])), Ok(Command::Repl));
        assert_eq!(parse_args(&args(&["repl"])), Ok(Command::Repl));
        assert_eq!(parse_args(&args(&["--help"])), Ok(Command::Help));
        assert_eq!(parse_args(&args(&["-h"])), Ok(Command::Help));
        assert_eq!(parse_args(&args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse_args(&args(&["-V"])), Ok(Command::Version));
    }

    #[test]
    fn parse_tokens_ast_and_venv() {
        assert_eq!(
            parse_args(&args(&["--tokens", "a.gg"])),
            Ok(Command::Tokens {
                file: "a.gg".to_string(),
            })
        );
        assert_eq!(
            parse_args(&args(&["--ast", "a.gg"])),
            Ok(Command::Ast {
                file: "a.gg".to_string(),
            })
        );
        assert_eq!(
            parse_args(&args(&["venv", "myproj"])),
            Ok(Command::Venv {
                name: "myproj".to_string(),
            })
        );
        assert!(parse_args(&args(&["venv"])).is_err());
        assert!(parse_args(&args(&["venv", "a", "b"])).is_err());
        assert!(parse_args(&args(&["--tokens"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
        assert!(parse_args(&args(&["file.gg"])).is_err());
    }

    #[test]
    fn parse_run_with_repeatable_venv() {
        assert_eq!(
            parse_args(&args(&["run", "main.gg"])),
            Ok(Command::Run {
                file: "main.gg".to_string(),
                venv_dirs: vec![],
            })
        );
        assert_eq!(
            parse_args(&args(&["run", "--venv", "a", "--venv", "b", "main.gg"])),
            Ok(Command::Run {
                file: "main.gg".to_string(),
                venv_dirs: vec![PathBuf::from("a"), PathBuf::from("b")],
            })
        );
        // Flags may come after the file too.
        assert_eq!(
            parse_args(&args(&["run", "main.gg", "--venv", "a"])),
            Ok(Command::Run {
                file: "main.gg".to_string(),
                venv_dirs: vec![PathBuf::from("a")],
            })
        );
        assert!(parse_args(&args(&["run"])).is_err());
        assert!(parse_args(&args(&["run", "--venv"])).is_err());
        assert!(parse_args(&args(&["run", "a.gg", "b.gg"])).is_err());
        assert!(parse_args(&args(&["run", "--bogus", "a.gg"])).is_err());
        assert!(parse_args(&args(&["run", "--venv", "a"])).is_err());
    }
}
