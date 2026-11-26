//! `shexel`, a command line evaluator to try the sheet expression language.
//!
//! Variables are declared and bound on the command line as
//! `--var name:Type=value`, where a type is `Int`, `Bool`, `String`,
//! `Option<T>` or `List<T>`, and a value is `3`, `true`, `"text"`, `none`,
//! `some(2)` or `[1, 2]`.
//!
//! ```text
//! $ shexel eval 'strength + 2' --var strength:Int=3
//! 5 : Int
//!
//! $ shexel eval 'bonus.unwrap_or(0) + 1' --var bonus:Option<Int>=none
//! 1 : Int
//!
//! $ shexel --var strength:Int=3          # a loop over stdin, one expression per line
//! > strength * 2
//! 6 : Int
//! ```
//!
//! Limits are `--max-depth` (nesting), `--memory` (64-bit words) and `--fuel`
//! (steps); `--explain` also prints the type, memory and steps. Errors
//! print the expression with a caret under what is wrong and exit non-zero.
//! With no expression, `shexel` reads expressions from stdin until the end.

mod vars;

use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use shexel::{
    DEFAULT_FUEL, DEFAULT_MAX_DEPTH, DEFAULT_MEMORY, Environment, Limits, Machine, Options,
};

use crate::vars::{Var, VarError, parse_var};

#[derive(Parser)]
#[command(name = "shexel", version, about = "Try sheet expressions")]
struct Cli {
    /// Declare and bind a variable, as `name:Type=value`.
    #[arg(long = "var", value_name = "NAME:TYPE=VALUE", global = true, value_parser = parse_var)]
    vars: Vec<Var>,
    /// The deepest an expression may nest.
    #[arg(long, default_value_t = DEFAULT_MAX_DEPTH, global = true)]
    max_depth: usize,
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
    let mut env = Environment::with_builtins();

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
                eprint!("{}", failure.render(source));
                false
            }
        }
    }

    fn try_eval(&self, source: &str) -> Result<String, Failure> {
        let options = Options::new().max_depth(self.cli.max_depth);
        let program =
            shexel::compile_with(source, &self.env, &options).map_err(Failure::Compile)?;
        let mut machine = Machine::new(Limits::new().memory(self.cli.memory).fuel(self.cli.fuel));
        let mut run = machine.load(&program).map_err(Failure::Run)?;

        // Variables the expression does not read are not bound.
        let read = run
            .globals()
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>();

        for var in &self.cli.vars {
            if read.contains(&var.name.as_str()) {
                run.bind(&var.name, var.value.clone())
                    .map_err(Failure::Run)?;
            }
        }

        let outcome = run.run().map_err(Failure::Run)?;
        let ty = program.result_type();
        let mut out = format!("{} : {ty}", outcome.value);

        if self.cli.explain {
            let memory = program.memory();
            out.push_str(&format!(
                "\ntype: {ty}\nmemory: {} words at most (stack {}, heap {}, constants {}, values read {}), {} used\nsteps: {}",
                memory.total(),
                memory.stack,
                memory.heap,
                memory.constants,
                memory.globals,
                outcome.memory,
                outcome.steps,
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

enum Failure {
    Compile(shexel::Error),
    Run(shexel::RunError),
}

impl Failure {
    /// The error as text: for a compile error, the expression with a caret
    /// under the span; runtime errors carry no span.
    fn render(&self, source: &str) -> String {
        match self {
            Failure::Run(error) => format!("error: {error}\n"),
            Failure::Compile(error) => {
                let span = error.span();
                let start = usize::try_from(span.start).unwrap_or(usize::MAX);
                let end = usize::try_from(span.end).unwrap_or(usize::MAX);
                let before = source.get(..start).unwrap_or(source);
                let column = before.chars().count();
                let width = source
                    .get(start..end)
                    .map_or(1, |s| s.chars().count())
                    .max(1);

                format!(
                    "error: {error}\n  | {source}\n  | {}{}\n",
                    " ".repeat(column),
                    "^".repeat(width)
                )
            }
        }
    }
}

impl From<String> for VarError {
    fn from(message: String) -> Self {
        VarError::new(message)
    }
}
