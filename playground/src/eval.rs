//! Evaluating the editor's expression against the page's variables.

use std::rc::Rc;

use evalite::{
    DEFAULT_FUEL, DEFAULT_MAX_DEPTH, DEFAULT_MAX_LEN, DEFAULT_MEMORY, Diagnostics, Environment,
    Labels, Limits, Machine, Options, RunError, Type, Value,
};

/// The machine's bounds, as the page sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub fuel: u64,
    pub memory: u32,
    pub depth: usize,
    pub len: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_FUEL,
            memory: DEFAULT_MEMORY,
            depth: DEFAULT_MAX_DEPTH,
            len: DEFAULT_MAX_LEN,
        }
    }
}

/// A variable the expression may read.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub name: String,
    pub ty: Type,
    pub value: Value,
}

/// What a successful run gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub value: String,
    pub ty: String,
    pub steps: u64,
    pub memory: u64,
}

/// One label of a run error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub message: String,
    pub primary: bool,
}

/// Why an expression gave no value.
#[derive(Clone, Debug, PartialEq)]
pub enum Failed {
    /// It does not compile.
    Compile(Rc<Diagnostics>),
    /// A variable could not be declared, or the run failed.
    Run(Vec<Mark>),
}

impl Bounds {
    pub fn options(self) -> Options {
        Options::new().max_depth(self.depth).max_len(self.len)
    }
}

/// Compile and run `source` with `bindings` declared and bound.
pub fn evaluate(source: &str, bindings: &[Binding], bounds: Bounds) -> Result<Done, Failed> {
    let env = environment(bindings).map_err(Failed::Run)?;

    let (program, debug) = evalite::compile_debug(source, &env, &bounds.options())
        .map_err(|errors| Failed::Compile(Rc::new(errors)))?;

    let mut machine = Machine::new(Limits::new().memory(bounds.memory).fuel(bounds.fuel));
    let failed = |error: RunError| Failed::Run(marks(error.labels(&program, Some(&debug))));
    let mut run = machine.load(&program).map_err(failed)?;

    let read = run
        .globals()
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();

    for binding in bindings {
        if read.contains(&binding.name.as_str()) {
            run.bind(&binding.name, binding.value.clone())
                .map_err(failed)?;
        }
    }

    let outcome = run.run().map_err(failed)?;

    Ok(Done {
        value: outcome.value().to_string(),
        ty: program.result_type().to_string(),
        steps: outcome.steps(),
        memory: outcome.memory(),
    })
}

/// The builtins, with `bindings` declared.
pub fn environment(bindings: &[Binding]) -> Result<Environment, Vec<Mark>> {
    let mut env = Environment::with_builtins();

    for binding in bindings {
        env.value(&binding.name, binding.ty.clone())
            .map_err(|error| {
                vec![Mark {
                    message: format!("`{}`: {error}", binding.name),
                    primary: true,
                }]
            })?;
    }

    Ok(env)
}

fn marks(labels: Labels<'_>) -> Vec<Mark> {
    labels
        .map(|label| Mark {
            message: label.message().to_string(),
            primary: label.is_primary(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(name: &str, value: i64) -> Binding {
        Binding {
            name: name.into(),
            ty: Type::int(),
            value: Value::int(value),
        }
    }

    #[test]
    fn runs() {
        let done = evaluate("a + b * 2", &[int("a", 1), int("b", 3)], Bounds::default()).unwrap();
        assert_eq!(done.value, "7");
        assert_eq!(done.ty, "Int");
        assert!(done.steps > 0);
    }

    #[test]
    fn type_errors_are_diagnostics() {
        let failed = evaluate("1 + true", &[], Bounds::default()).unwrap_err();
        assert!(matches!(failed, Failed::Compile(_)), "{failed:?}");
    }

    #[test]
    fn runs_out_of_fuel() {
        let bounds = Bounds {
            fuel: 3,
            ..Bounds::default()
        };
        let failed = evaluate("1 + 2 + 3 + 4 + 5", &[], bounds).unwrap_err();
        let Failed::Run(marks) = failed else {
            panic!("{failed:?}");
        };
        assert!(marks.iter().any(|m| m.primary));
    }
}
