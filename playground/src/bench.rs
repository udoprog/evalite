//! The benchmark snapshot `crates/evalite-compare` writes, embedded at build
//! time.

use serde::Deserialize;

const RESULTS: &str = include_str!("../../crates/evalite-compare/results.json");

/// How the snapshot was measured, for the reader.
pub const README: &str =
    "https://github.com/udoprog/evalite/blob/main/crates/evalite-compare/README.md";

#[derive(Debug, Deserialize)]
pub struct Results {
    pub measured_at: String,
    pub machine: Machine,
    pub engines: Vec<Engine>,
    pub expressions: Vec<Expression>,
}

#[derive(Debug, Deserialize)]
pub struct Machine {
    pub cpu: String,
    pub os: String,
}

#[derive(Debug, Deserialize)]
pub struct Engine {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct Expression {
    pub name: String,
    pub description: String,
    pub results: Vec<Measured>,
}

#[derive(Debug, Deserialize)]
pub struct Measured {
    pub engine: String,
    pub source: String,
    pub compile_ns: f64,
    pub evaluate_ns: f64,
}

/// Which stage of an engine's work is shown.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Compile,
    Evaluate,
}

impl Stage {
    pub fn of(self, measured: &Measured) -> f64 {
        match self {
            Stage::Compile => measured.compile_ns,
            Stage::Evaluate => measured.evaluate_ns,
        }
    }
}

/// The snapshot, or nothing if it does not parse.
pub fn results() -> Option<Results> {
    serde_json::from_str(RESULTS).ok()
}

/// The date part of an RFC 3339 time.
pub fn date(measured_at: &str) -> &str {
    measured_at.split('T').next().unwrap_or(measured_at)
}

/// A duration in nanoseconds, to three significant figures.
pub fn duration(ns: f64) -> String {
    let (value, unit) = if ns >= 1_000_000.0 {
        (ns / 1_000_000.0, "ms")
    } else if ns >= 1_000.0 {
        (ns / 1_000.0, "µs")
    } else {
        return format!("{ns:.0} ns");
    };

    if value >= 100.0 {
        format!("{value:.0} {unit}")
    } else if value >= 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.2} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_parses() {
        let results = results().expect("results.json parses");
        assert!(!results.expressions.is_empty());

        for expression in &results.expressions {
            for measured in &expression.results {
                assert!(
                    results.engines.iter().any(|e| e.name == measured.engine),
                    "{}: unknown engine {}",
                    expression.name,
                    measured.engine
                );
            }
        }
    }

    #[test]
    fn durations() {
        assert_eq!(duration(141.4), "141 ns");
        assert_eq!(duration(2178.3), "2.18 µs");
        assert_eq!(duration(24482.0), "24.5 µs");
        assert_eq!(duration(240_000.0), "240 µs");
        assert_eq!(duration(2_500_000.0), "2.50 ms");
    }
}
