//! Writes `results.json` from the estimates of the last `compare` bench run.
//!
//! ```sh
//! cargo run -p evalite-compare -- [<criterion dir>] [<output>]
//! ```
//!
//! Defaults to `target/criterion` and `crates/evalite-compare/results.json`,
//! run from the workspace root.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

/// The snapshot the playground renders.
#[derive(Serialize)]
struct Snapshot {
    /// When the snapshot was written, RFC 3339 in UTC.
    measured_at: String,
    machine: Machine,
    engines: Vec<Engine>,
    expressions: Vec<Expression>,
}

#[derive(Serialize)]
struct Machine {
    cpu: String,
    os: String,
}

#[derive(Serialize)]
struct Engine {
    name: &'static str,
    version: String,
}

#[derive(Serialize)]
struct Expression {
    name: &'static str,
    description: &'static str,
    /// One per engine, in `engines` order.
    results: Vec<Measurement>,
}

#[derive(Serialize)]
struct Measurement {
    engine: &'static str,
    source: String,
    /// The median time to compile, in nanoseconds.
    compile_ns: f64,
    /// The median time to evaluate a compiled expression, in nanoseconds.
    evaluate_ns: f64,
}

/// The part of criterion's `estimates.json` read.
#[derive(Deserialize)]
struct Estimates {
    median: Estimate,
}

#[derive(Deserialize)]
struct Estimate {
    point_estimate: f64,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let criterion = args
        .next()
        .map_or_else(|| PathBuf::from("target/criterion"), PathBuf::from);
    let output = args.next().map_or_else(
        || PathBuf::from("crates/evalite-compare/results.json"),
        PathBuf::from,
    );

    let mut expressions = Vec::new();

    for case in evalite_compare::cases() {
        let mut results = Vec::new();

        for engine in evalite_compare::ENGINES {
            let source = case
                .source(engine)
                .ok_or_else(|| format!("{}: no source for {engine}", case.name))?;

            results.push(Measurement {
                engine,
                source: source.to_owned(),
                compile_ns: median(&criterion, case.name, engine, "compile")?,
                evaluate_ns: median(&criterion, case.name, engine, "evaluate")?,
            });
        }

        expressions.push(Expression {
            name: case.name,
            description: case.description,
            results,
        });
    }

    let lock = fs::read_to_string("Cargo.lock").map_err(|e| format!("Cargo.lock: {e}"))?;

    let engines = evalite_compare::ENGINES
        .iter()
        .map(|name| {
            Ok(Engine {
                name,
                version: locked_version(&lock, name)
                    .ok_or_else(|| format!("{name} is not in Cargo.lock"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let snapshot = Snapshot {
        measured_at: jiff::Timestamp::now()
            .round(jiff::Unit::Second)?
            .to_string(),
        machine: Machine {
            cpu: cpu(),
            os: os(),
        },
        engines,
        expressions,
    };

    let mut json = serde_json::to_string_pretty(&snapshot)?;
    json.push('\n');
    fs::write(&output, json).map_err(|e| format!("{}: {e}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

/// The median of `compare/<case>/<engine>/<stage>`, in nanoseconds.
fn median(criterion: &Path, case: &str, engine: &str, stage: &str) -> Result<f64> {
    // criterion names a group's directory after its id, `/` replaced by `_`.
    let path = criterion
        .join(format!("compare_{case}_{engine}"))
        .join(stage)
        .join("new")
        .join("estimates.json");

    let data = fs::read(&path).map_err(|e| {
        format!(
            "{}: {e}; run `cargo bench -p evalite-compare` first",
            path.display()
        )
    })?;

    let estimates: Estimates =
        serde_json::from_slice(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(estimates.median.point_estimate)
}

/// The version of `name` in `Cargo.lock`.
fn locked_version(lock: &str, name: &str) -> Option<String> {
    let needle = format!("name = \"{name}\"");
    let mut lines = lock.lines();
    lines.find(|line| *line == needle)?;
    let line = lines.next()?;
    let version = line.strip_prefix("version = \"")?.strip_suffix('"')?;
    Some(version.to_owned())
}

/// The CPU's model name, from `/proc/cpuinfo` where there is one.
fn cpu() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find_map(|line| line.strip_prefix("model name"))
                .and_then(|rest| rest.split_once(':'))
                .map(|(_, name)| name.trim().to_owned())
        })
        .unwrap_or_else(|| std::env::consts::ARCH.to_owned())
}

/// The operating system, its release where known, and the architecture.
fn os() -> String {
    let name = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|release| {
            release
                .lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|name| name.trim_matches('"').to_owned())
        });

    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|k| k.trim().to_owned());

    let mut out = name.unwrap_or_else(|| std::env::consts::OS.to_owned());

    if let Some(kernel) = kernel {
        out.push_str(&format!(" (kernel {kernel})"));
    }

    out.push_str(&format!(", {}", std::env::consts::ARCH));
    out
}
