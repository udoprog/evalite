//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/evalite-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/evalite)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/evalite-cli.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/evalite-cli)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-evalite--cli-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/evalite-cli)
//! `evalite`, the command line for [evalite]: small, strictly typed
//! expressions that are safe to run on anyone's input.
//!
//! Variables are declared and bound with `--var name:Type=value`, where a
//! type is `Int`, `Bool`, `String`, `Option<T>` or `List<T>`, and a value is
//! `3`, `true`, `"text"`, `none`, `some(2)` or `[1, 2]`.
//!
//! ```text
//! $ evalite eval 'Int::min(wits, dexterity) + bonus.unwrap_or(0)' --var wits:Int=3 --var dexterity:Int=2 --var 'bonus:Option<Int>=some(1)'
//! 3 : Int
//!
//! $ evalite eval 'skills[3] * 2' --var 'skills:List<Int>=[1, 2]'
//! error: index 3 is out of range of a list of 2 entries
//!   | skills[3] * 2
//!   | ^^^^^^^^^
//!   = while running instruction 2
//!   = while running the program
//!
//! $ evalite --var strength:Int=3          # one expression per line from stdin
//! > strength * 2
//! 6 : Int
//! ```
//!
//! `--max-depth` (nesting), `--max-len` (bytes), `--memory` (64-bit words) and `--fuel` (steps)
//! set the limits, and `--explain` also prints the type, memory and steps.
//! An error prints the expression with a caret under what is wrong, then
//! what was being done, and exits non-zero.
//!
//! [evalite]: https://docs.rs/evalite

mod vars;

use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use evalite::{
    DEFAULT_FUEL, DEFAULT_MAX_DEPTH, DEFAULT_MAX_LEN, DEFAULT_MEMORY, Environment, Labels, Limits,
    Machine, Options, RunError, Span,
};

use crate::vars::{Var, VarError, parse_var};

/// Shown after the options in `--help`.
const AFTER_HELP: &str = "\
Variables:
  A type is Int, Bool, String, Option<T> or List<T>, and a value is 3, true,
  \"text\", none, some(2) or [1, 2].

Examples:
  evalite eval 'strength + 2' --var strength:Int=3
  evalite eval 'bonus.unwrap_or(0) + 1' --var 'bonus:Option<Int>=none'
  evalite --var strength:Int=3          # one expression per line from stdin

The language: https://docs.rs/evalite";

#[derive(Parser)]
#[command(
    name = "evalite",
    version,
    about = "Run small, strictly typed expressions, safe on anyone's input",
    after_help = AFTER_HELP
)]
struct Cli {
    /// Declare and bind a variable, as `name:Type=value`.
    #[arg(long = "var", value_name = "NAME:TYPE=VALUE", global = true, value_parser = parse_var)]
    vars: Vec<Var>,
    /// The deepest an expression may nest.
    #[arg(long, default_value_t = DEFAULT_MAX_DEPTH, global = true)]
    max_depth: usize,
    /// The longest an expression may be, in bytes.
    #[arg(long, default_value_t = DEFAULT_MAX_LEN, global = true)]
    max_len: usize,
    /// The memory the machine has, in 64-bit words.
    #[arg(long, default_value_t = DEFAULT_MEMORY, global = true)]
    memory: u32,
    /// The steps an expression may take.
    #[arg(long, default_value_t = DEFAULT_FUEL, global = true)]
    fuel: u64,
    /// Also print the type, the program's memory and the steps used.
    #[arg(long, global = true)]
    explain: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate one expression; without one, read expressions from stdin.
    Eval { expr: Option<String> },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Run the command; whether every expression evaluated.
fn run(cli: &Cli) -> Result<bool, Box<dyn std::error::Error>> {
    let mut env = Environment::new();

    for var in &cli.vars {
        env.value(&var.name, var.ty.clone())
            .map_err(|e| VarError::from(format!("`{}`: {e}", var.name)))?;
    }

    let session = Session { cli, env };

    match &cli.command {
        Some(Command::Eval { expr: Some(expr) }) => Ok(session.eval(expr)),
        _ => session.repl(),
    }
}

struct Session<'a> {
    cli: &'a Cli,
    env: Environment,
}

impl Session<'_> {
    /// Evaluate and print one expression; false when it failed.
    fn eval(&self, source: &str) -> bool {
        match self.try_eval(source) {
            Ok(output) => {
                println!("{output}");
                true
            }
            Err(failure) => {
                eprint!("{failure}");
                false
            }
        }
    }

    fn try_eval(&self, source: &str) -> Result<String, String> {
        let options = Options::new()
            .max_depth(self.cli.max_depth)
            .max_len(self.cli.max_len);
        let (program, debug) =
            evalite::compile_debug(source, &self.env, &options).map_err(|errors| {
                errors
                    .iter()
                    .map(|error| render(source, error.labels()))
                    .collect::<String>()
            })?;
        let mut machine = Machine::new(Limits::new().memory(self.cli.memory).fuel(self.cli.fuel));

        let failed = |error: RunError| render(source, error.labels(&program, Some(&debug)));

        let mut run = machine.load(&program).map_err(failed)?;

        // Variables the expression does not read are not bound.
        let read = run.globals().map(|(n, _)| n).collect::<Vec<_>>();

        for var in &self.cli.vars {
            if read.contains(&var.name.as_str()) {
                run.bind(&var.name, &var.value).map_err(failed)?;
            }
        }

        let outcome = run.run().map_err(failed)?;
        let ty = program.result_type();
        let mut out = format!("{} : {ty}", outcome.value());

        if self.cli.explain {
            let memory = program.memory();
            out.push_str(&format!(
                "\ntype: {ty}\nmemory: {} words at most (stack {}, heap {}, constants {}, values read {}), {} used\nsteps: {}",
                memory.total(),
                memory.stack(),
                memory.heap(),
                memory.constants(),
                memory.globals(),
                outcome.memory(),
                outcome.steps(),
            ));
        }

        Ok(out)
    }

    /// Evaluate stdin line by line; false when any line failed.
    fn repl(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let stdin = io::stdin();
        let tty = stdin.is_terminal();
        let mut ok = true;
        let mut out = io::stdout();

        loop {
            if tty {
                write!(out, "> ")?;
                out.flush()?;
            }

            let mut line = String::new();

            if stdin.lock().read_line(&mut line)? == 0 {
                break;
            }

            let line = line.trim();

            if !line.is_empty() && !self.eval(line) {
                ok = false;
            }
        }

        // A person at a terminal has seen each error; a script has not.
        Ok(ok || tty)
    }
}

/// An error as text, from its labels: the primary message, the
/// expression with a caret under the primary span, then every other label,
/// with a caret line of its own when it points elsewhere.
fn render(source: &str, labels: Labels<'_>) -> String {
    let mut out = String::new();
    let mut context = Vec::new();
    let mut shown = None;

    for label in labels {
        let (message, span) = (label.message().to_string(), label.span());

        if label.is_primary() {
            shown = span;

            out.push_str(&format!("error: {message}\n"));

            if let Some(span) = span {
                out.push_str(&caret(source, span));
            }
        } else {
            context.push((message, span));
        }
    }

    for (message, span) in context {
        out.push_str(&format!("  = {message}\n"));

        if let Some(span) = span.filter(|span| Some(*span) != shown) {
            out.push_str(&caret(source, span));
        }
    }

    out
}

/// `source` with a caret line under `span`.
fn caret(source: &str, span: Span<u32>) -> String {
    let start = usize::try_from(span.start).unwrap_or(usize::MAX);
    let end = usize::try_from(span.end).unwrap_or(usize::MAX);
    let before = source.get(..start).unwrap_or(source);
    let column = before.chars().count();
    let width = source
        .get(start..end)
        .map_or(1, |s| s.chars().count())
        .max(1);

    format!(
        "  | {source}\n  | {}{}\n",
        " ".repeat(column),
        "^".repeat(width)
    )
}

impl From<String> for VarError {
    fn from(message: String) -> Self {
        VarError::new(message)
    }
}
